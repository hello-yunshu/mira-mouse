#!/usr/bin/env bash
set -euo pipefail

npm run lint
npm run typecheck
npm test
cargo fmt --all --check

git diff --check
git diff --cached --check
