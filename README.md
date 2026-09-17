# slack

Small Rust Slack bot client with literal text by default and explicit formatting. Posts through
`chat.postMessage` using a bot token; no listener or public endpoint is needed.

```toml
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.2.0" }
```

Rust 2024. Default feature: `async` (`reqwest`). Optional: `blocking` (`ureq`).
Disable default features for message construction without an HTTP transport.
The crate is distributed by Git tags, not crates.io. `slack::VERSION` exposes
the package version.

## Async sending

```rust,no_run
# #[cfg(feature = "async")]
# async fn example() -> Result<(), slack::Error> {
use slack::{MessageBuilder, SlackBot, SlackMessage, SendOptions};
let bot = SlackBot::new("BOT_TOKEN", "C_DEFAULT_CHANNEL");
let message = MessageBuilder::new()
    .line("relay alert")
    .kv("service", "auction-api")
    .kv("slot", 12345)
    .kv("error", "simulation failed: unexpected `root`")
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
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.2.0", default-features = false, features = ["blocking"] }
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

## Literal text and explicit formatting

`SlackMessage::plain(text)` is the primary API. It retains the original source
text, including punctuation, backticks, underscores, and long values. At send
time, only `&`, `<`, and `>` are entity-encoded for Slack, and `mrkdwn` is disabled.
For example, literal `<@U123>` is displayed as text rather than a user mention.
`text()` returns the original source, not the transport-escaped representation.

`MessageBuilder::new().line(...).kv(...).build()` is an optional convenience for
joining plain-text lines. It has no formatting modes or per-value size budgets.

For caller-authored Slack formatting, use `SlackMessage::mrkdwn`:

```rust
use slack::SlackMessage;
let message = SlackMessage::mrkdwn("*service recovered*\n<https://example.com|Dashboard>");
assert!(message.is_mrkdwn());
```

Formatted messages are sent unchanged. Explicit links and mentions are active.
Do not pass arbitrary logs/user text as mrkdwn. `escape_text` can neutralize `&`,
`<`, and `>` in a literal fragment, but does not escape emphasis or code delimiters.
Use plain text when exact arbitrary content matters. No Unicode lookalikes are
substituted and no formatting-repair fallback is attempted.

Both modes disable automatic mention parsing, automatic URL linking, and link/media
previews. These flags do not neutralize explicit mention/link syntax in mrkdwn.

## Length and explicit truncation

Slack recommends 4,000 characters for readability, but truncates top-level messages
above 40,000. This crate does not impose the recommendation as a limit.

Both transports reject messages whose final `text` field exceeds 40,000 Unicode
scalar values with `Error::MessageTooLong { chars, limit }`, before making a request.
For plain text this check includes entity expansion (`&` becomes five characters
in `&amp;`), but excludes JSON escapes such as `\n`. This is a conservative check
on the submitted text, not a promise about Slack's internal display-length counting.
There is no automatic truncation or splitting. Empty messages are also rejected.
Block Kit has separate size rules and is not implemented by this crate.

To intentionally shorten literal content, call `truncate_text` explicitly:

```rust
use slack::{SlackMessage, truncate_text};
let message = SlackMessage::plain(truncate_text("a long diagnostic", 10));
assert_eq!(message.text(), "a long di…");
```

The helper counts source Unicode scalar values, includes `…` within the requested
budget, and returns an empty string for a zero budget. It preserves UTF-8 but may
split a grapheme cluster. Escape expansion may still cause send-time rejection.
Do not use it to truncate formatted mrkdwn: it does not repair formatting or entities.

## Migrating from 0.1.0

- Replace `MessageBuilder::new(ParseMode::Plain)` with `MessageBuilder::new()`.
- Replace formatted builders with explicit `SlackMessage::mrkdwn` for authored
  formatting, or plain messages for arbitrary data.
- Replace `heading`/`bold_line` with `line`, and `kv_code`/`error` with `kv` when
  plain output is appropriate. `ParseMode`, `ValueBudget`, and budget methods are removed.
- Apply `truncate_text` only where the caller intentionally wants to shorten data.
- Handle `Error::MessageTooLong` instead of relying on automatic truncation.
- `text()` now exposes original source text; transport escaping occurs once on send.

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
`CLIENT_SECRET` and `SIGNING_SECRET` match Slack’s app credential labels; they
are separate from the `SLACK_BOT_TOKEN` required for posting. See the setup guide
for credential mapping.

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
`SLACK_BOT_TOKEN` and `SLACK_CHANNEL_ID`. The async example sends a plain
message and one thread reply; the blocking example sends one plain message.
Use an authorized test channel only. They are compiled, never executed, in CI.

References: [posting API](https://docs.slack.dev/reference/methods/chat.postMessage/),
[formatting](https://docs.slack.dev/messaging/formatting-message-text/),
[rate limits](https://docs.slack.dev/apis/web-api/rate-limits/),
[webhooks](https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks/).
