use slack::{MessageBuilder, SendOptions, SlackBot};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Running this example sends two messages. Use a dedicated test channel.
    let bot = SlackBot::new(
        std::env::var("SLACK_BOT_TOKEN").map_err(|_| {
            "SLACK_BOT_TOKEN (Bot User OAuth Token) is required; CLIENT_SECRET and SIGNING_SECRET are separate app credentials"
        })?,
        std::env::var("SLACK_CHANNEL_ID")?,
    );
    let message = MessageBuilder::new()
        .line("service alert")
        .kv("service", "example")
        .kv("error", "database unavailable")
        .build();
    let sent = bot.send(&message).await?;
    bot.send_with_options(
        &slack::SlackMessage::plain("follow-up"),
        SendOptions {
            channel: Some(sent.channel),
            thread_ts: Some(sent.ts),
        },
    )
    .await?;
    Ok(())
}
