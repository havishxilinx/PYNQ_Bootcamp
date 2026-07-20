use crate::join_registry::JoinRegistry;
use crate::master::{MasterState, OperatorChannels};
use crate::scoreboard_state::ScoreboardState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc::error::TrySendError;

const SCOREBOARD_HTML: &str = include_str!("../static/scoreboard.html");
const OPERATOR_HTML: &str = include_str!("../static/operator.html");
const ARENA_HTML: &str = include_str!("../static/arena.html");

#[derive(Clone)]
pub struct AppState {
    pub master_state: MasterState,
    pub operator_channels: OperatorChannels,
    pub join_registry: JoinRegistry,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/", get(scoreboard_page))
        .route("/operator", get(operator_page))
        .route("/arena", get(arena_page))
        .route("/api/config", get(game_config))
        .route("/ws", get(ws_handler))
        .route("/api/start-tournament", post(start_tournament))
        .route("/api/start-match", post(start_match))
        .route("/api/register-team", post(register_team))
        .route("/api/move-team", post(move_team))
        .route("/api/close-registration", post(close_registration))
        .route("/api/join-status", get(join_status))
        .route("/api/admin/set-score", post(admin_set_score))
        .route("/api/admin/pause", post(admin_pause))
        .route("/api/admin/resume", post(admin_resume))
        .route("/api/admin/stop", post(admin_stop))
        .route("/api/admin/finish", post(admin_finish))
        .with_state(state)
}

pub async fn serve(port: u16, state: AppState) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn scoreboard_page() -> Html<&'static str> {
    Html(SCOREBOARD_HTML)
}

async fn operator_page() -> Html<&'static str> {
    Html(OPERATOR_HTML)
}

async fn arena_page() -> Html<&'static str> {
    Html(ARENA_HTML)
}

/// Serves the tunable timing/scoring constants (see `config::GameConfig`)
/// so the arena UI's countdown and tier display can mirror the same
/// values the referee is actually scoring with, instead of duplicating
/// them as hardcoded JS -- editing `data/game_config.json` before an event
/// updates both sides from one source.
async fn game_config() -> Json<crate::config::GameConfig> {
    Json(crate::config::get().clone())
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.master_state))
}

async fn handle_socket(mut socket: WebSocket, master_state: MasterState) {
    let mut receiver = master_state.subscribe();
    // Send the current state immediately -- a freshly-connected browser
    // shouldn't have to wait for the next change to see anything.
    let initial = serde_json::to_string(&master_state.snapshot()).unwrap_or_default();
    if socket.send(Message::Text(initial)).await.is_err() {
        return;
    }
    loop {
        if receiver.changed().await.is_err() {
            return; // MasterState was dropped -- server shutting down
        }
        let state: ScoreboardState = receiver.borrow_and_update().clone();
        let Ok(json) = serde_json::to_string(&state) else {
            continue;
        };
        if socket.send(Message::Text(json)).await.is_err() {
            return; // browser disconnected
        }
    }
}

/// Turns a channel send outcome into a status the operator's browser can
/// actually see. Silently swallowing this (as an earlier version of this
/// code did) meant a dead orchestration thread or a stalled receiver
/// looked identical to a real success -- the operator would see "ok" and
/// wait indefinitely for a match that was never going to start.
fn send_result_status<T>(result: Result<(), TrySendError<T>>) -> (StatusCode, &'static str) {
    match result {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(TrySendError::Full(_)) => (StatusCode::SERVICE_UNAVAILABLE, "busy, try again"),
        Err(TrySendError::Closed(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "orchestrator not running",
        ),
    }
}

async fn start_tournament(State(state): State<AppState>) -> impl IntoResponse {
    send_result_status(state.operator_channels.start_tournament.try_send(()))
}

#[derive(Deserialize)]
struct StartMatchRequest {
    arena: u32,
    winner: String,
    team_a_mac: String,
    team_b_mac: String,
    /// Bypasses the join-gating check below. Exists for the documented
    /// manual-MAC-entry fallback (see `prompt_and_assign` in `master.rs`)
    /// -- a board that never successfully calls `join_competition` (network
    /// issue, forgot `TEAM_SECRET`, etc.) shouldn't permanently block its
    /// match if the operator can otherwise confirm it's ready. Defaults to
    /// false so bypassing the check is always a deliberate, visible choice.
    #[serde(default)]
    force: bool,
}

async fn start_match(
    State(state): State<AppState>,
    Json(body): Json<StartMatchRequest>,
) -> impl IntoResponse {
    if !body.force {
        let joined_macs: std::collections::HashSet<String> = state
            .join_registry
            .snapshot()
            .into_values()
            .map(|info| info.mac)
            .collect();
        if !joined_macs.contains(&body.team_a_mac) || !joined_macs.contains(&body.team_b_mac) {
            return (
                StatusCode::CONFLICT,
                "both teams must call join_competition before a match can start (resubmit with force: true to override)",
            );
        }
    }

    let input = crate::master::MatchStartInput {
        puzzle_winner: body.winner,
        team_a_mac: body.team_a_mac,
        team_b_mac: body.team_b_mac,
    };
    let sender = match body.arena {
        1 => &state.operator_channels.match_start_arena1,
        2 => &state.operator_channels.match_start_arena2,
        _ => return (StatusCode::BAD_REQUEST, "arena must be 1 or 2"),
    };
    send_result_status(sender.try_send(input))
}

#[derive(Deserialize)]
struct RegisterTeamRequest {
    name: String,
    students: Vec<String>,
}

async fn register_team(
    State(state): State<AppState>,
    Json(body): Json<RegisterTeamRequest>,
) -> impl IntoResponse {
    send_result_status(state.operator_channels.registration.try_send(
        crate::master::RegistrationAction::RegisterTeam {
            name: body.name,
            students: body.students,
        },
    ))
}

#[derive(Deserialize)]
struct MoveTeamRequest {
    name: String,
    to_pool: u32,
}

async fn move_team(
    State(state): State<AppState>,
    Json(body): Json<MoveTeamRequest>,
) -> impl IntoResponse {
    send_result_status(state.operator_channels.registration.try_send(
        crate::master::RegistrationAction::MoveTeam {
            name: body.name,
            to_pool: body.to_pool,
        },
    ))
}

async fn close_registration(State(state): State<AppState>) -> impl IntoResponse {
    send_result_status(
        state
            .operator_channels
            .registration
            .try_send(crate::master::RegistrationAction::CloseRegistration),
    )
}

#[derive(Serialize)]
struct JoinStatusEntry {
    mac: String,
    seconds_ago: u64,
}

async fn join_status(State(state): State<AppState>) -> impl IntoResponse {
    let response: HashMap<String, JoinStatusEntry> = state
        .join_registry
        .snapshot()
        .into_iter()
        .map(|(team, info)| {
            (
                team,
                JoinStatusEntry {
                    mac: info.mac,
                    seconds_ago: info.joined_at.elapsed().as_secs(),
                },
            )
        })
        .collect();
    Json(response)
}

/// Resolves an arena number to its admin-command channel, matching the
/// same `1`/`2`/other-is-invalid pattern as `start_match`'s dispatch.
fn admin_sender(
    state: &AppState,
    arena: u32,
) -> Result<&tokio::sync::mpsc::Sender<crate::master::AdminCommand>, (StatusCode, &'static str)> {
    match arena {
        1 => Ok(&state.operator_channels.admin_arena1),
        2 => Ok(&state.operator_channels.admin_arena2),
        _ => Err((StatusCode::BAD_REQUEST, "arena must be 1 or 2")),
    }
}

#[derive(Deserialize)]
struct AdminSetScoreRequest {
    arena: u32,
    team: String,
    score: i32,
}

async fn admin_set_score(
    State(state): State<AppState>,
    Json(body): Json<AdminSetScoreRequest>,
) -> impl IntoResponse {
    let sender = match admin_sender(&state, body.arena) {
        Ok(sender) => sender,
        Err(response) => return response,
    };
    send_result_status(sender.try_send(crate::master::AdminCommand::SetScore {
        team: body.team,
        score: body.score,
    }))
}

#[derive(Deserialize)]
struct AdminArenaRequest {
    arena: u32,
}

async fn admin_pause(
    State(state): State<AppState>,
    Json(body): Json<AdminArenaRequest>,
) -> impl IntoResponse {
    let sender = match admin_sender(&state, body.arena) {
        Ok(sender) => sender,
        Err(response) => return response,
    };
    send_result_status(sender.try_send(crate::master::AdminCommand::Pause))
}

async fn admin_resume(
    State(state): State<AppState>,
    Json(body): Json<AdminArenaRequest>,
) -> impl IntoResponse {
    let sender = match admin_sender(&state, body.arena) {
        Ok(sender) => sender,
        Err(response) => return response,
    };
    send_result_status(sender.try_send(crate::master::AdminCommand::Resume))
}

async fn admin_stop(
    State(state): State<AppState>,
    Json(body): Json<AdminArenaRequest>,
) -> impl IntoResponse {
    let sender = match admin_sender(&state, body.arena) {
        Ok(sender) => sender,
        Err(response) => return response,
    };
    send_result_status(sender.try_send(crate::master::AdminCommand::Stop))
}

async fn admin_finish(
    State(state): State<AppState>,
    Json(body): Json<AdminArenaRequest>,
) -> impl IntoResponse {
    let sender = match admin_sender(&state, body.arena) {
        Ok(sender) => sender,
        Err(response) => return response,
    };
    send_result_status(sender.try_send(crate::master::AdminCommand::Finish))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::master::operator_channels;
    use crate::scoreboard_state::PoolPreview;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_app_state() -> (AppState, crate::master::MasterReceivers) {
        let initial = ScoreboardState::Idle {
            pool1: PoolPreview {
                teams: vec!["alpha".to_string()],
                total_matches: 0,
            },
            pool2: PoolPreview {
                teams: vec!["delta".to_string()],
                total_matches: 0,
            },
        };
        let master_state = MasterState::new(initial);
        let (operator_channels, rx) = operator_channels();
        (
            AppState {
                master_state,
                operator_channels,
                join_registry: crate::join_registry::JoinRegistry::new(),
            },
            rx,
        )
    }

    #[tokio::test]
    async fn health_route_returns_ok() {
        let (state, _rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn scoreboard_page_serves_html() {
        let (state, _rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn operator_page_serves_html() {
        let (state, _rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/operator")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn arena_page_serves_html() {
        let (state, _rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/arena")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn start_tournament_sends_on_the_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-tournament")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(rx.start.try_recv(), Ok(()));
    }

    #[tokio::test]
    async fn start_match_sends_the_winner_on_the_channel() {
        let (state, mut rx) = test_app_state();
        state.join_registry.record("alpha", "aa:aa");
        state.join_registry.record("delta", "bb:bb");
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-match")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"arena":1,"winner":"alpha","team_a_mac":"aa:aa","team_b_mac":"bb:bb"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.match_start_arena1.try_recv(),
            Ok(crate::master::MatchStartInput {
                puzzle_winner: "alpha".to_string(),
                team_a_mac: "aa:aa".to_string(),
                team_b_mac: "bb:bb".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn start_match_rejects_when_a_team_has_not_joined() {
        let (state, mut rx) = test_app_state();
        state.join_registry.record("alpha", "aa:aa");
        // "delta" (bb:bb) never joined.
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-match")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"arena":1,"winner":"alpha","team_a_mac":"aa:aa","team_b_mac":"bb:bb"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(rx.match_start_arena1.try_recv().is_err());
    }

    #[tokio::test]
    async fn start_match_force_bypasses_the_join_gate() {
        let (state, mut rx) = test_app_state();
        // Neither team ever joined -- this is the documented manual-fallback case.
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-match")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"arena":1,"winner":"alpha","team_a_mac":"aa:aa","team_b_mac":"bb:bb","force":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.match_start_arena1.try_recv(),
            Ok(crate::master::MatchStartInput {
                puzzle_winner: "alpha".to_string(),
                team_a_mac: "aa:aa".to_string(),
                team_b_mac: "bb:bb".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn start_match_for_arena_two_sends_on_the_arena_two_channel() {
        let (state, mut rx) = test_app_state();
        state.join_registry.record("gamma", "cc:cc");
        state.join_registry.record("epsilon", "dd:dd");
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-match")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"arena":2,"winner":"epsilon","team_a_mac":"cc:cc","team_b_mac":"dd:dd"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.match_start_arena2.try_recv(),
            Ok(crate::master::MatchStartInput {
                puzzle_winner: "epsilon".to_string(),
                team_a_mac: "cc:cc".to_string(),
                team_b_mac: "dd:dd".to_string(),
            })
        );
        // The arena-1 channel must NOT have received this submission.
        assert!(rx.match_start_arena1.try_recv().is_err());
    }

    #[tokio::test]
    async fn start_match_rejects_an_arena_number_other_than_one_or_two() {
        let (state, mut rx) = test_app_state();
        state.join_registry.record("alpha", "aa:aa");
        state.join_registry.record("delta", "bb:bb");
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-match")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"arena":99,"winner":"alpha","team_a_mac":"aa:aa","team_b_mac":"bb:bb"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(rx.match_start_arena1.try_recv().is_err());
        assert!(rx.match_start_arena2.try_recv().is_err());
    }

    #[tokio::test]
    async fn start_tournament_returns_500_when_the_orchestrator_is_gone() {
        let (state, rx) = test_app_state();
        drop(rx); // simulates run_master having exited
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-tournament")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn start_match_returns_503_when_the_channel_is_full() {
        let (state, mut rx) = test_app_state();
        state.join_registry.record("gamma", "cc:cc");
        state.join_registry.record("epsilon", "dd:dd");
        // Fill the capacity-1 arena-1 channel so the next try_send hits Full.
        state
            .operator_channels
            .match_start_arena1
            .try_send(crate::master::MatchStartInput {
                puzzle_winner: "first".to_string(),
                team_a_mac: "aa:aa".to_string(),
                team_b_mac: "bb:bb".to_string(),
            })
            .unwrap();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start-match")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"arena":1,"winner":"alpha","team_a_mac":"cc:cc","team_b_mac":"dd:dd"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            rx.match_start_arena1.try_recv(),
            Ok(crate::master::MatchStartInput {
                puzzle_winner: "first".to_string(),
                team_a_mac: "aa:aa".to_string(),
                team_b_mac: "bb:bb".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn register_team_sends_on_the_registration_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/register-team")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"gamma","students":["Wren"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.registration.try_recv(),
            Ok(crate::master::RegistrationAction::RegisterTeam {
                name: "gamma".to_string(),
                students: vec!["Wren".to_string()],
            })
        );
    }

    #[tokio::test]
    async fn move_team_sends_on_the_registration_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/move-team")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"gamma","to_pool":2}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.registration.try_recv(),
            Ok(crate::master::RegistrationAction::MoveTeam {
                name: "gamma".to_string(),
                to_pool: 2,
            })
        );
    }

    #[tokio::test]
    async fn close_registration_sends_on_the_registration_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/close-registration")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.registration.try_recv(),
            Ok(crate::master::RegistrationAction::CloseRegistration)
        );
    }

    #[tokio::test]
    async fn join_status_route_returns_ok_with_recorded_joins() {
        let (state, _rx) = test_app_state();
        state.join_registry.record("alpha", "aa:aa:aa:aa:aa:aa");
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/join-status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn join_status_route_returns_ok_when_nobody_has_joined() {
        let (state, _rx) = test_app_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/join-status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn post_json(app: Router, uri: &str, body: &str) -> axum::http::Response<Body> {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn admin_set_score_sends_on_the_arena_one_admin_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = post_json(
            app,
            "/api/admin/set-score",
            r#"{"arena":1,"team":"alpha","score":-5}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.admin_arena1.try_recv(),
            Ok(crate::master::AdminCommand::SetScore {
                team: "alpha".to_string(),
                score: -5,
            })
        );
        assert!(rx.admin_arena2.try_recv().is_err());
    }

    #[tokio::test]
    async fn admin_pause_sends_on_the_arena_two_admin_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = post_json(app, "/api/admin/pause", r#"{"arena":2}"#).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.admin_arena2.try_recv(),
            Ok(crate::master::AdminCommand::Pause)
        );
        assert!(rx.admin_arena1.try_recv().is_err());
    }

    #[tokio::test]
    async fn admin_resume_sends_on_the_admin_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = post_json(app, "/api/admin/resume", r#"{"arena":1}"#).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.admin_arena1.try_recv(),
            Ok(crate::master::AdminCommand::Resume)
        );
    }

    #[tokio::test]
    async fn admin_stop_sends_on_the_admin_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = post_json(app, "/api/admin/stop", r#"{"arena":1}"#).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.admin_arena1.try_recv(),
            Ok(crate::master::AdminCommand::Stop)
        );
    }

    #[tokio::test]
    async fn admin_finish_sends_on_the_admin_channel() {
        let (state, mut rx) = test_app_state();
        let app = build_router(state);
        let response = post_json(app, "/api/admin/finish", r#"{"arena":1}"#).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rx.admin_arena1.try_recv(),
            Ok(crate::master::AdminCommand::Finish)
        );
    }

    #[tokio::test]
    async fn admin_endpoints_reject_an_arena_number_other_than_one_or_two() {
        let (state, _rx) = test_app_state();
        let app = build_router(state);
        let response = post_json(app, "/api/admin/pause", r#"{"arena":99}"#).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
