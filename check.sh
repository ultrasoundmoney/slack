#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
cargo fmt --all -- --check
for features in '--no-default-features' '--no-default-features --features async' '--no-default-features --features blocking' '--all-features'; do
  read -r -a feature_args <<< "$features"
  cargo check --locked "${feature_args[@]}" --all-targets
  cargo clippy --locked "${feature_args[@]}" --all-targets -- -D warnings
  cargo test --locked "${feature_args[@]}" --all-targets
  cargo test --locked "${feature_args[@]}" --doc
done
