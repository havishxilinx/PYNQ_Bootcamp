use crate::messages::RefereeMessage;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Points awarded/deducted based on how long it took the active team to
/// act since being handed the turn (or since their previous action, if a
/// streak is continuing -- the clock resets on every action). Boundaries
/// and bonuses come from `data/game_config.json` (see `config::GameConfig`)
/// so they can be tuned without a rebuild. A full timeout uses the same
/// catch-all bonus as the slowest acted-tier -- no special-casing needed,
/// one function covers both (see `check_timeout`, which always calls this
/// with an elapsed value >= the configured turn timeout).
fn response_tier_bonus(elapsed: Duration) -> i32 {
    crate::config::get().tier_bonus(elapsed)
}

/// What happened as a result of processing a `report_result` message.
#[derive(Debug, Clone, PartialEq)]
pub enum ResultOutcome {
    CorrectMatch {
        messages: Vec<(String, RefereeMessage)>,
    },
    WrongMatch {
        messages: Vec<(String, RefereeMessage)>,
    },
    NoClaim {
        messages: Vec<(String, RefereeMessage)>,
    },
    GameOver {
        winner: String,
        messages: Vec<(String, RefereeMessage)>,
    },
}

impl ResultOutcome {
    pub fn into_messages(self) -> Vec<(String, RefereeMessage)> {
        match self {
            ResultOutcome::CorrectMatch { messages }
            | ResultOutcome::WrongMatch { messages }
            | ResultOutcome::NoClaim { messages }
            | ResultOutcome::GameOver { messages, .. } => messages,
        }
    }
}

/// What happened as a result of processing a `hint_request` message.
#[derive(Debug, Clone, PartialEq)]
pub enum HintOutcome {
    Accepted {
        riddle: String,
        messages: Vec<(String, RefereeMessage)>,
    },
    Rejected {
        reason: String,
        messages: Vec<(String, RefereeMessage)>,
    },
}

impl HintOutcome {
    pub fn into_messages(self) -> Vec<(String, RefereeMessage)> {
        match self {
            HintOutcome::Accepted { messages, .. } | HintOutcome::Rejected { messages, .. } => {
                messages
            }
        }
    }
}

pub struct GameState {
    teams: Vec<(String, String)>, // (team_name, board_id), in turn order
    grid: HashMap<String, String>,
    scores: HashMap<String, i32>,
    hints_used: HashMap<String, u32>,
    pairs_matched_by_team: HashMap<String, u32>,
    matched: HashSet<String>,
    revealed: HashSet<String>,
    active_idx: usize,
    streak: u32,
    /// The first card of the current pair -- `flip1.is_some()` is the
    /// sentinel for whether a pair is at least half-revealed this turn.
    flip1: Option<String>,
    /// The second card of the current pair, once revealed -- only ever
    /// set alongside `flip_revealed`, used for the "flip the card
    /// now" scoreboard banner (see `flip_pending_positions`).
    flip2: Option<String>,
    /// Set once the pair's second card is revealed (whether via a second
    /// `receive_flip` call or one `receive_flip_both` call) -- the sentinel
    /// `flip_pending_positions` uses to show the "flip the card now" banner.
    flip_revealed: bool,
    turn_start: Instant,
    /// Absolute wall-clock timestamp (Unix ms) the match started, purely
    /// for the scoreboard's cosmetic "time elapsed" display -- unlike
    /// `turn_start`, this never resets and has no gameplay effect (no
    /// overall match time limit exists).
    match_started_at_unix_ms: u64,
    /// `Some(remaining)` while an operator has paused the match -- freezes
    /// `turn_seconds_remaining` at this value instead of computing it from
    /// `turn_start`, and blocks new student actions from being processed
    /// (see `arena.rs`'s admin-command handling). `None` while running.
    paused_remaining: Option<Duration>,
}

impl GameState {
    pub fn new(teams: Vec<(String, String)>, grid: HashMap<String, String>) -> Self {
        let scores = teams.iter().map(|(name, _)| (name.clone(), 0)).collect();
        let hints_used = teams.iter().map(|(name, _)| (name.clone(), 0)).collect();
        let pairs_matched_by_team = teams.iter().map(|(name, _)| (name.clone(), 0)).collect();
        let match_started_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_millis() as u64;
        GameState {
            teams,
            grid,
            scores,
            hints_used,
            pairs_matched_by_team,
            matched: HashSet::new(),
            revealed: HashSet::new(),
            active_idx: 0,
            streak: 0,
            flip1: None,
            flip2: None,
            flip_revealed: false,
            turn_start: Instant::now(),
            match_started_at_unix_ms,
            paused_remaining: None,
        }
    }

    pub fn match_started_at_unix_ms(&self) -> u64 {
        self.match_started_at_unix_ms
    }

    pub fn active_team(&self) -> &str {
        &self.teams[self.active_idx].0
    }

    pub fn active_id(&self) -> &str {
        &self.teams[self.active_idx].1
    }

    pub fn total_pairs(&self) -> usize {
        self.grid.len() / 2
    }

    pub fn pairs_found(&self) -> usize {
        self.matched.len() / 2
    }

    pub fn all_pairs_found(&self) -> bool {
        self.pairs_found() == self.total_pairs()
    }

    /// The team with the highest score, tie-broken deterministically by
    /// `self.teams` order (last team wins ties) rather than by iterating
    /// `self.scores` (a `HashMap` whose order is randomized per process).
    /// Both the student-facing `game_over` message and the Master-facing
    /// `match_result` must call this same method — computing the winner
    /// independently in each place let them disagree on a tied match,
    /// telling students one team won while crediting the other as the
    /// tournament's pool winner.
    pub fn winner(&self) -> String {
        self.teams
            .iter()
            .max_by_key(|(name, _)| self.scores[name])
            .expect("teams is non-empty: GameState always has at least one team")
            .0
            .clone()
    }

    pub fn scores(&self) -> HashMap<String, i32> {
        self.scores.clone()
    }

    /// Operator override -- sets `team`'s score to an absolute value,
    /// overwriting whatever it currently is. A no-op if `team` isn't in
    /// this match (defends against a stale/mistyped admin command rather
    /// than panicking mid-match).
    pub fn set_score(&mut self, team: &str, value: i32) {
        if let Some(score) = self.scores.get_mut(team) {
            *score = value;
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused_remaining.is_some()
    }

    /// Freezes the turn timer at its current remaining value. A no-op if
    /// already paused (repeated Pause commands don't reset the frozen
    /// value to whatever's left at the time of the second call).
    pub fn pause(&mut self, now: Instant) {
        if self.paused_remaining.is_none() {
            self.paused_remaining = Some(self.turn_time_remaining_duration(now));
        }
    }

    /// Unfreezes the turn timer, resuming with exactly the time that was
    /// left when paused -- shifts `turn_start` forward by however long the
    /// real-world pause lasted, rather than losing that grace period.
    pub fn resume(&mut self, now: Instant) {
        if let Some(remaining) = self.paused_remaining.take() {
            self.turn_start = now
                - crate::config::get()
                    .turn_timeout()
                    .saturating_sub(remaining);
        }
    }

    fn turn_time_remaining_duration(&self, now: Instant) -> Duration {
        let elapsed = now.duration_since(self.turn_start);
        crate::config::get().turn_timeout().saturating_sub(elapsed)
    }

    pub fn is_revealed(&self, pos: &str) -> bool {
        self.revealed.contains(pos)
    }

    /// All confirmed-matched positions and the object found there —
    /// safe to expose publicly since a matched pair is already common
    /// knowledge to both teams via the shared-visibility rule.
    pub fn matched_positions(&self) -> HashMap<String, String> {
        self.matched
            .iter()
            .filter_map(|pos| self.grid.get(pos).map(|obj| (pos.clone(), obj.clone())))
            .collect()
    }

    /// Every position label on the board, sorted for a deterministic
    /// render order. Safe to expose publicly — it's just the grid's shape
    /// (e.g. "A1".."E6"), not the golden answer key.
    pub fn all_positions(&self) -> Vec<String> {
        let mut positions: Vec<String> = self.grid.keys().cloned().collect();
        positions.sort();
        positions
    }

    pub fn current_streak(&self) -> u32 {
        self.streak
    }

    /// Seconds left in the active team's turn, floored at 0 once the
    /// timeout has passed (doesn't go negative).
    pub fn turn_seconds_remaining(&self, now: Instant) -> u64 {
        if let Some(remaining) = self.paused_remaining {
            return remaining.as_secs();
        }
        self.turn_time_remaining_duration(now).as_secs()
    }

    pub fn hints_used_map(&self) -> HashMap<String, u32> {
        self.hints_used.clone()
    }

    pub fn pairs_matched_by_team(&self) -> HashMap<String, u32> {
        self.pairs_matched_by_team.clone()
    }

    fn next_team(&mut self) {
        self.active_idx = (self.active_idx + 1) % self.teams.len();
        self.streak = 0;
        self.flip1 = None;
        self.flip2 = None;
        self.flip_revealed = false;
        self.turn_start = Instant::now();
    }

    fn restart_turn_same_team(&mut self) {
        self.flip1 = None;
        self.flip2 = None;
        self.flip_revealed = false;
        self.turn_start = Instant::now();
    }

    fn push_turn_signals(&self, messages: &mut Vec<(String, RefereeMessage)>) {
        let active_id = self.active_id().to_string();
        for (_, id) in &self.teams {
            let msg = if *id == active_id {
                RefereeMessage::YourTurn { flip_num: 1 }
            } else {
                RefereeMessage::Wait {
                    active_team: self.active_team().to_string(),
                }
            };
            messages.push((id.clone(), msg));
        }
    }

    /// Messages to send once, right after game_start, to kick off turn 1.
    pub fn push_initial_turn_signals(&self) -> Vec<(String, RefereeMessage)> {
        let mut messages = vec![];
        self.push_turn_signals(&mut messages);
        messages
    }

    /// Handle a `flip` request from `team` for `pos`. Returns a
    /// `card_revealed` broadcast to everyone on success, or an `invalid`
    /// reply to just the requester on failure. Requests from a non-active
    /// team are silently ignored (empty vec).
    pub fn receive_flip(&mut self, team: &str, pos: &str) -> Vec<(String, RefereeMessage)> {
        let active_team = self.active_team().to_string();
        if team != active_team {
            return vec![];
        }
        let active_id = self.active_id().to_string();

        if self.matched.contains(pos) {
            return vec![(
                active_id,
                RefereeMessage::Invalid {
                    reason: format!("Position '{pos}' already matched"),
                },
            )];
        }
        if self.flip1.as_deref() == Some(pos) {
            return vec![(
                active_id,
                RefereeMessage::Invalid {
                    reason: format!("Position '{pos}' already flipped this turn"),
                },
            )];
        }
        if !self.grid.contains_key(pos) {
            return vec![(
                active_id,
                RefereeMessage::Invalid {
                    reason: format!("Position '{pos}' is not on the grid"),
                },
            )];
        }

        let is_second_flip = self.flip1.is_some();
        if self.flip1.is_none() {
            self.flip1 = Some(pos.to_string());
        } else {
            self.flip2 = Some(pos.to_string());
        }
        self.revealed.insert(pos.to_string());
        if is_second_flip {
            self.flip_revealed = true;
        }

        self.teams
            .iter()
            .map(|(_, id)| {
                (
                    id.clone(),
                    RefereeMessage::CardRevealed {
                        pos: pos.to_string(),
                    },
                )
            })
            .collect()
    }

    /// Handle a `flip_both` request from `team`: reveal two positions in one
    /// atomic action instead of two round-trips. Only valid as the very
    /// first action of a turn (`flip1` must still be `None`) so the two
    /// flip protocols can't mix mid-turn. Either both positions are
    /// revealed or neither is -- a bad second position doesn't leave the
    /// first one revealed with no way to recover this turn.
    pub fn receive_flip_both(
        &mut self,
        team: &str,
        pos1: &str,
        pos2: &str,
    ) -> Vec<(String, RefereeMessage)> {
        let active_team = self.active_team().to_string();
        if team != active_team {
            return vec![];
        }
        let active_id = self.active_id().to_string();

        if let Some(reason) = self.flip_both_invalid_reason(pos1, pos2) {
            return vec![(active_id, RefereeMessage::Invalid { reason })];
        }

        self.flip1 = Some(pos1.to_string());
        self.flip2 = Some(pos2.to_string());
        self.revealed.insert(pos1.to_string());
        self.revealed.insert(pos2.to_string());
        self.flip_revealed = true;

        [pos1, pos2]
            .iter()
            .flat_map(|pos| {
                self.teams.iter().map(move |(_, id)| {
                    (
                        id.clone(),
                        RefereeMessage::CardRevealed {
                            pos: pos.to_string(),
                        },
                    )
                })
            })
            .collect()
    }

    fn flip_both_invalid_reason(&self, pos1: &str, pos2: &str) -> Option<String> {
        if self.flip1.is_some() {
            return Some(
                "Already flipped a card this turn; use a single flip for the second card"
                    .to_string(),
            );
        }
        if pos1 == pos2 {
            return Some(format!("Cannot flip the same position '{pos1}' twice"));
        }
        for pos in [pos1, pos2] {
            if self.matched.contains(pos) {
                return Some(format!("Position '{pos}' already matched"));
            }
            if !self.grid.contains_key(pos) {
                return Some(format!("Position '{pos}' is not on the grid"));
            }
        }
        None
    }

    /// Both positions of the currently-revealed pair, for a "flip the
    /// physical card now" scoreboard announcement -- `Some` from the
    /// moment the pair's second card is revealed until this pair's
    /// result is processed and the turn resets.
    pub fn flip_pending_positions(&self) -> Option<(String, String)> {
        if !self.flip_revealed {
            return None;
        }
        match (&self.flip1, &self.flip2) {
            (Some(p1), Some(p2)) => Some((p1.clone(), p2.clone())),
            _ => None,
        }
    }

    /// Handle a `report_result` claim from the active team: their own
    /// comparison of the two cards they just flipped. The referee's job is
    /// to validate this claim against the golden answer key, not to
    /// re-derive the comparison itself.
    pub fn receive_result(
        &mut self,
        team: &str,
        pos1: &str,
        pos2: &str,
        claim: &str,
    ) -> Option<ResultOutcome> {
        let active_team = self.active_team().to_string();
        if team != active_team {
            return None;
        }

        let tier_bonus = response_tier_bonus(Instant::now().duration_since(self.turn_start));

        let golden1 = self.grid.get(pos1).cloned().unwrap_or_default();
        let golden2 = self.grid.get(pos2).cloned().unwrap_or_default();
        let actually_matches = !golden1.is_empty() && golden1 == golden2;
        let claims_match = claim == "match";

        if claims_match && actually_matches {
            self.streak += 1;
            let awarded = self.streak as i32 + tier_bonus;
            *self
                .scores
                .get_mut(&active_team)
                .expect("active team always has a score entry") += awarded;
            self.matched.insert(pos1.to_string());
            self.matched.insert(pos2.to_string());
            self.revealed.insert(pos1.to_string());
            self.revealed.insert(pos2.to_string());
            *self
                .pairs_matched_by_team
                .get_mut(&active_team)
                .expect("active team always has a pairs_matched_by_team entry") += 1;

            let mut messages: Vec<(String, RefereeMessage)> = self
                .teams
                .iter()
                .map(|(_, id)| {
                    (
                        id.clone(),
                        RefereeMessage::Match {
                            cls: golden1.clone(),
                            pos1: pos1.to_string(),
                            pos2: pos2.to_string(),
                            scorer: active_team.clone(),
                            scores: self.scores.clone(),
                            remaining: self.total_pairs() - self.pairs_found(),
                        },
                    )
                })
                .collect();

            if self.all_pairs_found() {
                let winner = self.winner();
                for (_, id) in &self.teams {
                    messages.push((
                        id.clone(),
                        RefereeMessage::GameOver {
                            winner: winner.clone(),
                            scores: self.scores.clone(),
                        },
                    ));
                }
                return Some(ResultOutcome::GameOver { winner, messages });
            }

            self.restart_turn_same_team();
            self.push_turn_signals(&mut messages);
            return Some(ResultOutcome::CorrectMatch { messages });
        }

        let wrong_match = claims_match && !actually_matches;
        // A wrong claim keeps the existing -1 flat penalty on top of the
        // tier; an explicit decline (correct "no match" call) gets the
        // tier alone -- declining is still an action, and is scored on
        // how fast it happened, unlike before this feature existed.
        let score_delta = if wrong_match {
            tier_bonus - 1
        } else {
            tier_bonus
        };
        *self
            .scores
            .get_mut(&active_team)
            .expect("active team always has a score entry") += score_delta;

        let mut messages: Vec<(String, RefereeMessage)> = self
            .teams
            .iter()
            .map(|(_, id)| {
                (
                    id.clone(),
                    RefereeMessage::NoMatch {
                        pos1: pos1.to_string(),
                        pos2: pos2.to_string(),
                        cls1: golden1.clone(),
                        cls2: golden2.clone(),
                        scores: self.scores.clone(),
                    },
                )
            })
            .collect();

        self.next_team();
        self.push_turn_signals(&mut messages);

        if wrong_match {
            Some(ResultOutcome::WrongMatch { messages })
        } else {
            Some(ResultOutcome::NoClaim { messages })
        }
    }

    /// Called periodically by the poll loop. If the active team has taken
    /// too long to act, forfeit their turn and switch -- as of the
    /// time-based scoring feature, this now costs the same flat -3 as
    /// the slowest acted-tier (previously: no penalty at all).
    pub fn check_timeout(&mut self, now: Instant) -> Option<Vec<(String, RefereeMessage)>> {
        if self.is_paused() {
            return None;
        }
        let elapsed = now.duration_since(self.turn_start);
        if elapsed < crate::config::get().turn_timeout() {
            return None;
        }
        let active_team = self.active_team().to_string();
        *self
            .scores
            .get_mut(&active_team)
            .expect("active team always has a score entry") += response_tier_bonus(elapsed);
        self.next_team();
        let mut messages = vec![];
        self.push_turn_signals(&mut messages);
        Some(messages)
    }

    /// Handle a paid hint request from the active team for a named object.
    /// Rejects outright (no cost) if the team's score is <= 0 or they've
    /// already used both hint slots this match. Otherwise checks whether
    /// the object is already fully resolved (both its positions revealed)
    /// — if so, rejects but still costs the point and counts against the
    /// cap. If exactly one position is revealed, hints at the other. If
    /// neither is revealed, hints at the lexicographically smaller one.
    pub fn receive_hint_request(&mut self, team: &str, object: &str) -> Option<HintOutcome> {
        let active_team = self.active_team().to_string();
        if team != active_team {
            return None;
        }

        let score = *self.scores.get(&active_team).unwrap_or(&0);
        if score <= 0 {
            return None;
        }
        let used = *self.hints_used.get(&active_team).unwrap_or(&0);
        if used >= crate::config::get().hint_cap {
            return None;
        }

        let mut positions: Vec<String> = self
            .grid
            .iter()
            .filter(|(_, cls)| cls.as_str() == object)
            .map(|(pos, _)| pos.clone())
            .collect();
        positions.sort();

        if positions.is_empty() {
            return Some(self.reject_hint(&active_team, "unknown object"));
        }

        let unrevealed: Vec<String> = positions
            .iter()
            .filter(|p| !self.revealed.contains(*p))
            .cloned()
            .collect();

        let target = match unrevealed.len() {
            0 => return Some(self.reject_hint(&active_team, "object already fully resolved")),
            _ => unrevealed[0].clone(), // 1 or 2 unrevealed -> take the first (lex smaller if 2)
        };

        *self
            .hints_used
            .get_mut(&active_team)
            .expect("active team always has a hints_used entry") += 1;
        *self
            .scores
            .get_mut(&active_team)
            .expect("active team always has a score entry") -= crate::config::get().hint_cost;

        let (row_digit_png_base64, col_digit_png_base64) =
            crate::hints::row_col_digit_images(&target);
        let messages = vec![(
            self.active_id().to_string(),
            RefereeMessage::HintResponse {
                row_digit_png_base64: row_digit_png_base64.clone(),
                col_digit_png_base64: col_digit_png_base64.clone(),
            },
        )];
        Some(HintOutcome::Accepted {
            riddle: format!("row={row_digit_png_base64},col={col_digit_png_base64}"),
            messages,
        })
    }

    fn reject_hint(&mut self, active_team: &str, reason: &str) -> HintOutcome {
        *self
            .hints_used
            .get_mut(active_team)
            .expect("active team always has a hints_used entry") += 1;
        *self
            .scores
            .get_mut(active_team)
            .expect("active team always has a score entry") -= crate::config::get().hint_cost;
        let messages = vec![(
            self.active_id().to_string(),
            RefereeMessage::HintRejected {
                reason: reason.to_string(),
            },
        )];
        HintOutcome::Rejected {
            reason: reason.to_string(),
            messages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_grid() -> HashMap<String, String> {
        [("A1", "dog"), ("A2", "dog"), ("A3", "cat"), ("A4", "cat")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn two_teams() -> Vec<(String, String)> {
        vec![
            ("alpha".to_string(), "id-a".to_string()),
            ("beta".to_string(), "id-b".to_string()),
        ]
    }

    /// Backdates `state.turn_start` so the next `receive_result` or
    /// `check_timeout` call sees exactly `elapsed_secs` of elapsed time --
    /// keeps response-tier scoring deterministic in tests instead of
    /// depending on how fast the test happens to execute (which would
    /// otherwise land in the fastest, +2, tier essentially always).
    fn age_turn_start(state: &mut GameState, elapsed_secs: u64) {
        state.turn_start = Instant::now() - Duration::from_secs(elapsed_secs);
    }

    #[test]
    fn flip_pending_positions_is_cleared_once_the_result_is_processed() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip("alpha", "A1");
        state.receive_flip("alpha", "A2");
        state.receive_result("alpha", "A1", "A2", "match");
        assert_eq!(state.flip_pending_positions(), None);
    }

    #[test]
    fn correct_match_stacks_streak_bonus_with_a_fast_response_tier() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 10); // tier +2
        state.receive_result("alpha", "A1", "A2", "match").unwrap();
        // streak 1 + tier 2 = 3
        assert_eq!(state.scores().get("alpha"), Some(&3));
    }

    #[test]
    fn wrong_match_stacks_flat_penalty_with_a_slow_response_tier() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 110); // tier -2 (110-20=90s effective)
        state.receive_result("alpha", "A1", "A3", "match").unwrap(); // wrong match
                                                                     // -1 flat penalty + tier -2 = -3
        assert_eq!(state.scores().get("alpha"), Some(&-3));
    }

    #[test]
    fn new_game_starts_with_team_zero_active_and_zero_scores() {
        let state = GameState::new(two_teams(), small_grid());
        assert_eq!(state.active_team(), "alpha");
        assert_eq!(state.scores().get("alpha"), Some(&0));
        assert_eq!(state.scores().get("beta"), Some(&0));
        assert_eq!(state.total_pairs(), 2);
        assert_eq!(state.pairs_found(), 0);
    }

    #[test]
    fn all_positions_returns_every_grid_label_sorted() {
        let state = GameState::new(two_teams(), small_grid());
        assert_eq!(state.all_positions(), vec!["A1", "A2", "A3", "A4"]);
    }

    #[test]
    fn flip_from_non_active_team_is_ignored() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip("beta", "A1");
        assert!(out.is_empty());
    }

    #[test]
    fn flip_of_unknown_position_is_invalid() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip("alpha", "Z9");
        assert_eq!(out.len(), 1);
        match &out[0].1 {
            RefereeMessage::Invalid { reason } => assert!(reason.contains("not on the grid")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn flip_of_same_position_twice_in_one_turn_is_invalid() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip("alpha", "A1");
        let out = state.receive_flip("alpha", "A1");
        assert_eq!(out.len(), 1);
        match &out[0].1 {
            RefereeMessage::Invalid { reason } => assert!(reason.contains("already flipped")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn flip_pending_positions_is_none_before_any_flip() {
        let state = GameState::new(two_teams(), small_grid());
        assert_eq!(state.flip_pending_positions(), None);
    }

    #[test]
    fn flip_pending_positions_is_none_after_only_the_first_flip() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip("alpha", "A1");
        assert_eq!(state.flip_pending_positions(), None);
    }

    #[test]
    fn flip_pending_positions_is_set_after_the_second_single_flip() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip("alpha", "A1");
        state.receive_flip("alpha", "A2");
        assert_eq!(
            state.flip_pending_positions(),
            Some(("A1".to_string(), "A2".to_string()))
        );
    }

    #[test]
    fn flip_pending_positions_is_set_after_flip_both() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip_both("alpha", "A1", "A2");
        assert_eq!(
            state.flip_pending_positions(),
            Some(("A1".to_string(), "A2".to_string()))
        );
    }

    #[test]
    fn valid_flip_broadcasts_card_revealed_to_both_teams() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip("alpha", "A1");
        assert_eq!(out.len(), 2);
        for (_, msg) in &out {
            assert_eq!(
                msg,
                &RefereeMessage::CardRevealed {
                    pos: "A1".to_string()
                }
            );
        }
    }

    #[test]
    fn flip_both_from_non_active_team_is_ignored() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip_both("beta", "A1", "A2");
        assert!(out.is_empty());
    }

    #[test]
    fn flip_both_of_unknown_position_is_invalid() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip_both("alpha", "A1", "Z9");
        assert_eq!(out.len(), 1);
        match &out[0].1 {
            RefereeMessage::Invalid { reason } => assert!(reason.contains("not on the grid")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn flip_both_of_the_same_position_twice_is_invalid() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip_both("alpha", "A1", "A1");
        assert_eq!(out.len(), 1);
        match &out[0].1 {
            RefereeMessage::Invalid { reason } => assert!(reason.contains("same position")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn flip_both_after_a_single_flip_this_turn_is_invalid() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip("alpha", "A1");
        let out = state.receive_flip_both("alpha", "A2", "A3");
        assert_eq!(out.len(), 1);
        match &out[0].1 {
            RefereeMessage::Invalid { reason } => assert!(reason.contains("Already flipped")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn flip_both_invalid_second_position_reveals_neither() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip_both("alpha", "A1", "Z9");
        assert_eq!(out.len(), 1);
        // Neither position should be revealed -- a subsequent single flip of
        // A1 must succeed as if flip_both never happened.
        let retry = state.receive_flip("alpha", "A1");
        assert_eq!(retry.len(), 2);
        for (_, msg) in &retry {
            assert_eq!(
                msg,
                &RefereeMessage::CardRevealed {
                    pos: "A1".to_string()
                }
            );
        }
    }

    #[test]
    fn valid_flip_both_broadcasts_card_revealed_for_both_positions_to_both_teams() {
        let mut state = GameState::new(two_teams(), small_grid());
        let out = state.receive_flip_both("alpha", "A1", "A3");
        assert_eq!(out.len(), 4);
        let revealed: Vec<&str> = out
            .iter()
            .map(|(_, msg)| match msg {
                RefereeMessage::CardRevealed { pos } => pos.as_str(),
                other => panic!("expected CardRevealed, got {other:?}"),
            })
            .collect();
        assert_eq!(revealed.iter().filter(|&&pos| pos == "A1").count(), 2);
        assert_eq!(revealed.iter().filter(|&&pos| pos == "A3").count(), 2);
    }

    #[test]
    fn flip_both_then_report_result_completes_a_turn_like_two_single_flips() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip_both("alpha", "A1", "A2");
        age_turn_start(&mut state, 70);
        let outcome = state.receive_result("alpha", "A1", "A2", "match").unwrap();
        match outcome {
            ResultOutcome::CorrectMatch { .. } => {}
            other => panic!("expected CorrectMatch, got {other:?}"),
        }
        assert_eq!(state.scores().get("alpha"), Some(&1));
        assert_eq!(state.pairs_found(), 1);
    }

    #[test]
    fn initial_turn_signals_send_your_turn_to_first_team_only() {
        let state = GameState::new(two_teams(), small_grid());
        let messages = state.push_initial_turn_signals();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages.iter().find(|(id, _)| id == "id-a").unwrap().1,
            RefereeMessage::YourTurn { flip_num: 1 }
        );
        assert_eq!(
            messages.iter().find(|(id, _)| id == "id-b").unwrap().1,
            RefereeMessage::Wait {
                active_team: "alpha".to_string()
            }
        );
    }

    #[test]
    fn correct_match_awards_one_point_and_continues_same_team() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 70);
        let outcome = state.receive_result("alpha", "A1", "A2", "match").unwrap();
        match outcome {
            ResultOutcome::CorrectMatch { .. } => {}
            other => panic!("expected CorrectMatch, got {other:?}"),
        }
        assert_eq!(state.scores().get("alpha"), Some(&1));
        assert_eq!(state.active_team(), "alpha"); // same team continues (streak)
        assert_eq!(state.pairs_found(), 1);
    }

    #[test]
    fn second_consecutive_correct_match_awards_two_more_points() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 70);
        state.receive_result("alpha", "A1", "A2", "match").unwrap(); // +1 -> score 1
        age_turn_start(&mut state, 70);
        let outcome = state.receive_result("alpha", "A3", "A4", "match").unwrap(); // +2 -> score 3
        match outcome {
            ResultOutcome::GameOver { winner, .. } => assert_eq!(winner, "alpha"),
            other => panic!("expected GameOver (all pairs found), got {other:?}"),
        }
        assert_eq!(state.scores().get("alpha"), Some(&3));
    }

    #[test]
    fn tied_score_deterministically_picks_the_last_team_in_turn_order() {
        // alpha claims A1/A2 (a real match) -> +1, continues.
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 70);
        state.receive_result("alpha", "A1", "A2", "match").unwrap();
        // alpha declines to claim A3/A4 -> turn passes to beta with no penalty,
        // A3/A4 remain unmatched.
        age_turn_start(&mut state, 70);
        state
            .receive_result("alpha", "A3", "A4", "no_match")
            .unwrap();
        // beta claims A3/A4 (a real match) -> +1, all pairs now found, tied 1-1.
        age_turn_start(&mut state, 70);
        let outcome = state.receive_result("beta", "A3", "A4", "match").unwrap();
        match outcome {
            ResultOutcome::GameOver { winner, .. } => {
                assert_eq!(state.scores().get("alpha"), Some(&1));
                assert_eq!(state.scores().get("beta"), Some(&1));
                // Deterministic by construction: ties resolve to the last
                // team in turn order (self.teams), not HashMap iteration
                // order, which would vary run to run.
                assert_eq!(winner, "beta");
            }
            other => panic!("expected GameOver, got {other:?}"),
        }
    }

    #[test]
    fn result_from_non_active_team_is_ignored() {
        // No age_turn_start needed: the team != active_team guard returns
        // None before response_tier_bonus is ever computed, so this test
        // is unaffected by elapsed time regardless of execution speed.
        let mut state = GameState::new(two_teams(), small_grid());
        let outcome = state.receive_result("beta", "A1", "A2", "match");
        assert!(outcome.is_none());
        assert_eq!(state.scores().get("beta"), Some(&0));
    }

    #[test]
    fn wrong_match_claim_penalizes_and_switches_team() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 70);
        // A1=dog, A3=cat: genuinely different, but the team claims "match"
        // (e.g. their own model misdetected one of them).
        let outcome = state.receive_result("alpha", "A1", "A3", "match").unwrap();
        match outcome {
            ResultOutcome::WrongMatch { .. } => {}
            other => panic!("expected WrongMatch, got {other:?}"),
        }
        assert_eq!(state.scores().get("alpha"), Some(&-1));
        assert_eq!(state.active_team(), "beta"); // turn switched immediately
    }

    #[test]
    fn no_claim_now_scores_the_response_tier_instead_of_always_zero() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 10); // tier +2 -- a fast, correct decline
        let outcome = state
            .receive_result("alpha", "A1", "A3", "no_match")
            .unwrap();
        match outcome {
            ResultOutcome::NoClaim { .. } => {}
            other => panic!("expected NoClaim, got {other:?}"),
        }
        // Before this feature, declining always scored 0 regardless of
        // speed. Now a decline is still a real action and gets the
        // response tier alone (no streak, no -1 flat penalty).
        assert_eq!(state.scores().get("alpha"), Some(&2));
        assert_eq!(state.active_team(), "beta");
    }

    #[test]
    fn no_claim_at_slow_speed_scores_a_negative_tier() {
        // The fast-decline case above only shows the tier can be a bonus.
        // A slow decline is a real, plausible live-event scenario (a team
        // takes a while to decide "no match") and must show up as a
        // penalty, not silently stay at 0 like the pre-feature behavior.
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 110); // tier -2
        let outcome = state
            .receive_result("alpha", "A1", "A3", "no_match")
            .unwrap();
        match outcome {
            ResultOutcome::NoClaim { .. } => {}
            other => panic!("expected NoClaim, got {other:?}"),
        }
        assert_eq!(state.scores().get("alpha"), Some(&-2));
        assert_eq!(state.active_team(), "beta");
    }

    #[test]
    fn no_claim_at_a_middling_pace_scores_the_plus_one_tier() {
        // The +2 and -2/-3 tiers are already exercised against real
        // scores above; the middle tiers (+1 and -1) were previously
        // only checked by the pure response_tier_bonus boundary test,
        // not by anything that verifies they actually reach a score.
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 50); // tier +1
        state
            .receive_result("alpha", "A1", "A3", "no_match")
            .unwrap();
        assert_eq!(state.scores().get("alpha"), Some(&1));
    }

    #[test]
    fn wrong_match_at_a_middling_slow_pace_scores_the_minus_one_tier() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 90); // tier -1
        state.receive_result("alpha", "A1", "A3", "match").unwrap(); // wrong match
                                                                     // -1 flat penalty + tier -1 = -2
        assert_eq!(state.scores().get("alpha"), Some(&-2));
    }

    #[test]
    fn receive_flip_records_position_as_revealed_even_without_a_match() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip("alpha", "A1");
        assert!(state.is_revealed("A1"));
        assert!(!state.is_revealed("A3")); // never flipped
    }

    #[test]
    fn wrong_match_still_leaves_positions_marked_revealed() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_flip("alpha", "A1");
        state.receive_flip("alpha", "A3");
        state.receive_result("alpha", "A1", "A3", "match").unwrap(); // wrong match: A1=dog, A3=cat, not actually a pair
        assert!(state.is_revealed("A1"));
        assert!(state.is_revealed("A3"));
        assert_eq!(state.pairs_found(), 0); // confirms they're NOT in `matched`
    }

    #[test]
    fn wrong_match_does_not_remove_cards_from_play() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A3", "match").unwrap();
        assert_eq!(state.pairs_found(), 0); // A1/A3 were never actually a pair
    }

    #[test]
    fn turn_not_yet_timed_out_returns_none() {
        let mut state = GameState::new(two_teams(), small_grid());
        let result = state.check_timeout(Instant::now());
        assert!(result.is_none());
        assert_eq!(state.active_team(), "alpha");
    }

    #[test]
    fn turn_timeout_now_applies_a_flat_penalty() {
        let mut state = GameState::new(two_teams(), small_grid());
        let later = Instant::now() + Duration::from_secs(121);
        let result = state.check_timeout(later);
        assert!(result.is_some());
        assert_eq!(state.active_team(), "beta");
        // Full timeout lands in the same -3 bucket as the slowest
        // acted-tier -- no special-casing needed in check_timeout itself.
        assert_eq!(state.scores().get("alpha"), Some(&-3));
    }

    #[test]
    fn hint_for_never_revealed_object_reveals_lexicographically_smaller_position() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 70);
        state.receive_result("alpha", "A1", "A2", "match").unwrap(); // score 1, needed for the score>0 rule
                                                                     // "cat" is at A3 and A4, neither revealed yet.
        let outcome = state.receive_hint_request("alpha", "cat");
        match outcome {
            Some(HintOutcome::Accepted { riddle, .. }) => {
                let (row, col) = crate::hints::row_col_digit_images("A3");
                assert_eq!(riddle, format!("row={row},col={col}"));
            }
            other => panic!("expected Accepted hint for A3, got {other:?}"),
        }
        assert_eq!(state.scores().get("alpha"), Some(&0)); // 1 - 1 for the hint
    }

    #[test]
    fn hint_for_partially_revealed_object_reveals_the_still_closed_position() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A2", "match").unwrap(); // score 1, continues (streak)
        state.receive_flip("alpha", "A3"); // reveal one half of the "cat" pair
        let outcome = state.receive_hint_request("alpha", "cat");
        match outcome {
            Some(HintOutcome::Accepted { riddle, .. }) => {
                let (row, col) = crate::hints::row_col_digit_images("A4");
                assert_eq!(riddle, format!("row={row},col={col}"));
            }
            other => panic!("expected Accepted hint for A4, got {other:?}"),
        }
    }

    #[test]
    fn hint_for_fully_resolved_object_is_rejected_and_still_costs_a_point() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 70);
        state.receive_result("alpha", "A1", "A2", "match").unwrap(); // score 1
                                                                     // both dog positions (A1, A2) already revealed via the match above.
        let outcome = state.receive_hint_request("alpha", "dog");
        match outcome {
            Some(HintOutcome::Rejected { .. }) => {}
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert_eq!(state.scores().get("alpha"), Some(&0)); // 1 - 1, still costs
    }

    #[test]
    fn hint_request_with_score_zero_is_refused_with_no_change() {
        let mut state = GameState::new(two_teams(), small_grid());
        // alpha's score is 0 at game start.
        let outcome = state.receive_hint_request("alpha", "dog");
        assert!(outcome.is_none());
        assert_eq!(state.scores().get("alpha"), Some(&0));
    }

    #[test]
    fn hint_request_at_cap_is_refused_with_no_additional_cost() {
        let mut state = GameState::new(two_teams(), small_grid());
        age_turn_start(&mut state, 70);
        state.receive_result("alpha", "A1", "A2", "match").unwrap(); // score 1
        state.receive_hint_request("alpha", "dog"); // "dog" already resolved -> rejected, score 0, slot 1/2
                                                    // score is now 0, so give alpha another point before testing the cap specifically.
        state.receive_flip("alpha", "A3");
        state.receive_flip("alpha", "A4");
        age_turn_start(&mut state, 70);
        state.receive_result("alpha", "A3", "A4", "match").unwrap(); // streak continues: +2 -> score 2
        state.receive_hint_request("alpha", "cat"); // "cat" already resolved above -> rejected, score 1, slot 2/2
        let outcome = state.receive_hint_request("alpha", "dog"); // 3rd attempt, cap reached
        assert!(outcome.is_none());
        assert_eq!(state.scores().get("alpha"), Some(&1)); // unchanged by the refused 3rd attempt
    }

    #[test]
    fn hint_request_for_unknown_object_is_rejected() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A2", "match").unwrap();
        let outcome = state.receive_hint_request("alpha", "elephant");
        match outcome {
            Some(HintOutcome::Rejected { .. }) => {}
            other => panic!("expected Rejected for unknown object, got {other:?}"),
        }
    }

    #[test]
    fn hint_request_from_non_active_team_is_ignored() {
        let mut state = GameState::new(two_teams(), small_grid());
        let outcome = state.receive_hint_request("beta", "dog");
        assert!(outcome.is_none());
    }

    #[test]
    fn matched_positions_returns_only_confirmed_pairs_with_their_object() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A2", "match").unwrap();
        let matched = state.matched_positions();
        assert_eq!(matched.get("A1"), Some(&"dog".to_string()));
        assert_eq!(matched.get("A2"), Some(&"dog".to_string()));
        assert_eq!(matched.len(), 2); // A3/A4 not yet matched
    }

    #[test]
    fn current_streak_reflects_consecutive_matches_this_turn() {
        let mut state = GameState::new(two_teams(), small_grid());
        assert_eq!(state.current_streak(), 0);
        state.receive_result("alpha", "A1", "A2", "match").unwrap();
        assert_eq!(state.current_streak(), 1);
        state.receive_result("alpha", "A3", "A4", "match").unwrap(); // also ends the game
        assert_eq!(state.current_streak(), 2);
    }

    #[test]
    fn current_streak_resets_to_zero_when_turn_switches() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A3", "match").unwrap(); // wrong match, switches turn
        assert_eq!(state.current_streak(), 0);
    }

    #[test]
    fn turn_seconds_remaining_counts_down_from_120() {
        let state = GameState::new(two_teams(), small_grid());
        let now = Instant::now();
        let just_started = state.turn_seconds_remaining(now);
        assert!((118..=120).contains(&just_started)); // allow tiny test-time drift

        let later = now + Duration::from_secs(100);
        let remaining = state.turn_seconds_remaining(later);
        assert!((19..=20).contains(&remaining)); // allow tiny test-time drift
    }

    #[test]
    fn turn_seconds_remaining_floors_at_zero_after_timeout() {
        let state = GameState::new(two_teams(), small_grid());
        let way_later = Instant::now() + Duration::from_secs(999);
        assert_eq!(state.turn_seconds_remaining(way_later), 0);
    }

    #[test]
    fn set_score_overwrites_an_absolute_value() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A2", "match").unwrap(); // score 1
        state.set_score("alpha", 42);
        assert_eq!(state.scores()["alpha"], 42);
    }

    #[test]
    fn set_score_is_a_no_op_for_an_unknown_team() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.set_score("not-a-team", 99);
        assert!(!state.scores().contains_key("not-a-team"));
    }

    #[test]
    fn pause_freezes_the_turn_timer() {
        let mut state = GameState::new(two_teams(), small_grid());
        let now = Instant::now();
        let paused_at = now + Duration::from_secs(30);
        state.pause(paused_at);
        assert!(state.is_paused());
        // Real time keeps moving, but the frozen value doesn't.
        let much_later = paused_at + Duration::from_secs(500);
        assert!((89..=90).contains(&state.turn_seconds_remaining(much_later))); // allow tiny test-time drift
    }

    #[test]
    fn resume_continues_from_exactly_where_it_was_paused() {
        let mut state = GameState::new(two_teams(), small_grid());
        let now = Instant::now();
        state.pause(now + Duration::from_secs(30)); // ~90s left when paused
        let resumed_at = now + Duration::from_secs(600); // paused for a long real-world gap
        state.resume(resumed_at);
        assert!(!state.is_paused());
        // Immediately after resuming, still ~90s left -- the pause gap didn't cost turn time.
        let just_resumed = state.turn_seconds_remaining(resumed_at);
        assert!((89..=90).contains(&just_resumed)); // allow tiny test-time drift
                                                    // And it counts down normally from there.
        assert_eq!(
            state.turn_seconds_remaining(resumed_at + Duration::from_secs(10)),
            just_resumed - 10
        );
    }

    #[test]
    fn resume_without_a_prior_pause_is_a_no_op() {
        let mut state = GameState::new(two_teams(), small_grid());
        let now = Instant::now();
        state.resume(now + Duration::from_secs(50));
        assert!(!state.is_paused());
        // Turn clock is untouched, still counting from the original turn_start.
        assert!((118..=120).contains(&state.turn_seconds_remaining(now))); // allow tiny test-time drift
    }

    #[test]
    fn check_timeout_does_not_fire_while_paused() {
        let mut state = GameState::new(two_teams(), small_grid());
        let now = Instant::now();
        state.pause(now);
        let way_later = now + Duration::from_secs(999);
        assert!(state.check_timeout(way_later).is_none());
    }

    #[test]
    fn hints_used_map_reflects_per_team_usage() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A2", "match").unwrap(); // score 1
        state.receive_hint_request("alpha", "dog"); // rejected (already resolved), still uses a slot
        let map = state.hints_used_map();
        assert_eq!(map.get("alpha"), Some(&1));
        assert_eq!(map.get("beta"), Some(&0));
    }

    #[test]
    fn pairs_matched_by_team_tracks_which_team_found_each_pair() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A2", "match").unwrap();
        let map = state.pairs_matched_by_team();
        assert_eq!(map.get("alpha"), Some(&1));
        assert_eq!(map.get("beta"), Some(&0));
    }

    #[test]
    fn pairs_matched_by_team_does_not_credit_wrong_matches() {
        let mut state = GameState::new(two_teams(), small_grid());
        state.receive_result("alpha", "A1", "A3", "match").unwrap(); // wrong match
        let map = state.pairs_matched_by_team();
        assert_eq!(map.get("alpha"), Some(&0));
    }

    /// Full tier-boundary coverage lives in `config::tests` against
    /// `GameConfig::tier_bonus` directly -- this just confirms the
    /// game_state wrapper actually delegates to it.
    #[test]
    fn response_tier_bonus_delegates_to_config() {
        assert_eq!(
            response_tier_bonus(Duration::from_secs(150)),
            crate::config::get().tier_bonus(Duration::from_secs(150))
        );
    }
}
