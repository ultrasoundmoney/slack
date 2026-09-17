#[cfg(any(feature = "async", feature = "blocking"))]
use crate::{MESSAGE_MAX_CHARS, SlackMessage};
use serde::Deserialize;
#[cfg(any(feature = "async", feature = "blocking"))]
use serde::Serialize;
#[cfg(any(feature = "async", feature = "blocking"))]
use std::borrow::Cow;
use std::{fmt, time::Duration};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Default)]
pub struct SendOptions {
    pub channel: Option<String>,
    pub thread_ts: Option<String>,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
pub struct SentMessage {
    pub channel: String,
    pub ts: String,
}
impl fmt::Debug for SentMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SentMessage { .. }")
    }
}

/// Remote response bodies and underlying HTTP errors are deliberately not retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Validation(&'static str),
    MessageTooLong { chars: usize, limit: usize },
    Transport,
    Http { status: u16 },
    SlackApi { code: String },
    MalformedResponse,
    RateLimited { retry_after: Option<Duration> },
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(reason) => write!(f, "invalid slack request: {reason}"),
            Self::MessageTooLong { chars, limit } => write!(
                f,
                "slack message text is {chars} characters after escaping; limit is {limit}"
            ),
            Self::Transport => f.write_str("slack transport failed; delivery may be uncertain"),
            Self::Http { status } => write!(f, "slack http status {status}"),
            Self::SlackApi { code } => write!(f, "slack api error: {code}"),
            Self::MalformedResponse => {
                f.write_str("malformed slack response; delivery may be uncertain")
            }
            Self::RateLimited { retry_after } => {
                write!(f, "slack rate limited; retry after {retry_after:?}")
            }
        }
    }
}
impl std::error::Error for Error {}

#[cfg(any(feature = "async", feature = "blocking"))]
#[derive(Serialize)]
pub(crate) struct Request<'a> {
    channel: &'a str,
    text: Cow<'a, str>,
    mrkdwn: bool,
    parse: &'static str,
    link_names: bool,
    unfurl_links: bool,
    unfurl_media: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
}
#[cfg(any(feature = "async", feature = "blocking"))]
impl<'a> Request<'a> {
    pub(crate) fn new(
        token: &str,
        channel: &'a str,
        message: &'a SlackMessage,
        options: &'a SendOptions,
    ) -> Result<Self, Error> {
        if token.trim().is_empty() || token.chars().any(char::is_control) {
            return Err(Error::Validation("invalid token"));
        }
        if channel.trim().is_empty() {
            return Err(Error::Validation("empty channel"));
        }
        if message.text().trim().is_empty() {
            return Err(Error::Validation("empty message"));
        }
        if options
            .thread_ts
            .as_ref()
            .is_some_and(|ts| ts.trim().is_empty())
        {
            return Err(Error::Validation("empty thread timestamp"));
        }
        let text = message.wire_text();
        let chars = text.chars().count();
        if chars > MESSAGE_MAX_CHARS {
            return Err(Error::MessageTooLong {
                chars,
                limit: MESSAGE_MAX_CHARS,
            });
        }
        Ok(Self {
            channel,
            text,
            mrkdwn: message.is_mrkdwn(),
            parse: "none",
            link_names: false,
            unfurl_links: false,
            unfurl_media: false,
            thread_ts: options.thread_ts.as_deref(),
        })
    }
}

#[cfg(any(feature = "async", feature = "blocking"))]
pub(crate) fn decode(
    status: u16,
    retry_after: Option<&str>,
    body: &[u8],
    token: &str,
) -> Result<SentMessage, Error> {
    if status == 429 {
        return Err(Error::RateLimited {
            retry_after: retry_after
                .and_then(|v| v.parse().ok())
                .map(Duration::from_secs),
        });
    }
    if !(200..300).contains(&status) {
        return Err(Error::Http { status });
    }
    #[derive(Deserialize)]
    struct Response {
        ok: bool,
        error: Option<String>,
        channel: Option<String>,
        ts: Option<String>,
    }
    let response: Response = serde_json::from_slice(body).map_err(|_| Error::MalformedResponse)?;
    if !response.ok {
        // Only expose known error codes. A proxy/remote body may echo arbitrary secrets.
        let code = response.error.as_deref().unwrap_or("unknown_error");
        let known = matches!(
            code,
            "invalid_auth"
                | "not_authed"
                | "token_revoked"
                | "account_inactive"
                | "missing_scope"
                | "channel_not_found"
                | "not_in_channel"
                | "is_archived"
                | "msg_too_long"
                | "no_text"
                | "invalid_arguments"
                | "ratelimited"
                | "internal_error"
                | "fatal_error"
                | "restricted_action"
                | "invalid_blocks"
                | "thread_not_found"
        );
        return Err(Error::SlackApi {
            code: if known && !code.contains(token) {
                code.to_owned()
            } else {
                "unknown_error".to_owned()
            },
        });
    }
    match (response.channel, response.ts) {
        (Some(channel), Some(ts)) if !channel.is_empty() && !ts.is_empty() => {
            Ok(SentMessage { channel, ts })
        }
        _ => Err(Error::MalformedResponse),
    }
}

#[cfg(any(feature = "async", feature = "blocking"))]
pub(crate) fn report(result: &Result<SentMessage, Error>) {
    match result {
        Ok(_) => tracing::debug!("slack message sent"),
        Err(error) => {
            let kind = match error {
                Error::Validation(_) => "validation",
                Error::MessageTooLong { .. } => "message_too_long",
                Error::Transport => "transport",
                Error::Http { .. } => "http",
                Error::SlackApi { .. } => "api",
                Error::MalformedResponse => "malformed_response",
                Error::RateLimited { .. } => "rate_limited",
            };
            tracing::warn!(error_kind = kind, "slack message failed");
        }
    }
}

#[cfg(all(test, any(feature = "async", feature = "blocking")))]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn logs_and_debug_omit_sensitive_fields() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(bytes.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            report(&decode(
                200,
                None,
                br#"{"ok":false,"error":"secret-token private-message CSECRET"}"#,
                "secret-token",
            ));
            report(&Ok(SentMessage {
                channel: "CSECRET".into(),
                ts: "1.2".into(),
            }));
        });
        let logs = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("slack message failed"));
        assert!(logs.contains("slack message sent"));
        for secret in ["secret-token", "private-message", "CSECRET"] {
            assert!(!logs.contains(secret));
        }
        assert!(
            !format!("{:?}", SlackMessage::plain("private-message")).contains("private-message")
        );
        #[cfg(feature = "async")]
        assert!(
            !format!("{:?}", crate::SlackBot::new("secret-token", "CSECRET"))
                .contains("secret-token")
        );
        #[cfg(feature = "blocking")]
        assert!(
            !format!(
                "{:?}",
                crate::blocking::BlockingSlackBot::new("secret-token", "CSECRET")
            )
            .contains("secret-token")
        );
    }
}
