# slack

Small Rust Slack bot client and safe operational message builders. Posts through
`chat.postMessage` using a bot token; no listener or public endpoint is needed.

```toml
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.1.0" }
```

Rust 2024. Default feature: `async` (`reqwest`). Optional: `blocking` (`ureq`).
Disable default features for message construction without an HTTP transport.
The crate is distributed by Git tags, not crates.io. `slack::VERSION` exposes
the package version.

## Async sending

```rust,no_run
# #[cfg(feature = "async")]
# async fn example() -> Result<(), slack::Error> {
use slack::{MessageBuilder, ParseMode, SlackBot, SlackMessage, SendOptions};
let bot = SlackBot::new("BOT_TOKEN", "C_DEFAULT_CHANNEL");
let message = MessageBuilder::new(ParseMode::Mrkdwn)
    .heading("relay alert")
    .kv("service", "auction-api")
    .kv_code("slot", 12345)
    .error("error", "simulation failed: unexpected `root`")
    .build();
let sent = bot.send(&message).await?;
bot.send_to("C_OTHER_CHANNEL", &SlackMessage::plain("status update")).await?;
bot.send_with_options(&SlackMessage::plain("follow-up"), SendOptions {
    channel: Some(sent.channel),
    thread_ts: Some(sent.ts),
}).await?;
# Ok(())
# }
```

## Blocking sending

Enable `blocking`, optionally disabling the default `async` feature:

```toml
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.1.0", default-features = false, features = ["blocking"] }
```

```rust,no_run
# #[cfg(feature = "blocking")]
# fn example() -> Result<(), slack::Error> {
use slack::{SlackMessage, blocking::BlockingSlackBot};
let bot = BlockingSlackBot::new("BOT_TOKEN", "C_DEFAULT_CHANNEL");
bot.send(&SlackMessage::plain("service started"))?;
# Ok(())
# }
```

Both clients support `with_client`, `with_timeout`, `send`, `send_to`, and
`send_with_options`. The default timeout is six seconds. Blocking calls belong
outside async executor threads. `with_base_url` supports test servers or trusted
proxies: the supplied endpoint receives your token. Injected async clients must
disable redirects and retries; default clients do so. Blocking requests disable
redirects per request.

## Message safety and budgets

`MessageBuilder::default()` and `SlackMessage::plain` use plain text. `Mrkdwn`
adds only the formatting requested through builder methods. User values are
never interpreted as raw Slack markup:

- `&`, `<`, and `>` become Slack entities, including in plain messages, so input
  such as `<@U123>` cannot create an explicit mention.
- In formatted messages, user-supplied `*`, `_`, `~`, and backticks become visible
  Unicode equivalents (`∗`, `＿`, `∼`, `ˋ`). Slack has no general backslash escape;
  this deliberate substitution prevents values from closing formatting spans.
- `heading`/`bold_line`, `line`, `kv`, `kv_code`, and `error` support common alerts.
  `kv_code` renders the entire key/value line as inline code. `error` uses a code
  block. Plain mode omits all formatting wrappers.
- A final serialized-text budget of 4,000 Unicode scalar values includes entities,
  separators, wrappers, and a visible `…` when content is truncated. Entity tokens
  and formatting wrappers are never cut apart. The renderer reserves indicator
  space, so some messages finish slightly below 4,000.
- Values default to 1,000 characters before escaping, preserving room for later
  fields. `kv_with_budget` and `error_with_budget` accept `ValueBudget::Chars(n)`
  or `Unlimited`. Unlimited bypasses only the value budget, never the final budget.
  A zero value budget intentionally omits the value.

These are conservative library limits, not Slack's hard maximum. Code-point
truncation preserves valid UTF-8 but may split a grapheme cluster. Empty messages
are rejected on send. Messages disable automatic mention parsing, URL parsing,
and link/media unfurling. There is no raw-markup escape hatch in v1.

## Delivery and errors

Each send makes one application-level HTTP request. There are no retry loops,
queues, automatic format fallbacks, or delivery guarantees. Callers own retry,
backpressure, persistence, and cross-process channel pacing.

A success requires HTTP success, JSON `ok: true`, and nonempty `channel` and `ts`.
The returned timestamp stays a string. Errors distinguish validation, transport,
HTTP status, Slack API rejection, malformed response, and HTTP 429 rate limiting.
`RateLimited` preserves numeric `Retry-After` seconds when present. Unknown remote
API error strings become `unknown_error`; bodies and underlying HTTP errors are
not retained. Known safe Slack error codes remain available for diagnosis.

Timeouts, lost responses, server errors, and malformed success responses may
leave delivery uncertain. Retrying can duplicate a message. A 429 response
should be rescheduled by the caller according to `Retry-After`; the crate never
sleeps. There is no cross-client or distributed rate limiter.

The crate's `tracing` events contain only success/failure and error category.
Client/message/result `Debug` output omits credentials and content. Callers should
also avoid enabling verbose HTTP-library diagnostics or logging their config,
request/response bodies, `text()`, or returned channel IDs.

## Slack setup

See [setup](docs/setup.md) and the [example app manifest](docs/slack-app-manifest.json).
Use a separate **Ultra Sound Ops** app with only `chat:write`, invited to intended
channels. Installation, token provisioning, production changes, and service
migration are outside this crate's release.

A bot token supports multiple joined channels and returns message identifiers.
Incoming webhooks instead give each channel its own posting credential, reducing
credential reach but requiring a secret per destination. Both are Slack apps,
both support rich messages, and both need an interaction receiver for buttons.
Sauron's existing adjustment webhook can remain separate.

## Migrating from Telegram

Rebuild messages from original domain values with this builder. Telegram
MarkdownV2 escaping and Telegram chat IDs are not Slack-compatible. Map channel
IDs in service configuration, not in this library. Initially keep Telegram's
interactive promotion controls in place; noninteractive dual delivery and
fallback policy belong to the caller. See [future builder controls](docs/builder-controls.md).

## Validation and examples

Run `./check.sh` for formatting, checks, Clippy, tests, and doctests across all
four feature combinations. Tests use local mocked endpoints; no Slack token is
needed. README examples are included as crate doctests.

The runnable examples **send real messages** when explicitly run with
`SLACK_BOT_TOKEN` and `SLACK_CHANNEL_ID`. The async example sends a formatted
message and one thread reply; the blocking example sends one plain message.
Use an authorized test channel only. They are compiled, never executed, in CI.

References: [posting API](https://docs.slack.dev/reference/methods/chat.postMessage/),
[formatting](https://docs.slack.dev/messaging/formatting-message-text/),
[rate limits](https://docs.slack.dev/apis/web-api/rate-limits/),
[webhooks](https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks/).
