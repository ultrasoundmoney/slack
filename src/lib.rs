#![doc = include_str!("../README.md")]
//! Small, single-request Slack bot client. See the README for setup and delivery semantics.
mod message;
mod protocol;
pub use message::{MESSAGE_MAX_CHARS, MessageBuilder, ParseMode, SlackMessage, ValueBudget};
pub use protocol::{DEFAULT_TIMEOUT, Error, SendOptions, SentMessage};
#[cfg(feature = "async")]
mod async_bot;
#[cfg(feature = "async")]
pub use async_bot::SlackBot;
#[cfg(feature = "blocking")]
pub mod blocking;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
