# ITERATION-009 Smoke Test Result

## App Info
- App path: `/Users/yunshu/Documents/GitHub/mira-mouse/target/release/bundle/macos/Mira.app/Contents/MacOS/mira`
- App version: `0.9.17`
- App SHA-256: `149eceb0c21a57df715815bff8b02521cac35a4936eafaeafa005430f60d87a2`
- OS: `Darwin 25.5.0`
- Date: `2026-07-27T19:36:28Z`

## Summary
- PASS: 6
- FAIL: 0
- SKIP: 7

## Scenarios

| Scenario | Status | Detail |
|---|---|---|
| detect-app | PASS | Found: /Users/yunshu/Documents/GitHub/mira-mouse/target/release/bundle/macos/Mira.app/Contents/MacOS/mira |
| manual-launch | PASS | Process started (pid=28006) |
| manual-launch-window | SKIP | osascript Accessibility permission denied (sandbox/unauthorized) |
| close-window-stays-alive | PASS | Process alive after window close (pid=28006) |
| hidden-launch | PASS | Process started (pid=28046) |
| hidden-launch-no-window | SKIP | osascript Accessibility permission denied (sandbox/unauthorized) |
| second-instance | PASS | Second launch invoked existing instance (pid_count=1) |
| second-instance-window | SKIP | osascript Accessibility permission denied (sandbox/unauthorized) |
| repeat-hidden | PASS | Repeated hidden did not spawn extra processes (pid_count=1) |
| repeat-hidden-no-window | SKIP | osascript Accessibility permission denied (sandbox/unauthorized) |
| ui-smoke-lighting-order | SKIP | Requires manual UI verification or Accessibility automation (see §7.3 checklist) |
| ui-smoke-subblock-count | SKIP | Requires manual UI verification |
| ui-smoke-advanced-settings | SKIP | Requires manual UI verification |

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
ITERATION-009 smoke test PASS (process-level). UI smoke requires manual verification.
