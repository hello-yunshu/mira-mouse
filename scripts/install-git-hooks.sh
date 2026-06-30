#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
hooks_dir="$repo_root/.git/hooks"

mkdir -p "$hooks_dir"
cp "$repo_root/scripts/git-hooks/pre-push" "$hooks_dir/pre-push"
chmod +x "$hooks_dir/pre-push"

echo "Installed Git pre-push hook: npm run check:quick"
