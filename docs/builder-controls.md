# Future builder promotion and demotion

Analysis only. V1 has no control buttons, inbound receiver, message-update API,
authorization policy, database access, or deployments.

## Receiver and boundaries

Add one logical interaction handler alongside Phoenix's control logic. A Slack
app's interactive components share an interaction destination. Outbound clients
in individual services should not each implement an inbound controller.

Two receiver choices remain for that future project:

- HTTPS: publicly reachable endpoint, raw-body HMAC verification with the Slack
  signing secret, timestamp replay checks, and strict payload validation.
- Socket Mode: outbound WebSocket connection and app-level token with
  `connections:write`; manage reconnects, connection replacement, acknowledgements,
  and duplicate delivery. No public ingress is required.

Acknowledge valid interactions within three seconds, independently of slow
control work. If using asynchronous processing, durably accept work before
acknowledging success and deduplicate before executing it. Add Block Kit buttons
and `chat.update` support to the generic crate only when implementing this path.
The `SentMessage` channel and timestamp already supply message-update references.

## Authorization and actions

Validate workspace, app, channel, stable Slack user ID, and the user's authority
for the specific builder/action. A verified Slack signature authenticates the
sender as Slack, not the human's authorization. Neither display names nor channel
membership alone should grant control rights. Builder admins and internal
operators may need different permissions.

Promotion can reuse the current promotion-token and demotion-kind domain model.
Demotion requires an explicit command containing builder, kind, reason, and a
confirmation step, routed through the existing control pathway so database state,
caches/gossip, and notifications remain consistent. Do not duplicate relay state
mutation inside this library. Revalidate current state when the action executes;
a stale button must not override a later demotion or unrelated operator change.

## Existing race to fix before sharing tokens

The inspected turbo-relay implementation checks token expiry and use in
`services/phoenix/src/telegram_callback_listener/callback_query.rs` before calling
`crates/global_database/src/data_api/demotions.rs`.

The transaction's token update matches token and builder but does not require
`is_used = false` or an unexpired timestamp, and does not check an affected-row
claim. Therefore two concurrent clicks can both pass the preliminary check.

Before introducing Slack, atomically claim an unused, unexpired token in the
same transaction as promotion; require exactly one claimed row. Bind the claim
to the intended builder, demotion kind, and applicable demotion generation.
Recheck eligibility, verify the resulting state, and roll back on failure.
Both Telegram and Slack must call this shared operation. Retrying a claimed token
must return the existing outcome rather than execute again. Remove tokens from
logs when extracting this logic.

## Audit and message lifecycle

The existing `record_promotion_token_click` stores a numeric Telegram user ID and
writes best-effort leaderboard metadata. Add platform-aware identity (Slack
workspace/user strings) and a durable action audit recording actor, reason,
target, state transition, request identity, time, and outcome. Keep leaderboard
statistics separate from the control audit.

Persist message references for all copies of an action. After completion, remove
or replace stale buttons and announce the result. Failed message updates are
retryable notification work; they must never re-execute a completed control
operation. A duplicate click should report the known outcome.

## Follow-up acceptance tests

Invalid signatures, stale timestamps, wrong workspace/channel, unauthorized
operators, malformed actions, expired/used tokens, concurrent Slack/Telegram
clicks, stale demotion generations, insufficient eligibility, database rollback,
slow work with timely acknowledgement, duplicate receiver deliveries, receiver
restart, and failed message updates must all be covered before rollout. Test in
staging with explicit authorization before enabling production controls.

References: [interactions](https://docs.slack.dev/interactivity/handling-user-interaction/),
[request verification](https://docs.slack.dev/authentication/verifying-requests-from-slack/),
[Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/).
