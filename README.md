# slack

The Slack client our services share for alerts and notifications. It posts as the **Ultra Sound
Ops** bot through the Web API, with:

- **A message builder** that turns titles, fields, errors and links into a well formatted rich
  text message. Values are always literal, so a log line full of `*`, `_` or `<@U123>` shows up
  exactly as written.
- **Optional retries**, so a network blip doesn't eat an alert and every service doesn't need its
  own retry loop.
- **Errors that say what went wrong**: the HTTP library's error chain, the status code, Slack's
  error code and the start of the response body.
- **Async and blocking clients** with the same API. The blocking one is for code outside an async
  runtime, like a dedicated writer thread.

```toml
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.4.0" }

# blocking only
slack = { git = "https://github.com/ultrasoundmoney/slack", tag = "v0.4.0", default-features = false, features = ["blocking"] }
```

| Feature | Gives you | Built on |
| --- | --- | --- |
| `async` (default) | `slack::SlackBot` | reqwest |
| `blocking` | `slack::blocking::SlackBot` | ureq |
| neither | messages only | |

## Sending an alert

```rust,no_run
# #[cfg(feature = "async")]
# async fn example() -> Result<(), slack::Error> {
use slack::{Message, MessageBuilder, Retry, SlackBot};

let bot = SlackBot::new("xoxb-...", "C0123456789")?
    .with_retry(Retry::at_least_once(Retry::ALERT_DELAYS));

let alert = MessageBuilder::new("bundle tracer persistence failed")
    .context("rbx-prod-mainnet")
    .kv("aggregation slot", 15646338)
    .code("error", "pool timed out while waiting for an open connection")
    .link("dashboard", "https://grafana.example.com/d/builder")
    .footer("relay submissions disabled until process restart")
    .build();

let sent = bot.send(&alert).await?;
bot.reply(&sent, &Message::plain("restarted, submissions resumed")).await?;
# Ok(())
# }
```

The blocking client is the same without `.await`:

```rust,no_run
# #[cfg(feature = "blocking")]
# fn example() -> Result<(), slack::Error> {
use slack::{Message, blocking::SlackBot};

let bot = SlackBot::new("xoxb-...", "C0123456789")?;
bot.send(&Message::plain("service started"))?;
# Ok(())
# }
```

`new` checks the token and channel, so a broken config fails at startup instead of on the first
alert. Each client sends to its default channel with `send`, elsewhere with `send_to`, and into a
thread with `reply`. `call` reaches any other Web API method, e.g. `chat.update` or
`apps.connections.open`.

## Messages

`MessageBuilder` is the usual way to write a message:

| Method | Renders as |
| --- | --- |
| `new(title)` | bold first line |
| `context(text)` | italic line, e.g. network and geo |
| `kv(key, value)` | **key:** value |
| `line(text)` | plain line |
| `code(label, text)` | **label:** then a preformatted block, for errors and multi-line output |
| `link(label, url)` | a link |
| `footer(text)` | last line, never cut |

It renders to a single `rich_text` block, so formatting comes from structure and values never
need escaping. The same content goes along as plain `text`, which Slack uses for notifications.
If Slack rejects the block (`invalid_blocks`, `msg_blocks_too_long`), the client sends that text
on its own instead of failing.

`Message::plain(text)` sends literal text with no formatting. `Message::with_blocks` attaches
your own Block Kit blocks, such as buttons, with the text as fallback.

### Size limits

The builder caps each value at 3,000 characters and the whole message at about 12,000, which
stays under the point where Slack has been seen to reject rich text. When fields don't fit, the
one that overflows gets cut, later ones are dropped with a "… n more fields omitted" line, and the
footer survives. Put the instruction the reader must see in the footer.

Plain messages are capped at 40,000 characters after escaping, Slack's own limit. Anything cut
ends in `… [truncated]`. `slack::truncate` does the same for your own strings.

## Retries

Retries are off by default. Turn them on per client:

```rust
use std::time::Duration;
use slack::Retry;

// Alerts: a duplicate beats a missing alert.
let alerts = Retry::at_least_once(Retry::ALERT_DELAYS); // 5s, 10s, 20s

// Things that must not double post: only retries when Slack can't have acted.
let notices = Retry::when_undelivered([Duration::from_secs(1), Duration::from_secs(5)]);
```

Only transient failures are retried: connection problems, timeouts, 429, 5xx, malformed responses
and Slack's own `ratelimited`, `internal_error` and the like. A wait is the configured delay or
Slack's `Retry-After`, whichever is longer. Each retry logs a warning with the error. With the
default six second timeout, `ALERT_DELAYS` gives up after about a minute.

`at_least_once` can post a message twice: if a request times out after Slack received it, Slack
may have posted it and the retry posts it again. `when_undelivered` never resends in that case.

The blocking client sleeps on the calling thread. If a slow alert must not hold up other work,
send from a thread of its own.

## Errors

`slack::Error` keeps everything it knows, and its `Display` is meant to go straight into a log or
a message to the team:

```text
slack transport failed during connect: error sending request for url (https://slack.com/api/chat.postMessage): client error (Connect): tcp connect error: Connection refused (os error 61)
slack api error not_in_channel: {"ok":false,"error":"not_in_channel"}
slack http status 503: upstream connect error or disconnect/reset before headers
```

| Variant | When |
| --- | --- |
| `Invalid` | rejected before sending, e.g. an empty message |
| `Transport { stage, source }` | no complete response; `stage` says whether the request got out |
| `Http { status, retry_after, body }` | non-2xx response |
| `Api { code, body }` | Slack answered `ok: false` |
| `Malformed { body, source }` | a 2xx response that doesn't parse or lacks `channel` and `ts` |

`Stage::Connect` means resolving, connecting or the TLS handshake failed, so nothing reached Slack.
Connecting gets three seconds of the request timeout, so a DNS or network problem shows up as such.
`Stage::Exchange` means the request may have arrived.

Instead of matching variants, ask the error: `is_transient()`, `delivery_uncertain()`,
`retry_after()` and `api_code()`. These drive `Retry` too. Bodies are capped at 500 characters,
and the token is masked if a response ever echoes it.

## Slack setup

The bot needs `chat:write` and an invite to each channel it posts in. Use channel IDs, not names.
See [setup](docs/setup.md) and the [app manifest](docs/slack-app-manifest.json).

## Development

`./check.sh` runs fmt, clippy, tests and doctests for every feature combination, like CI. Tests run
against a local mock server and need no token.

The examples post for real, so point them at a test channel:

```bash
SLACK_BOT_TOKEN=xoxb-... SLACK_CHANNEL_ID=C... cargo run --example async
```

Releases are git tags, see [RELEASING.md](RELEASING.md) and [CHANGELOG.md](CHANGELOG.md).
