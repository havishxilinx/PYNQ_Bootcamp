use crate::pool::ScheduleEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One arena's currently-live match info, as last reported by that
/// arena's score_update messages.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LiveArenaState {
    pub pool: u32,
    pub team_a: String,
    pub team_b: String,
    pub scores: HashMap<String, i32>,
    /// Position -> object name for pairs already matched this game
    /// (e.g. "A1" -> "dog"). Unmatched positions are never included here.
    pub matched: HashMap<String, String>,
    /// Every position label on the board (e.g. "A1".."E6"), sorted --
    /// lets the frontend render the full grid shape and fill in "?" for
    /// whatever isn't in `matched` yet, without ever seeing the golden
    /// answer key for unmatched cells.
    pub all_positions: Vec<String>,
    pub active_team: String,
    pub turn_seconds_remaining: u64,
    pub streak: HashMap<String, u32>,
    pub hints_used: HashMap<String, u32>,
    pub pairs_found: usize,
    pub total_pairs: usize,
    /// The team that solved the whole-board puzzle first and chose to go
    /// first this match -- shown as a one-time banner.
    pub puzzle_winner: String,
    /// Absolute Unix-ms timestamp the match started -- purely cosmetic
    /// (elapsed-time display), no gameplay effect, no overall time limit.
    pub match_started_at_unix_ms: u64,
    /// True while an operator has paused this match via the admin
    /// override controls -- the frontend shows a "PAUSED" indicator and
    /// freezes the displayed turn timer instead of ticking it down.
    pub is_paused: bool,
    /// `Some((pos1, pos2))` from the moment a pair's second card is
    /// revealed until its result is processed -- the frontend renders a
    /// "flip the card now" banner while this is set, and hides it once
    /// cleared.
    pub flip_pending_positions: Option<(String, String)>,
    /// URL of Genesis's live MJPEG view for this match, for the frontend
    /// to embed directly -- `None` whenever there's nothing to show
    /// (Genesis unconfigured, practice match, or an older Genesis server).
    pub genesis_stream_url: Option<String>,
    /// Whether Genesis is configured for this match at all -- fixed for
    /// the whole match, unlike `genesis_stream_url` which can start as
    /// `None` and only becomes `Some` once `admin_start_competition`
    /// actually succeeds. The arena UI uses this to decide its layout
    /// (compact, room reserved for video vs a fuller standalone view)
    /// without that decision flickering while Genesis is still starting up.
    pub genesis_configured: bool,
}

/// A pool's roster and round-robin schedule size, shown in the Idle
/// state and the operator console.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PoolPreview {
    pub teams: Vec<String>,
    pub total_matches: usize,
}

/// A student roster entry shown on the Registration screen and (for
/// display only, no gameplay impact) later screens if ever needed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisteredTeam {
    pub name: String,
    pub students: Vec<String>,
    /// Issued once at registration (`generate_team_secret`), required by
    /// the student's `join_competition(team, mac, secret)` call -- see
    /// the per-team join authentication design doc. Shown here so it's
    /// visible on the operator's registration view.
    pub secret: String,
}

/// One pool's live registration state: teams placed so far and the
/// round-robin schedule computed from those teams.
///
/// The schedule is re-derived each time a team is added; no matches
/// have started so all entries are in a pre-play status.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PoolRegistration {
    pub teams: Vec<RegisteredTeam>,
    pub schedule: Vec<ScheduleEntry>,
}

/// One team's round-robin record within a pool, for the standings table
/// shown alongside live pool play and the Grand Final.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TeamStanding {
    pub team: String,
    pub wins: u32,
    pub losses: u32,
    pub pairs_matched: u32,
}

/// What an arena is showing on the scoreboard before its match actually
/// starts -- inserted between a match becoming `Ready` and the operator
/// submitting the match-assign popup, so the audience sees a countdown
/// instead of a blank/idle panel during that dead time. `deadline_unix_ms`
/// is an absolute timestamp (milliseconds since the Unix epoch) so the
/// frontend can tick the countdown down locally between websocket
/// pushes, rather than needing the server to re-push every second.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "stage")]
pub enum PregameState {
    /// Shown from the moment a match becomes `Ready` until both teams have
    /// self-reported their MAC (`join_competition`) or an operator has
    /// recorded it manually (`/api/manual-join`) -- registration and the
    /// tournament schedule are typically built long before either team is
    /// actually present (e.g. Tuesday registration, Thursday matches), so
    /// the puzzle-race countdown must not start until someone could
    /// actually receive and answer it. No `deadline_unix_ms`/timer here on
    /// purpose -- nothing is timed yet.
    #[serde(rename = "waiting_for_teams")]
    WaitingForTeams {
        team_a: String,
        team_b: String,
        team_a_joined: bool,
        team_b_joined: bool,
    },
    /// Both teams have joined, but the puzzle-race riddle is deliberately
    /// held back until the operator explicitly starts it -- sending it the
    /// instant both MACs are known gave the operator no window to confirm
    /// both boards were actually ready before the clock started. No
    /// `deadline_unix_ms` here either, for the same reason as
    /// `WaitingForTeams`: nothing is timed yet.
    #[serde(rename = "ready_to_start")]
    ReadyToStart { team_a: String, team_b: String },
    #[serde(rename = "puzzle_race")]
    PuzzleRace {
        team_a: String,
        team_b: String,
        deadline_unix_ms: u64,
        riddle: String,
    },
    /// The operator has recorded who won the puzzle race, but the free hint
    /// is deliberately held back until a second, separate operator action
    /// (`BeginMatch`) -- confirming the winner and sending the free hint
    /// used to be the same click, which gave the operator no room to do
    /// anything (confer with the teams, etc.) in between. No
    /// `deadline_unix_ms` here either: nothing is timed until Start Match.
    #[serde(rename = "winner_confirmed")]
    WinnerConfirmed {
        team_a: String,
        team_b: String,
        winner: String,
    },
    #[serde(rename = "free_hints")]
    FreeHints {
        team_a: String,
        team_b: String,
        deadline_unix_ms: u64,
    },
}

/// Which arena the decided-but-not-yet-assigned Grand Final will run on,
/// plus the two teams playing it. The frontend needs the arena number to
/// know which per-arena match-start channel to submit to.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GrandFinalReady {
    pub arena: u32,
    pub team_a: String,
    pub team_b: String,
}

/// The complete state pushed to every connected browser. Only one
/// variant is active at a time -- the frontend switches its whole
/// rendering based on which variant it receives, per the approved
/// 5-state design (Registration -> Idle -> LivePoolPlay -> GrandFinal -> Champion).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "phase")]
pub enum ScoreboardState {
    #[serde(rename = "registration")]
    Registration {
        pool1: PoolRegistration,
        pool2: PoolRegistration,
    },
    #[serde(rename = "idle")]
    Idle {
        pool1: PoolPreview,
        pool2: PoolPreview,
    },
    #[serde(rename = "live_pool_play")]
    LivePoolPlay {
        arena1: Option<Box<LiveArenaState>>,
        arena2: Option<Box<LiveArenaState>>,
        /// Set while that arena's next match is in its pre-game puzzle
        /// race window; `None` once the match has actually started (or
        /// if the arena has no match pending at all).
        arena1_pregame: Option<Box<PregameState>>,
        arena2_pregame: Option<Box<PregameState>>,
        pool1_standings: Vec<TeamStanding>,
        pool2_standings: Vec<TeamStanding>,
        pool1_schedule: Vec<ScheduleEntry>,
        pool2_schedule: Vec<ScheduleEntry>,
        /// Set the instant both pools are complete and the Grand Final
        /// matchup is decided, but before the operator has assigned it to
        /// an arena. Neither pool's schedule ever contains this matchup --
        /// without this field, once every pool match reaches `Complete`,
        /// there is no `Ready` entry anywhere in the pushed state for the
        /// operator to click, and `run_master` deadlocks forever waiting
        /// on a match-start submission the UI has no way to trigger.
        grand_final_ready: Option<GrandFinalReady>,
    },
    #[serde(rename = "grand_final")]
    GrandFinal {
        /// Which physical arena (1 or 2) is hosting the Grand Final --
        /// needed by any consumer that has to route a command back to
        /// this specific arena (e.g. the operator admin panel's
        /// `/api/admin/*` calls), since `LiveArenaState` itself only
        /// carries `pool` (always 0 here), not the arena number.
        arena_num: u32,
        arena: Box<LiveArenaState>,
        pool1_standings: Vec<TeamStanding>,
        pool2_standings: Vec<TeamStanding>,
        pool1_schedule: Vec<ScheduleEntry>,
        pool2_schedule: Vec<ScheduleEntry>,
    },
    /// Terminal state once the Grand Final ends.
    #[serde(rename = "champion")]
    Champion {
        winner: String,
        /// The Grand Final's final score (not a running tournament total).
        scores: HashMap<String, i32>,
        pool1_winner: String,
        pool2_winner: String,
        /// The higher-scoring of the two pools' runner-ups, by pool
        /// standings alone -- there's no separate 3rd-place match.
        /// `None` if both pools only ever had one team each.
        third_place: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_arena_state() -> LiveArenaState {
        LiveArenaState {
            pool: 1,
            team_a: "alpha".to_string(),
            team_b: "beta".to_string(),
            scores: HashMap::from([("alpha".to_string(), 3), ("beta".to_string(), 1)]),
            matched: HashMap::from([("A1".to_string(), "dog".to_string())]),
            all_positions: vec!["A1".to_string(), "A2".to_string()],
            active_team: "alpha".to_string(),
            turn_seconds_remaining: 42,
            streak: HashMap::from([("alpha".to_string(), 2)]),
            hints_used: HashMap::new(),
            pairs_found: 1,
            total_pairs: 15,
            puzzle_winner: "alpha".to_string(),
            match_started_at_unix_ms: 1_800_000_000_000,
            is_paused: false,
            flip_pending_positions: None,
            genesis_stream_url: None,
            genesis_configured: false,
        }
    }

    fn sample_standings() -> Vec<TeamStanding> {
        vec![TeamStanding {
            team: "alpha".to_string(),
            wins: 1,
            losses: 0,
            pairs_matched: 5,
        }]
    }

    #[test]
    fn idle_state_serializes_with_phase_tag() {
        let state = ScoreboardState::Idle {
            pool1: PoolPreview {
                teams: vec!["alpha".to_string(), "beta".to_string()],
                total_matches: 1,
            },
            pool2: PoolPreview {
                teams: vec!["delta".to_string(), "epsilon".to_string()],
                total_matches: 1,
            },
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.starts_with(r#"{"phase":"idle","#));
        assert!(json.contains(r#""teams":["alpha","beta"]"#));
    }

    #[test]
    fn live_pool_play_serializes_arena_state_and_null_for_missing_arena() {
        let state = ScoreboardState::LivePoolPlay {
            arena1: Some(Box::new(sample_arena_state())),
            arena2: None,
            arena1_pregame: None,
            arena2_pregame: None,
            pool1_standings: sample_standings(),
            pool2_standings: vec![],
            pool1_schedule: vec![],
            pool2_schedule: vec![],
            grand_final_ready: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(r#""phase":"live_pool_play""#));
        assert!(json.contains(r#""turn_seconds_remaining":42"#));
        assert!(json.contains(r#""arena2":null"#));
        assert!(json.contains(
            r#""pool1_standings":[{"team":"alpha","wins":1,"losses":0,"pairs_matched":5}]"#
        ));
        assert!(json.contains(r#""grand_final_ready":null"#));
    }

    #[test]
    fn live_pool_play_serializes_the_grand_final_matchup_when_ready() {
        let state = ScoreboardState::LivePoolPlay {
            arena1: None,
            arena2: None,
            arena1_pregame: None,
            arena2_pregame: None,
            pool1_standings: sample_standings(),
            pool2_standings: sample_standings(),
            pool1_schedule: vec![],
            pool2_schedule: vec![],
            grand_final_ready: Some(GrandFinalReady {
                arena: 1,
                team_a: "alpha".to_string(),
                team_b: "delta".to_string(),
            }),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(
            json.contains(r#""grand_final_ready":{"arena":1,"team_a":"alpha","team_b":"delta"}"#)
        );
    }

    #[test]
    fn live_pool_play_serializes_puzzle_race_pregame_for_one_arena() {
        let state = ScoreboardState::LivePoolPlay {
            arena1: None,
            arena2: None,
            arena1_pregame: Some(Box::new(PregameState::PuzzleRace {
                team_a: "alpha".to_string(),
                team_b: "beta".to_string(),
                deadline_unix_ms: 1_800_000_000_000,
                riddle: "what am I?".to_string(),
            })),
            arena2_pregame: None,
            pool1_standings: vec![],
            pool2_standings: vec![],
            pool1_schedule: vec![],
            pool2_schedule: vec![],
            grand_final_ready: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(
            r#""arena1_pregame":{"stage":"puzzle_race","team_a":"alpha","team_b":"beta","deadline_unix_ms":1800000000000,"riddle":"what am I?"}"#
        ));
        assert!(json.contains(r#""arena2_pregame":null"#));
    }

    #[test]
    fn live_pool_play_serializes_free_hints_pregame_for_one_arena() {
        let state = ScoreboardState::LivePoolPlay {
            arena1: None,
            arena2: None,
            arena1_pregame: None,
            arena2_pregame: Some(Box::new(PregameState::FreeHints {
                team_a: "gamma".to_string(),
                team_b: "delta".to_string(),
                deadline_unix_ms: 1_800_000_000_000,
            })),
            pool1_standings: vec![],
            pool2_standings: vec![],
            pool1_schedule: vec![],
            pool2_schedule: vec![],
            grand_final_ready: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(
            r#""arena2_pregame":{"stage":"free_hints","team_a":"gamma","team_b":"delta","deadline_unix_ms":1800000000000}"#
        ));
    }

    #[test]
    fn grand_final_serializes_a_single_arena() {
        let state = ScoreboardState::GrandFinal {
            arena_num: 1,
            arena: Box::new(sample_arena_state()),
            pool1_standings: sample_standings(),
            pool2_standings: sample_standings(),
            pool1_schedule: vec![],
            pool2_schedule: vec![],
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(r#""phase":"grand_final""#));
        assert!(json.contains(r#""arena_num":1"#));
        assert!(json.contains(r#""team_a":"alpha""#));
    }

    #[test]
    fn registration_state_serializes_teams_and_schedule() {
        let state = ScoreboardState::Registration {
            pool1: PoolRegistration {
                teams: vec![RegisteredTeam {
                    name: "alpha".to_string(),
                    students: vec!["Priya".to_string(), "Jamal".to_string()],
                    secret: "abc12345".to_string(),
                }],
                schedule: vec![],
            },
            pool2: PoolRegistration {
                teams: vec![],
                schedule: vec![],
            },
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(r#""phase":"registration""#));
        assert!(json.contains(r#""name":"alpha""#));
        assert!(json.contains(r#""students":["Priya","Jamal"]"#));
        assert!(json.contains(r#""secret":"abc12345""#));
        assert!(json.contains(r#""pool2":{"teams":[],"schedule":[]}"#));
    }

    #[test]
    fn champion_state_includes_both_pool_winners() {
        let state = ScoreboardState::Champion {
            winner: "alpha".to_string(),
            scores: HashMap::from([("alpha".to_string(), 9), ("delta".to_string(), 6)]),
            pool1_winner: "alpha".to_string(),
            pool2_winner: "delta".to_string(),
            third_place: Some("beta".to_string()),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(r#""phase":"champion""#));
        assert!(json.contains(r#""pool1_winner":"alpha""#));
        assert!(json.contains(r#""pool2_winner":"delta""#));
        assert!(json.contains(r#""third_place":"beta""#));
    }

    #[test]
    fn live_arena_state_serializes_flip_pending_positions_when_present() {
        let mut arena = sample_arena_state();
        arena.flip_pending_positions = Some(("B3".to_string(), "D5".to_string()));
        let json = serde_json::to_string(&arena).unwrap();
        assert!(json.contains(r#""flip_pending_positions":["B3","D5"]"#));
    }

    #[test]
    fn live_arena_state_serializes_flip_pending_positions_as_null_when_absent() {
        let arena = sample_arena_state();
        let json = serde_json::to_string(&arena).unwrap();
        assert!(json.contains(r#""flip_pending_positions":null"#));
    }

    #[test]
    fn champion_state_serializes_a_missing_third_place_as_null() {
        let state = ScoreboardState::Champion {
            winner: "alpha".to_string(),
            scores: HashMap::from([("alpha".to_string(), 9), ("delta".to_string(), 6)]),
            pool1_winner: "alpha".to_string(),
            pool2_winner: "delta".to_string(),
            third_place: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(r#""third_place":null"#));
    }
}
