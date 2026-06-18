<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Evidence Status

Updated: 2026-06-18

## Environment

- `build-verified`: Node v26.0.0, npm 11.12.1, Git 2.50.1.
- `build-verified`: macOS 26.5.1 (Build 25F80) arm64.
- `partial`: Rust installed via Homebrew (`cargo 1.96.0`, `rustc 1.96.0`); `cargo fmt` applied; `cargo fetch`/`clippy`/`test`/`cargo-deny`/`cargo audit` are blocked by crates.io network timeouts on this host.
- `build-verified`: Tauri CLI 2.11.2 is installed via npm.
- `resolved`: the sibling plugin repository `mira-mouse-plugins` exists at `/Users/yunshu/Documents/GitHub/mira-mouse-plugins` and passes `npm run validate` and `npm test`.
- `source-confirmed`: the research bundle is present and gitignored; it was not enumerated or hashed during this session.
- `resolved`: both `mira-mouse` and `mira-mouse-plugins` are initialized as Git repositories (`main` branch); no remotes configured.
- `build-verified`: plugin repository release workflow (`release.yml`) rewritten to pack/sign with Node `scripts/pack-sign.mjs`; separate zero-cost workflow files removed; zero-cost release guide (`docs/zero-cost-release.md`) added; `src-tauri/tauri.conf.json` configured with ad-hoc macOS signing (`signingIdentity: "-"`).

## Capability evidence

- Frontend no-device / Fixture-demo UI is `fixture-verified` by Vitest (4/4 tests pass).
- Plugin package inspection, DSL, and malicious-package tests are checked in but cannot be executed without a successful `cargo fetch`; they remain `fixture-verified` pending Rust test execution.
- `mira.example-mock` plugin is `build-verified` as a TEST-ONLY signed `.mira-plugin` asset:
  - asset: `/Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/resources/plugins/mira-example-mock-1.0.0.mira-plugin`
  - sha256: `33a0fc66a8a55845d1cda56a6f06587d83c227892dd25e4792e64bce778a9f9a`
  - signed with TEST-ONLY key `TEST-ONLY-mira-plugins`
  - recorded in `plugins.lock.json` with `bundleByDefault: false`
- No device, write, installer, updater, performance, compatibility, or browser QA claim is `hardware-verified` or `build-verified`.
- No production `.mira-plugin` release asset exists; `mira.amaster` release is `blocked`.
