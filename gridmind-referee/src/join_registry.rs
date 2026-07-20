use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One team's most recently reported join: their MAC and when they sent
/// it. `joined_at` is used to compute "joined Xs ago" at request time
/// (see `src/web.rs`'s `/api/join-status` handler) -- it is never
/// serialized directly.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinInfo {
    pub mac: String,
    pub joined_at: Instant,
}

/// Shared, popup-scoped join status. Deliberately kept out of
/// `MasterState` -- see "Why join status is kept out of MasterState" in
/// the Join Competition design doc. Cheap to clone (an `Arc` under the
/// hood), matching the `MasterState` pattern.
#[derive(Clone)]
pub struct JoinRegistry {
    entries: Arc<Mutex<HashMap<String, JoinInfo>>>,
}

impl JoinRegistry {
    pub fn new() -> Self {
        JoinRegistry {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Records (or overwrites) a team's join. Each join naturally
    /// replaces whatever this team last reported -- no expiry needed at
    /// this event's scale (~40 teams total).
    pub fn record(&self, team: &str, mac: &str) {
        self.entries
            .lock()
            .expect("join registry lock poisoned")
            .insert(
                team.to_string(),
                JoinInfo {
                    mac: mac.to_string(),
                    joined_at: Instant::now(),
                },
            );
    }

    pub fn snapshot(&self) -> HashMap<String, JoinInfo> {
        self.entries
            .lock()
            .expect("join registry lock poisoned")
            .clone()
    }
}

impl Default for JoinRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_snapshot_contains_the_join() {
        let registry = JoinRegistry::new();
        registry.record("alpha", "aa:aa:aa:aa:aa:aa");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.get("alpha").unwrap().mac, "aa:aa:aa:aa:aa:aa");
    }

    #[test]
    fn snapshot_is_empty_when_nothing_has_joined() {
        let registry = JoinRegistry::new();
        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn a_second_join_overwrites_the_first_for_the_same_team() {
        let registry = JoinRegistry::new();
        registry.record("alpha", "aa:aa:aa:aa:aa:aa");
        registry.record("alpha", "bb:bb:bb:bb:bb:bb");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.get("alpha").unwrap().mac, "bb:bb:bb:bb:bb:bb");
    }

    #[test]
    fn two_different_teams_both_appear_in_the_snapshot() {
        let registry = JoinRegistry::new();
        registry.record("alpha", "aa:aa:aa:aa:aa:aa");
        registry.record("beta", "bb:bb:bb:bb:bb:bb");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
    }
}
