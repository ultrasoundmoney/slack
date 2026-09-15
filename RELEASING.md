# Releasing

1. Update Cargo.toml version and CHANGELOG.md; regenerate Cargo.lock.
2. Run `./check.sh` and review the full diff for credentials and unrelated files.
3. Commit with a Conventional Commit message and push to main.
4. Verify the GitHub Check workflow for that exact commit.
5. Create and push an annotated `v<version>` tag pointing to the validated commit.

This repository uses Git tags rather than crates.io publishing. No release step
sends messages or changes Slack configuration.
