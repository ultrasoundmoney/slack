use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    Error, Message, Retry, SentMessage, Stage,
    core::{
        CONNECT_TIMEOUT, Config, blocks_rejected, log_blocks_fallback, log_retry,
        post_message_body, serialize,
    },
};

/// Async client for the Slack Web API, authenticated with one token.
#[derive(Clone, Debug)]
pub struct SlackBot {
    client: reqwest::Client,
    config: Config,
}

impl SlackBot {
    /// Fails when the token or channel is unusable, so a bad config shows up at startup.
    pub fn new(
        token: impl Into<String>,
        default_channel: impl Into<String>,
    ) -> Result<Self, Error> {
        let config = Config::new(token.into(), default_channel.into())?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|error| Error::Transport {
                stage: Stage::Connect,
                source: Box::new(error),
            })?;

        Ok(Self { client, config })
    }

    /// Bounds each attempt, from connecting to reading the response. Defaults to six seconds.
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

    pub async fn send(&self, message: &Message) -> Result<SentMessage, Error> {
        self.post(&self.config.default_channel, message, None).await
    }

    pub async fn send_to(&self, channel: &str, message: &Message) -> Result<SentMessage, Error> {
        self.post(channel, message, None).await
    }

    /// Posts in the thread of `parent`.
    pub async fn reply(
        &self,
        parent: &SentMessage,
        message: &Message,
    ) -> Result<SentMessage, Error> {
        self.post(&parent.channel, message, Some(&parent.ts)).await
    }

    /// Calls any Web API method with a JSON body, e.g. `chat.update`. Returns the whole
    /// response, which must have `ok: true`.
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        let value = self.call_value(method, &serialize(body)?).await?;
        self.config.parse(value)
    }

    async fn post(
        &self,
        channel: &str,
        message: &Message,
        thread_ts: Option<&str>,
    ) -> Result<SentMessage, Error> {
        let body = post_message_body(channel, message, thread_ts)?;

        let value = match self.call_value("chat.postMessage", &body).await {
            Err(error) if blocks_rejected(message, &error) => {
                log_blocks_fallback(&error);
                let body = post_message_body(channel, &message.without_blocks(), thread_ts)?;
                self.call_value("chat.postMessage", &body).await?
            }
            result => result?,
        };

        self.config.sent_message(value)
    }

    async fn call_value(&self, method: &str, body: &Value) -> Result<Value, Error> {
        let mut retry = 0;
        loop {
            let error = match self.attempt(method, body).await {
                Ok(value) => return Ok(value),
                Err(error) => error,
            };
            let Some(delay) = self.config.retry.delay(retry, &error) else {
                return Err(error);
            };
            log_retry(&error, retry, delay);

            tokio::time::sleep(delay).await;
            retry += 1;
        }
    }

    async fn attempt(&self, method: &str, body: &Value) -> Result<Value, Error> {
        let response = self
            .client
            .post(self.config.url(method))
            .bearer_auth(&self.config.token)
            .json(body)
            .timeout(self.config.timeout)
            .send()
            .await
            .map_err(transport_error)?;

        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.bytes().await.map_err(transport_error)?;

        self.config.decode(status, retry_after.as_deref(), &body)
    }
}

fn transport_error(error: reqwest::Error) -> Error {
    let stage = if error.is_connect() {
        Stage::Connect
    } else {
        Stage::Exchange
    };
    Error::Transport {
        stage,
        source: Box::new(error),
    }
}
