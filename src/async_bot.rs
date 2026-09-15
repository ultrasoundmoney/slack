use crate::{
    DEFAULT_TIMEOUT, Error, SendOptions, SentMessage, SlackMessage,
    protocol::{Request, decode, report},
};
use std::time::Duration;

#[derive(Clone)]
pub struct SlackBot {
    client: reqwest::Client,
    token: String,
    default_channel: String,
    timeout: Duration,
    base_url: String,
}
impl std::fmt::Debug for SlackBot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SlackBot { .. }")
    }
}
impl SlackBot {
    pub fn new(token: impl Into<String>, default_channel: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .expect("rustls client configuration");
        Self::with_client(client, token, default_channel)
    }
    /// Injected clients must disable redirects and retries to retain single-request semantics.
    pub fn with_client(
        client: reqwest::Client,
        token: impl Into<String>,
        default_channel: impl Into<String>,
    ) -> Self {
        Self {
            client,
            token: token.into(),
            default_channel: default_channel.into(),
            timeout: DEFAULT_TIMEOUT,
            base_url: "https://slack.com/api".to_owned(),
        }
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// Overrides the API root for tests/proxies. This endpoint receives the bot token.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
    pub async fn send(&self, message: &SlackMessage) -> Result<SentMessage, Error> {
        self.send_with_options(message, SendOptions::default())
            .await
    }
    pub async fn send_to(
        &self,
        channel: impl Into<String>,
        message: &SlackMessage,
    ) -> Result<SentMessage, Error> {
        self.send_with_options(
            message,
            SendOptions {
                channel: Some(channel.into()),
                ..Default::default()
            },
        )
        .await
    }
    pub async fn send_with_options(
        &self,
        message: &SlackMessage,
        options: SendOptions,
    ) -> Result<SentMessage, Error> {
        let result = self.send_once(message, &options).await;
        report(&result);
        result
    }
    async fn send_once(
        &self,
        message: &SlackMessage,
        options: &SendOptions,
    ) -> Result<SentMessage, Error> {
        let request = Request::new(
            &self.token,
            options.channel.as_deref().unwrap_or(&self.default_channel),
            message,
            options,
        )?;
        let response = self
            .client
            .post(format!(
                "{}/chat.postMessage",
                self.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.token)
            .json(&request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        if !(200..300).contains(&status) {
            return decode(status, retry_after.as_deref(), &[], &self.token);
        }
        let body = response.bytes().await.map_err(|_| Error::Transport)?;
        decode(status, retry_after.as_deref(), &body, &self.token)
    }
}
