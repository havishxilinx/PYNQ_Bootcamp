use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single round-robin match: two team names, not yet assigned to a grid.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Matchup {
    pub team_a: String,
    pub team_b: String,
}

/// Generates the full round-robin schedule for a set of team names: every
/// team plays every other team exactly once. Order is deterministic,
/// based on input order — not randomized.
pub fn round_robin_schedule(teams: &[String]) -> Vec<Matchup> {
    let mut schedule = Vec::new();
    for i in 0..teams.len() {
        for j in (i + 1)..teams.len() {
            schedule.push(Matchup {
                team_a: teams[i].clone(),
                team_b: teams[j].clone(),
            });
        }
    }
    schedule
}

/// Per-match status within a pool's schedule, used by the Schedule
/// screen (operator + public scoreboard) to show which matches are
/// playable right now vs. not yet reachable vs. finished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MatchStatus {
    Locked,
    Ready,
    Live,
    Complete { winner: String },
}

/// Replaces the old pop-a-stack `Vec<Matchup>` design: entries persist
/// with updated status instead of disappearing once assigned/completed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub team_a: String,
    pub team_b: String,
    pub grid_id: String,
    // flatten is required -- the frontend reads `status` as a top-level
    // string on the entry object, not nested under a second "status" key.
    #[serde(flatten)]
    pub status: MatchStatus,
}

/// Builds a pool's full schedule from its round-robin matchups, all
/// entries sharing one grid id -- used by `--config` mode, which passes
/// its own explicit `grid_id` rather than drawing from a pool. The first
/// entry starts `Ready` (nothing blocks it), every other entry starts
/// `Locked` (its arena is busy with an earlier match first) -- this
/// mirrors the old stack's LIFO-pop behavior exactly, just made
/// persistent and inspectable instead of destructive.
pub fn build_schedule_entries(teams: &[String], grid_id: &str) -> Vec<ScheduleEntry> {
    round_robin_schedule(teams)
        .into_iter()
        .enumerate()
        .map(|(i, m)| ScheduleEntry {
            team_a: m.team_a,
            team_b: m.team_b,
            grid_id: grid_id.to_string(),
            status: if i == 0 {
                MatchStatus::Ready
            } else {
                MatchStatus::Locked
            },
        })
        .collect()
}

/// Same as `build_schedule_entries`, but assigns each entry a randomly
/// picked grid file from `grids` -- used by the live registration path
/// once a grid pool exists.
pub fn build_schedule_entries_with_grids(teams: &[String], grids: &[String]) -> Vec<ScheduleEntry> {
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();
    round_robin_schedule(teams)
        .into_iter()
        .enumerate()
        .map(|(i, m)| ScheduleEntry {
            team_a: m.team_a,
            team_b: m.team_b,
            grid_id: grids.choose(&mut rng).cloned().unwrap_or_default(),
            status: if i == 0 {
                MatchStatus::Ready
            } else {
                MatchStatus::Locked
            },
        })
        .collect()
}

/// Tracks one pool's round-robin standings as matches complete.
pub struct PoolStandings {
    wins: HashMap<String, u32>,
    losses: HashMap<String, u32>,
    pairs_matched: HashMap<String, u32>,
    teams: Vec<String>,
    matches_played: usize,
    total_matches: usize,
}

impl PoolStandings {
    pub fn new(teams: Vec<String>) -> Self {
        let total_matches = round_robin_schedule(&teams).len();
        let wins = teams.iter().map(|t| (t.clone(), 0)).collect();
        let losses = teams.iter().map(|t| (t.clone(), 0)).collect();
        let pairs_matched = teams.iter().map(|t| (t.clone(), 0)).collect();
        PoolStandings {
            wins,
            losses,
            pairs_matched,
            teams,
            matches_played: 0,
            total_matches,
        }
    }

    /// Record a completed match. Both teams' pairs_matched totals
    /// accumulate — the tiebreaker credits overall performance across
    /// all of a team's pool games, not just their wins.
    pub fn record_match(&mut self, winner: &str, loser: &str, winner_pairs: u32, loser_pairs: u32) {
        *self
            .wins
            .get_mut(winner)
            .expect("winner must be a pool member") += 1;
        *self
            .losses
            .get_mut(loser)
            .expect("loser must be a pool member") += 1;
        *self
            .pairs_matched
            .get_mut(winner)
            .expect("winner must be a pool member") += winner_pairs;
        *self
            .pairs_matched
            .get_mut(loser)
            .expect("loser must be a pool member") += loser_pairs;
        self.matches_played += 1;
    }

    pub fn wins(&self, team: &str) -> u32 {
        *self.wins.get(team).unwrap_or(&0)
    }

    pub fn losses(&self, team: &str) -> u32 {
        *self.losses.get(team).unwrap_or(&0)
    }

    pub fn pairs_matched(&self, team: &str) -> u32 {
        *self.pairs_matched.get(team).unwrap_or(&0)
    }

    /// Team names in this pool, in registration order.
    pub fn teams(&self) -> &[String] {
        &self.teams
    }

    /// True once every scheduled round-robin match for this pool has
    /// been recorded.
    pub fn is_complete(&self) -> bool {
        self.matches_played >= self.total_matches
    }

    /// The pool winner: most wins, ties broken by total pairs matched.
    /// Only meaningful once `is_complete()` is true. A full tie on both
    /// wins and pairs matched resolves deterministically to the last
    /// team in registration order (same pattern as GameState's winner
    /// tie-break for a single match).
    pub fn winner(&self) -> String {
        self.teams
            .iter()
            .max_by_key(|t| (self.wins(t), self.pairs_matched(t)))
            .expect("pool always has at least one team")
            .clone()
    }

    /// The pool runner-up: same ranking as `winner()`, excluding the
    /// winner. `None` if the pool has fewer than two teams. Only
    /// meaningful once `is_complete()` is true.
    pub fn runner_up(&self) -> Option<String> {
        let winner = self.winner();
        self.teams
            .iter()
            .filter(|t| **t != winner)
            .max_by_key(|t| (self.wins(t), self.pairs_matched(t)))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn three_teams_produces_three_matches() {
        let schedule = round_robin_schedule(&names(&["alpha", "beta", "gamma"]));
        assert_eq!(schedule.len(), 3);
        assert_eq!(
            schedule,
            vec![
                Matchup {
                    team_a: "alpha".into(),
                    team_b: "beta".into()
                },
                Matchup {
                    team_a: "alpha".into(),
                    team_b: "gamma".into()
                },
                Matchup {
                    team_a: "beta".into(),
                    team_b: "gamma".into()
                },
            ]
        );
    }

    #[test]
    fn two_teams_produces_one_match() {
        let schedule = round_robin_schedule(&names(&["delta", "epsilon"]));
        assert_eq!(
            schedule,
            vec![Matchup {
                team_a: "delta".into(),
                team_b: "epsilon".into()
            }]
        );
    }

    #[test]
    fn every_pair_appears_exactly_once() {
        let schedule = round_robin_schedule(&names(&["a", "b", "c", "d"]));
        assert_eq!(schedule.len(), 6); // 4 choose 2
        let mut seen = std::collections::HashSet::new();
        for m in &schedule {
            let key = (m.team_a.clone(), m.team_b.clone());
            assert!(seen.insert(key), "duplicate matchup found: {m:?}");
        }
    }

    #[test]
    fn new_standings_start_at_zero_for_all_teams() {
        let standings = PoolStandings::new(names(&["alpha", "beta"]));
        assert_eq!(standings.wins("alpha"), 0);
        assert_eq!(standings.losses("alpha"), 0);
        assert_eq!(standings.pairs_matched("alpha"), 0);
        assert!(!standings.is_complete());
    }

    #[test]
    fn teams_returns_registration_order() {
        let standings = PoolStandings::new(names(&["alpha", "beta"]));
        assert_eq!(
            standings.teams(),
            &["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn record_match_updates_winner_and_loser() {
        let mut standings = PoolStandings::new(names(&["alpha", "beta"]));
        standings.record_match("alpha", "beta", 9, 6);
        assert_eq!(standings.wins("alpha"), 1);
        assert_eq!(standings.losses("alpha"), 0);
        assert_eq!(standings.pairs_matched("alpha"), 9);
        assert_eq!(standings.wins("beta"), 0);
        assert_eq!(standings.losses("beta"), 1);
        assert_eq!(standings.pairs_matched("beta"), 6);
    }

    #[test]
    fn is_complete_once_every_scheduled_match_is_recorded() {
        let mut standings = PoolStandings::new(names(&["alpha", "beta", "gamma"]));
        assert!(!standings.is_complete());
        standings.record_match("alpha", "beta", 5, 4);
        assert!(!standings.is_complete());
        standings.record_match("alpha", "gamma", 5, 3);
        assert!(!standings.is_complete());
        standings.record_match("beta", "gamma", 6, 2);
        assert!(standings.is_complete());
    }

    #[test]
    fn winner_is_the_team_with_most_wins() {
        let mut standings = PoolStandings::new(names(&["alpha", "beta", "gamma"]));
        standings.record_match("alpha", "beta", 5, 4);
        standings.record_match("alpha", "gamma", 5, 3);
        standings.record_match("beta", "gamma", 6, 2);
        // alpha: 2 wins, beta: 1 win, gamma: 0 wins
        assert_eq!(standings.winner(), "alpha");
    }

    #[test]
    fn winner_tiebreaks_on_total_pairs_matched_when_wins_are_equal() {
        let mut standings = PoolStandings::new(names(&["alpha", "beta"]));
        standings.record_match("alpha", "beta", 9, 6);
        standings.record_match("beta", "alpha", 10, 3);
        // alpha: 1 win, 9+3=12 pairs. beta: 1 win, 6+10=16 pairs.
        assert_eq!(standings.wins("alpha"), 1);
        assert_eq!(standings.wins("beta"), 1);
        assert_eq!(standings.winner(), "beta"); // more total pairs matched
    }

    #[test]
    fn runner_up_is_the_team_with_second_most_wins() {
        let mut standings = PoolStandings::new(names(&["alpha", "beta", "gamma"]));
        standings.record_match("alpha", "beta", 5, 4);
        standings.record_match("alpha", "gamma", 5, 3);
        standings.record_match("beta", "gamma", 6, 2);
        // alpha: 2 wins, beta: 1 win, gamma: 0 wins
        assert_eq!(standings.runner_up(), Some("beta".to_string()));
    }

    #[test]
    fn runner_up_is_none_for_a_single_team_pool() {
        let standings = PoolStandings::new(names(&["alpha"]));
        assert_eq!(standings.runner_up(), None);
    }

    #[test]
    fn schedule_entries_start_ready_for_the_first_match_and_locked_for_the_rest() {
        let entries =
            build_schedule_entries(&names(&["alpha", "beta", "gamma"]), "example_grid.json");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].status, MatchStatus::Ready);
        assert_eq!(entries[1].status, MatchStatus::Locked);
        assert_eq!(entries[2].status, MatchStatus::Locked);
        assert_eq!(entries[0].team_a, "alpha");
        assert_eq!(entries[0].team_b, "beta");
    }

    #[test]
    fn build_schedule_entries_on_zero_or_one_team_produces_no_entries() {
        assert_eq!(
            build_schedule_entries(&names(&[]), "example_grid.json"),
            vec![]
        );
        assert_eq!(
            build_schedule_entries(&names(&["alpha"]), "example_grid.json"),
            vec![]
        );
    }

    #[test]
    fn build_schedule_entries_with_grids_assigns_one_grid_per_entry() {
        let teams = names(&["alpha", "beta", "gamma"]);
        let grids = vec!["grid_a.json".to_string(), "grid_b.json".to_string()];
        let entries = build_schedule_entries_with_grids(&teams, &grids);
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| grids.contains(&e.grid_id)));
    }

    #[test]
    fn schedule_entry_serializes_status_flat_not_nested() {
        // The frontend (operator.html/scoreboard.html) reads `entry.status`
        // as a plain string and `entry.winner` directly on the entry when
        // complete -- MatchStatus's own internal tag must be flattened into
        // ScheduleEntry, not nested under a second "status" key.
        let ready = ScheduleEntry {
            team_a: "alpha".to_string(),
            team_b: "beta".to_string(),
            grid_id: "example_grid.json".to_string(),
            status: MatchStatus::Ready,
        };
        assert_eq!(
            serde_json::to_string(&ready).unwrap(),
            r#"{"team_a":"alpha","team_b":"beta","grid_id":"example_grid.json","status":"ready"}"#
        );

        let complete = ScheduleEntry {
            team_a: "alpha".to_string(),
            team_b: "beta".to_string(),
            grid_id: "example_grid.json".to_string(),
            status: MatchStatus::Complete {
                winner: "alpha".to_string(),
            },
        };
        assert_eq!(
            serde_json::to_string(&complete).unwrap(),
            r#"{"team_a":"alpha","team_b":"beta","grid_id":"example_grid.json","status":"complete","winner":"alpha"}"#
        );
    }
}
