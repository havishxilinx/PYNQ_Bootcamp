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
    genesis_url: Option<String>,
    genesis_admin_password: String,
) -> Result<()> {
    let client = P2pClient::new(server, key, my_id);
    let genesis = genesis_url.map(|url| GenesisClient::new(&url));
    loop {
        let assignment = wait_for_assignment(&client)?;
        run_one_match(
            &client,
            master_id,
            arena_num,
            assignment,
            genesis.as_ref(),
            &genesis_admin_password,
        )?;
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
) -> Result<()> {
    let pool_num = assignment.pool;
    let grid = load_grid(&assignment.grid_id)?;
    // Order teams so the puzzle-race winner goes first (GameState always
    // starts with teams[0] active).
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
                        };
                        client.send(master_id, &serde_json::to_string(&msg)?)?;
                        if let Some(g) = genesis {
                            g.stop_competition(genesis_admin_password);
                        }
                        return Ok(());
                    }
                    // Shouldn't arrive mid-match; harmless to ignore if it does.
                    MasterToArena::AssignMatch { .. } => {}
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
            };
            client.send(master_id, &serde_json::to_string(&msg)?)?;
            if let Some(g) = genesis {
                g.stop_competition(genesis_admin_password);
            }
            break;
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
/// Genesis fields are deliberately NOT included: they were only ever used
/// to build the scoreboard's live video-stream URL, which only worked for
/// Genesis's "standard mode" sessions. Competition mode (needed for real
/// per-flip arm animation, see `GenesisClient::start_competition`) has no
/// per-match token to stream from at all, so that embed is gone -- Genesis
/// connection details now only flow to students directly via `GameStart`.
struct MatchReport<'a> {
    arena: u32,
    pool: u32,
    state: &'a GameState,
    now: Instant,
    puzzle_winner: &'a str,
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

fn send_all(client: &P2pClient, messages: Vec<(String, RefereeMessage)>) -> Result<()> {
    for (id, msg) in messages {
        client.send(&id, &serde_json::to_string(&msg)?)?;
    }
    Ok(())
}
