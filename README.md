# slack

Small Rust Slack bot client with literal text by default and explicit formatting. Posts through
`chat.postMessage` using a bot token; no listener or public endpoint is needed.

```toml
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.3.0" }
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
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.3.0", default-features = false, features = ["blocking"] }
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

`SlackMessage::plain(text)` is the primary API. It preserves literal punctuation,
backticks, and underscores, and automatically truncates text above 40,000 escaped
characters. At send time, only `&`, `<`, and `>` are entity-encoded for Slack, and
`mrkdwn` is disabled. Literal `<@U123>` is displayed as text rather than a user
mention. `text()` returns the possibly truncated source, not the escaped payload.

`MessageBuilder::new().line(...).kv(...).build()` assembles plain-text fields.
Each line, key/value value, and label has a fixed 4,000-character escaped budget.
These limits are fixed. The joined message is then capped at 40,000.

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

## Automatic size protection

Slack recommends 4,000 characters for readability and truncates top-level text
above 40,000. This crate uses 40,000 as a safety ceiling; callers remain
responsible for readable messages. There is no automatic splitting.

Plain messages are shortened at construction. The builder caps each line, value,
and label at 4,000, joins the fields, then truncates the combined message to a
40,000-character prefix. Labels, separators, newlines, and markers count toward
the aggregate limit. A single huge error usually leaves room for later context,
but many large fields can cause trailing instructions to be cut. Put critical
context early when composing potentially large messages.

Truncation retains a source prefix and appends `… [truncated]`, included within
the budget. Counts use Unicode scalar values after Slack escaping (`&` costs five characters), excluding JSON string encoding.
Prefixes end on complete source characters, never partial entities. This is a
conservative submitted-text budget, not a claim about Slack's internal counting.
UTF-8 is preserved, but a grapheme cluster can be split.

```rust
use slack::MessageBuilder;
let error = "unexpected object: ".repeat(100_000);
let message = MessageBuilder::new()
    .line("Settlement failed")
    .kv("report_id", 42)
    .kv("error", error)
    .line("Check the stored payment status before retrying.")
    .build();
assert!(message.text().contains("… [truncated]"));
assert!(message.text().ends_with("Check the stored payment status before retrying."));
```

Pass original values directly; callers do not need escape-width calculations or
Slack-specific error helpers. Use `SlackMessage::plain` for already composed text
or intentional fields longer than 4,000 characters; its truncation preserves a
prefix and cannot distinguish fields or instructions.

Both transports retain final validation and reject `mrkdwn` above 40,000 with
`Error::MessageTooLong { chars, limit }`. Raw formatted text is never automatically
cut or repaired. Empty messages are rejected. Block Kit is not implemented.

`truncate_text` remains an optional helper counting *unescaped source* scalar
values, with `…` included in its budget and an empty result for zero:

```rust
use slack::{SlackMessage, truncate_text};
let message = SlackMessage::plain(truncate_text("a long diagnostic", 10));
assert_eq!(message.text(), "a long di…");
```

It is not safe for truncating formatted mrkdwn. Automatic plain-message protection
also accounts for escape expansion, which this source-text helper does not.

## Migrating from 0.2.0

- Plain construction now truncates automatically; `text()` exposes that result.
- Builder lines, values, and labels have a fixed 4,000-character escaped cap.
  Use `SlackMessage::plain` for longer content; the aggregate cap still applies.
- Remove caller-side Slack escaping and error-budget helpers; pass original data.
- Raw `mrkdwn` retains its existing rejection behavior.
- Slack's `message_truncated` warning is logged without changing successful sends
  into errors. The warning contains no remote response text or message content.
- Review long alerts when upgrading consumers such as Phoenix; pinned v0.2.0
  dependencies do not change automatically.

## Migrating from 0.1.0

- Replace `MessageBuilder::new(ParseMode::Plain)` with `MessageBuilder::new()`.
- Replace formatted builders with explicit `SlackMessage::mrkdwn` for authored
  formatting, or plain messages for arbitrary data.
- Replace `heading`/`bold_line` with `line`, and `kv_code`/`error` with `kv` when
  plain output is appropriate. `ParseMode`, `ValueBudget`, and budget methods are removed.
- Use plain builders for automatic size protection, or explicit `mrkdwn` for authored formatting.
- `text()` exposes possibly truncated source text; transport escaping occurs once on send.

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

The crate's `tracing` events contain success/failure, error category, and a fixed
warning when Slack reports truncation; they never include remote warning text.
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
