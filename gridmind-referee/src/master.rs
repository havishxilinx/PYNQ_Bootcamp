use crate::pool::{
    build_schedule_entries, round_robin_schedule, MatchStatus, Matchup, PoolStandings,
    ScheduleEntry,
};

/// Snapshots a pool's current standings, ranked by wins then pairs
/// matched (the same tie-break `PoolStandings::winner` uses), for
/// display in the live scoreboard's standings table.
fn standings_snapshot(pool: &PoolStandings) -> Vec<TeamStanding> {
    let mut standings: Vec<TeamStanding> = pool
        .teams()
        .iter()
        .map(|team| TeamStanding {
            team: team.clone(),
            wins: pool.wins(team),
            losses: pool.losses(team),
            pairs_matched: pool.pairs_matched(team),
        })
        .collect();
    standings.sort_by(|a, b| {
        b.wins
            .cmp(&a.wins)
            .then(b.pairs_matched.cmp(&a.pairs_matched))
    });
    standings
}

/// What the orchestrator wants to happen next.
#[derive(Debug, Clone, PartialEq)]
pub enum NextAction {
    /// Assign this matchup to the given arena/pool.
    AssignMatch {
        arena: u32,
        pool: u32,
        matchup: Matchup,
        grid_id: String,
    },
    /// Both pools are done; assign the Grand Final to this arena.
    AssignGrandFinal {
        arena: u32,
        matchup: Matchup,
        grid_id: String,
    },
    /// The Grand Final is done; this team is champion.
    Champion { winner: String },
    /// Nothing to do right now (e.g. waiting on an in-progress match).
    Wait,
}

/// Tracks whether the Grand Final matchup has been decided and, if so,
/// whether it's already been handed out. `Option<Matchup>` alone can't
/// distinguish "never decided" from "decided and already assigned" — both
/// look like `None` after a `.take()` — which caused a real bug: calling
/// `next_action` for arena 2 right after arena 1 had just been assigned
/// the Grand Final would see `None`, treat it as "not yet decided", and
/// build and hand out a SECOND Grand Final assignment to arena 2. Since
/// `next_action(1)` then `next_action(2)` are called back-to-back on every
/// single loop iteration in `run_master`, this fired every time.
#[derive(Debug, Clone, PartialEq)]
enum GrandFinalState {
    NotReady,
    Ready(Matchup),
    Assigned,
}

pub struct Tournament {
    pool1_standings: PoolStandings,
    pool2_standings: PoolStandings,
    pool1_schedule: Vec<ScheduleEntry>,
    pool2_schedule: Vec<ScheduleEntry>,
    grand_final: GrandFinalState,
    champion: Option<String>,
    used_pregame_riddles: std::collections::HashSet<String>,
}

impl Tournament {
    pub fn new(pool1_teams: Vec<String>, pool2_teams: Vec<String>, grid_id: &str) -> Self {
        // Two-team dry-run mode: with exactly two teams total, force a
        // one-team-per-pool split regardless of how they're currently
        // divided (an operator could have moved both into the same pool
        // via the registration UI's "Move to Pool" button). Each
        // one-team pool is trivially "complete" (zero required
        // round-robin matches), so `next_action` goes straight to a
        // single Grand-Final-style match between them, reusing the
        // existing Grand Final machinery -- including its
        // champion-on-result wiring -- instead of a second code path.
        // Without this normalization, both teams ending up in one pool
        // would later panic in `pool_winners()`, which assumes every
        // pool has at least one team.
        let (pool1_teams, pool2_teams) = if pool1_teams.len() + pool2_teams.len() == 2 {
            let mut all: Vec<String> = pool1_teams.into_iter().chain(pool2_teams).collect();
            let team_b = all.pop().expect("checked total len == 2 above");
            let team_a = all.pop().expect("checked total len == 2 above");
            (vec![team_a], vec![team_b])
        } else {
            (pool1_teams, pool2_teams)
        };

        // Every match shares one grid_id here -- fine for --config mode
        // (which has exactly one configured grid) and for the many tests
        // that construct a Tournament directly by team names. The live
        // registration path builds its own grid-assigned schedules via
        // `build_schedule_entries_with_grids` and hands them to
        // `Tournament::from_schedules` instead of calling `new`.
        let pool1_schedule = build_schedule_entries(&pool1_teams, grid_id);
        let pool2_schedule = build_schedule_entries(&pool2_teams, grid_id);
        Tournament {
            pool1_standings: PoolStandings::new(pool1_teams),
            pool2_standings: PoolStandings::new(pool2_teams),
            pool1_schedule,
            pool2_schedule,
            grand_final: GrandFinalState::NotReady,
            champion: None,
            used_pregame_riddles: std::collections::HashSet::new(),
        }
    }

    /// Builds a `Tournament` from already-built schedules (e.g. ones with
    /// per-match grid assignments from `build_schedule_entries_with_grids`,
    /// or ones loaded back from a saved tournament state) instead of
    /// building fresh ones internally the way `new` does.
    pub fn from_schedules(
        pool1_teams: Vec<String>,
        pool2_teams: Vec<String>,
        pool1_schedule: Vec<ScheduleEntry>,
        pool2_schedule: Vec<ScheduleEntry>,
    ) -> Self {
        Tournament {
            pool1_standings: PoolStandings::new(pool1_teams),
            pool2_standings: PoolStandings::new(pool2_teams),
            pool1_schedule,
            pool2_schedule,
            grand_final: GrandFinalState::NotReady,
            champion: None,
            used_pregame_riddles: std::collections::HashSet::new(),
        }
    }

    /// Picks a riddle not yet used this tournament from `pool`, tracking
    /// it so the same riddle isn't handed to two different matches before
    /// the whole pool has been exhausted.
    pub fn pick_pregame_riddle(
        &mut self,
        pool: &[crate::content_pools::PregameRiddle],
    ) -> Option<crate::content_pools::PregameRiddle> {
        crate::content_pools::pick_unused(pool, &mut self.used_pregame_riddles).cloned()
    }

    pub fn pool1_schedule(&self) -> &[ScheduleEntry] {
        &self.pool1_schedule
    }

    pub fn pool2_schedule(&self) -> &[ScheduleEntry] {
        &self.pool2_schedule
    }

    /// Call once a match result arrives from an arena. `pool` is 1 or 2,
    /// or 0 to mean "this was the Grand Final". Marks the matching
    /// schedule entry Complete and unlocks the next Locked entry (if
    /// any) in that pool to Ready.
    pub fn record_result(
        &mut self,
        pool: u32,
        winner: &str,
        loser: &str,
        winner_pairs: u32,
        loser_pairs: u32,
    ) {
        match pool {
            1 => {
                self.pool1_standings
                    .record_match(winner, loser, winner_pairs, loser_pairs);
                complete_and_unlock_next(&mut self.pool1_schedule, winner);
            }
            2 => {
                self.pool2_standings
                    .record_match(winner, loser, winner_pairs, loser_pairs);
                complete_and_unlock_next(&mut self.pool2_schedule, winner);
            }
            0 => self.champion = Some(winner.to_string()),
            other => panic!("unknown pool id: {other}"),
        }
    }

    /// Both pools' winners, for the Champion screen. Only meaningful once
    /// both pools are complete (true by the time a Grand Final result
    /// arrives, since the Grand Final can't be assigned any earlier).
    pub fn pool_winners(&self) -> (String, String) {
        (self.pool1_standings.winner(), self.pool2_standings.winner())
    }

    /// The tournament's 3rd place: whichever pool's runner-up (the
    /// pool's own 2nd place, by the same wins/pairs-matched ranking used
    /// for pool winners) has the better record. There's no separate
    /// 3rd-place match -- this is decided from pool standings alone at
    /// Grand-Final time. `None` if both pools only had one team each.
    pub fn third_place(&self) -> Option<String> {
        let candidate = |standings: &PoolStandings| -> Option<(String, (u32, u32))> {
            let team = standings.runner_up()?;
            let key = (standings.wins(&team), standings.pairs_matched(&team));
            Some((team, key))
        };
        match (
            candidate(&self.pool1_standings),
            candidate(&self.pool2_standings),
        ) {
            (Some((team1, key1)), Some((team2, key2))) => {
                Some(if key2 > key1 { team2 } else { team1 })
            }
            (Some((team1, _)), None) => Some(team1),
            (None, Some((team2, _))) => Some(team2),
            (None, None) => None,
        }
    }

    /// What should happen next for the given arena (1 or 2 during pool
    /// play; either arena can host the Grand Final once it's ready).
    pub fn next_action(&mut self, arena: u32) -> NextAction {
        if let Some(winner) = &self.champion {
            return NextAction::Champion {
                winner: winner.clone(),
            };
        }

        if self.pool1_standings.is_complete() && self.pool2_standings.is_complete() {
            if self.grand_final == GrandFinalState::NotReady {
                self.grand_final = GrandFinalState::Ready(Matchup {
                    team_a: self.pool1_standings.winner(),
                    team_b: self.pool2_standings.winner(),
                });
            }
            // The Grand Final isn't its own schedule entry with a
            // pre-assigned grid (it's decided dynamically once both pools
            // finish) -- reuse pool 1's first match's grid rather than
            // threading a whole separate grid-pool reference through
            // `Tournament` for this one late assignment.
            let grand_final_grid_id = self
                .pool1_schedule
                .first()
                .map(|e| e.grid_id.clone())
                .unwrap_or_else(|| "example_grid.json".to_string());
            return match std::mem::replace(&mut self.grand_final, GrandFinalState::Assigned) {
                GrandFinalState::Ready(matchup) => NextAction::AssignGrandFinal {
                    arena,
                    matchup,
                    grid_id: grand_final_grid_id,
                },
                // Already assigned (to some arena, possibly this one on a
                // prior call) and waiting on a result, or was already
                // `Assigned` -- either way there's nothing new to hand out.
                GrandFinalState::Assigned | GrandFinalState::NotReady => NextAction::Wait,
            };
        }

        let schedule = match arena {
            1 => &mut self.pool1_schedule,
            2 => &mut self.pool2_schedule,
            other => panic!("unknown arena id: {other}"),
        };
        match schedule.iter_mut().find(|e| e.status == MatchStatus::Ready) {
            Some(entry) => {
                entry.status = MatchStatus::Live;
                NextAction::AssignMatch {
                    arena,
                    pool: arena,
                    matchup: Matchup {
                        team_a: entry.team_a.clone(),
                        team_b: entry.team_b.clone(),
                    },
                    grid_id: entry.grid_id.clone(),
                }
            }
            None => NextAction::Wait,
        }
    }
}

/// Marks the first Live entry matching this result Complete, then
/// unlocks the next Locked entry (if any) to Ready. There's always at
/// most one Live entry per pool at a time (the arena serializes
/// matches), so "first Live entry" is unambiguous.
fn complete_and_unlock_next(schedule: &mut [ScheduleEntry], winner: &str) {
    let Some(live_idx) = schedule.iter().position(|e| e.status == MatchStatus::Live) else {
        panic!("record_result called with no Live entry in schedule -- result arrived for an unassigned match");
    };
    schedule[live_idx].status = MatchStatus::Complete {
        winner: winner.to_string(),
    };
    if let Some(next) = schedule.get_mut(live_idx + 1) {
        if next.status == MatchStatus::Locked {
            next.status = MatchStatus::Ready;
        }
    }
}

use crate::scoreboard_state::{
    GrandFinalReady, LiveArenaState, PoolPreview, PoolRegistration, PregameState, RegisteredTeam,
    ScoreboardState, TeamStanding,
};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// Bridges the synchronous Tournament orchestration thread and the async
/// web server: the orchestration thread calls `update` whenever
/// scoreboard-relevant state changes; the web server's WebSocket handlers
/// call `subscribe` to receive those changes without polling.
///
/// `update` writes the Mutex and sends on the channel as two separate
/// steps, so a `snapshot()` and a `subscribe()`'d receiver's `borrow()`
/// can transiently disagree during a concurrent `update`. Use one or the
/// other within a single logical read, not both.
#[derive(Clone)]
pub struct MasterState {
    current: Arc<Mutex<ScoreboardState>>,
    sender: watch::Sender<ScoreboardState>,
}

impl MasterState {
    pub fn new(initial: ScoreboardState) -> Self {
        let (sender, _receiver) = watch::channel(initial.clone());
        MasterState {
            current: Arc::new(Mutex::new(initial)),
            sender,
        }
    }

    pub fn update(&self, new_state: ScoreboardState) {
        *self.current.lock().expect("scoreboard state lock poisoned") = new_state.clone();
        // A send error just means no browsers are connected right now --
        // not a failure, since `subscribe()` always sees the latest value
        // immediately regardless of when it was called.
        let _ = self.sender.send(new_state);
    }

    pub fn snapshot(&self) -> ScoreboardState {
        self.current
            .lock()
            .expect("scoreboard state lock poisoned")
            .clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<ScoreboardState> {
        self.sender.subscribe()
    }
}

/// What the operator submits from the match-assign popup: who solved
/// the puzzle first, plus both teams' board MACs (entered fresh every
/// match -- see design doc for why no caching).
#[derive(Debug, Clone, PartialEq)]
pub struct MatchStartInput {
    pub puzzle_winner: String,
    pub team_a_mac: String,
    pub team_b_mac: String,
}

/// An operator-console override for whichever match is currently live on
/// an arena. Sent from `web.rs`'s `/api/admin/*` routes, forwarded to the
/// actual arena process as a `MasterToArena` admin variant by
/// `run_arena_assignment_loop` (the arena binary itself has no HTTP
/// server of its own -- see `PROJECT_STATE.md`'s architecture notes).
#[derive(Debug, Clone, PartialEq)]
pub enum AdminCommand {
    SetScore { team: String, score: i32 },
    Pause,
    Resume,
    Stop,
    Finish,
}

impl AdminCommand {
    fn into_wire_message(self) -> crate::master_messages::MasterToArena {
        use crate::master_messages::MasterToArena;
        match self {
            AdminCommand::SetScore { team, score } => MasterToArena::AdminSetScore { team, score },
            AdminCommand::Pause => MasterToArena::AdminPause,
            AdminCommand::Resume => MasterToArena::AdminResume,
            AdminCommand::Stop => MasterToArena::AdminStop,
            AdminCommand::Finish => MasterToArena::AdminFinish,
        }
    }
}

/// One operator action during the Registration phase, sent over a
/// single consolidated channel (mirroring the existing typed-payload
/// pattern used by `match_start: Sender<MatchStartInput>`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegistrationAction {
    RegisterTeam { name: String, students: Vec<String> },
    MoveTeam { name: String, to_pool: u32 },
    CloseRegistration,
}

/// Places `team` into whichever of `pool1`/`pool2` currently has fewer
/// teams; a coin-flip breaks an exact tie. Keeps pools balanced without
/// requiring the operator to think about it during live registration.
/// Returns which pool (1 or 2) the team was placed into.
fn assign_to_smaller_pool(team: String, pool1: &mut Vec<String>, pool2: &mut Vec<String>) -> u32 {
    use rand::Rng;
    let goes_to_pool1 = match pool1.len().cmp(&pool2.len()) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => rand::thread_rng().gen::<bool>(),
    };
    if goes_to_pool1 {
        pool1.push(team);
        1
    } else {
        pool2.push(team);
        2
    }
}

/// Sender halves of the channels the operator's web actions feed into.
/// Cloned into axum's app state in a later task so POST handlers can
/// signal the synchronous orchestration thread. `match_start_arena1` and
/// `match_start_arena2` are separate (not one shared channel) because
/// each arena now runs its own independent assignment thread -- a
/// shared channel would let arena 2's submission accidentally be
/// received by arena 1's thread, or vice versa.
#[derive(Clone)]
pub struct OperatorChannels {
    pub start_tournament: tokio::sync::mpsc::Sender<()>,
    pub match_start_arena1: tokio::sync::mpsc::Sender<MatchStartInput>,
    pub match_start_arena2: tokio::sync::mpsc::Sender<MatchStartInput>,
    pub registration: tokio::sync::mpsc::Sender<RegistrationAction>,
    pub admin_arena1: tokio::sync::mpsc::Sender<AdminCommand>,
    pub admin_arena2: tokio::sync::mpsc::Sender<AdminCommand>,
}

/// Receiver halves of the channels `run_master` drains. Bundled so
/// `run_master`'s parameter list stays within clippy's 7-arg limit.
pub struct MasterReceivers {
    pub start: tokio::sync::mpsc::Receiver<()>,
    pub match_start_arena1: tokio::sync::mpsc::Receiver<MatchStartInput>,
    pub match_start_arena2: tokio::sync::mpsc::Receiver<MatchStartInput>,
    pub registration: tokio::sync::mpsc::Receiver<RegistrationAction>,
    pub admin_arena1: tokio::sync::mpsc::Receiver<AdminCommand>,
    pub admin_arena2: tokio::sync::mpsc::Receiver<AdminCommand>,
}

/// Builds a linked (sender-bundle, receivers) pair. The receivers are
/// consumed by `run_master`; the senders are cloned into the web server.
pub fn operator_channels() -> (OperatorChannels, MasterReceivers) {
    let (start_tx, start_rx) = tokio::sync::mpsc::channel(1);
    let (match_start_a1_tx, match_start_a1_rx) = tokio::sync::mpsc::channel(1);
    let (match_start_a2_tx, match_start_a2_rx) = tokio::sync::mpsc::channel(1);
    // 16, not 1: unlike the one-at-a-time start/puzzle-winner signals,
    // registrations can arrive in a quick burst as multiple teams sign
    // up back-to-back before run_master drains the channel.
    let (registration_tx, registration_rx) = tokio::sync::mpsc::channel(16);
    // 8, not 1: an operator could plausibly fire off a couple of quick
    // corrections (e.g. set-score then resume) before the arena thread's
    // next poll tick drains them.
    let (admin_a1_tx, admin_a1_rx) = tokio::sync::mpsc::channel(8);
    let (admin_a2_tx, admin_a2_rx) = tokio::sync::mpsc::channel(8);
    (
        OperatorChannels {
            start_tournament: start_tx,
            match_start_arena1: match_start_a1_tx,
            match_start_arena2: match_start_a2_tx,
            registration: registration_tx,
            admin_arena1: admin_a1_tx,
            admin_arena2: admin_a2_tx,
        },
        MasterReceivers {
            start: start_rx,
            match_start_arena1: match_start_a1_rx,
            match_start_arena2: match_start_a2_rx,
            registration: registration_rx,
            admin_arena1: admin_a1_rx,
            admin_arena2: admin_a2_rx,
        },
    )
}

pub fn initial_scoreboard_state(pool1_teams: &[String], pool2_teams: &[String]) -> ScoreboardState {
    ScoreboardState::Idle {
        pool1: PoolPreview {
            teams: pool1_teams.to_vec(),
            total_matches: round_robin_schedule(pool1_teams).len(),
        },
        pool2: PoolPreview {
            teams: pool2_teams.to_vec(),
            total_matches: round_robin_schedule(pool2_teams).len(),
        },
    }
}

/// A `Registration` state with no teams in either pool, suitable as the
/// starting state when teams will register dynamically rather than
/// being pre-loaded from a config file.
pub fn empty_registration_state() -> ScoreboardState {
    ScoreboardState::Registration {
        pool1: PoolRegistration {
            teams: vec![],
            schedule: vec![],
        },
        pool2: PoolRegistration {
            teams: vec![],
            schedule: vec![],
        },
    }
}

/// score_update's `scores` map always has exactly the match's two team
/// names as keys (GameState seeds both with 0 at match start) -- sorted
/// alphabetically here purely for stable, non-flickering display, not
/// because "team_a" vs "team_b" has any other meaning downstream.
fn team_names_from_scores(scores: &std::collections::HashMap<String, i32>) -> (String, String) {
    let mut names: Vec<String> = scores.keys().cloned().collect();
    names.sort();
    let team_a = names.first().cloned().unwrap_or_default();
    let team_b = names.get(1).cloned().unwrap_or_default();
    (team_a, team_b)
}

use crate::master_messages::{ArenaToMaster, MasterToArena};
use crate::messages::RefereeMessage;
use crate::p2p_client::P2pClient;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct PoolsConfig {
    pub pool1_teams: Vec<(String, String)>, // (team_name, board_id)
    pub pool2_teams: Vec<(String, String)>,
    pub grid_id: String,
}

pub fn load_pools_config(path: &str) -> Result<PoolsConfig> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read pools config at {path}"))?;
    serde_json::from_str(&data).with_context(|| format!("failed to parse pools config at {path}"))
}

const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Runs the Master's orchestration loop. When `config` is `Some`, uses the
/// pre-loaded pools config (static team list from `--config` CLI flag).
/// When `config` is `None`, runs a live registration pre-loop phase that
/// drains `RegistrationAction`s and pushes `ScoreboardState::Registration`
/// updates until `CloseRegistration` arrives, then falls through into the
/// main tournament loop unchanged.
/// Bundles auth/identity pieces and the `--fresh` flag so `run_master`'s
/// own parameter list stays within clippy's 7-arg limit.
pub struct AuthState {
    pub team_secrets: crate::team_secrets::TeamSecrets,
    pub join_registry: crate::join_registry::JoinRegistry,
    /// Ignore any existing `data/tournament_state.json` for this run
    /// (without deleting it) -- for a deliberate fresh start, e.g. a test
    /// run before the real event.
    pub fresh: bool,
}

pub fn run_master(
    server: &str,
    key: &str,
    my_id: &str,
    config: Option<PoolsConfig>,
    master_state: MasterState,
    mut rx: MasterReceivers,
    auth: AuthState,
) -> Result<()> {
    let AuthState {
        team_secrets,
        join_registry,
        fresh,
    } = auth;
    const SAVE_PATH: &str = "data/tournament_state.json";
    let tournament: Tournament = match config {
        Some(config) => {
            let pool1_names: Vec<String> =
                config.pool1_teams.iter().map(|(n, _)| n.clone()).collect();
            let pool2_names: Vec<String> =
                config.pool2_teams.iter().map(|(n, _)| n.clone()).collect();

            // Config-mode teams skip the registration UI entirely, so
            // there's no ScoreboardState::Registration screen to display
            // secrets on -- print them to the console instead, or the
            // operator would have no way to relay them to students.
            println!("Team secrets for join_competition (config mode -- no registration screen to show these):");
            for name in pool1_names.iter().chain(pool2_names.iter()) {
                let secret = crate::team_secrets::generate_team_secret();
                println!("  {name}: {secret}");
                team_secrets.set(name, secret);
            }

            println!("Waiting for operator to start the tournament (web console)...");
            rx.start
                .blocking_recv()
                .context("start-tournament channel closed before the tournament was started")?;
            println!("Tournament started.");

            Tournament::new(pool1_names, pool2_names, &config.grid_id)
        }
        None => {
            let grid_pool = crate::content_pools::list_grid_pool("data/grids")?;
            let saved = if fresh {
                None
            } else {
                load_saved_tournament_state(SAVE_PATH)?
            };
            let tournament = match saved {
                Some(state) if state.registration_closed => {
                    println!("Resuming saved tournament state from {SAVE_PATH} (registration already closed).");
                    let pool1_names: Vec<String> =
                        state.pool1_teams.iter().map(|t| t.name.clone()).collect();
                    let pool2_names: Vec<String> =
                        state.pool2_teams.iter().map(|t| t.name.clone()).collect();
                    for team in state.pool1_teams.iter().chain(state.pool2_teams.iter()) {
                        team_secrets.set(&team.name, team.secret.clone());
                    }
                    Tournament::from_schedules(
                        pool1_names,
                        pool2_names,
                        state.pool1_schedule.unwrap_or_default(),
                        state.pool2_schedule.unwrap_or_default(),
                    )
                }
                // A partial (not-yet-closed) save is intentionally not
                // replayed back into the registration UI -- resuming mid-
                // registration would need `run_registration_phase` to seed
                // its rosters from the saved teams first, which is a real
                // gap left for a future pass; today it only guarantees
                // resuming a *closed* registration (the main Tuesday ->
                // Thursday scenario), same as the design's primary case.
                _ => {
                    let (tournament, _board_ids, _grid_id) = run_registration_phase(
                        &master_state,
                        &mut rx.registration,
                        &team_secrets,
                        SAVE_PATH,
                        &grid_pool,
                    )?;
                    tournament
                }
            };
            tournament
        }
    };

    let client = std::sync::Arc::new(P2pClient::new(server, key, my_id));
    // These board IDs are a convention, not enforced by any type: the
    // arena processes MUST be launched with exactly these `--id` values
    // (`gridmind-referee arena --id arena-1-referee --arena-num 1 ...` and
    // the `-2-` equivalent), or the Master's assignments silently go to
    // nobody and the tournament stalls with no error.
    let arena_ids = ["arena-1-referee", "arena-2-referee"];
    let tournament = std::sync::Arc::new(std::sync::Mutex::new(tournament));

    // Push the freshly-built schedule immediately, before either arena
    // thread blocks on its first match-assign popup. Without this,
    // connected browsers would stay frozen on whatever phase was last
    // pushed (Registration, or the pre-Start-Tournament Idle screen) with
    // no visible Ready row to click -- deadlocking the handoff, since
    // that's the only way to unblock either thread.
    {
        let t = tournament.lock().expect("tournament lock poisoned");
        master_state.update(ScoreboardState::LivePoolPlay {
            arena1: None,
            arena2: None,
            arena1_pregame: None,
            arena2_pregame: None,
            pool1_standings: standings_snapshot(&t.pool1_standings),
            pool2_standings: standings_snapshot(&t.pool2_standings),
            pool1_schedule: t.pool1_schedule().to_vec(),
            pool2_schedule: t.pool2_schedule().to_vec(),
            grand_final_ready: None,
        });
    }

    for (idx, ((arena_id, match_start_rx), admin_rx)) in arena_ids
        .into_iter()
        .zip([rx.match_start_arena1, rx.match_start_arena2])
        .zip([rx.admin_arena1, rx.admin_arena2])
        .enumerate()
    {
        let arena_num = (idx + 1) as u32;
        let tournament = std::sync::Arc::clone(&tournament);
        let master_state = master_state.clone();
        let ctx = AssignContext {
            client: std::sync::Arc::clone(&client),
            master_state: master_state.clone(),
            tournament: std::sync::Arc::clone(&tournament),
            join_registry: join_registry.clone(),
        };
        std::thread::spawn(move || {
            if let Err(err) = run_arena_assignment_loop(
                tournament,
                master_state,
                ctx,
                arena_id,
                arena_num,
                match_start_rx,
                admin_rx,
            ) {
                eprintln!("arena {arena_num} assignment thread exited with an error: {err:#}");
            }
        });
    }

    loop {
        for raw in client.receive_all()? {
            if let Ok(msg) = serde_json::from_str::<ArenaToMaster>(&raw) {
                match msg {
                    ArenaToMaster::ScoreUpdate {
                        arena,
                        pool,
                        scores,
                        pairs_found,
                        total_pairs,
                        matched,
                        all_positions,
                        active_team,
                        turn_seconds_remaining,
                        streak,
                        hints_used,
                        puzzle_winner,
                        match_started_at_unix_ms,
                        is_paused,
                        flip_pending_positions,
                    } => {
                        println!(
                            "[arena {arena}, pool {pool}] {pairs_found}/{total_pairs} pairs, scores: {scores:?}"
                        );
                        let (team_a, team_b) = team_names_from_scores(&scores);
                        let arena_state = LiveArenaState {
                            pool,
                            team_a,
                            team_b,
                            scores,
                            matched,
                            all_positions,
                            active_team,
                            turn_seconds_remaining,
                            streak,
                            hints_used,
                            pairs_found,
                            total_pairs,
                            puzzle_winner,
                            match_started_at_unix_ms,
                            is_paused,
                            flip_pending_positions,
                        };
                        let (pool1_standings, pool2_standings, pool1_schedule, pool2_schedule) = {
                            let t = tournament.lock().expect("tournament lock poisoned");
                            (
                                standings_snapshot(&t.pool1_standings),
                                standings_snapshot(&t.pool2_standings),
                                t.pool1_schedule().to_vec(),
                                t.pool2_schedule().to_vec(),
                            )
                        };
                        if pool == 0 {
                            master_state.update(ScoreboardState::GrandFinal {
                                arena_num: arena,
                                arena: Box::new(arena_state),
                                pool1_standings,
                                pool2_standings,
                                pool1_schedule,
                                pool2_schedule,
                            });
                        } else {
                            // NOTE: this read-modify-write is not atomic, and as
                            // of the per-arena-thread refactor, `master_state`
                            // now has two writers -- this message loop, and
                            // each arena thread's `AssignGrandFinal` arm (in
                            // `run_arena_assignment_loop`). In practice this
                            // is still safe: `AssignGrandFinal` only fires
                            // once both pools are complete, which is mutually
                            // exclusive with a `ScoreUpdate` for pool-play
                            // arena state (this branch) arriving for either
                            // pool. If that mutual exclusion ever stops
                            // holding, this merge needs to move inside
                            // `MasterState` itself, under its own lock.
                            let (mut arena1, mut arena2, arena1_pregame, arena2_pregame) =
                                match master_state.snapshot() {
                                    ScoreboardState::LivePoolPlay {
                                        arena1,
                                        arena2,
                                        arena1_pregame,
                                        arena2_pregame,
                                        ..
                                    } => (arena1, arena2, arena1_pregame, arena2_pregame),
                                    _ => (None, None, None, None),
                                };
                            if arena == 1 {
                                arena1 = Some(Box::new(arena_state));
                            } else {
                                arena2 = Some(Box::new(arena_state));
                            }
                            master_state.update(ScoreboardState::LivePoolPlay {
                                arena1,
                                arena2,
                                arena1_pregame,
                                arena2_pregame,
                                pool1_standings,
                                pool2_standings,
                                pool1_schedule,
                                pool2_schedule,
                                grand_final_ready: None,
                            });
                        }
                    }
                    ArenaToMaster::MatchResult {
                        arena: _,
                        pool,
                        winner,
                        scores,
                        pairs_matched,
                    } => {
                        println!("[pool {pool}] match ended, winner: {winner}, scores: {scores:?}");
                        if pool != 0 {
                            let loser = scores
                                .keys()
                                .find(|k| *k != &winner)
                                .cloned()
                                .unwrap_or_default();
                            let winner_pairs = *pairs_matched.get(&winner).unwrap_or(&0);
                            let loser_pairs = *pairs_matched.get(&loser).unwrap_or(&0);
                            let (pool1_standings, pool2_standings, pool1_schedule, pool2_schedule) = {
                                let mut t = tournament.lock().expect("tournament lock poisoned");
                                t.record_result(pool, &winner, &loser, winner_pairs, loser_pairs);
                                (
                                    standings_snapshot(&t.pool1_standings),
                                    standings_snapshot(&t.pool2_standings),
                                    t.pool1_schedule().to_vec(),
                                    t.pool2_schedule().to_vec(),
                                )
                            };
                            // Refresh standings immediately so the table
                            // reflects the just-finished match without
                            // waiting for the next score_update (which may
                            // not arrive until a new match starts).
                            let (arena1, arena2, arena1_pregame, arena2_pregame) =
                                match master_state.snapshot() {
                                    ScoreboardState::LivePoolPlay {
                                        arena1,
                                        arena2,
                                        arena1_pregame,
                                        arena2_pregame,
                                        ..
                                    } => (arena1, arena2, arena1_pregame, arena2_pregame),
                                    _ => (None, None, None, None),
                                };
                            master_state.update(ScoreboardState::LivePoolPlay {
                                arena1,
                                arena2,
                                arena1_pregame,
                                arena2_pregame,
                                pool1_standings,
                                pool2_standings,
                                pool1_schedule,
                                pool2_schedule,
                                // Not yet known here -- the arena assignment
                                // thread's `next_action` call is what actually
                                // decides the Grand Final matchup; that push
                                // (see AssignGrandFinal in run_arena_assignment_loop)
                                // sets this field moments later.
                                grand_final_ready: None,
                            });
                        } else {
                            let (pool1_winner, pool2_winner, third_place) = {
                                let mut t = tournament.lock().expect("tournament lock poisoned");
                                t.record_result(0, &winner, "", 0, 0);
                                let (pool1_winner, pool2_winner) = t.pool_winners();
                                (pool1_winner, pool2_winner, t.third_place())
                            };
                            master_state.update(ScoreboardState::Champion {
                                winner: winner.clone(),
                                scores,
                                pool1_winner,
                                pool2_winner,
                                third_place,
                            });
                        }
                    }
                }
            }
        }

        sleep(POLL_INTERVAL);
    }
}

/// Drains `RegistrationAction`s, building up both pools' rosters live
/// and pushing an updated `ScoreboardState::Registration` after every
/// action, until `CloseRegistration` arrives. Returns the finalized
/// `Tournament` plus an empty board-id map (MACs are entered per-match,
/// not at registration time -- see Task 7) and a fixed grid id.
///
/// The grid is hardcoded to "example_grid.json" for this path -- there
/// is no web input for grid selection in this design; registration-mode
/// events are expected to use a single shared physical grid.
///
/// The caller is responsible for pushing the first `LivePoolPlay` state
/// once this returns -- this function only ever pushes `Registration`.
/// What gets written to `data/tournament_state.json` after every
/// registration action, and read back on Master startup to resume.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SavedTournamentState {
    pool1_teams: Vec<RegisteredTeam>,
    pool2_teams: Vec<RegisteredTeam>,
    registration_closed: bool,
    pool1_schedule: Option<Vec<ScheduleEntry>>,
    pool2_schedule: Option<Vec<ScheduleEntry>>,
}

fn save_tournament_state(path: &str, state: &SavedTournamentState) -> Result<()> {
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json).with_context(|| format!("writing save state to {path}"))
}

/// `Ok(None)` for "no save file yet" (normal on the very first run) vs
/// `Err` for "a save file exists but is corrupt" -- these must stay
/// distinguishable so a corrupt file fails loudly instead of silently
/// being treated as "start fresh" and quietly discarding real data.
fn load_saved_tournament_state(path: &str) -> Result<Option<SavedTournamentState>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            Ok(Some(serde_json::from_str(&text).with_context(|| {
                format!("parsing corrupt save file at {path}")
            })?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading save file at {path}")),
    }
}

fn run_registration_phase(
    master_state: &MasterState,
    registration_rx: &mut tokio::sync::mpsc::Receiver<RegistrationAction>,
    team_secrets: &crate::team_secrets::TeamSecrets,
    save_path: &str,
    grid_pool: &[String],
) -> Result<(
    Tournament,
    std::collections::HashMap<String, String>,
    String,
)> {
    let mut pool1_names: Vec<String> = Vec::new();
    let mut pool2_names: Vec<String> = Vec::new();
    let mut pool1_teams: Vec<RegisteredTeam> = Vec::new();
    let mut pool2_teams: Vec<RegisteredTeam> = Vec::new();

    println!("Waiting for teams to register via the web console...");

    loop {
        let action = registration_rx
            .blocking_recv()
            .context("registration channel closed before registration was closed")?;
        match action {
            RegistrationAction::RegisterTeam { name, students } => {
                let chosen_pool =
                    assign_to_smaller_pool(name.clone(), &mut pool1_names, &mut pool2_names);
                let secret = crate::team_secrets::generate_team_secret();
                team_secrets.set(&name, secret.clone());
                let team = RegisteredTeam {
                    name,
                    students,
                    secret,
                };
                if chosen_pool == 1 {
                    pool1_teams.push(team);
                } else {
                    pool2_teams.push(team);
                }
                println!(
                    "Registered {} teams (pool 1: {}, pool 2: {})",
                    pool1_teams.len() + pool2_teams.len(),
                    pool1_names.len(),
                    pool2_names.len()
                );
            }
            RegistrationAction::MoveTeam { name, to_pool } => {
                move_name(&name, to_pool, &mut pool1_names, &mut pool2_names);
                move_registered_team(&name, to_pool, &mut pool1_teams, &mut pool2_teams);
            }
            RegistrationAction::CloseRegistration => {
                println!("Registration closed. Building schedule...");
                break;
            }
        }
        master_state.update(ScoreboardState::Registration {
            pool1: PoolRegistration {
                teams: pool1_teams.clone(),
                schedule: build_schedule_entries(&pool1_names, "(grid assigned at close)"),
            },
            pool2: PoolRegistration {
                teams: pool2_teams.clone(),
                schedule: build_schedule_entries(&pool2_names, "(grid assigned at close)"),
            },
        });
        save_tournament_state(
            save_path,
            &SavedTournamentState {
                pool1_teams: pool1_teams.clone(),
                pool2_teams: pool2_teams.clone(),
                registration_closed: false,
                pool1_schedule: None,
                pool2_schedule: None,
            },
        )?;
    }

    let pool1_schedule = crate::pool::build_schedule_entries_with_grids(&pool1_names, grid_pool);
    let pool2_schedule = crate::pool::build_schedule_entries_with_grids(&pool2_names, grid_pool);
    save_tournament_state(
        save_path,
        &SavedTournamentState {
            pool1_teams: pool1_teams.clone(),
            pool2_teams: pool2_teams.clone(),
            registration_closed: true,
            pool1_schedule: Some(pool1_schedule.clone()),
            pool2_schedule: Some(pool2_schedule.clone()),
        },
    )?;

    Ok((
        Tournament::from_schedules(pool1_names, pool2_names, pool1_schedule, pool2_schedule),
        std::collections::HashMap::new(),
        String::new(),
    ))
}

/// Moves a team name from whichever of pool1/pool2 currently holds it
/// into `to_pool` (1 or 2). No-op if the name isn't found, or if it's
/// already in the target pool.
fn move_name(name: &str, to_pool: u32, pool1: &mut Vec<String>, pool2: &mut Vec<String>) {
    if let Some(idx) = pool1.iter().position(|t| t == name) {
        if to_pool == 2 {
            pool2.push(pool1.remove(idx));
        }
    } else if let Some(idx) = pool2.iter().position(|t| t == name) {
        if to_pool == 1 {
            pool1.push(pool2.remove(idx));
        }
    }
}

/// Same as `move_name` but for the `RegisteredTeam` roster (matched by
/// `.name`), kept in sync with the plain-name pools above.
fn move_registered_team(
    name: &str,
    to_pool: u32,
    pool1: &mut Vec<RegisteredTeam>,
    pool2: &mut Vec<RegisteredTeam>,
) {
    if let Some(idx) = pool1.iter().position(|t| t.name == name) {
        if to_pool == 2 {
            pool2.push(pool1.remove(idx));
        }
    } else if let Some(idx) = pool2.iter().position(|t| t.name == name) {
        if to_pool == 1 {
            pool1.push(pool2.remove(idx));
        }
    }
}

/// An absolute deadline `secs` seconds from now, expressed as
/// milliseconds since the Unix epoch. Absolute (not "seconds
/// remaining") so the frontend can tick the countdown down locally
/// between websocket pushes instead of needing a re-push every second.
fn unix_ms_after(secs: u64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before the Unix epoch");
    now.as_millis() as u64 + secs * 1000
}

/// Sets (or clears) one arena's pregame countdown without disturbing the
/// other arena's -- both arenas run fully independent assignment
/// threads (see "Concurrent Arena Assignment" in PROJECT_STATE.md) and
/// each has its own pregame state, so a snapshot/rebuild here must
/// preserve whatever the OTHER arena's pregame field currently holds.
/// A no-op if the current state isn't `LivePoolPlay` (shouldn't happen
/// at either of this function's call sites, but a stale/unexpected
/// phase should never panic here).
fn set_arena_pregame(master_state: &MasterState, arena: u32, pregame: Option<Box<PregameState>>) {
    let ScoreboardState::LivePoolPlay {
        arena1,
        arena2,
        arena1_pregame,
        arena2_pregame,
        pool1_standings,
        pool2_standings,
        pool1_schedule,
        pool2_schedule,
        grand_final_ready,
    } = master_state.snapshot()
    else {
        return;
    };
    let (arena1_pregame, arena2_pregame) = if arena == 1 {
        (pregame, arena2_pregame)
    } else {
        (arena1_pregame, pregame)
    };
    master_state.update(ScoreboardState::LivePoolPlay {
        arena1,
        arena2,
        arena1_pregame,
        arena2_pregame,
        pool1_standings,
        pool2_standings,
        pool1_schedule,
        pool2_schedule,
        grand_final_ready,
    });
}

/// Clears both the pregame countdown AND that arena's stale live match
/// data, in one push, right before `AssignMatch` is sent for a brand new
/// match. Using `set_arena_pregame(None)` alone here would leave the
/// *previous* match's `arena1`/`arena2` state in place, so the scoreboard
/// would flash back to showing the old, already-finished match for the
/// few seconds between the ceremony ending and the new match's first real
/// score update -- a confusing "did it revert?" moment for anyone
/// watching live.
fn clear_arena_for_new_match(master_state: &MasterState, arena: u32) {
    let ScoreboardState::LivePoolPlay {
        arena1,
        arena2,
        arena1_pregame,
        arena2_pregame,
        pool1_standings,
        pool2_standings,
        pool1_schedule,
        pool2_schedule,
        grand_final_ready,
    } = master_state.snapshot()
    else {
        return;
    };
    let (arena1, arena1_pregame) = if arena == 1 {
        (None, None)
    } else {
        (arena1, arena1_pregame)
    };
    let (arena2, arena2_pregame) = if arena == 2 {
        (None, None)
    } else {
        (arena2, arena2_pregame)
    };
    master_state.update(ScoreboardState::LivePoolPlay {
        arena1,
        arena2,
        arena1_pregame,
        arena2_pregame,
        pool1_standings,
        pool2_standings,
        pool1_schedule,
        pool2_schedule,
        grand_final_ready,
    });
}

/// Owned (not borrowed) so it can be moved into a per-arena thread in a
/// later task -- a spawned `std::thread` needs `'static` captured data,
/// which a borrowed `&P2pClient`/`&str` tied to `run_master`'s stack
/// frame can't satisfy. `Arc<P2pClient>` keeps cloning cheap (one
/// underlying `reqwest::blocking::Client`, shared).
struct AssignContext {
    client: std::sync::Arc<P2pClient>,
    /// Bundled in here (rather than a separate `prompt_and_assign`
    /// parameter) to stay within clippy's 7-argument limit.
    master_state: MasterState,
    tournament: std::sync::Arc<std::sync::Mutex<Tournament>>,
    join_registry: crate::join_registry::JoinRegistry,
}

/// One arena's independent assignment loop: repeatedly asks the shared
/// `Tournament` what to do next for this arena, and blocks (via
/// `prompt_and_assign`) on the operator's popup submission when there's
/// a match to assign. Runs on its own thread so arena 1 and arena 2 can
/// each be mid-assignment (including a future, longer pre-game ceremony
/// wait) without blocking each other -- see this plan's header comment
/// for why this matters.
///
/// Exits (returns) once this arena's thread has nothing further to do,
/// which today only happens after observing `NextAction::Champion` --
/// the OTHER arena's thread (or this one, if it gets there first) is
/// responsible for the actual champion announcement; both threads exiting
/// is fine, since `run_master`'s own thread (the message-receiving loop)
/// is what keeps the process alive for the web server.
///
/// Not panic-safe by design: if this function panics (e.g. `next_action`'s
/// `panic!` on an arena/pool id outside {0,1,2}, which can't currently
/// happen given `arena_num` is always derived from the fixed 2-element
/// `arena_ids` array, but would apply to any future change that relaxes
/// that), the spawning `std::thread::spawn` closure does not catch it --
/// this arena's thread dies silently with no operator-visible message,
/// and that arena's tournament progress permanently stalls while the
/// process and the other arena keep running. A panic here also poisons
/// the shared `tournament` mutex, which then panics every other lock
/// holder too (the other arena's thread, and `run_master`'s message
/// loop) -- so in practice a panic here still crashes the whole process,
/// matching the pre-refactor behavior where this was all one thread, just
/// via a less direct path (a secondary "lock poisoned" panic rather than
/// the original error).
fn run_arena_assignment_loop(
    tournament: std::sync::Arc<std::sync::Mutex<Tournament>>,
    master_state: MasterState,
    ctx: AssignContext,
    arena_id: &str,
    arena_num: u32,
    mut match_start_rx: tokio::sync::mpsc::Receiver<MatchStartInput>,
    mut admin_rx: tokio::sync::mpsc::Receiver<AdminCommand>,
) -> Result<()> {
    let mut board_ids: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    loop {
        // Relay any pending operator overrides straight to this arena's
        // process -- harmless to check even outside an active match (the
        // arena just discards an admin command with nothing to apply it
        // to; see `wait_for_assignment` in `arena.rs`).
        while let Ok(command) = admin_rx.try_recv() {
            let wire_message = command.into_wire_message();
            let result = serde_json::to_string(&wire_message)
                .map_err(anyhow::Error::from)
                .and_then(|json| ctx.client.send(arena_id, &json));
            if let Err(err) = result {
                eprintln!("arena {arena_num}: failed to relay admin command: {err:#}");
            }
        }

        let action = tournament
            .lock()
            .expect("tournament lock poisoned")
            .next_action(arena_num);
        match action {
            NextAction::AssignMatch {
                arena,
                pool,
                matchup,
                grid_id,
            } => {
                prompt_and_assign(
                    &ctx,
                    &mut board_ids,
                    arena_id,
                    arena,
                    MatchAssignment {
                        pool,
                        matchup: &matchup,
                        grid_id: &grid_id,
                    },
                    &mut match_start_rx,
                )?;
            }
            NextAction::AssignGrandFinal {
                arena,
                matchup,
                grid_id,
            } => {
                let (arena1, arena2, arena1_pregame, arena2_pregame) = match master_state.snapshot()
                {
                    ScoreboardState::LivePoolPlay {
                        arena1,
                        arena2,
                        arena1_pregame,
                        arena2_pregame,
                        ..
                    } => (arena1, arena2, arena1_pregame, arena2_pregame),
                    _ => (None, None, None, None),
                };
                let (pool1_standings, pool2_standings, pool1_schedule, pool2_schedule) = {
                    let t = tournament.lock().expect("tournament lock poisoned");
                    (
                        standings_snapshot(&t.pool1_standings),
                        standings_snapshot(&t.pool2_standings),
                        t.pool1_schedule().to_vec(),
                        t.pool2_schedule().to_vec(),
                    )
                };
                master_state.update(ScoreboardState::LivePoolPlay {
                    arena1,
                    arena2,
                    arena1_pregame,
                    arena2_pregame,
                    pool1_standings,
                    pool2_standings,
                    pool1_schedule,
                    pool2_schedule,
                    grand_final_ready: Some(GrandFinalReady {
                        arena,
                        team_a: matchup.team_a.clone(),
                        team_b: matchup.team_b.clone(),
                    }),
                });
                prompt_and_assign(
                    &ctx,
                    &mut board_ids,
                    arena_id,
                    arena,
                    MatchAssignment {
                        pool: 0,
                        matchup: &matchup,
                        grid_id: &grid_id,
                    },
                    &mut match_start_rx,
                )?;
            }
            NextAction::Champion { winner } => {
                if arena_num == 1 {
                    println!("\n🏆 TOURNAMENT CHAMPION: {winner}\n");
                }
                return Ok(());
            }
            NextAction::Wait => {
                sleep(POLL_INTERVAL);
            }
        }
    }
}

/// Picks a random (position, object) pair from the grid. Every object in
/// a freshly-loaded grid counts as "unresolved" at pre-game time, since
/// the match hasn't started -- no need to filter by match state here.
fn pick_random_grid_position(
    grid: &std::collections::HashMap<String, String>,
) -> Option<(String, String)> {
    use rand::seq::IteratorRandom;
    grid.iter()
        .map(|(pos, cls)| (pos.clone(), cls.clone()))
        .choose(&mut rand::thread_rng())
}

/// Grid dimensions derived from its own position labels (max row letter,
/// max column number) -- grid size varies per file, nothing hardcodes it.
fn grid_dimensions(grid: &std::collections::HashMap<String, String>) -> (u32, u32) {
    let mut max_row = 0u32;
    let mut max_col = 0u32;
    for pos in grid.keys() {
        if let Some((row_char, col)) = crate::hints::parse_position(pos) {
            max_row = max_row.max(crate::hints::row_letter_to_number(row_char));
            max_col = max_col.max(col);
        }
    }
    (max_row, max_col)
}

/// Generates and sends the shared, non-competitive free hint (grid-derived
/// object + quadrant riddle, split into QR fragments both teams must
/// assemble themselves) and shows its own countdown on the scoreboard,
/// right after the pre-game riddle stage and before the match actually
/// starts. Best-effort on send -- a failed free-hint delivery should never
/// block or fail the match itself.
fn send_free_hint(
    ctx: &AssignContext,
    arena: u32,
    matchup: &Matchup,
    grid_id: &str,
    team_a_id: &str,
    team_b_id: &str,
) -> Result<()> {
    let Ok(grid) = crate::grid::load_grid(grid_id) else {
        return Ok(());
    };
    let (grid_rows, grid_cols) = grid_dimensions(&grid);
    let Some((pos, object)) = pick_random_grid_position(&grid) else {
        return Ok(());
    };

    let quadrant = crate::hints::quadrant_for_position(&pos, grid_rows, grid_cols);
    let object_riddle =
        crate::content_pools::load_object_riddle("data/object_riddles.json", &object)
            .unwrap_or_else(|| format!("I am a {object}."));
    let quadrant_riddle =
        crate::content_pools::load_quadrant_riddle("data/quadrant_riddles.json", quadrant)
            .unwrap_or_default();
    let combined = format!("{object_riddle} {quadrant_riddle}");
    let fragments = crate::hints::split_into_fragments(&combined);
    let total = fragments.len() as u32;

    let deadline = unix_ms_after(crate::config::get().free_hint_secs);
    set_arena_pregame(
        &ctx.master_state,
        arena,
        Some(Box::new(PregameState::FreeHints {
            team_a: matchup.team_a.clone(),
            team_b: matchup.team_b.clone(),
            deadline_unix_ms: deadline,
        })),
    );

    for (idx, fragment) in fragments.iter().enumerate() {
        let qr_png_base64 = crate::hints::render_qr_png_base64(fragment);
        let msg = RefereeMessage::FreeHintFragment {
            index: idx as u32,
            total,
            qr_png_base64,
        };
        let payload = serde_json::to_string(&msg)?;
        ctx.client.send(team_a_id, &payload).ok();
        ctx.client.send(team_b_id, &payload).ok();
    }

    // Actually hold this stage open for the full window, matching what the
    // scoreboard countdown already implies -- sending the fragments is
    // near-instant, so without this wait the "free hints" panel flashed by
    // faster than anyone could read it.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_millis() as u64;
    if deadline > now {
        sleep(Duration::from_millis(deadline - now));
    }

    clear_arena_for_new_match(&ctx.master_state, arena);
    Ok(())
}

/// Bundles the three pieces of a decided matchup (rather than three
/// separate `prompt_and_assign` parameters) to stay within clippy's
/// 7-argument limit.
struct MatchAssignment<'a> {
    pool: u32,
    matchup: &'a Matchup,
    grid_id: &'a str,
}

fn prompt_and_assign(
    ctx: &AssignContext,
    board_ids: &mut std::collections::HashMap<String, String>,
    arena_id: &str,
    arena: u32,
    assignment: MatchAssignment,
    match_start_rx: &mut tokio::sync::mpsc::Receiver<MatchStartInput>,
) -> Result<()> {
    let MatchAssignment {
        pool,
        matchup,
        grid_id,
    } = assignment;
    println!(
        "\nNext match for arena {arena}: {} vs {}",
        matchup.team_a, matchup.team_b
    );
    println!("Waiting for operator to record the puzzle-race winner and board MACs via the web console...");

    let riddle_pool =
        crate::content_pools::load_pregame_riddles("data/pregame_riddles.json").unwrap_or_default();
    let riddle = ctx
        .tournament
        .lock()
        .expect("tournament lock poisoned")
        .pick_pregame_riddle(&riddle_pool)
        .map(|r| r.riddle)
        .unwrap_or_else(|| "(no riddle available)".to_string());

    // Best-effort: only reaches a team's board if they've already called
    // join_competition (their MAC is known via the join registry before
    // the operator has entered anything). A team relying on the manual
    // MAC-entry fallback won't receive this over the wire -- the human
    // referee is still the fallback delivery path for them, same as the
    // rest of this pre-game ceremony already assumes.
    let known_macs = ctx.join_registry.snapshot();
    for team in [&matchup.team_a, &matchup.team_b] {
        if let Some(info) = known_macs.get(team) {
            ctx.client
                .send(
                    &info.mac,
                    &serde_json::to_string(&RefereeMessage::PregameRiddle {
                        riddle: riddle.clone(),
                    })?,
                )
                .ok();
        }
    }

    set_arena_pregame(
        &ctx.master_state,
        arena,
        Some(Box::new(PregameState::PuzzleRace {
            team_a: matchup.team_a.clone(),
            team_b: matchup.team_b.clone(),
            deadline_unix_ms: unix_ms_after(crate::config::get().puzzle_race_secs),
            riddle,
        })),
    );

    let input = match_start_rx
        .blocking_recv()
        .context("match-start channel closed unexpectedly")?;

    set_arena_pregame(&ctx.master_state, arena, None);

    // Freshly-entered MACs always win -- no lookup/fallback needed here,
    // the operator just supplied them. board_ids is still updated so it
    // reflects the latest known MAC per team, even though nothing reads
    // it back before the next match overwrites it again.
    let team_a_id = input.team_a_mac;
    let team_b_id = input.team_b_mac;
    board_ids.insert(matchup.team_a.clone(), team_a_id.clone());
    board_ids.insert(matchup.team_b.clone(), team_b_id.clone());

    send_free_hint(ctx, arena, matchup, grid_id, &team_a_id, &team_b_id)?;

    let msg = MasterToArena::AssignMatch {
        arena,
        pool,
        team_a: matchup.team_a.clone(),
        team_a_id,
        team_b: matchup.team_b.clone(),
        team_b_id,
        grid_id: grid_id.to_string(),
        first_turn_team: input.puzzle_winner,
    };
    ctx.client.send(arena_id, &serde_json::to_string(&msg)?)
}

#[cfg(test)]
mod operator_channel_tests {
    use super::*;

    #[test]
    fn start_tournament_signal_reaches_the_receiver() {
        let (channels, mut rx) = operator_channels();
        let handle = std::thread::spawn(move || rx.start.blocking_recv());
        channels.start_tournament.blocking_send(()).unwrap();
        assert_eq!(handle.join().unwrap(), Some(()));
    }

    #[test]
    fn puzzle_winner_for_arena_one_reaches_the_arena_one_receiver() {
        let (channels, mut rx) = operator_channels();
        let handle = std::thread::spawn(move || rx.match_start_arena1.blocking_recv());
        let input = MatchStartInput {
            puzzle_winner: "alpha".to_string(),
            team_a_mac: "aa:aa".to_string(),
            team_b_mac: "bb:bb".to_string(),
        };
        channels
            .match_start_arena1
            .blocking_send(input.clone())
            .unwrap();
        assert_eq!(handle.join().unwrap(), Some(input));
    }

    #[test]
    fn puzzle_winner_for_arena_two_reaches_the_arena_two_receiver() {
        let (channels, mut rx) = operator_channels();
        let handle = std::thread::spawn(move || rx.match_start_arena2.blocking_recv());
        let input = MatchStartInput {
            puzzle_winner: "epsilon".to_string(),
            team_a_mac: "cc:cc".to_string(),
            team_b_mac: "dd:dd".to_string(),
        };
        channels
            .match_start_arena2
            .blocking_send(input.clone())
            .unwrap();
        assert_eq!(handle.join().unwrap(), Some(input));
    }
}

#[cfg(test)]
mod master_state_tests {
    use super::*;
    use crate::scoreboard_state::PoolPreview;
    use std::collections::HashMap;

    fn idle_state() -> ScoreboardState {
        ScoreboardState::Idle {
            pool1: PoolPreview {
                teams: vec!["alpha".to_string()],
                total_matches: 0,
            },
            pool2: PoolPreview {
                teams: vec!["delta".to_string()],
                total_matches: 0,
            },
        }
    }

    fn champion_state() -> ScoreboardState {
        ScoreboardState::Champion {
            winner: "alpha".to_string(),
            scores: HashMap::new(),
            pool1_winner: "alpha".to_string(),
            pool2_winner: "delta".to_string(),
            third_place: None,
        }
    }

    #[test]
    fn snapshot_returns_the_initial_state() {
        let state = MasterState::new(idle_state());
        assert_eq!(state.snapshot(), idle_state());
    }

    #[test]
    fn update_changes_the_snapshot() {
        let state = MasterState::new(idle_state());
        state.update(champion_state());
        assert_eq!(state.snapshot(), champion_state());
    }

    #[test]
    fn set_arena_pregame_sets_the_target_arena_without_disturbing_the_other() {
        let master_state = MasterState::new(ScoreboardState::LivePoolPlay {
            arena1: None,
            arena2: None,
            arena1_pregame: None,
            arena2_pregame: Some(Box::new(PregameState::PuzzleRace {
                team_a: "gamma".to_string(),
                team_b: "delta".to_string(),
                deadline_unix_ms: 123,
                riddle: "riddle b".to_string(),
            })),
            pool1_standings: vec![],
            pool2_standings: vec![],
            pool1_schedule: vec![],
            pool2_schedule: vec![],
            grand_final_ready: None,
        });

        set_arena_pregame(
            &master_state,
            1,
            Some(Box::new(PregameState::PuzzleRace {
                team_a: "alpha".to_string(),
                team_b: "beta".to_string(),
                deadline_unix_ms: 456,
                riddle: "riddle a".to_string(),
            })),
        );

        match master_state.snapshot() {
            ScoreboardState::LivePoolPlay {
                arena1_pregame,
                arena2_pregame,
                ..
            } => {
                assert_eq!(
                    arena1_pregame,
                    Some(Box::new(PregameState::PuzzleRace {
                        team_a: "alpha".to_string(),
                        team_b: "beta".to_string(),
                        deadline_unix_ms: 456,
                        riddle: "riddle a".to_string(),
                    }))
                );
                // Arena 2's pregame state must survive untouched.
                assert_eq!(
                    arena2_pregame,
                    Some(Box::new(PregameState::PuzzleRace {
                        team_a: "gamma".to_string(),
                        team_b: "delta".to_string(),
                        deadline_unix_ms: 123,
                        riddle: "riddle b".to_string(),
                    }))
                );
            }
            other => panic!("expected LivePoolPlay, got {other:?}"),
        }
    }

    fn sample_live_arena(team_a: &str, team_b: &str) -> LiveArenaState {
        LiveArenaState {
            pool: 1,
            team_a: team_a.to_string(),
            team_b: team_b.to_string(),
            scores: HashMap::new(),
            matched: HashMap::new(),
            all_positions: vec![],
            active_team: team_a.to_string(),
            turn_seconds_remaining: 120,
            streak: HashMap::new(),
            hints_used: HashMap::new(),
            pairs_found: 0,
            total_pairs: 4,
            puzzle_winner: team_a.to_string(),
            match_started_at_unix_ms: 1_800_000_000_000,
            is_paused: false,
            flip_pending_positions: None,
        }
    }

    #[test]
    fn clear_arena_for_new_match_clears_pregame_and_stale_live_data_for_that_arena_only() {
        let master_state = MasterState::new(ScoreboardState::LivePoolPlay {
            arena1: Some(Box::new(sample_live_arena("alpha", "delta"))),
            arena2: Some(Box::new(sample_live_arena("gamma", "epsilon"))),
            arena1_pregame: None,
            arena2_pregame: Some(Box::new(PregameState::PuzzleRace {
                team_a: "gamma".to_string(),
                team_b: "epsilon".to_string(),
                deadline_unix_ms: 123,
                riddle: "riddle b".to_string(),
            })),
            pool1_standings: vec![],
            pool2_standings: vec![],
            pool1_schedule: vec![],
            pool2_schedule: vec![],
            grand_final_ready: None,
        });

        clear_arena_for_new_match(&master_state, 1);

        match master_state.snapshot() {
            ScoreboardState::LivePoolPlay {
                arena1,
                arena2,
                arena1_pregame,
                arena2_pregame,
                ..
            } => {
                assert_eq!(arena1, None, "arena 1's stale match data must be cleared");
                assert_eq!(arena1_pregame, None);
                // Arena 2's live data and pregame state must survive untouched.
                assert_eq!(
                    arena2,
                    Some(Box::new(sample_live_arena("gamma", "epsilon")))
                );
                assert!(arena2_pregame.is_some());
            }
            other => panic!("expected LivePoolPlay, got {other:?}"),
        }
    }

    #[test]
    fn set_arena_pregame_is_a_no_op_when_state_is_not_live_pool_play() {
        let master_state = MasterState::new(champion_state());

        set_arena_pregame(
            &master_state,
            1,
            Some(Box::new(PregameState::PuzzleRace {
                team_a: "alpha".to_string(),
                team_b: "beta".to_string(),
                deadline_unix_ms: 456,
                riddle: "riddle a".to_string(),
            })),
        );

        assert!(matches!(
            master_state.snapshot(),
            ScoreboardState::Champion { .. }
        ));
    }

    #[test]
    fn subscribers_can_read_the_latest_update_synchronously() {
        let state = MasterState::new(idle_state());
        let receiver = state.subscribe();
        state.update(champion_state());
        assert_eq!(*receiver.borrow(), champion_state());
    }

    #[test]
    fn cloned_handles_share_the_same_underlying_state() {
        let state = MasterState::new(idle_state());
        let cloned = state.clone();
        cloned.update(champion_state());
        assert_eq!(state.snapshot(), champion_state());
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;

    #[test]
    fn register_team_action_reaches_the_receiver() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<RegistrationAction>(8);
        let handle = std::thread::spawn(move || rx.blocking_recv());
        tx.blocking_send(RegistrationAction::RegisterTeam {
            name: "alpha".to_string(),
            students: vec!["Priya".to_string()],
        })
        .unwrap();
        match handle.join().unwrap() {
            Some(RegistrationAction::RegisterTeam { name, students }) => {
                assert_eq!(name, "alpha");
                assert_eq!(students, vec!["Priya".to_string()]);
            }
            other => panic!("expected RegisterTeam, got {other:?}"),
        }
    }

    #[test]
    fn smaller_pool_wins_the_auto_balance() {
        let mut pool1 = vec!["alpha".to_string()];
        let mut pool2: Vec<String> = vec![];
        let chosen = assign_to_smaller_pool("beta".to_string(), &mut pool1, &mut pool2);
        assert_eq!(chosen, 2);
        assert_eq!(pool1, vec!["alpha".to_string()]);
        assert_eq!(pool2, vec!["beta".to_string()]);
    }

    #[test]
    fn equal_pools_still_places_the_team_somewhere() {
        let mut pool1: Vec<String> = vec![];
        let mut pool2: Vec<String> = vec![];
        let chosen = assign_to_smaller_pool("alpha".to_string(), &mut pool1, &mut pool2);
        assert!(chosen == 1 || chosen == 2);
        assert_eq!(pool1.len() + pool2.len(), 1);
    }

    #[test]
    fn registration_phase_pushes_state_and_returns_a_working_tournament() {
        let master_state = MasterState::new(ScoreboardState::Registration {
            pool1: PoolRegistration {
                teams: vec![],
                schedule: vec![],
            },
            pool2: PoolRegistration {
                teams: vec![],
                schedule: vec![],
            },
        });
        let (tx, mut rx) = tokio::sync::mpsc::channel::<RegistrationAction>(8);
        // 4 teams guarantees a 2-2 split regardless of how the coin flip
        // lands on any individual registration (each pool always grows
        // by exactly one team per two registrations), so both pools end
        // up with a real scheduled match -- not the degenerate 1-team
        // case where a pool is trivially "complete" with zero matches.
        for (name, student) in [
            ("alpha", "Priya"),
            ("beta", "Jamal"),
            ("gamma", "Wren"),
            ("delta", "Noor"),
        ] {
            tx.blocking_send(RegistrationAction::RegisterTeam {
                name: name.to_string(),
                students: vec![student.to_string()],
            })
            .unwrap();
        }
        tx.blocking_send(RegistrationAction::CloseRegistration)
            .unwrap();

        let save_dir = tempfile::tempdir().unwrap();
        let save_path = save_dir.path().join("tournament_state.json");
        let (mut tournament, board_ids, _grid_id) = run_registration_phase(
            &master_state,
            &mut rx,
            &crate::team_secrets::TeamSecrets::new(),
            save_path.to_str().unwrap(),
            &["example_grid.json".to_string()],
        )
        .unwrap();

        assert!(board_ids.is_empty());
        match master_state.snapshot() {
            ScoreboardState::Registration { pool1, pool2 } => {
                assert_eq!(pool1.teams.len(), 2);
                assert_eq!(pool2.teams.len(), 2);
            }
            other => panic!("expected Registration, got {other:?}"),
        }
        match tournament.next_action(1) {
            NextAction::AssignMatch { pool, .. } => assert_eq!(pool, 1),
            other => panic!("expected AssignMatch for arena 1, got {other:?}"),
        }
        match tournament.next_action(2) {
            NextAction::AssignMatch { pool, .. } => assert_eq!(pool, 2),
            other => panic!("expected AssignMatch for arena 2, got {other:?}"),
        }
    }

    #[test]
    fn move_team_action_relocates_between_pools_in_both_representations() {
        let master_state = MasterState::new(ScoreboardState::Registration {
            pool1: PoolRegistration {
                teams: vec![],
                schedule: vec![],
            },
            pool2: PoolRegistration {
                teams: vec![],
                schedule: vec![],
            },
        });
        let (tx, mut rx) = tokio::sync::mpsc::channel::<RegistrationAction>(8);
        tx.blocking_send(RegistrationAction::RegisterTeam {
            name: "alpha".to_string(),
            students: vec!["Priya".to_string()],
        })
        .unwrap();
        // Wherever the coin flip put "alpha", force it to pool 2 -- this
        // exercises both move_name (plain-name list, feeds Tournament::new)
        // and move_registered_team (roster list, feeds the pushed state)
        // staying in sync with each other.
        tx.blocking_send(RegistrationAction::MoveTeam {
            name: "alpha".to_string(),
            to_pool: 2,
        })
        .unwrap();
        tx.blocking_send(RegistrationAction::CloseRegistration)
            .unwrap();

        let save_dir = tempfile::tempdir().unwrap();
        let save_path = save_dir.path().join("tournament_state.json");
        let (_tournament, _board_ids, _grid_id) = run_registration_phase(
            &master_state,
            &mut rx,
            &crate::team_secrets::TeamSecrets::new(),
            save_path.to_str().unwrap(),
            &["example_grid.json".to_string()],
        )
        .unwrap();

        match master_state.snapshot() {
            ScoreboardState::Registration { pool1, pool2 } => {
                assert!(pool1.teams.is_empty());
                assert_eq!(pool2.teams.len(), 1);
                assert_eq!(pool2.teams[0].name, "alpha");
            }
            other => panic!("expected Registration, got {other:?}"),
        }
    }

    #[test]
    fn registering_a_team_writes_the_save_file() {
        let master_state = MasterState::new(ScoreboardState::Registration {
            pool1: PoolRegistration {
                teams: vec![],
                schedule: vec![],
            },
            pool2: PoolRegistration {
                teams: vec![],
                schedule: vec![],
            },
        });
        let dir = tempfile::tempdir().unwrap();
        let save_path = dir.path().join("tournament_state.json");
        let (tx, mut rx) = tokio::sync::mpsc::channel::<RegistrationAction>(8);
        tx.blocking_send(RegistrationAction::RegisterTeam {
            name: "alpha".to_string(),
            students: vec![],
        })
        .unwrap();
        tx.blocking_send(RegistrationAction::CloseRegistration)
            .unwrap();

        run_registration_phase(
            &master_state,
            &mut rx,
            &crate::team_secrets::TeamSecrets::new(),
            save_path.to_str().unwrap(),
            &["example_grid.json".to_string()],
        )
        .unwrap();

        let saved = std::fs::read_to_string(&save_path).unwrap();
        assert!(saved.contains("alpha"));
        assert!(saved.contains("\"registration_closed\": true"));
    }

    #[test]
    fn resumes_from_a_saved_file_with_registration_still_open() {
        let dir = tempfile::tempdir().unwrap();
        let save_path = dir.path().join("tournament_state.json");
        std::fs::write(
            &save_path,
            serde_json::to_string(&SavedTournamentState {
                pool1_teams: vec![RegisteredTeam {
                    name: "alpha".to_string(),
                    students: vec![],
                    secret: "abc".to_string(),
                }],
                pool2_teams: vec![],
                registration_closed: false,
                pool1_schedule: None,
                pool2_schedule: None,
            })
            .unwrap(),
        )
        .unwrap();

        let loaded = load_saved_tournament_state(save_path.to_str().unwrap()).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().pool1_teams[0].name, "alpha");
    }

    #[test]
    fn missing_save_file_returns_none_not_an_error() {
        let result =
            load_saved_tournament_state("/nonexistent/path/tournament_state.json").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn corrupt_save_file_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let save_path = dir.path().join("tournament_state.json");
        std::fs::write(&save_path, "not valid json").unwrap();
        assert!(load_saved_tournament_state(save_path.to_str().unwrap()).is_err());
    }
}

#[cfg(test)]
mod empty_registration_state_tests {
    use super::*;

    #[test]
    fn empty_registration_state_starts_with_no_teams_in_either_pool() {
        match empty_registration_state() {
            ScoreboardState::Registration { pool1, pool2 } => {
                assert!(pool1.teams.is_empty());
                assert!(pool2.teams.is_empty());
            }
            other => panic!("expected Registration, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn next_action_assigns_the_first_scheduled_matchup() {
        let mut tournament = Tournament::new(
            names(&["alpha", "beta"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        match tournament.next_action(1) {
            NextAction::AssignMatch {
                arena,
                pool,
                matchup,
                ..
            } => {
                assert_eq!(arena, 1);
                assert_eq!(pool, 1);
                assert_eq!(
                    matchup,
                    Matchup {
                        team_a: "alpha".into(),
                        team_b: "beta".into()
                    }
                );
            }
            other => panic!("expected AssignMatch, got {other:?}"),
        }
    }

    #[test]
    fn next_action_waits_once_pool_schedule_is_exhausted() {
        let mut tournament = Tournament::new(
            names(&["alpha", "beta"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        tournament.next_action(1); // consumes the only pool 1 matchup
        tournament.record_result(1, "alpha", "beta", 9, 6);
        match tournament.next_action(1) {
            NextAction::Wait => {}
            other => panic!("expected Wait, got {other:?}"),
        }
    }

    #[test]
    fn schedule_snapshot_shows_ready_then_live_then_complete() {
        let mut tournament = Tournament::new(
            names(&["alpha", "beta", "gamma"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        assert_eq!(tournament.pool1_schedule()[0].status, MatchStatus::Ready);
        assert_eq!(tournament.pool1_schedule()[1].status, MatchStatus::Locked);

        tournament.next_action(1); // assigns alpha vs beta
        assert_eq!(tournament.pool1_schedule()[0].status, MatchStatus::Live);

        tournament.record_result(1, "alpha", "beta", 9, 6);
        match &tournament.pool1_schedule()[0].status {
            MatchStatus::Complete { winner } => assert_eq!(winner, "alpha"),
            other => panic!("expected Complete, got {other:?}"),
        }
        // Finishing match 0 unlocks match 1 (alpha vs gamma).
        assert_eq!(tournament.pool1_schedule()[1].status, MatchStatus::Ready);
    }

    #[test]
    fn completing_the_last_scheduled_match_does_not_panic_on_missing_next_entry() {
        // pool 2 here has only one scheduled match (delta vs epsilon) --
        // completing it exercises the `get_mut(live_idx + 1)` returning
        // `None` path in `complete_and_unlock_next`.
        let mut tournament = Tournament::new(
            names(&["alpha", "beta"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        tournament.next_action(2);
        tournament.record_result(2, "delta", "epsilon", 8, 5);
        match &tournament.pool2_schedule()[0].status {
            MatchStatus::Complete { winner } => assert_eq!(winner, "delta"),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn next_action_assigns_grand_final_once_both_pools_complete() {
        let mut tournament = Tournament::new(
            names(&["alpha", "beta"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        tournament.next_action(1);
        tournament.record_result(1, "alpha", "beta", 9, 6);
        tournament.next_action(2);
        tournament.record_result(2, "delta", "epsilon", 8, 5);

        match tournament.next_action(1) {
            NextAction::AssignGrandFinal { matchup, .. } => {
                assert_eq!(
                    matchup,
                    Matchup {
                        team_a: "alpha".into(),
                        team_b: "delta".into()
                    }
                );
            }
            other => panic!("expected AssignGrandFinal, got {other:?}"),
        }
    }

    #[test]
    fn next_action_does_not_double_assign_grand_final_to_the_other_arena() {
        // Regression test: run_master calls next_action(1) then
        // next_action(2) back-to-back on every loop iteration. Once the
        // Grand Final is ready and handed to arena 1, the very next call
        // for arena 2 (in the same iteration, before any result comes
        // back) must NOT also be handed a Grand Final assignment.
        let mut tournament = Tournament::new(
            names(&["alpha", "beta"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        tournament.next_action(1);
        tournament.record_result(1, "alpha", "beta", 9, 6);
        tournament.next_action(2);
        tournament.record_result(2, "delta", "epsilon", 8, 5);

        match tournament.next_action(1) {
            NextAction::AssignGrandFinal { .. } => {}
            other => panic!("expected AssignGrandFinal for arena 1, got {other:?}"),
        }
        // Immediately calling for arena 2, exactly as run_master does on
        // the same loop iteration, before any result has come back.
        match tournament.next_action(2) {
            NextAction::Wait => {}
            other => panic!("expected Wait for arena 2, got {other:?}"),
        }
    }

    #[test]
    fn next_action_reports_champion_after_grand_final_result() {
        let mut tournament = Tournament::new(
            names(&["alpha", "beta"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        tournament.next_action(1);
        tournament.record_result(1, "alpha", "beta", 9, 6);
        tournament.next_action(2);
        tournament.record_result(2, "delta", "epsilon", 8, 5);
        tournament.next_action(1); // assigns the grand final
        tournament.record_result(0, "alpha", "delta", 10, 8); // pool=0 means grand final

        match tournament.next_action(1) {
            NextAction::Champion { winner } => assert_eq!(winner, "alpha"),
            other => panic!("expected Champion, got {other:?}"),
        }
    }

    #[test]
    fn pool_winners_reports_each_pools_winner() {
        let mut tournament = Tournament::new(
            names(&["alpha", "beta"]),
            names(&["delta", "epsilon"]),
            "example_grid.json",
        );
        tournament.next_action(1);
        tournament.record_result(1, "alpha", "beta", 9, 6);
        tournament.next_action(2);
        tournament.record_result(2, "delta", "epsilon", 8, 5);
        assert_eq!(
            tournament.pool_winners(),
            ("alpha".to_string(), "delta".to_string())
        );
    }

    #[test]
    fn two_total_teams_go_straight_to_a_grand_final() {
        let mut tournament =
            Tournament::new(names(&["alpha"]), names(&["beta"]), "example_grid.json");
        match tournament.next_action(1) {
            NextAction::AssignGrandFinal { arena, matchup, .. } => {
                assert_eq!(arena, 1);
                assert_eq!(
                    matchup,
                    Matchup {
                        team_a: "alpha".into(),
                        team_b: "beta".into()
                    }
                );
            }
            other => panic!("expected AssignGrandFinal, got {other:?}"),
        }
    }

    #[test]
    fn two_total_teams_go_straight_to_a_grand_final_even_if_both_are_in_the_same_pool() {
        // Guards against a real panic risk: if an operator uses the
        // registration UI's "Move to Pool" button and ends up with both
        // teams in the same pool, the two-team shortcut must still kick
        // in -- otherwise `pool_winners()` would later panic trying to
        // find a winner in an empty pool.
        let mut tournament =
            Tournament::new(names(&["alpha", "beta"]), names(&[]), "example_grid.json");
        match tournament.next_action(1) {
            NextAction::AssignGrandFinal { matchup, .. } => {
                assert_eq!(
                    matchup,
                    Matchup {
                        team_a: "alpha".into(),
                        team_b: "beta".into()
                    }
                );
            }
            other => panic!("expected AssignGrandFinal, got {other:?}"),
        }
    }

    #[test]
    fn two_total_teams_declares_a_champion_without_panicking_on_pool_winners() {
        let mut tournament =
            Tournament::new(names(&["alpha"]), names(&["beta"]), "example_grid.json");
        tournament.next_action(1); // assigns the grand final
        tournament.record_result(0, "alpha", "beta", 9, 6);
        match tournament.next_action(1) {
            NextAction::Champion { winner } => assert_eq!(winner, "alpha"),
            other => panic!("expected Champion, got {other:?}"),
        }
        // Must not panic even though neither pool ever held a real match.
        assert_eq!(
            tournament.pool_winners(),
            ("alpha".to_string(), "beta".to_string())
        );
    }
}
