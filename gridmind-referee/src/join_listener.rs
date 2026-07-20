use crate::join_registry::JoinRegistry;
use crate::messages::StudentMessage;
use crate::p2p_client::P2pClient;
use crate::team_secrets::TeamSecrets;
use std::thread::sleep;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

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
) {
    for raw in raw_messages {
        if let Ok(StudentMessage::Join { team, mac, secret }) =
            serde_json::from_str::<StudentMessage>(raw)
        {
            if team_secrets.verify(&team, &secret) {
                registry.record(&team, &mac);
            }
        }
    }
}

/// Runs forever, polling `client`'s own inbox (expected to be constructed
/// with the lobby board ID, e.g. `"{master_id}-lobby"`) and feeding
/// whatever arrives through `process_join_messages`. Meant to be spawned
/// on its own thread, separate from `run_master`'s loop -- see "Why join
/// status is kept out of MasterState" in the Join Competition design doc
/// for why a shared loop would be too late to be useful here.
pub fn run_join_listener(client: P2pClient, registry: JoinRegistry, team_secrets: TeamSecrets) {
    loop {
        match client.receive_all() {
            Ok(messages) => process_join_messages(&messages, &registry, &team_secrets),
            Err(err) => eprintln!("join listener: receive_all error: {err:#}"),
        }
        sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_with_the_correct_secret_is_recorded() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();
        team_secrets.set("alpha", "abc12345".to_string());

        let raw = vec![
            r#"{"type":"join","team":"alpha","mac":"aa:aa:aa:aa:aa:aa","secret":"abc12345"}"#
                .to_string(),
        ];
        process_join_messages(&raw, &registry, &team_secrets);

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
        process_join_messages(&raw, &registry, &team_secrets);

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
        process_join_messages(&raw, &registry, &team_secrets);

        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn non_join_messages_are_ignored_without_error() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();

        let raw =
            vec![r#"{"type":"flip_both","team":"alpha","pos1":"A1","pos2":"A2"}"#.to_string()];
        process_join_messages(&raw, &registry, &team_secrets);

        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn garbage_input_does_not_panic() {
        let registry = JoinRegistry::new();
        let team_secrets = TeamSecrets::new();
        let raw = vec!["not json at all".to_string()];
        process_join_messages(&raw, &registry, &team_secrets);
        assert!(registry.snapshot().is_empty());
    }
}
