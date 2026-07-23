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
        /// URL of Genesis's live MJPEG view for this match, for the arena
        /// UI to embed directly -- `None` when Genesis isn't configured
        /// for this arena, this is a practice match, or the connected
        /// Genesis server predates the competition-mode streaming fix.
        /// `#[serde(default)]` so older arena binaries that predate this
        /// field still parse.
        #[serde(default)]
        genesis_stream_url: Option<String>,
        /// Whether Genesis is configured for this match at all (a
        /// `--genesis-url` was passed and this isn't a practice match) --
        /// unlike `genesis_stream_url`, this is fixed for the whole match
        /// and doesn't flicker if `admin_start_competition` is briefly slow
        /// or fails. The arena UI uses this (not the stream URL) to decide
        /// whether to reserve screen space for video at all, so a slow
        /// Genesis start doesn't cause the layout to jump around.
        #[serde(default)]
        genesis_configured: bool,
    },
    #[serde(rename = "match_result")]
    MatchResult {
        arena: u32,
        pool: u32,
        winner: String,
        scores: HashMap<String, i32>,
        pairs_matched: HashMap<String, u32>,
        /// True for a Practice Mode match (see `AdminCommand::StartPractice`)
        /// -- the Master clears that arena's live scoreboard state but never
        /// touches `Tournament`/pool standings for one of these. `#[serde(default)]`
        /// so older arena binaries that predate Practice Mode still parse.
        #[serde(default)]
        practice: bool,
    },
    /// Sent instead of `MatchResult` when the operator uses `AdminStop` --
    /// no winner/scores to report, since the match was voided, not
    /// finished. Lets the Master revert the schedule entry back to
    /// `Ready` (see `Tournament::void_live_match`) so the exact same
    /// matchup gets handed out again, instead of getting permanently
    /// stuck `Live` with no result ever arriving to unblock it.
    #[serde(rename = "match_voided")]
    MatchVoided {
        arena: u32,
        pool: u32,
        #[serde(default)]
        practice: bool,
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
    /// Starts a Practice Mode match: `team_a` plays alone against the
    /// referee's own built-in bot opponent (`game_state::BOT_TEAM_NAME`/
    /// `BOT_BOARD_ID`) -- no puzzle race, no free hint, no Genesis, and the
    /// result never touches tournament/pool standings. See
    /// `AdminCommand::StartPractice` and `web.rs`'s `/api/start-practice-match`.
    #[serde(rename = "assign_practice_match")]
    AssignPracticeMatch {
        team_a: String,
        team_a_id: String,
        grid_id: String,
    },
    /// Operator-console overrides for a live match. See `web.rs`'s
    /// `/api/admin/*` routes and `arena.rs`'s handling in `run_one_match`.
    #[serde(rename = "admin_set_score")]
    AdminSetScore { team: String, score: i32 },
    #[serde(rename = "admin_pause")]
    AdminPause,
    #[serde(rename = "admin_resume")]
    AdminResume,
    /// Halts the match immediately and reports `MatchVoided` (not
    /// `MatchResult`, no winner/scores) -- the schedule entry reverts to
    /// `Ready` so the exact same matchup is handed out again the next
    /// time this arena is free, rather than getting stuck `Live` forever.
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
    fn match_voided_round_trips_from_json() {
        let json = r#"{"type":"match_voided","arena":1,"pool":1}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ArenaToMaster::MatchVoided {
                arena: 1,
                pool: 1,
                practice: false,
            }
        );
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
    fn score_update_defaults_genesis_stream_url_to_none_when_absent() {
        let json = r#"{"type":"score_update","arena":1,"pool":1,"scores":{"alpha":5,"beta":3},"pairs_found":7,"total_pairs":15,"matched":{"A1":"dog"},"all_positions":["A1","A2"],"active_team":"alpha","turn_seconds_remaining":37,"streak":{"alpha":2,"beta":0},"hints_used":{"alpha":0,"beta":1},"puzzle_winner":"alpha","match_started_at_unix_ms":1784000000000,"is_paused":false}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::ScoreUpdate {
                genesis_stream_url,
                ..
            } => {
                assert_eq!(genesis_stream_url, None);
            }
            other => panic!("expected ScoreUpdate, got {other:?}"),
        }
    }

    #[test]
    fn score_update_round_trips_genesis_stream_url_when_present() {
        let json = r#"{"type":"score_update","arena":1,"pool":1,"scores":{"alpha":5,"beta":3},"pairs_found":7,"total_pairs":15,"matched":{"A1":"dog"},"all_positions":["A1","A2"],"active_team":"alpha","turn_seconds_remaining":37,"streak":{"alpha":2,"beta":0},"hints_used":{"alpha":0,"beta":1},"puzzle_winner":"alpha","match_started_at_unix_ms":1784000000000,"is_paused":false,"genesis_stream_url":"http://127.0.0.1:8080/stream/competition"}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::ScoreUpdate {
                genesis_stream_url,
                ..
            } => {
                assert_eq!(
                    genesis_stream_url,
                    Some("http://127.0.0.1:8080/stream/competition".to_string())
                );
            }
            other => panic!("expected ScoreUpdate, got {other:?}"),
        }
    }

    #[test]
    fn score_update_defaults_genesis_configured_to_false_when_absent() {
        let json = r#"{"type":"score_update","arena":1,"pool":1,"scores":{"alpha":5,"beta":3},"pairs_found":7,"total_pairs":15,"matched":{"A1":"dog"},"all_positions":["A1","A2"],"active_team":"alpha","turn_seconds_remaining":37,"streak":{"alpha":2,"beta":0},"hints_used":{"alpha":0,"beta":1},"puzzle_winner":"alpha","match_started_at_unix_ms":1784000000000,"is_paused":false}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::ScoreUpdate {
                genesis_configured, ..
            } => {
                assert!(!genesis_configured);
            }
            other => panic!("expected ScoreUpdate, got {other:?}"),
        }
    }

    #[test]
    fn score_update_genesis_configured_can_be_true_while_stream_url_is_still_none() {
        // Genesis can be configured for this match (a --genesis-url was
        // passed, it's not a practice match) while admin_start_competition
        // hasn't succeeded yet -- genesis_configured must stay true so the
        // arena UI's layout doesn't flicker, independent of the stream URL.
        let json = r#"{"type":"score_update","arena":1,"pool":1,"scores":{"alpha":5,"beta":3},"pairs_found":7,"total_pairs":15,"matched":{"A1":"dog"},"all_positions":["A1","A2"],"active_team":"alpha","turn_seconds_remaining":37,"streak":{"alpha":2,"beta":0},"hints_used":{"alpha":0,"beta":1},"puzzle_winner":"alpha","match_started_at_unix_ms":1784000000000,"is_paused":false,"genesis_configured":true}"#;
        let msg: ArenaToMaster = serde_json::from_str(json).unwrap();
        match msg {
            ArenaToMaster::ScoreUpdate {
                genesis_stream_url,
                genesis_configured,
                ..
            } => {
                assert!(genesis_configured);
                assert_eq!(genesis_stream_url, None);
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
