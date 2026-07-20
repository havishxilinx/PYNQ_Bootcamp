use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

const CONFIG_PATH: &str = "data/game_config.json";

/// One scoring bracket: teams acting within `max_elapsed_secs` (after
/// `physical_flip_offset_secs` has been subtracted) earn `bonus` points.
/// Tiers are checked in list order, so the last entry is effectively the
/// catch-all for anything slower -- its `max_elapsed_secs` value doesn't
/// matter as long as nothing needs to match beyond it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoringTier {
    pub max_elapsed_secs: u64,
    pub bonus: i32,
}

/// All tunable gameplay timing/scoring knobs, loaded from
/// `data/game_config.json` so they can be adjusted before an event without
/// a rebuild. See `GameConfig::default()` for the values this ships with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameConfig {
    pub turn_timeout_secs: u64,
    pub physical_flip_offset_secs: u64,
    pub puzzle_race_secs: u64,
    pub free_hint_secs: u64,
    pub hint_cap: u32,
    pub hint_cost: i32,
    pub scoring_tiers: Vec<ScoringTier>,
}

impl Default for GameConfig {
    fn default() -> Self {
        GameConfig {
            turn_timeout_secs: 120,
            physical_flip_offset_secs: 20,
            puzzle_race_secs: 120,
            free_hint_secs: 60,
            hint_cap: 2,
            hint_cost: 1,
            scoring_tiers: vec![
                ScoringTier {
                    max_elapsed_secs: 20,
                    bonus: 2,
                },
                ScoringTier {
                    max_elapsed_secs: 40,
                    bonus: 1,
                },
                ScoringTier {
                    max_elapsed_secs: 60,
                    bonus: 0,
                },
                ScoringTier {
                    max_elapsed_secs: 80,
                    bonus: -1,
                },
                ScoringTier {
                    max_elapsed_secs: 100,
                    bonus: -2,
                },
                ScoringTier {
                    max_elapsed_secs: 999_999,
                    bonus: -3,
                },
            ],
        }
    }
}

impl GameConfig {
    pub fn turn_timeout(&self) -> Duration {
        Duration::from_secs(self.turn_timeout_secs)
    }

    pub fn physical_flip_offset(&self) -> Duration {
        Duration::from_secs(self.physical_flip_offset_secs)
    }

    /// Scoring bonus/penalty for a raw elapsed duration, after subtracting
    /// the physical flip offset. Falls back to the last configured tier's
    /// bonus if every tier's `max_elapsed_secs` is exceeded (matches the
    /// unconditional `_ => -3` catch-all this replaced), or to 0 if
    /// `scoring_tiers` is somehow empty.
    pub fn tier_bonus(&self, elapsed: Duration) -> i32 {
        let effective_secs = elapsed
            .saturating_sub(self.physical_flip_offset())
            .as_secs();
        self.scoring_tiers
            .iter()
            .find(|tier| effective_secs <= tier.max_elapsed_secs)
            .or_else(|| self.scoring_tiers.last())
            .map(|tier| tier.bonus)
            .unwrap_or(0)
    }

    fn load_from(path: &str) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading game config at {path}"))?;
        serde_json::from_str(&text).with_context(|| format!("parsing game config at {path}"))
    }
}

static CONFIG: OnceLock<GameConfig> = OnceLock::new();

/// Loads `data/game_config.json` (relative to CWD, same convention as the
/// other `data/` files) once per process and caches it for the rest of the
/// run -- gameplay constants shouldn't shift mid-match. Falls back to
/// `GameConfig::default()` with a warning if the file is missing or
/// malformed, so a bad edit can't crash the referee outright.
pub fn get() -> &'static GameConfig {
    CONFIG.get_or_init(|| {
        GameConfig::load_from(CONFIG_PATH).unwrap_or_else(|err| {
            eprintln!("config: {err:#}; using built-in defaults");
            GameConfig::default()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_bonus_matches_default_boundaries() {
        let cfg = GameConfig::default();
        assert_eq!(cfg.tier_bonus(Duration::from_secs(0)), 2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(40)), 2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(41)), 1);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(60)), 1);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(61)), 0);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(80)), 0);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(81)), -1);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(100)), -1);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(101)), -2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(120)), -2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(121)), -3);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(150)), -3);
    }

    #[test]
    fn default_config_round_trips_through_json() {
        let cfg = GameConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: GameConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, parsed);
    }
}
