//! Optional synchronous transport; do not call it on an async executor thread.
use crate::{
    DEFAULT_TIMEOUT, Error, SendOptions, SentMessage, SlackMessage,
    protocol::{Request, decode, report},
};
use std::time::Duration;

#[derive(Clone)]
pub struct BlockingSlackBot {
    agent: ureq::Agent,
    token: String,
    default_channel: String,
    timeout: Duration,
    base_url: String,
}
impl std::fmt::Debug for BlockingSlackBot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlockingSlackBot { .. }")
    }
}
impl BlockingSlackBot {
    pub fn new(token: impl Into<String>, default_channel: impl Into<String>) -> Self {
        Self::with_client(ureq::Agent::new_with_defaults(), token, default_channel)
    }
    pub fn with_client(
        agent: ureq::Agent,
        token: impl Into<String>,
        default_channel: impl Into<String>,
    ) -> Self {
        Self {
            agent,
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
    pub fn send(&self, message: &SlackMessage) -> Result<SentMessage, Error> {
        self.send_with_options(message, SendOptions::default())
    }
    pub fn send_to(
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
    }
    pub fn send_with_options(
        &self,
        message: &SlackMessage,
        options: SendOptions,
    ) -> Result<SentMessage, Error> {
        let result = self.send_once(message, &options);
        report(&result);
        result
    }
    fn send_once(
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
        let mut response = self
            .agent
            .post(format!(
                "{}/chat.postMessage",
                self.base_url.trim_end_matches('/')
            ))
            .config()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(self.timeout))
            .build()
            .header("authorization", format!("Bearer {}", self.token))
            .send_json(&request)
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
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|_| Error::Transport)?;
        decode(status, retry_after.as_deref(), &body, &self.token)
    }
}
