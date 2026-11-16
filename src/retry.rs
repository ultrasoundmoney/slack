use std::time::Duration;

use crate::Error;

/// When a failed call is sent again. Off by default.
///
/// Only transient failures are retried, see [`Error::is_transient`]. Each wait is the configured
/// delay or Slack's `Retry-After`, whichever is longer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Retry {
    delays: Vec<Duration>,
    allow_duplicates: bool,
}

impl Retry {
    /// Waits used by the examples and our alerting: three retries over about a minute once the
    /// default request timeout is included.
    pub const ALERT_DELAYS: [Duration; 3] = [
        Duration::from_secs(5),
        Duration::from_secs(10),
        Duration::from_secs(20),
    ];

    pub fn none() -> Self {
        Self::default()
    }

    /// Retries only when Slack cannot have acted on the failed attempt, so it never duplicates a
    /// message. Covers connection failures and rate limiting.
    pub fn when_undelivered(delays: impl Into<Vec<Duration>>) -> Self {
        Self {
            delays: delays.into(),
            allow_duplicates: false,
        }
    }

    /// Also retries when the outcome is unknown, such as a timeout after the request was sent.
    /// A message can then appear twice. Right for alerts, where a duplicate beats silence.
    pub fn at_least_once(delays: impl Into<Vec<Duration>>) -> Self {
        Self {
            delays: delays.into(),
            allow_duplicates: true,
        }
    }

    /// The wait before retry number `retry` (zero-based), or `None` to give up.
    #[cfg_attr(not(any(feature = "async", feature = "blocking")), allow(dead_code))]
    pub(crate) fn delay(&self, retry: usize, error: &Error) -> Option<Duration> {
        let retryable =
            error.is_transient() && (self.allow_duplicates || !error.delivery_uncertain());
        let delay = self.delays.get(retry).filter(|_| retryable)?;

        Some(
            error
                .retry_after()
                .map_or(*delay, |retry_after| retry_after.max(*delay)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Stage;

    fn transport(stage: Stage) -> Error {
        Error::Transport {
            stage,
            source: "boom".into(),
        }
    }

    #[test]
    fn retries_follow_the_configured_delays_then_stop() {
        let retry = Retry::at_least_once(Retry::ALERT_DELAYS);
        let error = transport(Stage::Exchange);

        let delays: Vec<_> = (0..5).map_while(|n| retry.delay(n, &error)).collect();

        assert_eq!(delays, Retry::ALERT_DELAYS);
    }

    #[test]
    fn when_undelivered_skips_uncertain_failures() {
        let retry = Retry::when_undelivered([Duration::from_secs(1)]);

        assert_eq!(
            retry.delay(0, &transport(Stage::Connect)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(retry.delay(0, &transport(Stage::Exchange)), None);
    }

    #[test]
    fn permanent_failures_and_none_never_retry() {
        let rejected = Error::Api {
            code: "channel_not_found".into(),
            body: String::new(),
        };

        assert_eq!(
            Retry::at_least_once([Duration::ZERO]).delay(0, &rejected),
            None
        );
        assert_eq!(Retry::none().delay(0, &transport(Stage::Connect)), None);
    }

    #[test]
    fn waits_at_least_retry_after() {
        let retry = Retry::when_undelivered([Duration::from_secs(1)]);
        let rate_limited = Error::Http {
            status: 429,
            retry_after: Some(Duration::from_secs(30)),
            body: String::new(),
        };

        assert_eq!(retry.delay(0, &rate_limited), Some(Duration::from_secs(30)));
    }
}
