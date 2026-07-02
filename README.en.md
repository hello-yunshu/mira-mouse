<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<p align="center">
  <img src="public/app-icon.png" width="96" height="96" alt="Mira logo">
</p>

<h1 align="center">Mira</h1>

<p align="center">
  A modern, quiet, plugin-driven mouse settings client.
</p>

<p align="center">
  <a href="README.md">中文</a> ·
  <a href="#features">Features</a> ·
  <a href="#install">Install</a> ·
  <a href="#supported-devices">Supported Devices</a> ·
  <a href="#development">Development</a>
</p>

<p align="center">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square">
  <img alt="React" src="https://img.shields.io/badge/React-19-61DAFB?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-runtime-000000?style=flat-square">
  <img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-7C3AED?style=flat-square">
</p>

## Overview

Mira is an unofficial, privacy-respecting mouse settings client for macOS, Windows, and Linux. A plugin-driven architecture adapts to different mouse brands, providing DPI adjustment, lighting control, and polling rate configuration.

- No telemetry, accounts, or ads
- Cross-platform desktop app
- Device protocols decoupled from the UI; plugins extend independently

## Features

- **DPI adjustment**: multi-stage DPI with per-profile configuration
- **Polling rate**: 125 / 250 / 500 / 1000 / 2000 / 4000 / 8000 Hz
- **Lighting control**: mouse and receiver RGB with color, effect, speed, brightness
- **Night mode**: auto close/restore lighting based on local time, supports cross-midnight
- **Battery status**: real-time mouse and receiver charge level
- **Multi-device**: connect multiple devices simultaneously, independent configs
- **Theme switching**: follows system dark/light theme, Dock icon syncs
- **Auto update**: built-in update check, no manual download

## Install

### macOS

Homebrew is recommended:

```bash
brew tap hello-yunshu/mira
brew trust hello-yunshu/mira
brew install --cask mira
```

Direct DMG download is also supported. See [macOS install notes](docs/install-macos.md) and [Homebrew install notes](docs/install-homebrew.md).

### Windows

Download `Mira_Windows_<version>_x64-setup.exe` and run the installer.

### Linux

Download `Mira_Linux_<version>_amd64.AppImage`, make it executable, and run:

```bash
chmod +x Mira_Linux_*_amd64.AppImage
./Mira_Linux_*_amd64.AppImage
```

All platform artifacts are published on [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases).

> Unsigned community packages trigger Gatekeeper or SmartScreen warnings; releases ship with SHA-256 checksums. See [security notes](docs/unsigned-release-security.md).

## Supported Devices

| Brand | Protocol | Connection | Status |
|---|---|---|---|
| AMaster / Angry Miao | Protocol A | USB / 2.4G receiver | Hardware verification in progress |
| Logitech | HID++ 2.0 | USB / 2.4G receiver | Hardware verification in progress |

Mira is not authorized, endorsed, or sponsored by any device manufacturer. Manufacturer names are used only to describe compatibility.

## Development

Mira follows one rule: **protocols belong to plugins; the interface belongs to the host app.** Plugins ship as signed declarative `.mira-plugin` packages; the host owns the UI, permission boundary, HID access, and updates.

Plugin repository: [`hello-yunshu/mira-mouse-plugins`](https://github.com/hello-yunshu/mira-mouse-plugins)

Development commands:

```bash
npm install
npm run dev              # frontend preview
npm exec tauri dev       # desktop preview
npm run lint && npm run typecheck && npm test -- --run
cargo test
```

Further docs:

- [Plugin package format](docs/plugin-package-format.md) · [Plugin SDK](docs/plugin-sdk.md) · [Protocol DSL](docs/protocol-dsl.md)
- [Plugin security](docs/plugin-security.md) · [Threat model](docs/threat-model.md) · [Security policy](SECURITY.md)
- [macOS install](docs/install-macos.md) · [Homebrew install](docs/install-homebrew.md)

## License

Code and build definitions are licensed under AGPL-3.0-or-later. Original documentation and non-trademark visual material are licensed under CC-BY-SA-4.0. See [`LICENSE`](LICENSE), [`NOTICE`](NOTICE), and [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
