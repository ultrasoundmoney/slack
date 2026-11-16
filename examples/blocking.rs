//! Sends one alert from a plain thread. Posts for real: point it at a test channel.
//!
//! SLACK_BOT_TOKEN=xoxb-... SLACK_CHANNEL_ID=C... cargo run --example blocking --features blocking
use slack::{MessageBuilder, Retry, blocking::SlackBot};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bot = SlackBot::new(
        std::env::var("SLACK_BOT_TOKEN")?,
        std::env::var("SLACK_CHANNEL_ID")?,
    )?
    .with_retry(Retry::at_least_once(Retry::ALERT_DELAYS));

    let alert = MessageBuilder::new("example alert from a blocking thread")
        .kv("service", "example")
        .code("error", "database unavailable")
        .build();
    bot.send(&alert)?;

    Ok(())
}
