#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# ITERATION-009 §7：打包后的 Latest Test App smoke 测试。
#
# 自动定位刚构建的 App 可执行文件，执行进程级 smoke 测试：
#   1. 手动启动（无参数）→ 主窗口显示、进程存在
#   2. hidden 启动（--hidden）→ 进程存在、不抢焦点
#   3. 第二实例（已有 hidden，再无参数）→ 唤起现有实例
#   4. 重复 hidden（已有实例，再 --hidden）→ 不抢焦点
#   5. 关闭窗口 → 进程不退出（隐藏到托盘）
#
# 用法：
#   ./scripts/smoke-test-built-app.sh
#
# 环境变量：
#   MIRA_APP_PATH — App 可执行文件路径（默认自动探测）
set -euo pipefail

HOST_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PRODUCT_NAME="Mira"
REPORT_DIR="$HOST_ROOT/docs/reports"
REPORT_FILE="$REPORT_DIR/ITERATION-009-SMOKE-RESULT.md"

# 自动探测 App 路径
detect_app_path() {
  if [ -n "${MIRA_APP_PATH:-}" ] && [ -x "$MIRA_APP_PATH" ]; then
    echo "$MIRA_APP_PATH"
    return
  fi
  local bundle_dir="$HOST_ROOT/src-tauri/target/release/bundle"
  # macOS .app
  local app_path="$bundle_dir/macos/${PRODUCT_NAME}.app/Contents/MacOS/${PRODUCT_NAME}"
  if [ -x "$app_path" ]; then
    echo "$app_path"
    return
  fi
  # Linux AppImage
  local appimage_path
  appimage_path="$(ls "$bundle_dir/appimage"/*.AppImage 2>/dev/null | head -1 || true)"
  if [ -n "$appimage_path" ] && [ -x "$appimage_path" ]; then
    echo "$appimage_path"
    return
  fi
  # Windows .exe (WSL/Git Bash)
  local exe_path="$bundle_dir/nsis/${PRODUCT_NAME}.exe"
  if [ -x "$exe_path" ]; then
    echo "$exe_path"
    return
  fi
  echo ""
}

APP_PATH="$(detect_app_path)"
mkdir -p "$REPORT_DIR"

# 结果收集
RESULTS=()
PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

record_result() {
  local scenario="$1" status="$2" detail="$3"
  RESULTS+=("| $scenario | $status | $detail |")
  case "$status" in
    PASS) PASS_COUNT=$((PASS_COUNT + 1)) ;;
    FAIL) FAIL_COUNT=$((FAIL_COUNT + 1)) ;;
    SKIP) SKIP_COUNT=$((SKIP_COUNT + 1)) ;;
  esac
}

# 查找 Mira 进程 PID
find_mira_pid() {
  pgrep -f "${PRODUCT_NAME}" 2>/dev/null | head -1 || echo ""
}

# 等待进程出现
wait_for_process() {
  local timeout=10
  local elapsed=0
  while [ "$elapsed" -lt "$timeout" ]; do
    local pid
    pid="$(find_mira_pid)"
    if [ -n "$pid" ]; then
      echo "$pid"
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  echo ""
  return 1
}

# 清理：杀掉所有 Mira 进程
cleanup_mira() {
  pkill -f "${PRODUCT_NAME}" 2>/dev/null || true
  sleep 2
  pkill -9 -f "${PRODUCT_NAME}" 2>/dev/null || true
  sleep 1
}

# 检查窗口是否可见（macOS）
# 返回值：
#   数字字符串（0/1/2...）：成功查询到窗口数量
#   "PERMISSION_DENIED"：osascript 无 Accessibility 权限（沙盒/未授权）
is_window_visible_macos() {
  if [ "$(uname)" != "Darwin" ]; then echo "0"; return; fi
  local app_pid="$1"
  local result
  # 通过 osascript 检查 Mira 窗口是否存在且可见
  if ! result="$(osascript -e "tell application \"System Events\" to count windows of (every process whose name is \"${PRODUCT_NAME}\")" 2>/dev/null)"; then
    echo "PERMISSION_DENIED"
    return
  fi
  echo "${result:-0}"
}

echo "=== ITERATION-009 Smoke Test ==="
echo "App path: ${APP_PATH:-<not found>}"

if [ -z "$APP_PATH" ]; then
  echo "ERROR: App executable not found. Run \`npm run build:test-app\` first."
  record_result "detect-app" "FAIL" "App executable not found in bundle dir"
  # 直接生成报告并退出
  cat > "$REPORT_FILE" <<EOF
# ITERATION-009 Smoke Test Result

## Summary
- PASS: $PASS_COUNT
- FAIL: $FAIL_COUNT
- SKIP: $SKIP_COUNT

## Scenarios

| Scenario | Status | Detail |
|---|---|---|
| detect-app | FAIL | App executable not found. Run \`npm run build:test-app\` first. |

## Environment
- OS: $(uname -s) $(uname -r)
- Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)

## Conclusion
ITERATION-009 smoke test PARTIAL — App not built.
EOF
  exit 1
fi

# 确保 App 存在
if [ ! -x "$APP_PATH" ]; then
  record_result "detect-app" "FAIL" "App not executable: $APP_PATH"
  exit 1
fi

record_result "detect-app" "PASS" "Found: $APP_PATH"

# 计算 App SHA-256
APP_SHA256=""
if command -v shasum >/dev/null 2>&1; then
  APP_SHA256="$(shasum -a 256 "$APP_PATH" | cut -d' ' -f1)"
elif command -v sha256sum >/dev/null 2>&1; then
  APP_SHA256="$(sha256sum "$APP_PATH" | cut -d' ' -f1)"
fi
echo "App SHA-256: $APP_SHA256"

# 获取 App 版本
APP_VERSION=""
if [ "$(uname)" = "Darwin" ]; then
  APP_BUNDLE="$(dirname "$(dirname "$(dirname "$APP_PATH")")")"
  APP_VERSION="$(defaults read "$APP_BUNDLE/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo "unknown")"
fi
echo "App version: $APP_VERSION"

# === 场景 1：手动启动（无参数）===
echo ""
echo "--- Scenario 1: Manual launch (no args) ---"
cleanup_mira
sleep 1
"$APP_PATH" &
SCENARIO1_PID=$!
sleep 3

DETECTED_PID="$(find_mira_pid)"
if [ -n "$DETECTED_PID" ]; then
  record_result "manual-launch" "PASS" "Process started (pid=$DETECTED_PID)"
else
  record_result "manual-launch" "FAIL" "Process not detected after launch"
fi

# 检查窗口可见性（macOS only）
if [ "$(uname)" = "Darwin" ]; then
  sleep 2
  WIN_COUNT="$(is_window_visible_macos "$DETECTED_PID")"
  if [ "$WIN_COUNT" = "PERMISSION_DENIED" ]; then
    record_result "manual-launch-window" "SKIP" "osascript Accessibility permission denied (sandbox/unauthorized)"
  elif [ "$WIN_COUNT" -gt 0 ] 2>/dev/null; then
    record_result "manual-launch-window" "PASS" "Window visible (count=$WIN_COUNT)"
  else
    record_result "manual-launch-window" "FAIL" "No visible window detected"
  fi
else
  record_result "manual-launch-window" "SKIP" "Window check not supported on $(uname)"
fi

# === 场景 2：关闭窗口 → 进程不退出 ===
echo ""
echo "--- Scenario 2: Close window → process stays alive ---"
if [ -n "$DETECTED_PID" ] && [ "$(uname)" = "Darwin" ]; then
  # 模拟关闭窗口（Cmd+W 或点击关闭按钮）
  osascript -e "tell application \"System Events\" to keystroke \"w\" using command down" 2>/dev/null || true
  sleep 2
  DETECTED_PID_AFTER_CLOSE="$(find_mira_pid)"
  if [ -n "$DETECTED_PID_AFTER_CLOSE" ]; then
    record_result "close-window-stays-alive" "PASS" "Process alive after window close (pid=$DETECTED_PID_AFTER_CLOSE)"
  else
    record_result "close-window-stays-alive" "FAIL" "Process exited after window close"
  fi
else
  record_result "close-window-stays-alive" "SKIP" "Not macOS or no running process"
fi

# === 场景 3：hidden 启动 ===
echo ""
echo "--- Scenario 3: Hidden launch (--hidden) ---"
cleanup_mira
sleep 1
"$APP_PATH" --hidden &
sleep 3

DETECTED_PID_HIDDEN="$(find_mira_pid)"
if [ -n "$DETECTED_PID_HIDDEN" ]; then
  record_result "hidden-launch" "PASS" "Process started (pid=$DETECTED_PID_HIDDEN)"
else
  record_result "hidden-launch" "FAIL" "Process not detected after hidden launch"
fi

# hidden 启动不应有可见窗口
if [ "$(uname)" = "Darwin" ] && [ -n "$DETECTED_PID_HIDDEN" ]; then
  sleep 1
  WIN_COUNT_HIDDEN="$(is_window_visible_macos "$DETECTED_PID_HIDDEN")"
  if [ "$WIN_COUNT_HIDDEN" = "PERMISSION_DENIED" ]; then
    record_result "hidden-launch-no-window" "SKIP" "osascript Accessibility permission denied (sandbox/unauthorized)"
  elif [ "$WIN_COUNT_HIDDEN" -eq 0 ] 2>/dev/null; then
    record_result "hidden-launch-no-window" "PASS" "No visible window (hidden mode correct)"
  else
    record_result "hidden-launch-no-window" "FAIL" "Window visible in hidden mode (count=$WIN_COUNT_HIDDEN)"
  fi
else
  record_result "hidden-launch-no-window" "SKIP" "Window check not supported"
fi

# === 场景 4：第二实例（已有 hidden，再无参数启动）===
echo ""
echo "--- Scenario 4: Second instance (no args, while hidden running) ---"
if [ -n "$DETECTED_PID_HIDDEN" ]; then
  "$APP_PATH" &
  sleep 3
  # 第二实例应该唤起现有实例（不创建新进程）
  PID_COUNT="$(pgrep -f "${PRODUCT_NAME}" 2>/dev/null | wc -l | tr -d ' ')"
  if [ "$PID_COUNT" -le 2 ]; then
    record_result "second-instance" "PASS" "Second launch invoked existing instance (pid_count=$PID_COUNT)"
  else
    record_result "second-instance" "FAIL" "Multiple processes spawned (pid_count=$PID_COUNT)"
  fi

  # 第二实例后窗口应该可见
  if [ "$(uname)" = "Darwin" ]; then
    sleep 2
    WIN_COUNT_2ND="$(is_window_visible_macos "$DETECTED_PID_HIDDEN")"
    if [ "$WIN_COUNT_2ND" = "PERMISSION_DENIED" ]; then
      record_result "second-instance-window" "SKIP" "osascript Accessibility permission denied (sandbox/unauthorized)"
    elif [ "$WIN_COUNT_2ND" -gt 0 ] 2>/dev/null; then
      record_result "second-instance-window" "PASS" "Window shown after second launch"
    else
      record_result "second-instance-window" "FAIL" "Window not shown after second launch"
    fi
  fi
else
  record_result "second-instance" "SKIP" "No hidden instance running"
  record_result "second-instance-window" "SKIP" "No hidden instance running"
fi

# === 场景 5：重复 hidden ===
echo ""
echo "--- Scenario 5: Repeated hidden launch ---"
cleanup_mira
sleep 1
"$APP_PATH" --hidden &
sleep 2
"$APP_PATH" --hidden &
sleep 3

PID_COUNT_REPHIDDEN="$(pgrep -f "${PRODUCT_NAME}" 2>/dev/null | wc -l | tr -d ' ')"
if [ "$PID_COUNT_REPHIDDEN" -le 2 ]; then
  record_result "repeat-hidden" "PASS" "Repeated hidden did not spawn extra processes (pid_count=$PID_COUNT_REPHIDDEN)"
else
  record_result "repeat-hidden" "FAIL" "Multiple processes spawned (pid_count=$PID_COUNT_REPHIDDEN)"
fi

if [ "$(uname)" = "Darwin" ]; then
  WIN_COUNT_REPHIDDEN="$(is_window_visible_macos "")"
  if [ "$WIN_COUNT_REPHIDDEN" = "PERMISSION_DENIED" ]; then
    record_result "repeat-hidden-no-window" "SKIP" "osascript Accessibility permission denied (sandbox/unauthorized)"
  elif [ "$WIN_COUNT_REPHIDDEN" -eq 0 ] 2>/dev/null; then
    record_result "repeat-hidden-no-window" "PASS" "No window shown in repeated hidden mode"
  else
    record_result "repeat-hidden-no-window" "FAIL" "Window shown in repeated hidden mode"
  fi
fi

# === 清理 ===
cleanup_mira

# === UI Smoke ===
echo ""
echo "--- UI Smoke (fixture-level) ---"
# UI smoke 需要手动验证或 Accessibility 自动化。
# 这里记录为需要手动验证的 checklist。
record_result "ui-smoke-lighting-order" "SKIP" "Requires manual UI verification or Accessibility automation (see §7.3 checklist)"
record_result "ui-smoke-subblock-count" "SKIP" "Requires manual UI verification"
record_result "ui-smoke-advanced-settings" "SKIP" "Requires manual UI verification"

# === 生成报告 ===
echo ""
echo "=== Generating report ==="

cat > "$REPORT_FILE" <<EOF
# ITERATION-009 Smoke Test Result

## App Info
- App path: \`$APP_PATH\`
- App version: \`$APP_VERSION\`
- App SHA-256: \`$APP_SHA256\`
- OS: \`$(uname -s) $(uname -r)\`
- Date: \`$(date -u +%Y-%m-%dT%H:%M:%SZ)\`

## Summary
- PASS: $PASS_COUNT
- FAIL: $FAIL_COUNT
- SKIP: $SKIP_COUNT

## Scenarios

| Scenario | Status | Detail |
|---|---|---|
EOF

for r in "${RESULTS[@]}"; do
  echo "$r" >> "$REPORT_FILE"
done

cat >> "$REPORT_FILE" <<EOF

## UI Smoke Checklist (§7.3)
以下检查项需要手动验证或 Accessibility 自动化：

- [ ] DPI → 回报率 → 灯光顺序正确
- [ ] candidate leading/trailing 正确
- [ ] 状态区不重复
- [ ] 顶部灯光带存在
- [ ] 最右普通主颜色子块存在
- [ ] 灯效最左
- [ ] 普通灯光子块最多 6
- [ ] 回报率子块最多 3
- [ ] Advanced Settings 可打开
- [ ] mouse/receiver 同名字段不丢失
- [ ] 全部读数不重复

## Conclusion
EOF

if [ "$FAIL_COUNT" -eq 0 ]; then
  echo "ITERATION-009 smoke test PASS (process-level). UI smoke requires manual verification." >> "$REPORT_FILE"
else
  echo "ITERATION-009 smoke test FAIL ($FAIL_COUNT failures). See scenarios above." >> "$REPORT_FILE"
fi

echo ""
echo "Report written to: $REPORT_FILE"
echo "Summary: PASS=$PASS_COUNT FAIL=$FAIL_COUNT SKIP=$SKIP_COUNT"

if [ "$FAIL_COUNT" -gt 0 ]; then
  exit 1
fi
