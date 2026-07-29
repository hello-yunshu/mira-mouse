#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Packaged Latest Test App smoke test.
#
# PASS is emitted only for an observed assertion. Missing window automation is
# PLATFORM_BLOCKED and makes the command exit non-zero; it is never converted
# into a process-level PASS.
set -euo pipefail

HOST_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_MANIFEST="$HOST_ROOT/target/test-app/manifest.json"
REPORT_FILE="${MIRA_SMOKE_REPORT:-$HOST_ROOT/target/test-app/smoke-report.md}"
PRODUCT_NAME="Mira"

detect_app_path() {
  if [ -n "${MIRA_APP_PATH:-}" ] && [ -x "$MIRA_APP_PATH" ]; then
    echo "$MIRA_APP_PATH"
    return
  fi
  if [ -f "$BUILD_MANIFEST" ]; then
    local manifest_path
    manifest_path="$(node -e 'const fs=require("fs");const m=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));process.stdout.write(m.executablePath||"")' "$BUILD_MANIFEST")"
    if [ -n "$manifest_path" ] && [ -x "$manifest_path" ]; then
      echo "$manifest_path"
      return
    fi
  fi
  local candidates=(
    "$HOST_ROOT/target/test-app/cargo/release/bundle/macos/Mira.app/Contents/MacOS/mira"
    "$HOST_ROOT/target/release/bundle/macos/Mira.app/Contents/MacOS/mira"
    "$HOST_ROOT/target/test-app/cargo/release/bundle/nsis/Mira.exe"
  )
  local candidate
  for candidate in "${candidates[@]}"; do
    if [ -x "$candidate" ]; then
      echo "$candidate"
      return
    fi
  done
  local appimage
  appimage="$(find "$HOST_ROOT/target/test-app/cargo/release/bundle" -type f -name '*.AppImage' -perm -111 -print -quit 2>/dev/null || true)"
  echo "$appimage"
}

APP_PATH="$(detect_app_path)"
mkdir -p "$(dirname "$REPORT_FILE")"

RESULTS=()
PASS_COUNT=0
FAIL_COUNT=0
BLOCKED_COUNT=0

record_result() {
  local scenario="$1" status="$2" detail="$3"
  RESULTS+=("| $scenario | $status | $detail |")
  case "$status" in
    PASS) PASS_COUNT=$((PASS_COUNT + 1)) ;;
    FAIL) FAIL_COUNT=$((FAIL_COUNT + 1)) ;;
    PLATFORM_BLOCKED) BLOCKED_COUNT=$((BLOCKED_COUNT + 1)) ;;
  esac
}

app_pids() {
  ps -axo pid=,command= | awk -v app="$APP_PATH" '$2 == app { print $1 }'
}

app_pid_count() {
  local count
  count="$(app_pids | awk 'NF { count++ } END { print count+0 }')"
  echo "$count"
}

cleanup_app() {
  local pids
  pids="$(app_pids || true)"
  if [ -n "$pids" ]; then
    # shellcheck disable=SC2086
    kill $pids 2>/dev/null || true
    sleep 2
  fi
  pids="$(app_pids || true)"
  if [ -n "$pids" ]; then
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
  fi
}

window_count_macos() {
  local result attempt=0
  while [ "$attempt" -lt 20 ]; do
    if result="$(osascript 2>&1 <<EOF
set targetPid to $1
tell application "System Events"
  set targetProcess to first process whose unix id is targetPid
  return count of windows of targetProcess
end tell
EOF
    )"; then
      echo "${result:-0}"
      return
    fi
    sleep 0.25
    attempt=$((attempt + 1))
  done
  echo "ERROR:$result"
}

window_check() {
  local scenario="$1" pid="$2" expected="$3"
  if [ "$(uname -s)" != "Darwin" ]; then
    record_result "$scenario" "PLATFORM_BLOCKED" "Window automation is currently implemented for macOS only"
    return
  fi
  local count
  count="$(window_count_macos "$pid")"
  if [[ "$count" == ERROR:* ]]; then
    record_result "$scenario" "PLATFORM_BLOCKED" "System Events window query failed: ${count#ERROR:}"
  elif [ "$expected" = "visible" ] && [ "$count" -gt 0 ]; then
    record_result "$scenario" "PASS" "Visible window count=$count"
  elif [ "$expected" = "hidden" ] && [ "$count" -eq 0 ]; then
    record_result "$scenario" "PASS" "No visible window"
  else
    record_result "$scenario" "FAIL" "Expected $expected window state, observed count=$count"
  fi
}

wait_for_single_pid() {
  local attempts=0
  while [ "$attempts" -lt 20 ]; do
    local count
    count="$(app_pid_count)"
    if [ "$count" -eq 1 ]; then
      app_pids | head -1
      return 0
    fi
    sleep 0.5
    attempts=$((attempts + 1))
  done
  return 1
}

write_report() {
  local app_sha="" app_version="unknown"
  if [ -n "$APP_PATH" ] && [ -f "$APP_PATH" ]; then
    if command -v shasum >/dev/null 2>&1; then
      app_sha="$(shasum -a 256 "$APP_PATH" | awk '{print $1}')"
    else
      app_sha="$(sha256sum "$APP_PATH" | awk '{print $1}')"
    fi
    if [ "$(uname -s)" = "Darwin" ]; then
      local bundle
      bundle="$(dirname "$(dirname "$(dirname "$APP_PATH")")")"
      app_version="$(defaults read "$bundle/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo unknown)"
    fi
  fi
  {
    echo "# ITERATION-009 Smoke Test Result"
    echo
    echo "- App path: \`$APP_PATH\`"
    echo "- App version: \`$app_version\`"
    echo "- App SHA-256: \`$app_sha\`"
    echo "- OS: \`$(uname -s) $(uname -r)\`"
    echo
    echo "## Summary"
    echo
    echo "- PASS: $PASS_COUNT"
    echo "- FAIL: $FAIL_COUNT"
    echo "- PLATFORM_BLOCKED: $BLOCKED_COUNT"
    echo
    echo "| Scenario | Status | Detail |"
    echo "|---|---|---|"
    local row
    for row in "${RESULTS[@]}"; do echo "$row"; done
    echo
    if [ "$FAIL_COUNT" -eq 0 ] && [ "$BLOCKED_COUNT" -eq 0 ]; then
      echo "ITERATION-009 packaged smoke COMPLETE."
    else
      echo "ITERATION-009 packaged smoke PARTIAL."
    fi
  } > "$REPORT_FILE"
}

trap cleanup_app EXIT

if [ -z "$APP_PATH" ] || [ ! -x "$APP_PATH" ]; then
  record_result "detect-app" "FAIL" "Executable not found; run npm run build:test-app"
  write_report
  exit 1
fi
record_result "detect-app" "PASS" "Found exact executable"

if plugin_check="$("$APP_PATH" --test-bundled-plugins 2>&1)" \
  && [[ "$plugin_check" == *'"pluginCount":3'* ]] \
  && [[ "$plugin_check" == *'"mira.amaster"'* ]] \
  && [[ "$plugin_check" == *'"mira.logitech-hidpp"'* ]] \
  && [[ "$plugin_check" == *'"mira.razer-viper"'* ]]; then
  record_result "bundled-plugin-runtime" "PASS" "Packaged binary verified all 3 signed default plugins"
else
  record_result "bundled-plugin-runtime" "FAIL" "Packaged binary plugin self-check failed: ${plugin_check:-no output}"
fi

cleanup_app
"$APP_PATH" &
CLOSE_OBSERVED=0
if MANUAL_PID="$(wait_for_single_pid)"; then
  record_result "manual-launch-process" "PASS" "Exactly one process pid=$MANUAL_PID"
  window_check "manual-launch-window" "$MANUAL_PID" "visible"
else
  record_result "manual-launch-process" "FAIL" "Expected exactly one process"
fi

if [ "$(uname -s)" = "Darwin" ] && [ -n "${MANUAL_PID:-}" ]; then
  if osascript >/dev/null 2>&1 <<EOF
set targetPid to $MANUAL_PID
tell application "System Events"
  set targetProcess to first process whose unix id is targetPid
  click first button of window 1 of targetProcess whose description is "close button"
end tell
EOF
  then
    sleep 2
    if kill -0 "$MANUAL_PID" 2>/dev/null; then
      local_count="$(window_count_macos "$MANUAL_PID")"
      if [ "$local_count" = "0" ]; then
        record_result "close-window-hides" "PASS" "Window closed and process stayed alive"
        CLOSE_OBSERVED=1
      elif [[ "$local_count" == ERROR:* ]]; then
        record_result "close-window-hides" "PLATFORM_BLOCKED" "Window state query failed after clicking the native close button"
      else
        record_result "close-window-hides" "FAIL" "Window remained visible after AXClose"
      fi
    else
      record_result "close-window-hides" "FAIL" "Process exited after closing its window"
    fi
  else
    record_result "close-window-hides" "PLATFORM_BLOCKED" "System Events could not click the native close button"
  fi
else
  record_result "close-window-hides" "PLATFORM_BLOCKED" "Close automation unavailable"
fi

if [ "$CLOSE_OBSERVED" -eq 1 ]; then
  if osascript >/dev/null 2>&1 <<EOF
tell application "System Events"
  tell first process whose unix id is $MANUAL_PID
    if (count of menu bars) < 2 then error "Mira status menu bar not found"
    set trayItem to menu bar item 1 of menu bar 2
    click trayItem
    delay 1
    set openItem to missing value
    repeat with candidate in menu items of menu 1 of trayItem
      try
        set itemName to name of candidate as text
        if itemName is "打开 Mira" or itemName is "Open Mira" then
          set openItem to candidate
          exit repeat
        end if
      end try
    end repeat
    if openItem is missing value then error "Mira open menu item not found"
    click openItem
  end tell
end tell
EOF
  then
    sleep 2
    window_check "tray-menu-reopen" "$MANUAL_PID" "visible"
  else
    record_result "tray-menu-reopen" "PLATFORM_BLOCKED" "System Events could not click Mira's native tray menu"
  fi
else
  record_result "tray-menu-reopen" "PLATFORM_BLOCKED" "Close-to-tray was not observable"
fi

cleanup_app
"$APP_PATH" --hidden &
if HIDDEN_PID="$(wait_for_single_pid)"; then
  record_result "hidden-launch-process" "PASS" "Exactly one process pid=$HIDDEN_PID"
  window_check "hidden-launch-window" "$HIDDEN_PID" "hidden"
else
  record_result "hidden-launch-process" "FAIL" "Expected exactly one hidden process"
fi

if [ -n "${HIDDEN_PID:-}" ]; then
  "$APP_PATH" &
  sleep 3
  if [ "$(app_pid_count)" -eq 1 ]; then
    record_result "second-instance-process" "PASS" "Interactive second launch reused existing process"
    window_check "second-instance-window" "$HIDDEN_PID" "visible"
  else
    record_result "second-instance-process" "FAIL" "Interactive second launch produced extra processes"
  fi
fi

cleanup_app
"$APP_PATH" --hidden &
if REPEAT_PID="$(wait_for_single_pid)"; then
  "$APP_PATH" --hidden &
  sleep 3
  if [ "$(app_pid_count)" -eq 1 ]; then
    record_result "repeat-hidden-process" "PASS" "Repeated hidden launch reused existing process"
    window_check "repeat-hidden-window" "$REPEAT_PID" "hidden"
  else
    record_result "repeat-hidden-process" "FAIL" "Repeated hidden launch produced extra processes"
  fi
else
  record_result "repeat-hidden-process" "FAIL" "Initial hidden process did not stabilize"
fi

# The packaged binary smoke and DOM contract tests are intentionally reported
# separately. Unit tests cannot masquerade as packaged-window observations.
if (cd "$HOST_ROOT" && npm test -- --run src/App.device.test.tsx >/dev/null); then
  record_result "ui-contract-fixtures" "PASS" "Packaged UI contracts covered by App.device tests"
else
  record_result "ui-contract-fixtures" "FAIL" "App.device contract tests failed"
fi

write_report
echo "Report written to: $REPORT_FILE"
echo "PASS=$PASS_COUNT FAIL=$FAIL_COUNT PLATFORM_BLOCKED=$BLOCKED_COUNT"

if [ "$FAIL_COUNT" -gt 0 ]; then exit 1; fi
if [ "$BLOCKED_COUNT" -gt 0 ]; then exit 2; fi
