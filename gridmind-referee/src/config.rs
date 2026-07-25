use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

const CONFIG_PATH: &str = "data/game_config.json";

fn default_paid_hint_pause_secs() -> u64 {
    30
}

fn default_message_gap_ms() -> u64 {
    50
}

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
    /// How long the arena actually holds `your_turn`/`wait` back after a
    /// resolved `hint_response`/`hint_rejected` bundled into the same
    /// batch (see `push_turn_signals`/`resolve_queued_hint` in
    /// `game_state.rs`) -- a real pause, matching what
    /// `physical_flip_offset_secs` already does for a `match`/`no_match`
    /// bundle (see `arena::send_all_delaying_turn_signal`). Long enough
    /// for the client's on-board MNIST decode (which involves reloading
    /// the DPU overlay) to finish before the recipient's own turn clock
    /// starts ticking against them. `#[serde(default)]` so an older
    /// `game_config.json` without this key still loads.
    #[serde(default = "default_paid_hint_pause_secs")]
    pub paid_hint_pause_secs: u64,
    pub hint_cap: u32,
    pub hint_cost: i32,
    /// Flat score change for an incorrect "match" claim -- deliberately
    /// standalone, not combined with `scoring_tiers` (see `tier_bonus`'s
    /// doc comment for why the two used to interact badly).
    pub wrong_match_penalty: i32,
    /// Flat score change when a turn times out with no action at all --
    /// standalone for the same reason `wrong_match_penalty` is, and kept
    /// separate from `scoring_tiers` so its value doesn't have to track
    /// whatever the tier table's own worst bracket happens to be.
    pub timeout_penalty: i32,
    /// Only ever applied to a *correct* match -- see `tier_bonus`'s doc
    /// comment. A wrong claim, a correct decline, and a timeout each score
    /// their own flat, speed-independent outcome instead.
    pub scoring_tiers: Vec<ScoringTier>,
    /// Fallback delay before sending a `your_turn`/`wait` pair that's
    /// bundled with something other than a `match`/`no_match`/hint
    /// resolution -- those two get their own real pauses instead
    /// (`physical_flip_offset_secs`/`paid_hint_pause_secs`, see
    /// `arena::send_all_delaying_turn_signal`). Some board clients only
    /// read the first message off a poll instead of draining every queued
    /// one, so a `your_turn` bundled into the same round-trip as anything
    /// else can be silently dropped without at least this much of a gap.
    /// Only takes effect when a turn signal is actually bundled with
    /// something -- a timeout-triggered switch with nothing queued for the
    /// new team still sends immediately.
    pub turn_signal_delay_ms: u64,
    /// Minimum real gap enforced between *every* individual outgoing
    /// message, anywhere the referee sends more than one in a row (see
    /// `arena::send_all`, `master::send_free_hint_fragments`,
    /// `master::send_riddle_to_known_teams`) -- so no two messages ever go
    /// out at literally the same instant. Deliberately much smaller than
    /// `turn_signal_delay_ms`: that one exists to give a client time to
    /// actually process a distinct logical batch (the outcome of a turn
    /// vs. the next turn's signal), this one just guarantees genuinely
    /// separate delivery instants -- a multi-fragment free hint sent to
    /// both teams would otherwise take `turn_signal_delay_ms` times the
    /// fragment count to fully deliver.
    #[serde(default = "default_message_gap_ms")]
    pub message_gap_ms: u64,
}

impl Default for GameConfig {
    fn default() -> Self {
        GameConfig {
            turn_timeout_secs: 120,
            physical_flip_offset_secs: 20,
            puzzle_race_secs: 120,
            free_hint_secs: 60,
            paid_hint_pause_secs: default_paid_hint_pause_secs(),
            hint_cap: 2,
            hint_cost: 1,
            wrong_match_penalty: -2,
            timeout_penalty: -3,
            scoring_tiers: vec![
                ScoringTier {
                    max_elapsed_secs: 20,
                    bonus: 2,
                },
                ScoringTier {
                    max_elapsed_secs: 60,
                    bonus: 0,
                },
                ScoringTier {
                    max_elapsed_secs: 100,
                    bonus: -2,
                },
            ],
            turn_signal_delay_ms: 1000,
            message_gap_ms: default_message_gap_ms(),
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

    pub fn paid_hint_pause(&self) -> Duration {
        Duration::from_secs(self.paid_hint_pause_secs)
    }

    pub fn turn_signal_delay(&self) -> Duration {
        Duration::from_millis(self.turn_signal_delay_ms)
    }

    pub fn message_gap(&self) -> Duration {
        Duration::from_millis(self.message_gap_ms)
    }

    /// Scoring bonus for a *correct match's* raw elapsed duration.
    /// Deliberately only called from the correct-match path -- earlier
    /// this applied to every outcome (wrong claims, declines, timeouts
    /// too), which meant a fast wrong guess could still net a positive
    /// score (e.g. +2 tier - 1 flat penalty = +1) since the speed bonus
    /// could outweigh a small flat penalty. Now a wrong claim, a correct
    /// decline, and a timeout each score their own flat, speed-independent
    /// outcome instead (see `wrong_match_penalty`/`timeout_penalty`), so
    /// being wrong is never rewarded no matter how fast it happens. Falls
    /// back to the last configured tier's bonus if every tier's
    /// `max_elapsed_secs` is exceeded, or to 0 if `scoring_tiers` is
    /// somehow empty.
    ///
    /// `elapsed` is expected to already exclude physical/hint delay time
    /// -- `GameState::receive_flip`/`receive_flip_both` and
    /// `resolve_hint`'s accepted path push `turn_start` forward by
    /// `physical_flip_offset_secs`/`paid_hint_pause_secs` the instant each
    /// delay is actually incurred, so `Instant::now() - turn_start` is
    /// already the team's genuine thinking time by the time this is
    /// called. No further subtraction happens here -- doing so on top of
    /// the now-already-excluded `turn_start` would double-count the same
    /// offset.
    pub fn tier_bonus(&self, elapsed: Duration) -> i32 {
        let effective_secs = elapsed.as_secs();
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
        // Straight wall-clock boundaries (0-20s/21-60s/61-100s) -- no
        // offset subtracted here anymore, since GameState now excludes
        // physical_flip_offset_secs/paid_hint_pause_secs from elapsed
        // directly (turn_start is pushed forward the instant each delay
        // is incurred), so by the time tier_bonus is called the physical
        // flip's own 20s is already excluded from `elapsed`.
        assert_eq!(cfg.tier_bonus(Duration::from_secs(0)), 2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(20)), 2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(21)), 0);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(60)), 0);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(61)), -2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(100)), -2);
        assert_eq!(cfg.tier_bonus(Duration::from_secs(101)), -2); // catch-all: last tier's bonus
        assert_eq!(cfg.tier_bonus(Duration::from_secs(150)), -2);
    }

    #[test]
    fn default_config_round_trips_through_json() {
        let cfg = GameConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: GameConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, parsed);
    }
}
