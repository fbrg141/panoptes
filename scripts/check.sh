#!/bin/sh
# Every check a change must pass: Rust formatting, clippy (warnings are
# errors) and every test, including the Pi extension's own (Node, which Pi
# needs anyway). The pre-push hook runs it too.
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node --test crates/panoptes-pi/src/*.test.js
