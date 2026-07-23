use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Messages a student board sends to the Arena Agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StudentMessage {
    #[serde(rename = "flip")]
    Flip { team: String, pos: String },
    /// Reveal two positions in one round-trip instead of two sequential
    /// `flip` calls. Added alongside `Flip` (not replacing it) so both
    /// protocols are available side by side while the team compares them.
    #[serde(rename = "flip_both")]
    FlipBoth {
        team: String,
        pos1: String,
        pos2: String,
    },
    #[serde(rename = "report_result")]
    ReportResult {
        team: String,
        pos1: String,
        pos2: String,
        cls1: String,
        cls2: String,
        claim: String,
    },
    #[serde(rename = "hint_request")]
    HintRequest { team: String, object: String },
    /// Sent once by a student's board when it learns its arena assignment,
    /// to a dedicated "lobby" board ID (not an arena's ID) -- see the Join
    /// Competition design doc. `secret` is the per-team secret issued at
    /// registration; a mismatch is silently dropped by the listener, not
    /// rejected with an error response.
    #[serde(rename = "join")]
    Join {
        team: String,
        mac: String,
        secret: String,
    },
}

/// Messages the Arena Agent sends to student boards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RefereeMessage {
    #[serde(rename = "game_start")]
    GameStart {
        teams: Vec<String>,
        total_pairs: usize,
        /// Which Genesis simulated arm belongs to the recipient of this
        /// specific message -- 0 or 1. `competition_card_flip.py`'s scene
        /// always builds `"robots": [robot_red, robot_blue]` in that fixed
        /// order (0=red/left, 1=blue/right); there's no way for a student
        /// to discover or choose their arm on their own, so the referee
        /// assigns it deterministically: board 0 is the puzzle-race winner
        /// who moves first, board 1 is the other team. Always present
        /// (unlike the two fields below) since it costs nothing to send,
        /// but only meaningful when `genesis_team_id` is `Some` -- clients
        /// (the `pynqsim` student library) must treat this as undefined
        /// and not act on it when Genesis isn't configured for this match.
        robot_id: u32,
        /// The fixed team id (`"team_red"` for `robot_id` 0, `"team_blue"`
        /// for `robot_id` 1) this student's board should pass to its own
        /// `pynqsim.SimulationClient.join_competition(team_id)` call --
        /// Genesis's competition mode has no concept of GridMind's actual
        /// team names, only these two hardcoded ids. `None` when Genesis
        /// isn't configured for this arena.
        #[serde(default)]
        genesis_team_id: Option<String>,
        /// Base URL of the Genesis server this match's scene lives on,
        /// for the student's `SimulationClient` to connect to. `None`
        /// when Genesis isn't configured for this arena.
        #[serde(default)]
        genesis_url: Option<String>,
    },
    /// Sent to both teams during the pre-game window. Whoever solves it
    /// first is judged manually by the human operator (via the existing
    /// match-assign popup buttons) -- there is no wire message carrying a
    /// team's answer back to the referee, by explicit design decision.
    #[serde(rename = "pregame_riddle")]
    PregameRiddle { riddle: String },
    /// One fragment of the shared free hint, delivered identically to both
    /// teams. `index`/`total` let a team know how many more to expect and
    /// in what order to assemble them. Plain text -- previously delivered
    /// as a QR-encoded PNG a team had to decode themselves; simplified to
    /// a direct string since nothing about the hint actually required a
    /// vision round-trip.
    #[serde(rename = "free_hint_fragment")]
    FreeHintFragment {
        index: u32,
        total: u32,
        text: String,
    },
    #[serde(rename = "your_turn")]
    YourTurn { flip_num: u32 },
    #[serde(rename = "wait")]
    Wait { active_team: String },
    #[serde(rename = "card_revealed")]
    CardRevealed { pos: String },
    #[serde(rename = "invalid")]
    Invalid { reason: String },
    #[serde(rename = "match")]
    Match {
        cls: String,
        pos1: String,
        pos2: String,
        scorer: String,
        scores: HashMap<String, i32>,
        remaining: usize,
    },
    /// Deliberately doesn't carry the real classes at `pos1`/`pos2` --
    /// scoring is always decided server-side against the grid's real
    /// answer key regardless of what a client thinks it saw, so a
    /// misdetected pair has to be caught by re-observing it, not by the
    /// referee handing out the answer on a wrong guess.
    #[serde(rename = "no_match")]
    NoMatch {
        pos1: String,
        pos2: String,
        scores: HashMap<String, i32>,
    },
    #[serde(rename = "game_over")]
    GameOver {
        winner: String,
        scores: HashMap<String, i32>,
    },
    /// Row and column as MNIST-style digit images, each base64-encoded
    /// PNGs -- replaces the earlier plain-text riddle.
    #[serde(rename = "hint_response")]
    HintResponse {
        row_digit_png_base64: String,
        col_digit_png_base64: String,
    },
    #[serde(rename = "hint_rejected")]
    HintRejected { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_message_serializes_with_type_tag() {
        let msg = StudentMessage::Flip {
            team: "alpha".into(),
            pos: "B3".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"flip","team":"alpha","pos":"B3"}"#);
    }

    #[test]
    fn flip_message_round_trips_from_json() {
        let json = r#"{"type":"flip","team":"alpha","pos":"B3"}"#;
        let msg: StudentMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            StudentMessage::Flip {
                team: "alpha".into(),
                pos: "B3".into()
            }
        );
    }

    #[test]
    fn flip_both_message_round_trips_from_json() {
        let json = r#"{"type":"flip_both","team":"alpha","pos1":"B3","pos2":"D5"}"#;
        let msg: StudentMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            StudentMessage::FlipBoth {
                team: "alpha".into(),
                pos1: "B3".into(),
                pos2: "D5".into(),
            }
        );
    }

    #[test]
    fn report_result_round_trips_from_json() {
        let json = r#"{"type":"report_result","team":"alpha","pos1":"A6","pos2":"C3","cls1":"boat","cls2":"boat","claim":"match"}"#;
        let msg: StudentMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            StudentMessage::ReportResult {
                team: "alpha".into(),
                pos1: "A6".into(),
                pos2: "C3".into(),
                cls1: "boat".into(),
                cls2: "boat".into(),
                claim: "match".into(),
            }
        );
    }

    #[test]
    fn hint_request_round_trips_from_json() {
        let json = r#"{"type":"hint_request","team":"alpha","object":"boat"}"#;
        let msg: StudentMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            StudentMessage::HintRequest {
                team: "alpha".into(),
                object: "boat".into()
            }
        );
    }

    #[test]
    fn hint_response_serializes_with_type_tag() {
        let msg = RefereeMessage::HintResponse {
            row_digit_png_base64: "rowpng".into(),
            col_digit_png_base64: "colpng".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"hint_response","row_digit_png_base64":"rowpng","col_digit_png_base64":"colpng"}"#
        );
    }

    #[test]
    fn hint_rejected_serializes_with_type_tag() {
        let msg = RefereeMessage::HintRejected {
            reason: "already fully resolved".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"hint_rejected","reason":"already fully resolved"}"#
        );
    }

    #[test]
    fn join_message_round_trips_from_json() {
        let json =
            r#"{"type":"join","team":"alpha","mac":"aa:aa:aa:aa:aa:aa","secret":"abc12345"}"#;
        let msg: StudentMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            StudentMessage::Join {
                team: "alpha".into(),
                mac: "aa:aa:aa:aa:aa:aa".into(),
                secret: "abc12345".into(),
            }
        );
    }

    #[test]
    fn join_message_serializes_with_type_tag() {
        let msg = StudentMessage::Join {
            team: "alpha".into(),
            mac: "aa:aa:aa:aa:aa:aa".into(),
            secret: "abc12345".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"join","team":"alpha","mac":"aa:aa:aa:aa:aa:aa","secret":"abc12345"}"#
        );
    }

    #[test]
    fn pregame_riddle_serializes_with_type_tag() {
        let msg = RefereeMessage::PregameRiddle {
            riddle: "what am I?".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"pregame_riddle","riddle":"what am I?"}"#);
    }

    #[test]
    fn free_hint_fragment_serializes_with_type_tag() {
        let msg = RefereeMessage::FreeHintFragment {
            index: 0,
            total: 3,
            text: "I bark".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"free_hint_fragment","index":0,"total":3,"text":"I bark"}"#
        );
    }

    #[test]
    fn game_start_round_trips_robot_id_and_genesis_fields() {
        let msg = RefereeMessage::GameStart {
            teams: vec!["alpha".to_string(), "beta".to_string()],
            total_pairs: 15,
            robot_id: 1,
            genesis_team_id: Some("team_blue".to_string()),
            genesis_url: Some("http://127.0.0.1:9002".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RefereeMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn game_start_defaults_genesis_fields_to_none_when_absent() {
        let json =
            r#"{"type":"game_start","teams":["alpha","beta"],"total_pairs":15,"robot_id":0}"#;
        let msg: RefereeMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            RefereeMessage::GameStart {
                teams: vec!["alpha".to_string(), "beta".to_string()],
                total_pairs: 15,
                robot_id: 0,
                genesis_team_id: None,
                genesis_url: None,
            }
        );
    }
}
