use slack::{MessageBuilder, ParseMode, SendOptions, SlackBot};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Running this example sends two messages. Use a dedicated test channel.
    let bot = SlackBot::new(
        std::env::var("SLACK_BOT_TOKEN")?,
        std::env::var("SLACK_CHANNEL_ID")?,
    );
    let message = MessageBuilder::new(ParseMode::Mrkdwn)
        .heading("service alert")
        .kv("service", "example")
        .error("error", "database unavailable")
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
