#![doc = include_str!("../README.md")]
#[cfg(any(feature = "async", feature = "blocking"))]
mod core;
mod error;
mod message;
mod retry;

pub use error::{Error, Stage};
pub use message::{MESSAGE_MAX_CHARS, Message, MessageBuilder, escape_text, truncate};
pub use retry::Retry;

#[cfg(feature = "async")]
mod async_bot;
#[cfg(feature = "async")]
pub use async_bot::SlackBot;
#[cfg(feature = "blocking")]
pub mod blocking;

/// Per attempt request timeout unless overridden with `with_timeout`.
pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

/// Where a posted message landed. Pass it to `reply` to thread under it.
#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct SentMessage {
    pub channel: String,
    pub ts: String,
}
