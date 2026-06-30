<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Changelog

## 0.3.8 - 2026-07-01

- Fixed macOS update relaunch to reopen the `.app` bundle instead of directly
  spawning the bundle's internal executable, avoiding the system "choose an app
  to open this" prompt introduced around 0.3.6.
- Kept launch-at-login and "hide window at login" as separate settings; only
  the combination starts Mira in the background.
- Set Windows updater installs to quiet mode so automatic updates stay silent.

## 0.3.7 - 2026-07-01

- Added multi-device support and refreshed UI messaging across the app.
- Fixed the Linux udev hotplug monitor API and added a local pre-push guard.
- Refined the Material Design UI with a dedicated guidelines doc and polished
  styles, tooltip, and dashboard layout.
- Simplified macOS Dock icon theme switching: the Dock icon is now provided by
  the packaged `icon.icns` (with a matching `icon-dark.icns`), removing the
  runtime `setApplicationIconImage` override that caused visual discrepancies.
- Cleaned up repository contents: stopped tracking Tauri-generated schemas and
  the local test config, hardened `.gitignore`, and refreshed reference docs.

## 0.3.1 - 2026-06-28

- Branded the Windows NSIS installer and uninstaller with Mira liquid-glass
  header/sidebar assets, shared app icons, Simplified Chinese + English
  language support, and a dedicated `Mira` Start Menu folder.
- Added a reproducible Windows installer asset generator and npm script so
  NSIS bitmap assets stay aligned with Mira's default glass theme.
- Refined the macOS DMG background brightness to better match the current
  installer visual direction.
- Hid Windows helper command windows when opening external links and switched
  Windows system theme detection to direct registry reads.

## 0.3.0 - 2026-06-28

- Removed the Windows system title bar (minimize/maximize/close) and shipped a
  custom `WindowsWindowControls` component (minimize + close) rendered by the
  frontend, with matching `.windows-window-controls` styles.
- Hid the tray battery percentage and receiver battery toggles on Windows
  because the Windows system tray cannot render text beside the icon.
- Added a DPI step-mismatch guard in `DpiEditModal` so the Apply button stays
  disabled when the entered value is not on the declared step grid.
- Surfaced a friendly "operation unavailable" notification when a mutation
  fails with "is not available on this device", pointing users to close
  official software (Logi Options+, G HUB, AMaster) that may hold the device.
- Extended `plugin_capabilities` to enrich capability metadata (DPI min/max/step
  and polling-rate `allowed` values) from `protocol/workflows.json` mutation
  inputs, so plugins no longer need to hardcode numeric ranges in `plugin.json`.
  New Rust unit tests cover both DPI range enrichment and Select option
  enrichment following the current writable mutation.
- Made `pluginRange` accept legacy top-level `min/max/step` metadata in
  addition to the typed `metadata.range`, and made `pluginOptions` filter
  device-reported polling rates through the plugin-declared option list while
  preserving plugin labels.
- Switched the Windows backdrop to Acrylic-first (translucent frosted glass)
  with Mica as the fallback, and applied `set_decorations(false)` on Windows
  startup to drop the system chrome.
- Unified floating-glass styling through `--floating-glass-bg`,
  `--floating-glass-blur`, and `--floating-glass-shadow` CSS variables across
  tooltips, notifications, edit modals, and device detail panels; raised
  `--glass-popup` opacity for stronger legibility.
- Repositioned `nav-links` as an absolutely-positioned sibling of `top-nav`
  with platform-specific right alignment (Windows: aligned to dashboard right
  edge via `right: max(16px, calc(50% - 234px))`; macOS: fixed `right: 16px`),
  and added `<option>` background colors for light/dark themes.
- Hardened `prefers-reduced-transparency: reduce` to disable all backdrop
  filters and replace glass popups with solid backgrounds.
- Added a `Clean stale bundle outputs` step in the CI pipeline to remove
  cached bundle artifacts before each Tauri build, preventing stale files
  from leaking into new releases.
- Added `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`
  to suppress the command-line window on Windows release builds.

## 0.2.0 - 2026-06-27

- Initial public release of the brand-neutral Mira application shell with
  declarative plugin contracts, HID++ and AMaster plugin manifests, and the
  bounded workflow runtime.
- No hardware compatibility or signed release is claimed.
