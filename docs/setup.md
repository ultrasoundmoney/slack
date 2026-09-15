# Ultra Sound Ops setup

This guide describes future installation. Creating the Rust repository does not
install an app or provision any Slack credentials.

1. Create a Slack app from `slack-app-manifest.json` in the intended workspace.
   Keep it separate from Sauron's adjustment integration.
2. Add the appropriate maintainers as app collaborators using their individual
   Slack accounts. Do not share personal logins.
3. Review OAuth scopes: only bot `chat:write` is requested. Install the app into
   the workspace through Slack's UI and obtain its bot token (`xoxb-…`). No OAuth
   web server is needed for this manually installed internal app.
4. Invite **Ultra Sound Ops** to each intended public or private channel. Copy
   each channel's ID from Slack's channel details. Channel names are not config
   identifiers. No `chat:write.public`, history access, or channel discovery scope
   is required.
5. Store the token in the deployment's secret-management system and inject it
   into the service. Pass it and the desired channel ID to the crate constructor.
   The library does not read environment variables. Never commit the token,
   log it, or paste it into commands that will enter shell history.
6. Only after authorization, run a single example in a dedicated test channel.
   Verify the app identity, destination, formatting, and thread behavior.

A shared bot token can post to all channels the app has joined, regardless of
which channel a service uses by default. Separate apps/tokens are needed for
stronger isolation. An application-side channel list does not narrow the token's
Slack permissions.

## Rotation

For manual credential replacement, generate/reinstall credentials through Slack's
supported app-management flow, update the secret store, restart or reload callers
with a newly constructed client, verify in the test channel, then revoke obsolete
credentials when appropriate. Check Slack's UI for the exact impact of reinstall
or revocation before using it during a live migration.

Slack also offers OAuth token rotation with expiring access tokens and refresh
tokens. V1 does not implement refresh: leave automatic token rotation disabled
unless the caller separately manages refresh and replaces clients. Never assume
an old token remains valid after changing installation credentials.

References: [app manifests](https://docs.slack.dev/app-manifests/),
[token rotation](https://docs.slack.dev/authentication/using-token-rotation/).
