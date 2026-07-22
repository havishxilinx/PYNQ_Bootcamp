use crate::game_state::GameState;
use crate::genesis_client::GenesisClient;
use crate::grid::load_grid;
use crate::master_messages::{ArenaToMaster, MasterToArena};
use crate::messages::{RefereeMessage, StudentMessage};
use crate::p2p_client::P2pClient;
use anyhow::Result;
use std::thread::sleep;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Models the real-world bottleneck a `flip_both` request is meant to
/// reduce: a human physically flipping each card. Opt-in via
/// `GRIDMIND_SIMULATED_FLIP_MS` (milliseconds per card) so normal test runs
/// and demos stay instant by default, but a timing comparison between
/// `flip` and `flip_both` can model a realistic per-card flip delay.
fn simulate_human_flip_delay(card_count: u32) {
    let per_card_ms: u64 = std::env::var("GRIDMIND_SIMULATED_FLIP_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if per_card_ms > 0 {
        sleep(Duration::from_millis(per_card_ms * u64::from(card_count)));
    }
}

struct AssignedMatch {
    pool: u32,
    team_a: String,
    team_a_id: String,
    team_b: String,
    team_b_id: String,
    grid_id: String,
    first_turn_team: String,
    /// True for a Practice Mode match -- `team_b`/`team_b_id` are already
    /// filled in as `game_state::BOT_TEAM_NAME`/`BOT_BOARD_ID` in this
    /// case, so the rest of `run_one_match`'s team-ordering logic works
    /// unchanged; this flag only gates the handful of things that
    /// genuinely differ (no Genesis, no GameStart sent to the bot, and
    /// driving `GameState::maybe_play_bot_turn` each poll tick).
    is_practice: bool,
}

/// Groups `run_arena`'s Genesis-related CLI parameters -- its parameter
/// list crossed clippy's `too_many_arguments` threshold once the stream
/// port was added. `url: None` means Genesis is unconfigured for this
/// arena entirely; the other two fields are then unused.
pub struct GenesisConfig {
    pub url: Option<String>,
    pub admin_password: String,
    pub stream_port: u16,
}

/// Runs forever: waits for a match assignment from the Master, plays it
/// to completion, reports the result, then waits for the next one.
///
/// `pool` is deliberately NOT a static parameter here — it must come from
/// each `assign_match` message instead, since it's 0 for the Grand Final
/// and the arena's fixed pool number otherwise. A prior version took a
/// static `pool_num` from the CLI and always reported that back in
/// `match_result`, which meant a Grand Final's result was reported with
/// the arena's normal pool number instead of 0, so the Master never
/// recognized it as the Grand Final and the tournament never reached
/// `Champion`. Always trusting the incoming assignment's `pool` field
/// fixes this.
pub fn run_arena(
    server: &str,
    key: &str,
    my_id: &str,
    master_id: &str,
    arena_num: u32,
    genesis_config: GenesisConfig,
) -> Result<()> {
    let client = P2pClient::new(server, key, my_id);
    let genesis = genesis_config.url.map(|url| GenesisClient::new(&url));
    loop {
        // A communication failure here used to propagate straight out of
        // `run_arena` via `?` and kill the whole Arena process -- even
        // after `P2pClient`'s own internal retries are exhausted (a real,
        // extended broker/network outage, not just a blip), losing this
        // one match/assignment attempt is far better than losing the
        // entire arena for the rest of the tournament. Log loudly (this is
        // exactly what operators are told to watch, see operators-guide.md)
        // and go back to waiting for the next assignment instead.
        let assignment = match wait_for_assignment(&client) {
            Ok(assignment) => assignment,
            Err(err) => {
                eprintln!(
                    "arena {arena_num}: failed to receive the next match assignment: {err:#} -- retrying"
                );
                continue;
            }
        };
        if let Err(err) = run_one_match(
            &client,
            master_id,
            arena_num,
            assignment,
            genesis.as_ref(),
            &genesis_config.admin_password,
            genesis_config.stream_port,
        ) {
            eprintln!(
                "arena {arena_num}: match ended early due to a communication error: {err:#} -- waiting for the next assignment"
            );
        }
    }
}

fn wait_for_assignment(client: &P2pClient) -> Result<AssignedMatch> {
    loop {
        for raw in client.receive_all()? {
            match serde_json::from_str::<MasterToArena>(&raw) {
                Ok(MasterToArena::AssignMatch {
                    pool,
                    team_a,
                    team_a_id,
                    team_b,
                    team_b_id,
                    grid_id,
                    first_turn_team,
                    ..
                }) => {
                    return Ok(AssignedMatch {
                        pool,
                        team_a,
                        team_a_id,
                        team_b,
                        team_b_id,
                        grid_id,
                        first_turn_team,
                        is_practice: false,
                    });
                }
                Ok(MasterToArena::AssignPracticeMatch {
                    team_a,
                    team_a_id,
                    grid_id,
                }) => {
                    return Ok(AssignedMatch {
                        pool: crate::game_state::PRACTICE_POOL,
                        team_a: team_a.clone(),
                        team_a_id,
                        team_b: crate::game_state::BOT_TEAM_NAME.to_string(),
                        team_b_id: crate::game_state::BOT_BOARD_ID.to_string(),
                        grid_id,
                        first_turn_team: team_a,
                        is_practice: true,
                    });
                }
                Ok(other) => {
                    eprintln!(
                        "arena: discarding admin command received before a match was assigned: {other:?}"
                    );
                }
                Err(_) => {
                    eprintln!(
                        "arena: discarding unrecognized message while waiting for assignment: {raw}"
                    );
                }
            }
        }
        sleep(POLL_INTERVAL);
    }
}

fn run_one_match(
    client: &P2pClient,
    master_id: &str,
    arena_num: u32,
    assignment: AssignedMatch,
    genesis: Option<&GenesisClient>,
    genesis_admin_password: &str,
    genesis_stream_port: u16,
) -> Result<()> {
    let pool_num = assignment.pool;
    let grid = load_grid(&assignment.grid_id)?;
    // Practice matches never touch Genesis -- there's no second real board
    // for a second arm to belong to, and no per-match token/stream needed
    // just to watch a bot play. Shadowing to None here means every other
    // `if let Some(g) = genesis` below already does the right thing.
    let genesis = if assignment.is_practice {
        None
    } else {
        genesis
    };
    // `None` whenever Genesis isn't configured for this arena (or this is
    // a practice match) -- also `None` if the configured Genesis server
    // predates the competition-mode streaming fix, since the resulting
    // URL simply 404s and the arena UI hides the video zone on a broken
    // image rather than erroring.
    let genesis_stream_url =
        genesis.and_then(|g| g.competition_stream_url(genesis_stream_port));
    // Order teams so the puzzle-race winner goes first (GameState always
    // starts with teams[0] active) -- for a practice match this is always
    // team_a, since `wait_for_assignment` sets `first_turn_team` to it.
    let teams = if assignment.first_turn_team == assignment.team_a {
        vec![
            (assignment.team_a.clone(), assignment.team_a_id.clone()),
            (assignment.team_b.clone(), assignment.team_b_id.clone()),
        ]
    } else {
        vec![
            (assignment.team_b.clone(), assignment.team_b_id.clone()),
            (assignment.team_a.clone(), assignment.team_a_id.clone()),
        ]
    };
    if let Some(g) = genesis {
        g.start_competition(genesis_admin_password, &grid);
    }
    let mut state = GameState::new(teams.clone(), grid);

    let team_names: Vec<String> = teams.iter().map(|(name, _)| name.clone()).collect();

    let genesis_url = genesis.map(|g| g.base_url().to_string());
    for (robot_id, (_, id)) in teams.iter().enumerate() {
        // The bot has no real board listening -- nothing to send it.
        if assignment.is_practice && id == crate::game_state::BOT_BOARD_ID {
            continue;
        }
        // Genesis's competition mode only knows these two hardcoded team
        // ids -- see `GameStart::genesis_team_id`'s doc comment.
        let genesis_team_id = genesis.map(|_| {
            if robot_id == 0 {
                "team_red"
            } else {
                "team_blue"
            }
            .to_string()
        });
        client.send(
            id,
            &serde_json::to_string(&RefereeMessage::GameStart {
                teams: team_names.clone(),
                total_pairs: state.total_pairs(),
                robot_id: robot_id as u32,
                genesis_team_id,
                genesis_url: genesis_url.clone(),
            })?,
        )?;
    }
    send_all(client, state.push_initial_turn_signals())?;
    // Report the freshly-started match to the Master immediately, not just
    // reactively on the first student message -- otherwise the scoreboard
    // has genuinely nothing for this arena (pregame already cleared, no
    // score update sent yet) until either team's first flip, which
    // `arena.html` renders as "Arena N idle" even though the match is
    // actually running and both boards already have `your_turn`/`wait`.
    report_to_master(
        client,
        master_id,
        MatchReport {
            arena: arena_num,
            pool: pool_num,
            state: &state,
            now: Instant::now(),
            puzzle_winner: &assignment.first_turn_team,
            genesis_stream_url: genesis_stream_url.clone(),
        },
    )?;

    loop {
        for raw in client.receive_all()? {
            if let Ok(admin) = serde_json::from_str::<MasterToArena>(&raw) {
                match admin {
                    MasterToArena::AdminSetScore { team, score } => state.set_score(&team, score),
                    MasterToArena::AdminPause => state.pause(Instant::now()),
                    MasterToArena::AdminResume => state.resume(Instant::now()),
                    MasterToArena::AdminStop => {
                        if let Some(g) = genesis {
                            g.stop_competition(genesis_admin_password);
                        }
                        return Ok(());
                    }
                    MasterToArena::AdminFinish => {
                        let msg = ArenaToMaster::MatchResult {
                            arena: arena_num,
                            pool: pool_num,
                            winner: state.winner(),
                            scores: state.scores(),
                            pairs_matched: state.pairs_matched_by_team(),
                            practice: assignment.is_practice,
                        };
                        client.send(master_id, &serde_json::to_string(&msg)?)?;
                        if let Some(g) = genesis {
                            g.stop_competition(genesis_admin_password);
                        }
                        return Ok(());
                    }
                    // Shouldn't arrive mid-match; harmless to ignore if it does.
                    MasterToArena::AssignMatch { .. } => {}
                    MasterToArena::AssignPracticeMatch { .. } => {
                        eprintln!(
                            "arena: discarding practice-match request while a match is already in progress"
                        );
                    }
                }
                report_to_master(
                    client,
                    master_id,
                    MatchReport {
                        arena: arena_num,
                        pool: pool_num,
                        state: &state,
                        now: Instant::now(),
                        puzzle_winner: &assignment.first_turn_team,
                        genesis_stream_url: genesis_stream_url.clone(),
                    },
                )?;
                continue;
            }

            let Ok(msg) = serde_json::from_str::<StudentMessage>(&raw) else {
                continue;
            };
            if state.is_paused() {
                continue;
            }
            let outgoing = match msg {
                StudentMessage::Flip { team, pos } => {
                    simulate_human_flip_delay(1);
                    state.receive_flip(&team, &pos)
                }
                StudentMessage::FlipBoth { team, pos1, pos2 } => {
                    simulate_human_flip_delay(2);
                    state.receive_flip_both(&team, &pos1, &pos2)
                }
                StudentMessage::ReportResult {
                    team,
                    pos1,
                    pos2,
                    claim,
                    ..
                } => match state.receive_result(&team, &pos1, &pos2, &claim) {
                    Some(outcome) => outcome.into_messages(),
                    None => vec![],
                },
                StudentMessage::HintRequest { team, object } => {
                    match state.receive_hint_request(&team, &object) {
                        Some(outcome) => outcome.into_messages(),
                        None => vec![],
                    }
                }
                // Join is handled by the lobby listener, not the arena.
                StudentMessage::Join { .. } => vec![],
            };
            send_all(client, outgoing)?;
            report_to_master(
                client,
                master_id,
                MatchReport {
                    arena: arena_num,
                    pool: pool_num,
                    state: &state,
                    now: Instant::now(),
                    puzzle_winner: &assignment.first_turn_team,
                    genesis_stream_url: genesis_stream_url.clone(),
                },
            )?;
        }

        if state.all_pairs_found() {
            let msg = ArenaToMaster::MatchResult {
                arena: arena_num,
                pool: pool_num,
                winner: state.winner(),
                scores: state.scores(),
                pairs_matched: state.pairs_matched_by_team(),
                practice: assignment.is_practice,
            };
            client.send(master_id, &serde_json::to_string(&msg)?)?;
            if let Some(g) = genesis {
                g.stop_competition(genesis_admin_password);
            }
            break;
        }

        if assignment.is_practice {
            let bot_messages = state.maybe_play_bot_turn(Instant::now());
            if !bot_messages.is_empty() {
                send_all(client, bot_messages)?;
                report_to_master(
                    client,
                    master_id,
                    MatchReport {
                        arena: arena_num,
                        pool: pool_num,
                        state: &state,
                        now: Instant::now(),
                        puzzle_winner: &assignment.first_turn_team,
                        genesis_stream_url: genesis_stream_url.clone(),
                    },
                )?;
            }
        }

        if let Some(outgoing) = state.check_timeout(Instant::now()) {
            send_all(client, outgoing)?;
        }

        sleep(POLL_INTERVAL);
    }

    Ok(())
}

/// Groups `report_to_master`'s match-context parameters (as opposed to
/// `client`/`master_id`, which are transport/routing concerns) into one
/// struct -- this function's parameter list crossed clippy's
/// `too_many_arguments` threshold once Genesis fields were added here.
/// A struct keeps future field additions a one-line change here instead
/// of a change at every one of this function's three call sites.
///
/// `genesis_stream_url` is computed once per match (not per report) in
/// `run_one_match` and threaded through unchanged -- it only depends on
/// whether Genesis is configured for this arena, not on live game state.
/// It was dropped entirely once GridMind switched to competition mode
/// (standard-mode's per-session stream token doesn't exist in competition
/// mode), and is only back now that Genesis's own `admin_start_competition`
/// handler registers its simulation under a fixed stream key -- see
/// `GenesisClient::competition_stream_url`.
struct MatchReport<'a> {
    arena: u32,
    pool: u32,
    state: &'a GameState,
    now: Instant,
    puzzle_winner: &'a str,
    genesis_stream_url: Option<String>,
}

fn report_to_master(client: &P2pClient, master_id: &str, report: MatchReport) -> Result<()> {
    let msg = ArenaToMaster::ScoreUpdate {
        arena: report.arena,
        pool: report.pool,
        scores: report.state.scores(),
        pairs_found: report.state.pairs_found(),
        total_pairs: report.state.total_pairs(),
        matched: report.state.matched_positions(),
        all_positions: report.state.all_positions(),
        active_team: report.state.active_team().to_string(),
        turn_seconds_remaining: report.state.turn_seconds_remaining(report.now),
        genesis_stream_url: report.genesis_stream_url,
        streak: report
            .state
            .scores()
            .keys()
            .map(|team| {
                let streak = if team == report.state.active_team() {
                    report.state.current_streak()
                } else {
                    0
                };
                (team.clone(), streak)
            })
            .collect(),
        hints_used: report.state.hints_used_map(),
        puzzle_winner: report.puzzle_winner.to_string(),
        match_started_at_unix_ms: report.state.match_started_at_unix_ms(),
        is_paused: report.state.is_paused(),
        flip_pending_positions: report.state.flip_pending_positions(),
    };
    client.send(master_id, &serde_json::to_string(&msg)?)
}

/// Delivers every `(recipient, message)` pair, continuing past a single
/// recipient's delivery failure (after `P2pClient::send`'s own internal
/// retries are exhausted) instead of aborting the rest of the batch --
/// `receive_flip_both` broadcasts 4 messages (2 positions x 2 teams) in
/// one batch, and losing delivery to one recipient must not also cost the
/// other 3 their messages. Only a `serde_json` serialization failure
/// (which should never happen for a `RefereeMessage`) still propagates,
/// since that's a genuine programming bug, not a network condition.
fn send_all(client: &P2pClient, messages: Vec<(String, RefereeMessage)>) -> Result<()> {
    for (id, msg) in messages {
        let payload = serde_json::to_string(&msg)?;
        if let Err(err) = client.send(&id, &payload) {
            eprintln!(
                "arena: failed to deliver a message to {id} after retries: {err:#} -- {id} may now be stuck waiting on it"
            );
        }
    }
    Ok(())
}
