#!/usr/bin/env bash
set -euo pipefail

npm run check:quick
npm run build
npm run check:boundaries
npm run check:structured

cargo run --package xtask -- plugins sync --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
