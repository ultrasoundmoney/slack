# Changelog

## 0.4.0

A redesign around three goals: errors that say what went wrong, retries in the crate instead of
in every caller, and messages that are formatted, not just escaped.

- Errors keep their cause: the HTTP library's error chain, the response status and body, and
  Slack's own error code, verbatim. `Transport` records whether the request got out
  (`Stage::Connect` or `Stage::Exchange`), helped by a three second connect timeout.
- `Error::is_transient`, `delivery_uncertain`, `retry_after` and `api_code` replace caller-side
  classification.
- Opt-in retries: `with_retry(Retry::at_least_once(..))` or `Retry::when_undelivered(..)`, honoring
  `Retry-After`. `Retry::ALERT_DELAYS` is 5s, 10s, 20s.
- `MessageBuilder::new(title)` renders a rich text block with `context`, `kv`, `line`, `code`,
  `link` and a `footer` that is never cut, plus plain fallback text. If Slack rejects the blocks
  the text is sent alone.
- `SlackBot::new` validates the token and channel and returns a `Result`. `reply` threads under a
  `SentMessage`; `call` reaches any Web API method; `Message::with_blocks` attaches custom blocks.
- Breaking: `SlackMessage` is now `Message`, `BlockingSlackBot` is `blocking::SlackBot`,
  `truncate_text` is `truncate`. Removed `SlackMessage::mrkdwn`, `SendOptions`, `send_with_options`,
  `with_client`, `VERSION`, `MessageTooLong` and `TransportFailure`. `Error` is no longer
  `Clone`/`PartialEq`; match on it or use its methods. The crate no longer logs every failure,
  only retries; callers log the final error.

## 0.3.0

- Automatically bound plain messages to 40,000 escaped characters with visible truncation.
- Bound builder lines, values, and labels to 4,000 escaped characters with no new public API.
- Cap the joined message to a 40,000-character prefix; aggregate overflow can cut trailing fields or instructions.
- Log Slack's `message_truncated` warning without exposing remote text or failing accepted sends.
- Breaking behavior: plain text and builder values may now be shortened; `text()` returns the resulting source text. Raw mrkdwn rejection and explicit `truncate_text` semantics are unchanged.

## 0.2.0

- Preserve plain message contents; escape only Slack's special parsing characters at send time.
- Add explicit caller-authored `SlackMessage::mrkdwn` and literal-fragment `escape_text`.
- Replace automatic 4,000/1,000-character truncation with a 40,000-character send-time check and `MessageTooLong` error.
- Add opt-in `truncate_text` for literal source text.
- Breaking: remove `ParseMode`, `ValueBudget`, formatting/field-budget builder methods;
  `MessageBuilder::new()` now assembles plain-text `line`/`kv` content only.
- `SlackMessage::text()` now returns original source text, not escaped output.

## 0.1.0

- Async and blocking bot-token posting, channel overrides, and thread replies.
- Safe plain-text and mrkdwn operational message builders with bounded output.
- Single-attempt sends, structured errors, rate-limit information, redacted tracing.
- Ultra Sound Ops manifest, setup guide, and future builder-control analysis.
