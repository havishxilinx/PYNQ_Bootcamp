use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// The only scene in Genesis's real API matching GridMind's game --
/// `scenes/competition_card_flip.py` builds a 5-row x 6-col grid (30 cells,
/// 15 pairs), matching GridMind's own grid shape (5 letter-rows x 6
/// numeric-cols) exactly, so `build_card_layout` needs no reshaping.
const SCENE: &str = "competition_card_flip";

/// Default timeout for ordinary admin calls (e.g. `admin_stop_competition`),
/// which just flip a flag server-side and return immediately.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// `admin_start_competition` isn't a simple flag flip -- it builds an
/// entire physics scene server-side first (backend/GPU init, loading the
/// robot models, laying out the card grid) before responding, which can
/// easily take longer than a few seconds, especially on a cold start.
/// Genesis's own HTTP server is single-threaded, so a timeout here doesn't
/// even stop that work -- it just means the referee gives up waiting and
/// wrongly treats a still-in-progress build as a failure.
const SCENE_BUILD_TIMEOUT: Duration = Duration::from_secs(45);

/// HTTP client for the separately-owned Genesis simulated-arm server.
/// Purely cosmetic: every method here swallows all failures (network
/// errors, non-2xx responses, unexpected JSON, or a body-level
/// `"status": "error"`) and never lets Genesis affect the real match.
pub struct GenesisClient {
    base_url: String,
    http: reqwest::blocking::Client,
}

impl GenesisClient {
    pub fn new(base_url: &str) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");
        GenesisClient {
            base_url: base_url.to_string(),
            http,
        }
    }

    /// The base URL this client was constructed with -- used by callers
    /// that need to hand it onward (e.g. to students via `GameStart`) in
    /// addition to using it internally for requests.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// URL of the live MJPEG view for the single, currently-active
    /// competition-mode match, served by Genesis's separate stream server
    /// (`stream_server.py`) -- a different process/port than the admin
    /// API this client posts actions to. Requires the Genesis-side fix
    /// that registers the competition simulation under the fixed
    /// `"competition"` key (see `GenesisRequestHandler.COMPETITION_STREAM_TOKEN`
    /// server-side); without it this URL 404s.
    ///
    /// `stream_port` is a separate value from `base_url`'s own port --
    /// Genesis's admin API and stream server listen on two different
    /// ports on the same host (see `GENESIS_STREAM_PORT` in Genesis's own
    /// config). Returns `None` only if `base_url` isn't a well-formed
    /// `scheme://host[:port]` URL, which never happens for a real
    /// deployment (defensive only).
    pub fn competition_stream_url(&self, stream_port: u16) -> Option<String> {
        let (scheme, rest) = self.base_url.split_once("://")?;
        let host = rest.split(['/', ':']).next()?;
        Some(format!("{scheme}://{host}:{stream_port}/stream/competition"))
    }

    /// Starts Genesis's own "competition mode" for this match -- the only
    /// mode that exposes real per-flip arm animation (`flip_card`) and
    /// turn-gating, as opposed to the earlier `create_env`/"standard mode"
    /// integration, which could only reach generic, position-unaware
    /// `move_robot`/`gripper` calls. Sends GridMind's actual grid as a
    /// best-effort `card_layout` (see `build_card_layout`). Unlike
    /// standard mode, this doesn't return a per-match token -- students
    /// each get their own token later by calling `join_competition` with
    /// a fixed `"team_red"`/`"team_blue"` id (see `GameStart::genesis_team_id`).
    /// Returns whether the call succeeded; never lets a failure affect the
    /// real match.
    pub fn start_competition(&self, admin_password: &str, grid: &HashMap<String, String>) -> bool {
        let body = json!({
            "action": "admin_start_competition",
            "token": Value::Null,
            "params": {
                "password": admin_password,
                "scene": SCENE,
                "card_layout": { "grid": build_card_layout(grid) },
            },
        });
        self.post_action("admin_start_competition", &body, SCENE_BUILD_TIMEOUT)
            .is_some()
    }

    /// Stops Genesis's competition scene at match end. Never lets a
    /// failure affect the real match.
    pub fn stop_competition(&self, admin_password: &str) {
        let body = json!({
            "action": "admin_stop_competition",
            "token": Value::Null,
            "params": { "password": admin_password },
        });
        // no response body needed, just best-effort delivery
        let _ = self.post_action("admin_stop_competition", &body, DEFAULT_TIMEOUT);
    }

    /// Shared request/response handling for both actions: posts `body`,
    /// and returns the parsed response JSON only if the request actually
    /// succeeded end to end. The real Genesis server always answers with
    /// HTTP 200, even on error -- success or failure is signaled by a
    /// `"status": "ok"`/`"error"` field inside the JSON body, not the
    /// HTTP status code. `response.status().is_success()` is still
    /// checked first as a defensive layer against a misbehaving
    /// intermediary (e.g. a reverse proxy returning a real 5xx), which
    /// the real server itself will never produce.
    fn post_action(&self, action_name: &str, body: &Value, timeout: Duration) -> Option<Value> {
        let response = match self.http.post(&self.base_url).timeout(timeout).json(body).send() {
            Ok(response) => response,
            Err(err) => {
                eprintln!(
                    "genesis: {action_name} request failed: {err} (cosmetic only, match unaffected)"
                );
                return None;
            }
        };
        if !response.status().is_success() {
            eprintln!(
                "genesis: {action_name} returned status {} (cosmetic only, match unaffected)",
                response.status()
            );
            return None;
        }
        let value = match response.json::<Value>() {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "genesis: failed to parse {action_name} response: {err} (cosmetic only, match unaffected)"
                );
                return None;
            }
        };
        if value.get("status").and_then(Value::as_str) != Some("ok") {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            eprintln!(
                "genesis: {action_name} returned an error: {message} (cosmetic only, match unaffected)"
            );
            return None;
        }
        Some(value)
    }
}

/// GridMind's own grid is keyed "A1".."E6" -- 5 letter-rows (A-E) x 6
/// numeric-cols (1-6), 30 cells total. Flatten in that natural row-major
/// order (sorting the keys as plain strings gives correct row-major order
/// for every grid file that exists today, since each one uses a single
/// uppercase letter row and a single-digit column -- this would silently
/// misorder if a future grid ever used a column >= 10, e.g. "A10" sorting
/// before "A2") and chunk by row width -- Genesis's own scene is also a
/// 5-row x 6-col grid, so each name lands at the exact same [row][col] it
/// occupies in GridMind, no reshaping needed. If a grid ever has a
/// different cell count (e.g. the 2x4/3x5 dev-fixture grids), Genesis's
/// own layout validation already rejects a mismatched cell count and
/// falls back to a random layout -- no special-casing needed here.
fn build_card_layout(grid: &HashMap<String, String>) -> Vec<Vec<String>> {
    let mut positions: Vec<&String> = grid.keys().collect();
    positions.sort();
    let names: Vec<String> = positions.iter().map(|pos| grid[*pos].clone()).collect();
    names.chunks(6).map(|chunk| chunk.to_vec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_grid() -> HashMap<String, String> {
        HashMap::from([
            ("A1".to_string(), "dog".to_string()),
            ("A2".to_string(), "dog".to_string()),
        ])
    }

    #[test]
    fn start_competition_posts_the_real_action_token_params_envelope() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(json!({
                "action": "admin_start_competition",
                "token": null,
            })))
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        client.start_competition("admin123", &test_grid());
        mock.assert();
    }

    #[test]
    fn start_competition_sends_password_scene_and_card_layout_in_params() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(json!({
                "params": {
                    "password": "admin123",
                    "scene": "competition_card_flip",
                    "card_layout": { "grid": [["dog", "dog"]] },
                }
            })))
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        client.start_competition("admin123", &test_grid());
        mock.assert();
    }

    #[test]
    fn start_competition_returns_true_on_a_successful_response() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        assert!(client.start_competition("admin123", &test_grid()));
    }

    #[test]
    fn start_competition_returns_false_when_status_is_error_even_with_http_200() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(r#"{"status":"error","message":"boom"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        assert!(!client.start_competition("admin123", &test_grid()));
    }

    #[test]
    fn start_competition_returns_false_and_does_not_panic_on_a_non_2xx_response() {
        let mut server = mockito::Server::new();
        let _mock = server.mock("POST", "/").with_status(500).create();

        let client = GenesisClient::new(&server.url());
        assert!(!client.start_competition("admin123", &test_grid()));
    }

    #[test]
    fn start_competition_returns_false_and_does_not_panic_when_unreachable() {
        // Port 1 is reserved; nothing will ever be listening there.
        let client = GenesisClient::new("http://127.0.0.1:1");
        assert!(!client.start_competition("admin123", &test_grid()));
    }

    #[test]
    fn stop_competition_posts_the_password_in_the_envelope() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Json(json!({
                "action": "admin_stop_competition",
                "token": null,
                "params": { "password": "admin123" },
            })))
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        client.stop_competition("admin123");
        mock.assert();
    }

    #[test]
    fn stop_competition_does_not_panic_on_failure() {
        let mut server = mockito::Server::new();
        let _mock = server.mock("POST", "/").with_status(500).create();

        let client = GenesisClient::new(&server.url());
        client.stop_competition("admin123");
    }

    #[test]
    fn stop_competition_does_not_panic_when_unreachable() {
        let client = GenesisClient::new("http://127.0.0.1:1");
        client.stop_competition("admin123");
    }

    #[test]
    fn base_url_returns_the_configured_url() {
        let client = GenesisClient::new("http://example.com:9002");
        assert_eq!(client.base_url(), "http://example.com:9002");
    }

    #[test]
    fn competition_stream_url_swaps_in_the_stream_port_and_keeps_the_host() {
        let client = GenesisClient::new("http://example.com:9002");
        assert_eq!(
            client.competition_stream_url(8080),
            Some("http://example.com:8080/stream/competition".to_string())
        );
    }

    #[test]
    fn competition_stream_url_works_with_a_bare_ip_and_no_path() {
        let client = GenesisClient::new("http://127.0.0.1:9002");
        assert_eq!(
            client.competition_stream_url(8080),
            Some("http://127.0.0.1:8080/stream/competition".to_string())
        );
    }

    #[test]
    fn competition_stream_url_returns_none_for_a_malformed_base_url() {
        let client = GenesisClient::new("not-a-url");
        assert_eq!(client.competition_stream_url(8080), None);
    }

    #[test]
    fn build_card_layout_keeps_the_grids_natural_five_rows_of_six() {
        let grid: HashMap<String, String> = (0..30)
            .map(|i| {
                let row = (b'A' + (i / 6) as u8) as char;
                let col = (i % 6) + 1;
                (format!("{row}{col}"), format!("obj{i}"))
            })
            .collect();
        let layout = build_card_layout(&grid);
        assert_eq!(layout.len(), 5);
        for row in &layout {
            assert_eq!(row.len(), 6);
        }
        // Each name lands at the same [row][col] GridMind itself uses --
        // no transpose, e.g. "A1" (row 0, col 0) is "obj0".
        assert_eq!(layout[0][0], "obj0");
        assert_eq!(layout[0][5], "obj5");
        assert_eq!(layout[4][0], "obj24");
    }
}
