# Changelog

## 0.3.0

- Automatically bound plain messages to 40,000 escaped characters with visible truncation.
- Bound builder fields to 4,000 escaped characters by default; add `value_limit` for overrides.
- Shrink the largest values first on aggregate overflow; omit trailing fields with a count only when labels and minimal values cannot fit.
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
