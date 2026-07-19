<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<p align="center">
  <img src="public/app-icon.png" width="128" height="128" alt="Mira logo">
</p>

<h1 align="center">Mira</h1>

<p align="center">
  A modern, quiet, plugin-driven mouse settings client.<br>
  Available on macOS, Windows, and Linux. No accounts, no cloud service, no vendor drivers.
</p>

<p align="center">
  <a href="https://github.com/hello-yunshu/mira-mouse/releases"><img alt="Release" src="https://img.shields.io/github/v/release/hello-yunshu/mira-mouse?style=flat-square&color=7C3AED"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-24C8DB?style=flat-square">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square">
  <img alt="React" src="https://img.shields.io/badge/React-19-61DAFB?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-runtime-000000?style=flat-square">
  <img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-7C3AED?style=flat-square">
</p>

<p align="center">
  <a href="README.md">中文</a> ·
  <a href="#overview">Overview</a> ·
  <a href="#features">Features</a> ·
  <a href="#screenshots">Screenshots</a> ·
  <a href="#install">Install</a> ·
  <a href="#supported-devices">Supported Devices</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#faq">FAQ</a> ·
  <a href="#development">Development</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="ROADMAP.md">Roadmap</a> ·
  <a href="SUPPORT.md">Support</a>
</p>

---

## Overview

Mira is an unofficial mouse settings tool for macOS, Windows, and Linux. No accounts, no cloud service, no vendor drivers — install and go. Network access is limited to user-selected app, plugin, and local-AI component updates.

## Highlights

### Plugin-driven, declarative protocols

Mira hardcodes no device logic. Each mouse is supported via a signed plugin package, distributed independently — new devices arrive without a host-app update. Plugins describe device facts and protocol workflows only; they contain no executable code, keeping the security boundary clear and auditable.

### Fully local, zero data collection

No telemetry, no accounts, no network reporting. All settings and device interactions run locally — nothing is sent to any server. The only network request is update checking, which can be disabled.

### Cross-platform, synced releases

macOS, Windows, and Linux share the same core codebase and plugin ecosystem, with version-synced releases. The experience is consistent across platforms.

### Bounded protocols, predictable behavior

The protocol DSL defaults to 64 steps, 16 reads, 2s total delay. No expression evaluation, filesystem, network, or arbitrary loops. Every workflow is Fixture-testable, making behavior predictable and auditable.

### Local AI battery prediction

Battery estimation runs locally using device context (DPI, polling rate, lighting mode). The model ships with the host app; no data is uploaded.

## Features

| Category | Capability |
|---|---|
| **DPI adjustment** | Multi-stage DPI with per-profile configuration |
| **Polling rate** | 125 ~ 8000 Hz, dynamically read from device capabilities |
| **Lighting control** | Mouse and receiver RGB with color, effect, speed, brightness |
| **Night mode** | Auto close/restore lighting based on local time, supports cross-midnight |
| **Battery status** | Real-time mouse and receiver charge level, tray icon reflects battery |
| **Multi-device** | Connect multiple devices simultaneously, independent configs |
| **Theme switching** | Follows system dark/light theme, macOS Dock icon syncs |
| **Auto update** | Built-in update check, notifies on new releases |
| **Local AI battery prediction** | Device-context-aware (DPI, polling rate, lighting) local battery estimation |

## Screenshots

<!-- Screenshots pending: suggest providing the following in docs/screenshots/
- Main dashboard (dark / light)
- DPI adjustment panel
- Lighting control panel
- Battery status and tray icon
- Multi-device switching
-->

> Screenshots will be added in a later release. For a preview of the interface design, see the [Material design guidelines](docs/mira-material-design-guidelines.md).

## Install

### macOS

Homebrew is recommended:

```bash
brew tap hello-yunshu/mira
brew trust hello-yunshu/mira
brew install --cask mira
```

Direct DMG download is also supported: [macOS install notes](docs/install-macos.md) · [Homebrew install notes](docs/install-homebrew.md)

### Windows

Download `Mira_Windows_<version>_x64-setup.exe` and run the installer.

### Linux

Download `Mira_Linux_<version>_amd64.AppImage`, make it executable, and run:

```bash
chmod +x Mira_Linux_*_amd64.AppImage
./Mira_Linux_*_amd64.AppImage
```

All platform artifacts are published on [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases).

> Unsigned community packages trigger Gatekeeper or SmartScreen warnings on first launch — this is expected. Releases ship with SHA-256 checksums. See [security notes](docs/unsigned-release-security.md).

## Supported Devices

| Brand | Protocol | Connection | Status |
|---|---|---|---|
| AMaster / Angry Miao | Protocol A | USB / 2.4G receiver | Supported |
| Logitech | HID++ 2.0 | USB / 2.4G receiver | Supported |

Mira is not authorized, endorsed, or sponsored by any device manufacturer. Manufacturer names are used only to describe compatibility.

Want your device supported? Check the [Plugin SDK](docs/plugin-sdk.md) or open a device-support request on [GitHub Issues](https://github.com/hello-yunshu/mira-mouse/issues).

## Architecture

Mira follows one rule: **protocols belong to plugins; the interface belongs to the host app.**

```
┌─────────────────────────────────────────────────┐
│                  Mira Host App                   │
│  ┌───────────┐  ┌──────────┐  ┌──────────────┐  │
│  │    UI     │  │ Perm &   │  │  HID access  │  │
│  │  (React)  │  │ signing  │  │  & updates   │  │
│  └───────────┘  └──────────┘  └──────────────┘  │
│                        │                         │
│            ┌───────────┴───────────┐             │
│            │  Plugin runtime (bounded) │         │
│            └───────────┬───────────┘             │
└────────────────────────┼────────────────────────┘
                         │ signed declarative .mira-plugin packages
         ┌───────────────┴───────────────┐
         │                               │
  ┌──────┴──────┐                ┌───────┴──────┐
  │  AMaster    │                │   Logitech   │
  │   plugin    │                │    plugin    │
  └─────────────┘                └──────────────┘
```

- **Host app** owns UI rendering, permission boundary, HID access, and auto-updates.
- **Plugins** ship as signed declarative `.mira-plugin` packages — they describe device facts and protocol workflows only, with no executable code.
- **Protocol DSL** is bounded and testable: default limits of 64 steps, 16 reads, 2s total delay. No expression evaluation, filesystem, network, or arbitrary loops.

More architecture and security details: [Plugin SDK](docs/plugin-sdk.md) · [Protocol DSL](docs/protocol-dsl.md) · [Plugin security](docs/plugin-security.md) · [Threat model](docs/threat-model.md)

Plugin repository: [`hello-yunshu/mira-mouse-plugins`](https://github.com/hello-yunshu/mira-mouse-plugins)

## FAQ

<details>
<summary><b>First launch says "damaged" or "unidentified developer"?</b></summary>

This is normal for unsigned apps. On macOS, go to "System Settings > Privacy & Security" and click "Open Anyway", or run:

```bash
xattr -cr /Applications/Mira.app
```

See [security notes](docs/unsigned-release-security.md).

</details>

<details>
<summary><b>Is Bluetooth supported?</b></summary>

Only USB direct connection and 2.4G receiver are supported; Bluetooth is not.

</details>

<details>
<summary><b>Does Mira collect my data?</b></summary>

No. Mira has no telemetry, no accounts, and no network reporting — everything runs locally.

</details>

<details>
<summary><b>My mouse isn't on the list. What now?</b></summary>

Open a device-support request on [Issues](https://github.com/hello-yunshu/mira-mouse/issues), or adapt it yourself using the [Plugin SDK](docs/plugin-sdk.md).

</details>

<details>
<summary><b>Device shows as occupied (0xE00002C5)?</b></summary>

Official configuration tools (AMasterLauncher, Logi Options+, G HUB) exclusively hold HID devices. Close them before using Mira.

</details>

## Development

```bash
git clone https://github.com/hello-yunshu/mira-mouse.git
cd mira-mouse
npm install
npm run dev              # frontend preview (Vite)
npm run sidecar:build    # build the host mira-runtime sidecar before first desktop run
npm exec tauri dev       # desktop preview (full Tauri runtime)
```

Code quality checks:

```bash
npm run lint && npm run typecheck && npm test -- --run   # frontend
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test  # Rust
npm run check:boundaries  # repo boundary scan
npm run check:ci          # local CI equivalent
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contribution guide.

### Documentation index

| Document | Description |
|---|---|
| [Plugin package format](docs/plugin-package-format.md) | `.mira-plugin` package structure and manifest spec |
| [Plugin SDK](docs/plugin-sdk.md) | capability declarations, placement, metadata contracts |
| [Protocol DSL](docs/protocol-dsl.md) | Bounded workflow syntax and limits |
| [Plugin security](docs/plugin-security.md) | Signing, trust roots, permission boundary |
| [Plugin testing](docs/plugin-testing.md) | Fixture evidence and testing conventions |
| [Threat model](docs/threat-model.md) | Assets, untrusted inputs, and controls |
| [Local AI Runtime](docs/local-ai-analysis-plan.md) | Rill runtime, model, and WASM handler boundaries and release chain |
| [macOS install](docs/install-macos.md) | DMG and Gatekeeper notes |
| [Homebrew install](docs/install-homebrew.md) | Tap variables and upgrade flow |
| [Linux permissions](docs/linux-permissions.md) | udev rules and hot-plug |
| [Windows install](docs/install-windows.md) | NSIS installer notes |
| [License notes](docs/license-notes.md) | AGPL and CC-BY-SA scope |
| [Security notes](docs/unsigned-release-security.md) | Unsigned release security considerations |
| [Material design guidelines](docs/mira-material-design-guidelines.md) | Interface material, glass, cards, buttons, and floating layers |
| [Comment & doc style](docs/comment-and-doc-style.md) | Code comment and documentation conventions |

## Project Status

Mira is under active development. Current version: **0.9.7** (2026-07-20).

- macOS, Windows, and Linux installers released
- AMaster (Protocol A) and Logitech (HID++ 2.0) protocols supported
- Declarative plugin packages and signature verification implemented
- Local AI battery prediction integrated
- See [CHANGELOG.md](CHANGELOG.md) for the changelog and [ROADMAP.md](ROADMAP.md) for the roadmap

## License

Code and build definitions are licensed under AGPL-3.0-or-later. Original documentation and non-trademark visual material are licensed under CC-BY-SA-4.0. See [`LICENSE`](LICENSE), [`NOTICE`](NOTICE), and [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

The Mira name and marks are governed by [TRADEMARKS.md](TRADEMARKS.md).
