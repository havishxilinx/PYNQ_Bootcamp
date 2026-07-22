use anyhow::{Context, Result};
use std::thread::sleep;
use std::time::Duration;

/// A transient broker hiccup (network blip, momentary broker overload)
/// shouldn't be allowed to surface as a fatal error on the first try --
/// doing so previously let a single failed `/send` call during
/// `receive_flip_both`'s 4-message broadcast crash the whole Arena process
/// mid-match, leaving both boards waiting forever on a `card_revealed`
/// that would never arrive.
const RETRY_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(250);

pub struct P2pClient {
    server: String,
    key: String,
    id: String,
    http: reqwest::blocking::Client,
}

impl P2pClient {
    pub fn new(server: &str, key: &str, id: &str) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");
        P2pClient {
            server: server.to_string(),
            key: key.to_string(),
            id: id.to_string(),
            http,
        }
    }

    /// Retries `attempt` up to `RETRY_ATTEMPTS` times, pausing `RETRY_DELAY`
    /// between tries, before giving up with the last error. Safe to apply
    /// to both `send` and `receive_all`: a duplicate `send` on an
    /// ambiguous (timed-out-but-maybe-delivered) retry is harmless here --
    /// every wire message this carries is already idempotent on the
    /// receiving end (e.g. `card_revealed` for an already-processed
    /// position is a no-op, see `game_state.rs`).
    fn with_retries<T>(mut attempt: impl FnMut() -> Result<T>) -> Result<T> {
        let mut last_err = None;
        for attempt_number in 1..=RETRY_ATTEMPTS {
            match attempt() {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if attempt_number < RETRY_ATTEMPTS {
                        sleep(RETRY_DELAY);
                    }
                    last_err = Some(err);
                }
            }
        }
        Err(last_err.expect("loop runs at least once, so an error was always recorded"))
    }

    pub fn send(&self, recipient_id: &str, message: &str) -> Result<()> {
        Self::with_retries(|| {
            self.http
                .post(format!("http://{}/send", self.server))
                .form(&[
                    ("key", self.key.as_str()),
                    ("id", recipient_id),
                    ("message", message),
                ])
                .send()
                .context("send request failed")?
                .error_for_status()
                .context("broker returned an error status for send")?;
            Ok(())
        })
    }

    /// Drains and returns all queued messages addressed to this client's own id.
    pub fn receive_all(&self) -> Result<Vec<String>> {
        Self::with_retries(|| {
            let resp = self
                .http
                .get(format!("http://{}/receive_all", self.server))
                .form(&[("key", self.key.as_str()), ("id", self.id.as_str())])
                .send()
                .context("receive_all request failed")?
                .error_for_status()
                .context("broker returned an error status for receive_all")?;
            let text = resp.text().context("failed to read receive_all response")?;
            Ok(text
                .lines()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_posts_form_encoded_message_to_broker() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/send")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("key".into(), "testkey".into()),
                mockito::Matcher::UrlEncoded("id".into(), "team-a".into()),
                mockito::Matcher::UrlEncoded("message".into(), "hello".into()),
            ]))
            .with_status(200)
            .with_body("Message sent to team-a")
            .create();

        let client = P2pClient::new(&server.host_with_port(), "testkey", "referee-1");
        client.send("team-a", "hello").unwrap();
        mock.assert();
    }

    #[test]
    fn receive_all_parses_newline_delimited_messages() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/receive_all")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("key".into(), "testkey".into()),
                mockito::Matcher::UrlEncoded("id".into(), "referee-1".into()),
            ]))
            .with_status(200)
            .with_body("msg-one\nmsg-two\n")
            .create();

        let client = P2pClient::new(&server.host_with_port(), "testkey", "referee-1");
        let messages = client.receive_all().unwrap();
        assert_eq!(messages, vec!["msg-one".to_string(), "msg-two".to_string()]);
        mock.assert();
    }

    #[test]
    fn receive_all_returns_empty_vec_when_no_messages_queued() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/receive_all")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("key".into(), "testkey".into()),
                mockito::Matcher::UrlEncoded("id".into(), "referee-1".into()),
            ]))
            .with_status(200)
            .with_body("")
            .create();

        let client = P2pClient::new(&server.host_with_port(), "testkey", "referee-1");
        let messages = client.receive_all().unwrap();
        assert!(messages.is_empty());
        mock.assert();
    }

    #[test]
    fn send_retries_on_a_transient_broker_error_and_eventually_succeeds() {
        let mut server = mockito::Server::new();
        // First call fails (matched once), then a second mock on the same
        // route takes over and succeeds -- mockito falls through to the
        // next matching mock once an `.expect(n)`-bounded one is exhausted.
        let failing_mock = server
            .mock("POST", "/send")
            .with_status(500)
            .expect(1)
            .create();
        let succeeding_mock = server
            .mock("POST", "/send")
            .with_status(200)
            .with_body("Message sent to team-a")
            .create();

        let client = P2pClient::new(&server.host_with_port(), "testkey", "referee-1");
        client.send("team-a", "hello").unwrap();
        failing_mock.assert();
        succeeding_mock.assert();
    }

    #[test]
    fn send_gives_up_after_retry_attempts_are_exhausted() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/send")
            .with_status(500)
            .expect(RETRY_ATTEMPTS as usize)
            .create();

        let client = P2pClient::new(&server.host_with_port(), "testkey", "referee-1");
        let result = client.send("team-a", "hello");
        assert!(result.is_err());
        mock.assert();
    }

    #[test]
    fn receive_all_retries_on_a_transient_broker_error_and_eventually_succeeds() {
        let mut server = mockito::Server::new();
        let failing_mock = server
            .mock("GET", "/receive_all")
            .with_status(500)
            .expect(1)
            .create();
        let succeeding_mock = server
            .mock("GET", "/receive_all")
            .with_status(200)
            .with_body("msg-one\n")
            .create();

        let client = P2pClient::new(&server.host_with_port(), "testkey", "referee-1");
        let messages = client.receive_all().unwrap();
        assert_eq!(messages, vec!["msg-one".to_string()]);
        failing_mock.assert();
        succeeding_mock.assert();
    }
}
