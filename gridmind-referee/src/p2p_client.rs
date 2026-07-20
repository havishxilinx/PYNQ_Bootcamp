use anyhow::{Context, Result};
use std::time::Duration;

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

    pub fn send(&self, recipient_id: &str, message: &str) -> Result<()> {
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
    }

    /// Drains and returns all queued messages addressed to this client's own id.
    pub fn receive_all(&self) -> Result<Vec<String>> {
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
}
