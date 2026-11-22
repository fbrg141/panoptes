#!/bin/sh
# Every check a change must pass: Rust formatting, clippy (warnings are
# errors), every test and the commit-message linter, including the Pi
# extension's own tests (Node, which Pi needs anyway). The pre-push hook runs
# it too.
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node --test crates/panoptes-pi/src/*.test.js
sh scripts/test-commit-msg.sh
