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

    /// Sets up the Genesis scene for a match, sending GridMind's actual
    /// grid as a best-effort `card_layout` (see `build_card_layout`).
    /// Returns the server's `token` if it gave one, for use in the
    /// matching `destroy_env` call and for distributing to students via
    /// `GameStart` -- `None` on any failure.
    pub fn create_env(&self, grid: &HashMap<String, String>) -> Option<String> {
        let body = json!({
            "action": "create_env",
            "token": Value::Null,
            "params": {
                "scene": SCENE,
                "card_layout": { "grid": build_card_layout(grid) },
            },
        });
        let value = self.post_action("create_env", &body)?;
        value
            .get("token")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
    }

    /// Tears down the Genesis scene at match end. `token` should be
    /// whatever `create_env` returned, if anything -- sent as JSON `null`
    /// when absent, matching the real server's envelope shape exactly
    /// (it tolerates an unknown/missing token gracefully rather than
    /// erroring). Never lets a failure affect the real match.
    pub fn destroy_env(&self, token: Option<&str>) {
        let body = json!({
            "action": "destroy_env",
            "token": token,
            "params": {},
        });
        let _ = self.post_action("destroy_env", &body); // no response body needed, just best-effort delivery
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
    fn create_env_posts_the_real_action_token_params_envelope() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(json!({
                "action": "create_env",
                "token": null,
            })))
            .with_status(200)
            .with_body(r#"{"status":"ok","token":"abc123"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        client.create_env(&test_grid());
        mock.assert();
    }

    #[test]
    fn create_env_sends_scene_and_card_layout_in_params() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(json!({
                "params": {
                    "scene": "competition_card_flip",
                    "card_layout": { "grid": [["dog", "dog"]] },
                }
            })))
            .with_status(200)
            .with_body(r#"{"status":"ok","token":"abc123"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        client.create_env(&test_grid());
        mock.assert();
    }

    #[test]
    fn create_env_extracts_token_from_a_successful_response() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(r#"{"status":"ok","token":"abc123"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        assert_eq!(client.create_env(&test_grid()), Some("abc123".to_string()));
    }

    #[test]
    fn create_env_returns_none_when_status_is_error_even_with_http_200() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(r#"{"status":"error","message":"boom"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        assert_eq!(client.create_env(&test_grid()), None);
    }

    #[test]
    fn create_env_returns_none_when_the_response_has_no_token() {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        assert_eq!(client.create_env(&test_grid()), None);
    }

    #[test]
    fn create_env_returns_none_and_does_not_panic_on_a_non_2xx_response() {
        let mut server = mockito::Server::new();
        let _mock = server.mock("POST", "/").with_status(500).create();

        let client = GenesisClient::new(&server.url());
        assert_eq!(client.create_env(&test_grid()), None);
    }

    #[test]
    fn create_env_returns_none_and_does_not_panic_when_unreachable() {
        // Port 1 is reserved; nothing will ever be listening there.
        let client = GenesisClient::new("http://127.0.0.1:1");
        assert_eq!(client.create_env(&test_grid()), None);
    }

    #[test]
    fn destroy_env_posts_the_token_in_the_envelope_when_present() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Json(json!({
                "action": "destroy_env",
                "token": "abc123",
                "params": {},
            })))
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        client.destroy_env(Some("abc123"));
        mock.assert();
    }

    #[test]
    fn destroy_env_posts_null_token_when_absent() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Json(json!({
                "action": "destroy_env",
                "token": null,
                "params": {},
            })))
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create();

        let client = GenesisClient::new(&server.url());
        client.destroy_env(None);
        mock.assert();
    }

    #[test]
    fn destroy_env_does_not_panic_on_failure() {
        let mut server = mockito::Server::new();
        let _mock = server.mock("POST", "/").with_status(500).create();

        let client = GenesisClient::new(&server.url());
        client.destroy_env(None);
    }

    #[test]
    fn destroy_env_does_not_panic_when_unreachable() {
        let client = GenesisClient::new("http://127.0.0.1:1");
        client.destroy_env(Some("abc123"));
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
