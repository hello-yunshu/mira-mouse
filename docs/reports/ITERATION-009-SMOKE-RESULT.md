# ITERATION-009 Smoke Test Result

- App path: `/Users/yunshu/Documents/GitHub/mira-mouse/target/test-app/cargo/release/bundle/macos/Mira.app/Contents/MacOS/mira`
- App version: `0.9.17`
- App SHA-256: `088829217910667aaf53607e62ef12304051676f039aa6577c85d9d2476409e1`
- OS: `Darwin 25.5.0`
- Date: `2026-07-28`

## Summary

- PASS: 13
- FAIL: 0
- PLATFORM_BLOCKED: 0

| Scenario | Status | Detail |
|---|---|---|
| detect-app | PASS | Found exact packaged executable |
| bundled-plugin-runtime | PASS | Packaged binary parsed its embedded lock and verified all 3 signed default plugins |
| manual-launch-process | PASS | Exactly one process |
| manual-launch-window | PASS | Visible window count = 1 |
| close-window-hides | PASS | Native close button closed the window; process stayed alive |
| tray-menu-reopen | PASS | Native Mira tray menu reopened the window |
| hidden-launch-process | PASS | Exactly one process |
| hidden-launch-window | PASS | No visible window |
| second-instance-process | PASS | Interactive second launch reused the existing process |
| second-instance-window | PASS | Existing process displayed one window |
| repeat-hidden-process | PASS | Repeated hidden launch reused the existing process |
| repeat-hidden-window | PASS | No visible window |
| ui-contract-fixtures | PASS | `src/App.device.test.tsx` passed |

## Conclusion

ITERATION-009 packaged smoke COMPLETE on the current macOS platform.

This report covers the packaged plugin lock/resource/hash/signature/device-descriptor
path, process/window/tray behavior, and the device/UI fixture contract suite. A
final native UI inspection also showed `mira.amaster`, `mira.logitech-hidpp`, and
`mira.razer-viper` as signature-verified default plugins and detected the connected
`AM INFINITY MOUSE .100`. It does not claim real AM35 hardware writes or remote CI
results.
