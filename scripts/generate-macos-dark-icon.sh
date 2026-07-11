#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE="$ROOT/public/app-icon-dark.png"
OUTPUT="$ROOT/src-tauri/icons/icon-dark.icns"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/mira-dark-icon.XXXXXX")"
ICONSET="$WORK/icon-dark.iconset"
trap 'rm -rf "$WORK"' EXIT

# Match the normal macOS icon.icns safe area exactly: 836 px artwork centered
# on a 1024 px transparent canvas (94 px on every side).
mkdir -p "$ICONSET"
sips -z 836 836 "$SOURCE" --out "$WORK/artwork.png" >/dev/null
sips -p 1024 1024 "$WORK/artwork.png" --out "$WORK/padded.png" >/dev/null

for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$WORK/padded.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  doubled=$((size * 2))
  sips -z "$doubled" "$doubled" "$WORK/padded.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$ICONSET" -o "$OUTPUT"
echo "generated $OUTPUT with the normal icon's macOS safe area"
