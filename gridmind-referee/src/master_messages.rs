use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Messages an Arena Agent sends to the Master.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
// ScoreUpdate carries many HashMap/Vec fields; the size difference vs MatchResult is expected for
// a wire-message enum that is never stored in a collection.
#[allow(clippy::large_enum_variant)]
pub enum ArenaToMaster {
    #[serde(rename = "score_update")]
    ScoreUpdate {
        arena: u32,
        pool: u32,
        scores: HashMap<String, i32>,
        pairs_found: usize,
        total_pairs: usize,
        matched: HashMap<String, String>,
        all_positions: Vec<String>,
        active_team: String,
        turn_seconds_remaining: u64,
        streak: HashMap<String, u32>,
        hints_used: HashMap<String, u32>,
        puzzle_winner: String,
        match_started_at_unix_ms: u64,
        is_paused: bool,
        /// `Some((pos1, pos2))` from the moment a pair's second card is
        /// revealed until its result is processed -- lets the scoreboard
        /// show a "flip the card now" banner for the physical referee.
        // serde errors on a missing key for Option<T> without this -- older
        // arena binaries that predate this field omit it entirely.
        #[serde(default)]
        flip_pending_positions: Option<(String, String)>,
    },
    #[serde(rename = "match_result")]
    MatchResult {
        arena: u32,
        pool: u32,
        winner: String,
        scores: HashMap<String, i32>,
        pairs_matched: HashMap<String, u32>,
    },
}

/// Messages the Master sends to an Arena Agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MasterToArena {
    #[serde(rename = "assign_match")]
    AssignMatch {
        arena: u32,
        pool: u32,
        team_a: String,
        team_a_id: String,
        team_b: String,
        team_b_id: String,
        grid_id: String,
        first_turn_team: String,
    },
    /// Operator-console overrides for a live match. See `web.rs`'s
    /// `/api/admin/*` routes and `arena.rs`'s handling in `run_one_match`.
    #[serde(rename = "admin_set_score")]
    AdminSetScore { team: String, score: i32 },
    #[serde(rename = "admin_pause")]
    AdminPause,
    #[serde(rename = "admin_resume")]
    AdminResume,
    /// Halts the match immediately with no result sent to the Master --
    /// the tournament schedule does not advance. The operator re-triggers
    /// `start_match` for the same matchup when ready to replay.
    #[serde(rename = "admin_stop")]
    AdminStop,
    /// Ends the match immediately, crediting whoever's currently ahead
    /// (same tie-break as a natural finish, see `GameState::winner`) --
    /// the tournament schedule advances normally, same as a real finish.
    #[serde(rename = "admin_finish")]
    AdminFinish,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_result_round_trips_from_json() {
        let json = r#"{"type":"match_result","arena":1,"pool":1,"winner":"alpha","scores":{"alpha":9,"beta":6},"pairs_matched":{"alpha":9,"beta":6}}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::MatchResult {
                arena,
                pool,
                winner,
                ..
            } => {
                assert_eq!(arena, 1);
                assert_eq!(pool, 1);
                assert_eq!(winner, "alpha");
            }
            other => panic!("expected MatchResult, got {other:?}"),
        }
    }

    #[test]
    fn score_update_round_trips_from_json() {
        let json = r#"{"type":"score_update","arena":1,"pool":1,"scores":{"alpha":5,"beta":3},"pairs_found":7,"total_pairs":15,"matched":{"A1":"dog"},"all_positions":["A1","A2"],"active_team":"alpha","turn_seconds_remaining":37,"streak":{"alpha":2,"beta":0},"hints_used":{"alpha":0,"beta":1},"puzzle_winner":"alpha","match_started_at_unix_ms":1784000000000,"is_paused":false}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::ScoreUpdate {
                active_team,
                turn_seconds_remaining,
                ..
            } => {
                assert_eq!(active_team, "alpha");
                assert_eq!(turn_seconds_remaining, 37);
            }
            other => panic!("expected ScoreUpdate, got {other:?}"),
        }
    }

    #[test]
    fn score_update_defaults_flip_pending_positions_to_none_when_absent() {
        let json = r#"{"type":"score_update","arena":1,"pool":1,"scores":{"alpha":5,"beta":3},"pairs_found":7,"total_pairs":15,"matched":{"A1":"dog"},"all_positions":["A1","A2"],"active_team":"alpha","turn_seconds_remaining":37,"streak":{"alpha":2,"beta":0},"hints_used":{"alpha":0,"beta":1},"puzzle_winner":"alpha","match_started_at_unix_ms":1784000000000,"is_paused":false}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::ScoreUpdate {
                flip_pending_positions,
                ..
            } => {
                assert_eq!(flip_pending_positions, None);
            }
            other => panic!("expected ScoreUpdate, got {other:?}"),
        }
    }

    #[test]
    fn score_update_round_trips_flip_pending_positions_when_present() {
        let json = r#"{"type":"score_update","arena":1,"pool":1,"scores":{"alpha":5,"beta":3},"pairs_found":7,"total_pairs":15,"matched":{"A1":"dog"},"all_positions":["A1","A2"],"active_team":"alpha","turn_seconds_remaining":37,"streak":{"alpha":2,"beta":0},"hints_used":{"alpha":0,"beta":1},"puzzle_winner":"alpha","match_started_at_unix_ms":1784000000000,"is_paused":false,"flip_pending_positions":["B3","D5"]}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::ScoreUpdate {
                flip_pending_positions,
                ..
            } => {
                assert_eq!(
                    flip_pending_positions,
                    Some(("B3".to_string(), "D5".to_string()))
                );
            }
            other => panic!("expected ScoreUpdate, got {other:?}"),
        }
    }

    #[test]
    fn assign_match_serializes_with_type_tag() {
        let msg = MasterToArena::AssignMatch {
            arena: 1,
            pool: 1,
            team_a: "alpha".into(),
            team_a_id: "test-team-alpha".into(),
            team_b: "beta".into(),
            team_b_id: "test-team-beta".into(),
            grid_id: "grid_1".into(),
            first_turn_team: "alpha".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: MasterToArena = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn admin_set_score_round_trips_from_json() {
        let json = r#"{"type":"admin_set_score","team":"alpha","score":-5}"#;
        let msg: MasterToArena = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            MasterToArena::AdminSetScore {
                team: "alpha".into(),
                score: -5,
            }
        );
    }

    #[test]
    fn admin_pause_resume_stop_finish_round_trip_from_json() {
        assert_eq!(
            serde_json::from_str::<MasterToArena>(r#"{"type":"admin_pause"}"#).unwrap(),
            MasterToArena::AdminPause
        );
        assert_eq!(
            serde_json::from_str::<MasterToArena>(r#"{"type":"admin_resume"}"#).unwrap(),
            MasterToArena::AdminResume
        );
        assert_eq!(
            serde_json::from_str::<MasterToArena>(r#"{"type":"admin_stop"}"#).unwrap(),
            MasterToArena::AdminStop
        );
        assert_eq!(
            serde_json::from_str::<MasterToArena>(r#"{"type":"admin_finish"}"#).unwrap(),
            MasterToArena::AdminFinish
        );
    }
}
