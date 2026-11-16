//! Sends an alert and a threaded reply. Posts for real: point it at a test channel.
//!
//! SLACK_BOT_TOKEN=xoxb-... SLACK_CHANNEL_ID=C... cargo run --example async
use slack::{Message, MessageBuilder, Retry, SlackBot};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bot = SlackBot::new(
        std::env::var("SLACK_BOT_TOKEN")?,
        std::env::var("SLACK_CHANNEL_ID")?,
    )?
    .with_retry(Retry::at_least_once(Retry::ALERT_DELAYS));

    let alert = MessageBuilder::new("example alert")
        .context("network: mainnet | geo: rbx")
        .kv("slot", 12345)
        .code(
            "error",
            "pool timed out while waiting for an open connection\n  caused by: io timeout",
        )
        .link("dashboard", "https://grafana.example.com")
        .footer("relay submissions disabled until process restart")
        .build();

    let sent = bot.send(&alert).await?;
    bot.reply(&sent, &Message::plain("follow-up in thread"))
        .await?;

    Ok(())
}
