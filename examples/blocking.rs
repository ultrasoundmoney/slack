use slack::{SlackMessage, blocking::BlockingSlackBot};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Running this example sends a message. Use a dedicated test channel.
    let bot = BlockingSlackBot::new(
        std::env::var("SLACK_BOT_TOKEN")?,
        std::env::var("SLACK_CHANNEL_ID")?,
    );
    bot.send(&SlackMessage::plain("hello from the blocking example"))?;
    Ok(())
}
