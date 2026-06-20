<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Mira

Mira is an unofficial, privacy-respecting mouse settings client for macOS, Windows, and Linux. It uses signed, declarative `.mira-plugin` packages; plugins cannot execute native code, scripts, web content, or arbitrary WASM.

## Current status

This repository is pre-release. The UI and core contracts have offline tests, but no device is `hardware-verified`, no stable plugin Release exists, and no installer has completed platform packaging verification. Do not interpret source or Fixture coverage as device compatibility.

Planned community downloads use these stable names:

- macOS: `Mira_<version>_universal-unsigned.dmg`
- Windows: `Mira_<version>_x64-setup-unsigned.exe`
- Linux: `Mira_<version>_x86_64.AppImage`, `mira_<version>_amd64.deb`, `mira-<version>-1.x86_64.rpm`

Unsigned community packages will carry Gatekeeper or SmartScreen warnings. Verify the SHA-256 published with the GitHub Release. Community downloads will be published at https://github.com/hello-yunshu/mira-mouse/releases.

The default product is designed to bundle only a locked `mira.amaster` baseline from the separate Mira Mouse Plugins Release. Logitech and Razer experiments are never bundled by default. Actual model support must appear in a hardware evidence matrix before it is described as compatible.

Mira has no telemetry, account, ads, or resident network service. Diagnostic exports are previewed and redact stable identifiers. See [Security](SECURITY.md), [unsigned release guidance](docs/unsigned-release-security.md), [zero-cost release guide](docs/zero-cost-release.md), and the platform installation notes.

Mira is not authorized, endorsed, or sponsored by any device manufacturer. Manufacturer names are used only where necessary to describe compatibility research.

Code and build definitions are licensed under AGPL-3.0-or-later. Original documentation and non-trademark visual material are CC-BY-SA-4.0. This engineering license choice is not legal advice.

