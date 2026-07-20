use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// The only scene in Genesis's real API matching GridMind's game --
/// confirmed against the real server source (`scenes/competition_card_flip.py`)
/// to be a 6-row x 5-col grid (30 cells, 15 pairs), matching GridMind's own
/// grid sizes exactly (earlier "4x5 mismatch" concerns were based on a
/// stale doc, not the real implementation).
const SCENE: &str = "competition_card_flip";

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
        self.post_action("admin_start_competition", &body).is_some()
    }

    /// Stops Genesis's competition scene at match end. Never lets a
    /// failure affect the real match.
    pub fn stop_competition(&self, admin_password: &str) {
        let body = json!({
            "action": "admin_stop_competition",
            "token": Value::Null,
            "params": { "password": admin_password },
        });
        let _ = self.post_action("admin_stop_competition", &body); // no response body needed, just best-effort delivery
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
    fn post_action(&self, action_name: &str, body: &Value) -> Option<Value> {
        let response = match self.http.post(&self.base_url).json(body).send() {
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
/// before "A2"), then deliberately reshape into Genesis's mirror-transposed
/// fixed 6-row x 5-col scene layout (same 30 cells, rows/cols swapped).
/// Cell count matches so every name-pair from the real grid survives --
/// only exact spatial position doesn't carry over, which doesn't matter
/// for a memory-matching game's correctness. If a grid ever has a
/// different cell count (e.g. the 2x4/3x5 dev-fixture grids), Genesis's
/// own layout validation already rejects a mismatched cell count and
/// falls back to a random layout -- no special-casing needed here.
fn build_card_layout(grid: &HashMap<String, String>) -> Vec<Vec<String>> {
    let mut positions: Vec<&String> = grid.keys().collect();
    positions.sort();
    let names: Vec<String> = positions.iter().map(|pos| grid[*pos].clone()).collect();
    names.chunks(5).map(|chunk| chunk.to_vec()).collect()
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
    fn build_card_layout_reshapes_a_thirty_cell_grid_into_six_rows_of_five() {
        let grid: HashMap<String, String> = (0..30)
            .map(|i| {
                let row = (b'A' + (i / 6) as u8) as char;
                let col = (i % 6) + 1;
                (format!("{row}{col}"), format!("obj{i}"))
            })
            .collect();
        let layout = build_card_layout(&grid);
        assert_eq!(layout.len(), 6);
        for row in &layout {
            assert_eq!(row.len(), 5);
        }
    }
}
