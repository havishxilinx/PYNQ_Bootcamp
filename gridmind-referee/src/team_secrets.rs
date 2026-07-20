use rand::distributions::Alphanumeric;
use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Shared team-name -> secret map, populated once per team at
/// registration and read by the join-listener thread to validate `Join`
/// messages. Deliberately not part of `MasterState` -- see "Why join
/// status is kept out of MasterState" in the Join Competition design doc;
/// the same reasoning applies here, since this is written from
/// `run_master`'s registration phase and read from a separate thread.
#[derive(Clone)]
pub struct TeamSecrets {
    secrets: Arc<Mutex<HashMap<String, String>>>,
}

impl TeamSecrets {
    pub fn new() -> Self {
        TeamSecrets {
            secrets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set(&self, team: &str, secret: String) {
        self.secrets
            .lock()
            .expect("team secrets lock poisoned")
            .insert(team.to_string(), secret);
    }

    /// Returns `false` for an unknown team name and for a wrong secret
    /// alike, so callers can't distinguish "no such team" from "bad
    /// secret" -- this prevents team-name enumeration by a rogue board
    /// probing Join messages.
    pub fn verify(&self, team: &str, secret: &str) -> bool {
        match self
            .secrets
            .lock()
            .expect("team secrets lock poisoned")
            .get(team)
        {
            Some(known) => known == secret,
            None => false,
        }
    }
}

impl Default for TeamSecrets {
    fn default() -> Self {
        Self::new()
    }
}

/// Generates a random 8-character alphanumeric secret -- short enough for
/// a student to copy into their own project code by hand.
pub fn generate_team_secret() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_accepts_the_matching_secret() {
        let secrets = TeamSecrets::new();
        secrets.set("alpha", "abc12345".to_string());
        assert!(secrets.verify("alpha", "abc12345"));
    }

    #[test]
    fn verify_rejects_a_wrong_secret() {
        let secrets = TeamSecrets::new();
        secrets.set("alpha", "abc12345".to_string());
        assert!(!secrets.verify("alpha", "wrongsecret"));
    }

    #[test]
    fn verify_rejects_an_unknown_team() {
        let secrets = TeamSecrets::new();
        assert!(!secrets.verify("ghost", "anything"));
    }

    #[test]
    fn set_overwrites_a_previous_secret_for_the_same_team() {
        let secrets = TeamSecrets::new();
        secrets.set("alpha", "first111".to_string());
        secrets.set("alpha", "second22".to_string());
        assert!(!secrets.verify("alpha", "first111"));
        assert!(secrets.verify("alpha", "second22"));
    }

    #[test]
    fn generate_team_secret_produces_eight_alphanumeric_characters() {
        let secret = generate_team_secret();
        assert_eq!(secret.len(), 8);
        assert!(secret.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn generate_team_secret_is_not_constant() {
        // Not a proof of randomness, but catches an accidentally-constant
        // implementation.
        let a = generate_team_secret();
        let b = generate_team_secret();
        assert_ne!(a, b);
    }
}
