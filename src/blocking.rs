//! Synchronous client, for threads outside an async runtime. Retries sleep the calling thread.
use std::{io::ErrorKind, thread, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    Error, Message, Retry, SentMessage, Stage,
    core::{Config, blocks_rejected, log_blocks_fallback, log_retry, post_message_body, serialize},
};

/// Blocking client for the Slack Web API, authenticated with one token.
#[derive(Clone, Debug)]
pub struct SlackBot {
    agent: ureq::Agent,
    config: Config,
}

impl SlackBot {
    /// Fails when the token or channel is unusable, so a bad config shows up at startup.
    pub fn new(
        token: impl Into<String>,
        default_channel: impl Into<String>,
    ) -> Result<Self, Error> {
        Ok(Self {
            agent: ureq::Agent::new_with_defaults(),
            config: Config::new(token.into(), default_channel.into())?,
        })
    }

    /// Bounds each attempt, from resolving to reading the response. Defaults to six seconds.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.config.retry = retry;
        self
    }

    /// Points the client somewhere other than `https://slack.com/api`, e.g. a test server.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.config.base_url = base_url.into();
        self
    }

    pub fn send(&self, message: &Message) -> Result<SentMessage, Error> {
        self.post(&self.config.default_channel, message, None)
    }

    pub fn send_to(&self, channel: &str, message: &Message) -> Result<SentMessage, Error> {
        self.post(channel, message, None)
    }

    /// Posts in the thread of `parent`.
    pub fn reply(&self, parent: &SentMessage, message: &Message) -> Result<SentMessage, Error> {
        self.post(&parent.channel, message, Some(&parent.ts))
    }

    /// Calls any Web API method with a JSON body, e.g. `chat.update`. Returns the whole
    /// response, which must have `ok: true`.
    pub fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        let value = self.call_value(method, &serialize(body)?)?;
        self.config.parse(value)
    }

    fn post(
        &self,
        channel: &str,
        message: &Message,
        thread_ts: Option<&str>,
    ) -> Result<SentMessage, Error> {
        let body = post_message_body(channel, message, thread_ts)?;

        let value = match self.call_value("chat.postMessage", &body) {
            Err(error) if blocks_rejected(message, &error) => {
                log_blocks_fallback(&error);
                let body = post_message_body(channel, &message.without_blocks(), thread_ts)?;
                self.call_value("chat.postMessage", &body)?
            }
            result => result?,
        };

        self.config.sent_message(value)
    }

    fn call_value(&self, method: &str, body: &Value) -> Result<Value, Error> {
        let mut retry = 0;
        loop {
            let error = match self.attempt(method, body) {
                Ok(value) => return Ok(value),
                Err(error) => error,
            };
            let Some(delay) = self.config.retry.delay(retry, &error) else {
                return Err(error);
            };
            log_retry(&error, retry, delay);

            thread::sleep(delay);
            retry += 1;
        }
    }

    fn attempt(&self, method: &str, body: &Value) -> Result<Value, Error> {
        let connect_timeout = Some(self.config.connect_timeout());
        let mut response = self
            .agent
            .post(self.config.url(method))
            .config()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(self.config.timeout))
            .timeout_resolve(connect_timeout)
            .timeout_connect(connect_timeout)
            .build()
            .header("authorization", format!("Bearer {}", self.config.token))
            .send_json(body)
            .map_err(transport_error)?;

        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value: &ureq::http::HeaderValue| value.to_str().ok())
            .map(str::to_owned);
        let body = response.body_mut().read_to_vec().map_err(transport_error)?;

        self.config.decode(status, retry_after.as_deref(), &body)
    }
}

fn transport_error(error: ureq::Error) -> Error {
    let stage = match &error {
        ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::ConnectProxyFailed(_)
        | ureq::Error::Tls(_)
        | ureq::Error::TlsRequired
        | ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => Stage::Connect,
        ureq::Error::Io(io) if io.kind() == ErrorKind::ConnectionRefused => Stage::Connect,
        _ => Stage::Exchange,
    };
    Error::Transport {
        stage,
        source: Box::new(error),
    }
}
