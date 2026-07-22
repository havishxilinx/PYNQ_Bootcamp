use crate::join_registry::JoinRegistry;
use crate::master::MasterState;
use crate::messages::{RefereeMessage, StudentMessage};
use crate::p2p_client::P2pClient;
use crate::scoreboard_state::{PregameState, ScoreboardState};
use crate::team_secrets::TeamSecrets;
use std::thread::sleep;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// If `team`'s match is currently in its puzzle-race pregame stage, sends
/// them the riddle right now. Closes the exact gap that caused riddles to
/// silently never reach a team that joined even slightly after their
/// match became "Ready": `master.rs`'s `prompt_and_assign` only sends the
/// riddle once, to whoever's MAC is already known via the join registry
/// at that instant -- a team that connects a moment later would otherwise
/// never receive it over the wire at all, with no error anywhere. Best-effort
/// (a send failure here is no worse than the original one-shot send was).
fn resend_riddle_if_pregame_active_for(team: &str, mac: &str, master_state: &MasterState, client: &P2pClient) {
    let ScoreboardState::LivePoolPlay {
        arena1_pregame,
        arena2_pregame,
        ..
    } = master_state.snapshot()
    else {
        return;
    };
    for pregame in [arena1_pregame, arena2_pregame].into_iter().flatten() {
        if let PregameState::PuzzleRace {
            team_a,
            team_b,
            riddle,
            ..
        } = *pregame
        {
            if team == team_a || team == team_b {
                if let Ok(payload) =
                    serde_json::to_string(&RefereeMessage::PregameRiddle { riddle })
                {
                    client.send(mac, &payload).ok();
                }
                return;
            }
        }
    }
}

/// Decodes each raw message as a `StudentMessage`; anything that isn't a
/// `Join` (or fails to parse at all -- e.g. a stray message from some
/// other flow) is silently ignored. A `Join` whose `secret` doesn't match
/// `team_secrets`' record for that team name is also silently dropped
/// (not recorded, no error sent back) -- matches the existing pattern in
/// `game_state.rs::receive_result`, which silently returns `None` for a
/// team name that isn't the current active team.
pub fn process_join_messages(
    raw_messages: &[String],
    registry: &JoinRegistry,
    team_secrets: &TeamSecrets,
    master_state: &MasterState,
    client: &P2pClient,
) {
    for raw in raw_messages {
        if let Ok(StudentMessage::Join { team, mac, secret }) =
            serde_json::from_str::<StudentMessage>(raw)
        {
            if team_secrets.verify(&team, &secret) {
                registry.record(&team, &mac);
                resend_riddle_if_pregame_active_for(&team, &mac, master_state, client);
            }
        }
    }
}

/// Runs forever, polling `client`'s own inbox (expected to be constructed
/// with the lobby board ID, e.g. `"{master_id}-lobby"`) and feeding
/// whatever arrives through `process_join_messages`. Meant to be spawned
/// on its own thread, separate from `run_master`'s loop -- see "Why join
/// status is kept out of MasterState" in the Join Competition design doc
/// for why a shared loop would be too late to be useful here. Reading
/// `master_state` here (to check for an active pregame stage) doesn't
/// violate that reasoning -- only writing join status into it would.
pub fn run_join_listener(
    client: P2pClient,
    registry: JoinRegistry,
    team_secrets: TeamSecrets,
    master_state: MasterState,
) {
    loop {
        match client.receive_all() {
            Ok(messages) => {
                process_join_messages(&messages, &registry, &team_secrets, &master_state, &client)
            }
            Err(err) => eprintln!("join listener: receive_all error: {err:#}"),
        }
        sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoreboard_state::{PoolPreview, PregameState};

    fn idle_master_state() -> MasterState {
        MasterState::new(ScoreboardState::Idle {
            pool1: PoolPreview {
                teams: vec![],
                total_matches: 0,
            },
            pool2: PoolPreview {
                teams: vec![],
                total_matches: 0,
            },
        })
    }

    /// `127.0.0.1:1` is reserved and never has anything listening -- fine
    /// for tests that must not actually trigger a send.
    fn unreachable_client() -> P2pClient {
        P2pClient::new("127.0.0.1:1", "testkey", "lobby")
    }

    #[test]
    fn a_join_with_the_correct_secret_is_recorded() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();
        team_secrets.set("alpha", "abc12345".to_string());

        let raw = vec![
            r#"{"type":"join","team":"alpha","mac":"aa:aa:aa:aa:aa:aa","secret":"abc12345"}"#
                .to_string(),
        ];
        process_join_messages(
            &raw,
            &registry,
            &team_secrets,
            &idle_master_state(),
            &unreachable_client(),
        );

        assert_eq!(
            registry.snapshot().get("alpha").unwrap().mac,
            "aa:aa:aa:aa:aa:aa"
        );
    }

    #[test]
    fn a_join_with_the_wrong_secret_is_dropped() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();
        team_secrets.set("alpha", "abc12345".to_string());

        let raw = vec![
            r#"{"type":"join","team":"alpha","mac":"aa:aa:aa:aa:aa:aa","secret":"wrongsecret"}"#
                .to_string(),
        ];
        process_join_messages(
            &raw,
            &registry,
            &team_secrets,
            &idle_master_state(),
            &unreachable_client(),
        );

        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn a_join_for_an_unregistered_team_is_dropped() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();

        let raw = vec![
            r#"{"type":"join","team":"ghost","mac":"aa:aa:aa:aa:aa:aa","secret":"anything1"}"#
                .to_string(),
        ];
        process_join_messages(
            &raw,
            &registry,
            &team_secrets,
            &idle_master_state(),
            &unreachable_client(),
        );

        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn non_join_messages_are_ignored_without_error() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();

        let raw =
            vec![r#"{"type":"flip_both","team":"alpha","pos1":"A1","pos2":"A2"}"#.to_string()];
        process_join_messages(
            &raw,
            &registry,
            &team_secrets,
            &idle_master_state(),
            &unreachable_client(),
        );

        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn garbage_input_does_not_panic() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();
        let raw = vec!["not json at all".to_string()];
        process_join_messages(
            &raw,
            &registry,
            &team_secrets,
            &idle_master_state(),
            &unreachable_client(),
        );
        assert!(registry.snapshot().is_empty());
    }

    fn live_pool_play_with_pregame(
        arena1_pregame: Option<Box<PregameState>>,
    ) -> ScoreboardState {
        ScoreboardState::LivePoolPlay {
            arena1: None,
            arena2: None,
            arena1_pregame,
            arena2_pregame: None,
            pool1_standings: vec![],
            pool2_standings: vec![],
            pool1_schedule: vec![],
            pool2_schedule: vec![],
            grand_final_ready: None,
        }
    }

    #[test]
    fn a_late_join_during_an_active_puzzle_race_resends_the_riddle() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();
        team_secrets.set("alpha", "abc12345".to_string());
        let master_state = MasterState::new(live_pool_play_with_pregame(Some(Box::new(
            PregameState::PuzzleRace {
                team_a: "alpha".to_string(),
                team_b: "beta".to_string(),
                deadline_unix_ms: 9_999_999_999_999,
                riddle: "what am I?".to_string(),
            },
        ))));

        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/send")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("id".into(), "aa:aa:aa:aa:aa:aa".into()),
                mockito::Matcher::UrlEncoded(
                    "message".into(),
                    r#"{"type":"pregame_riddle","riddle":"what am I?"}"#.into(),
                ),
            ]))
            .with_status(200)
            .create();
        let client = P2pClient::new(&server.host_with_port(), "testkey", "lobby");

        let raw = vec![
            r#"{"type":"join","team":"alpha","mac":"aa:aa:aa:aa:aa:aa","secret":"abc12345"}"#
                .to_string(),
        ];
        process_join_messages(&raw, &registry, &team_secrets, &master_state, &client);

        mock.assert();
    }

    #[test]
    fn a_join_for_a_team_not_in_the_active_pregame_does_not_resend() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();
        team_secrets.set("gamma", "abc12345".to_string());
        let master_state = MasterState::new(live_pool_play_with_pregame(Some(Box::new(
            PregameState::PuzzleRace {
                team_a: "alpha".to_string(),
                team_b: "beta".to_string(),
                deadline_unix_ms: 9_999_999_999_999,
                riddle: "what am I?".to_string(),
            },
        ))));

        // No mock registered at all -- any request here fails the test.
        let raw = vec![
            r#"{"type":"join","team":"gamma","mac":"cc:cc:cc:cc:cc:cc","secret":"abc12345"}"#
                .to_string(),
        ];
        process_join_messages(&raw, &registry, &team_secrets, &master_state, &unreachable_client());

        assert_eq!(
            registry.snapshot().get("gamma").unwrap().mac,
            "cc:cc:cc:cc:cc:cc"
        );
    }
}
