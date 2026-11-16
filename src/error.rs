use std::{fmt, time::Duration};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// How far a request got before its transport failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Resolving, connecting or the TLS handshake. Nothing reached Slack.
    Connect,
    /// The request was sent, or partly sent, and no complete response came back. Slack may have
    /// acted on it.
    Exchange,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect => f.write_str("connect"),
            Self::Exchange => f.write_str("exchange"),
        }
    }
}

/// Everything we know about a failed call, meant to be logged or shown as is.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Rejected before any network traffic, e.g. an empty message or a malformed token.
    Invalid(&'static str),
    Transport {
        stage: Stage,
        source: BoxError,
    },
    /// A non-2xx response. `body` is the start of the response body.
    Http {
        status: u16,
        retry_after: Option<Duration>,
        body: String,
    },
    /// Slack answered `ok: false`. `body` is the start of the response body, which often names
    /// the offending field or missing scope.
    Api {
        code: String,
        body: String,
    },
    /// A 2xx response we could not make sense of.
    Malformed {
        body: String,
        source: Option<serde_json::Error>,
    },
}

/// Slack error codes that describe a passing condition on Slack's side.
const TRANSIENT_API_CODES: &[&str] = &[
    "ratelimited",
    "rate_limited",
    "internal_error",
    "fatal_error",
    "service_unavailable",
    "request_timeout",
];

/// Slack documents these as possibly having partly succeeded.
const UNCERTAIN_API_CODES: &[&str] = &["internal_error", "fatal_error", "request_timeout"];

impl Error {
    /// The failure may clear up if the same request is sent again later.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Invalid(_) => false,
            Self::Transport { .. } | Self::Malformed { .. } => true,
            Self::Http { status, .. } => *status == 408 || *status == 429 || *status >= 500,
            Self::Api { code, .. } => TRANSIENT_API_CODES.contains(&code.as_str()),
        }
    }

    /// Slack may have acted on the request even though the call failed. Sending again can
    /// duplicate a message.
    pub fn delivery_uncertain(&self) -> bool {
        match self {
            Self::Invalid(_) => false,
            Self::Transport { stage, .. } => *stage == Stage::Exchange,
            Self::Malformed { .. } => true,
            Self::Http { status, .. } => *status == 408 || *status >= 500,
            Self::Api { code, .. } => UNCERTAIN_API_CODES.contains(&code.as_str()),
        }
    }

    /// How long Slack asked us to wait, from a 429's `Retry-After` header.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Http { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// The Slack error code, when Slack answered `ok: false`.
    pub fn api_code(&self) -> Option<&str> {
        match self {
            Self::Api { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid slack request: {reason}"),
            Self::Transport { stage, source } => {
                write!(f, "slack transport failed during {stage}")?;
                std::iter::successors(
                    Some(source.as_ref() as &(dyn std::error::Error + 'static)),
                    |error| error.source(),
                )
                .try_for_each(|error| write!(f, ": {error}"))
            }
            Self::Http {
                status,
                retry_after,
                body,
            } => {
                write!(f, "slack http status {status}")?;
                if let Some(retry_after) = retry_after {
                    write!(f, ", retry after {}s", retry_after.as_secs())?;
                }
                if !body.is_empty() {
                    write!(f, ": {body}")?;
                }
                Ok(())
            }
            Self::Api { code, body } => write!(f, "slack api error {code}: {body}"),
            Self::Malformed { body, source } => {
                f.write_str("malformed slack response")?;
                if let Some(source) = source {
                    write!(f, " ({source})")?;
                }
                write!(f, ": {body}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport { source, .. } => Some(source.as_ref()),
            Self::Malformed {
                source: Some(source),
                ..
            } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(code: &str) -> Error {
        Error::Api {
            code: code.to_string(),
            body: String::new(),
        }
    }

    fn http(status: u16) -> Error {
        Error::Http {
            status,
            retry_after: None,
            body: String::new(),
        }
    }

    fn transport(stage: Stage) -> Error {
        Error::Transport {
            stage,
            source: "boom".into(),
        }
    }

    #[test]
    fn classifies_retry_and_duplicate_risk() {
        let cases = [
            (Error::Invalid("empty message"), false, false),
            (transport(Stage::Connect), true, false),
            (transport(Stage::Exchange), true, true),
            (http(429), true, false),
            (http(503), true, true),
            (http(403), false, false),
            (api("channel_not_found"), false, false),
            (api("ratelimited"), true, false),
            (api("service_unavailable"), true, false),
            (api("internal_error"), true, true),
            (
                Error::Malformed {
                    body: String::new(),
                    source: None,
                },
                true,
                true,
            ),
        ];

        cases.iter().for_each(|(error, transient, uncertain)| {
            assert_eq!(error.is_transient(), *transient, "{error}");
            assert_eq!(error.delivery_uncertain(), *uncertain, "{error}");
        });
    }

    #[test]
    fn display_includes_the_whole_source_chain() {
        let io = std::io::Error::new(std::io::ErrorKind::TimedOut, "resolve timed out");
        let error = Error::Transport {
            stage: Stage::Connect,
            source: Box::new(io),
        };

        assert_eq!(
            error.to_string(),
            "slack transport failed during connect: resolve timed out"
        );
    }
}
