//! Request building and response decoding shared by both transports.
use std::{fmt, time::Duration};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{DEFAULT_TIMEOUT, Error, Message, Retry, SentMessage, escape_text, truncate};

/// Cap on resolving and connecting, so a failure there shows up as such instead of using the
/// whole request timeout.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_BASE_URL: &str = "https://slack.com/api";
const BODY_SNIPPET_CHARS: usize = 500;

#[derive(Clone)]
pub(crate) struct Config {
    pub token: String,
    pub default_channel: String,
    pub base_url: String,
    pub timeout: Duration,
    pub retry: Retry,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("default_channel", &self.default_channel)
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl Config {
    pub fn new(token: String, default_channel: String) -> Result<Self, Error> {
        let token = token.trim().to_owned();
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(Error::Invalid("token is empty or contains whitespace"));
        }
        if default_channel.trim().is_empty() {
            return Err(Error::Invalid("empty default channel"));
        }

        Ok(Self {
            token,
            default_channel,
            base_url: DEFAULT_BASE_URL.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            retry: Retry::none(),
        })
    }

    pub fn url(&self, method: &str) -> String {
        format!("{}/{method}", self.base_url.trim_end_matches('/'))
    }

    #[cfg(feature = "blocking")]
    pub fn connect_timeout(&self) -> Duration {
        CONNECT_TIMEOUT.min(self.timeout)
    }

    fn snippet(&self, body: &str) -> String {
        truncate(body, BODY_SNIPPET_CHARS).replace(&self.token, "[token]")
    }

    pub fn decode(
        &self,
        status: u16,
        retry_after: Option<&str>,
        body: &[u8],
    ) -> Result<Value, Error> {
        let snippet = || self.snippet(&String::from_utf8_lossy(body));

        if !(200..300).contains(&status) {
            return Err(Error::Http {
                status,
                retry_after: retry_after
                    .and_then(|value| value.trim().parse().ok())
                    .map(Duration::from_secs),
                body: snippet(),
            });
        }

        let value: Value = serde_json::from_slice(body).map_err(|source| Error::Malformed {
            body: snippet(),
            source: Some(source),
        })?;

        match value.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(value),
            Some(false) => Err(Error::Api {
                code: value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_error")
                    .to_owned(),
                body: snippet(),
            }),
            None => Err(Error::Malformed {
                body: snippet(),
                source: None,
            }),
        }
    }

    pub fn parse<T: DeserializeOwned>(&self, value: Value) -> Result<T, Error> {
        T::deserialize(&value).map_err(|source| Error::Malformed {
            body: self.snippet(&value.to_string()),
            source: Some(source),
        })
    }

    pub fn sent_message(&self, value: Value) -> Result<SentMessage, Error> {
        let truncated = value
            .pointer("/response_metadata/warnings")
            .and_then(Value::as_array)
            .is_some_and(|warnings| warnings.iter().any(|w| w == "message_truncated"));
        if truncated {
            tracing::warn!("slack reported message truncation");
        }

        let sent: SentMessage = self.parse(value.clone())?;
        if sent.channel.is_empty() || sent.ts.is_empty() {
            return Err(Error::Malformed {
                body: self.snippet(&value.to_string()),
                source: None,
            });
        }

        Ok(sent)
    }
}

/// The `chat.postMessage` body. Text is literal: escaped, mrkdwn off, no automatic linking,
/// mentions or previews.
pub(crate) fn post_message_body(
    channel: &str,
    message: &Message,
    thread_ts: Option<&str>,
) -> Result<Value, Error> {
    if channel.trim().is_empty() {
        return Err(Error::Invalid("empty channel"));
    }
    if message.text().trim().is_empty() {
        return Err(Error::Invalid("empty message"));
    }

    let mut body = json!({
        "channel": channel,
        "text": escape_text(message.text()),
        "mrkdwn": false,
        "parse": "none",
        "link_names": false,
        "unfurl_links": false,
        "unfurl_media": false,
    });
    if let Some(blocks) = message.blocks() {
        body["blocks"] = blocks.clone();
    }
    if let Some(thread_ts) = thread_ts {
        body["thread_ts"] = thread_ts.into();
    }

    Ok(body)
}

pub(crate) fn serialize(body: &impl serde::Serialize) -> Result<Value, Error> {
    serde_json::to_value(body)
        .map_err(|_| Error::Invalid("request body does not serialize to json"))
}

/// Slack refused the blocks themselves, so the text alone is still worth sending.
pub(crate) fn blocks_rejected(message: &Message, error: &Error) -> bool {
    message.blocks().is_some()
        && matches!(
            error.api_code(),
            Some("invalid_blocks" | "invalid_blocks_format" | "msg_blocks_too_long")
        )
}

pub(crate) fn log_blocks_fallback(error: &Error) {
    tracing::warn!(%error, "slack rejected message blocks; sending text only");
}

pub(crate) fn log_retry(error: &Error, retry: usize, delay: Duration) {
    tracing::warn!(%error, retry = retry + 1, ?delay, "slack call failed; retrying");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::new("xoxb-secret".into(), "C1".into()).unwrap()
    }

    #[test]
    fn tokens_are_checked_once_at_construction() {
        assert!(Config::new(" xoxb-1\n".into(), "C1".into()).is_ok());
        assert!(Config::new("xoxb 1".into(), "C1".into()).is_err());
        assert!(Config::new("".into(), "C1".into()).is_err());
        assert!(Config::new("xoxb-1".into(), " ".into()).is_err());
    }

    #[test]
    fn errors_keep_the_body_without_the_token() {
        let error = config()
            .decode(
                200,
                None,
                br#"{"ok":false,"error":"invalid_blocks","echo":"xoxb-secret"}"#,
            )
            .unwrap_err();

        assert_eq!(error.api_code(), Some("invalid_blocks"));
        assert!(error.to_string().contains("[token]"));
        assert!(!error.to_string().contains("xoxb-secret"));
    }

    #[test]
    fn non_success_status_keeps_retry_after_and_body() {
        let error = config().decode(429, Some("17"), b"slow down").unwrap_err();

        assert_eq!(error.retry_after(), Some(Duration::from_secs(17)));
        assert_eq!(
            error.to_string(),
            "slack http status 429, retry after 17s: slow down"
        );
    }

    #[test]
    fn success_needs_channel_and_ts() {
        let config = config();

        let missing_ts = config
            .decode(200, None, br#"{"ok":true,"channel":"C1"}"#)
            .and_then(|value| config.sent_message(value));
        assert!(matches!(missing_ts, Err(Error::Malformed { .. })));

        let sent = config
            .decode(200, None, br#"{"ok":true,"channel":"C1","ts":"1.2"}"#)
            .and_then(|value| config.sent_message(value))
            .unwrap();
        assert_eq!(sent.ts, "1.2");
    }
}
