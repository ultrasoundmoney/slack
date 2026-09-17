use slack::{SlackMessage, blocking::BlockingSlackBot};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Running this example sends a message. Use a dedicated test channel.
    let bot = BlockingSlackBot::new(
        std::env::var("SLACK_BOT_TOKEN").map_err(|_| {
            "SLACK_BOT_TOKEN (Bot User OAuth Token) is required; CLIENT_SECRET and SIGNING_SECRET are separate app credentials"
        })?,
        std::env::var("SLACK_CHANNEL_ID")?,
    );
    bot.send(&SlackMessage::plain("hello from the blocking example"))?;
    Ok(())
}
