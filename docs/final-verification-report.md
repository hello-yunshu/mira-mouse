<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Final Verification Report

Verification date: 2026-06-18. Host: macOS 26.5.1 arm64. Node v26.0.0, npm 11.12.1, Tauri CLI 2.11.2.

## Scope

- Main repository: `/Users/yunshu/Documents/GitHub/mira-mouse`
- Sibling plugin repository: `/Users/yunshu/Documents/GitHub/mira-mouse-plugins` — **exists**, passes `npm run validate` and `npm test`.
- Both repositories are now initialized as Git repositories on branch `main`; no remotes are configured.
- Rust toolchain installed via Homebrew: `cargo 1.96.0`, `rustc 1.96.0` (no `rustup`).
- `cargo fetch`/`clippy`/`test` are blocked by crates.io network timeouts in this environment.
- Zero-cost release workflows and guide have been added for maintainers who do not purchase code-signing certificates.

## Commands re-run in this session

| Command | Result | Evidence level |
|---|---|---|
| `npm run lint` | pass (exit 0) | build-verified |
| `npm run typecheck` | pass (exit 0) | build-verified |
| `npm test` | 4/4 Vitest tests pass | fixture-verified |
| `npm run build` | pass; `dist/index.html`, `dist/assets/index-DmaPnBL3.css` (5.65 kB), `dist/assets/index-_Di61aw2.js` (195.91 kB) | build-verified |
| `npm run check:boundaries` | pass; brand boundary clean | build-verified |
| `npm run check:structured` | pass; 12 YAML files parse, required files non-empty, Actions SHA-pinned | build-verified |
| `npm audit --omit=dev --audit-level=high` | 0 vulnerabilities | build-verified |
| `npm exec tauri -- --version` | `tauri-cli 2.11.2` | build-verified |
| `cargo fmt --all` | applied formatting | build-verified |
| `cargo fmt --all -- --check` | pass (after formatting) | build-verified |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | blocked: crates.io fetch timeout | blocked |
| `cargo test --workspace --locked` | blocked: crates.io fetch timeout | blocked |
| `cargo fetch --locked` | blocked: crates.io transfer timeout after retries | blocked |

Plugin repository commands:

| Command | Result | Evidence level |
|---|---|---|
| `npm run validate` | pass; 30 JSON files and four manifests validated | build-verified |
| `npm test` | pass; 4/4 protocol/policy fixtures | fixture-verified |
| `node scripts/pack-sign.mjs plugins/example-mock <cache-path>` | produced signed TEST-ONLY `.mira-plugin` | build-verified |
| `node scripts/verify-test.mjs <package> TEST-ONLY-mira-plugins.pub` | verification passed | build-verified |

## Functional status

- **Frontend (fixture-verified)**: quiet no-device state, mock dashboard, battery/polling/profile/DPI stages, lighting preview, application-layer receiver-link disclosure, theme fallback, reduced-motion/reduced-transparency support.
- **Plugin API/runtime (source complete, Rust execution blocked)**: manifest/capability contract, package whitelist, SHA-256 checksums, Ed25519 signature verification, basic bounded DSL, and malicious-package tests are checked in but not executed due to crates.io timeouts.
- **Plugin CLI/xtask (source complete, Rust execution blocked)**: `validate`, `test`, `pack`, `inspect`, `new`, and `--locked`/`--offline` sync are implemented; `sign` is correctly blocked without a configured protected key. A zero-cost Node helper (`scripts/pack-sign.mjs`) packs and signs plugins with deterministic ZIP timestamps; `scripts/verify-test.mjs` validates them.
- **Tauri backend (skeleton)**: registers `single-instance` and `autostart` plugins, exposes `device_snapshot` (always returns `None` to avoid mock-in-production) and `can_install_update`. No tray icon, no autostart state command, no HID commands, no settings/about/diagnostics pages.
- **No hardware verification**: no devices, no writes, no installer smoke tests, no browser QA performed.

## Artifacts

- Frontend production build: `/Users/yunshu/Documents/GitHub/mira-mouse/dist/`
- TEST-ONLY signed plugin package: `/Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/resources/plugins/mira-example-mock-1.0.0.mira-plugin`
  - sha256: `33a0fc66a8a55845d1cda56a6f06587d83c227892dd25e4792e64bce778a9f9a`
  - publisher key id: `TEST-ONLY-mira-plugins`
  - recorded in `plugins.lock.json` with `bundleByDefault: false`
- TEST-ONLY public key: `/Users/yunshu/Documents/GitHub/mira-mouse-plugins/TEST-ONLY-mira-plugins.pub` (gitignored)
- Native installers (DMG/EXE/AppImage/DEB/RPM): **none produced locally** — blocked by crates.io network timeouts. The zero-cost GitHub Actions workflow can produce unsigned drafts on runners with network access.
- Production `.mira-plugin` release asset: **none** — `mira.amaster` lock entry still has `BLOCKED_*` placeholders. The zero-cost plugin workflow can produce self-signed plugin releases once the secret is configured.
- Expected CI asset names: `Mira_<version>_universal-unsigned.dmg`, `Mira_<version>_x64-setup-unsigned.exe`, `Mira_<version>_x86_64.AppImage`, `mira_<version>_amd64.deb`, `mira-<version>-1.x86_64.rpm`.

## Security and supply chain

- `plugins.lock.json` remains `releaseReady: false` because `mira.amaster` still contains `BLOCKED_*` placeholders; the TEST-ONLY `mira.example-mock` entry cannot silently promote a release.
- Workflows use minimal default permissions, full Action commit SHAs, concurrency controls, and protected release environments.
- `check:boundaries` confirms no brand VID/PID/protocol constants or device fixtures exist in the core repository outside allowed documentation/lock placeholders.
- No production signing keys are tracked; TEST-ONLY keys are gitignored.
- REUSE/SPDX configuration and `deny.toml` exist; `cargo-deny`/`cargo audit` cannot run until `cargo fetch` succeeds.

## Distribution and first launch

- No release exists.
- Unsigned community packages must be downloaded only from an official GitHub Release and verified against published SHA-256.
- macOS Gatekeeper, Windows SmartScreen/Unknown Publisher, and Linux udev warnings apply.
- `unsigned-community`, platform signing, Apple notarization, updater signing, and plugin signing are distinct evidence states; none are claimed as completed.

## Residual blockers

1. **Crates.io network timeouts**: prevent `cargo fetch`, `clippy`, tests, `cargo-deny`, `cargo audit`, plugin CLI runtime, and native Tauri build on this host. GitHub Actions runners usually do not have this problem.
2. **`mira.amaster` release metadata**: `plugins.lock.json` still contains `BLOCKED_*` placeholders for the default-bundled plugin.
3. **Production plugin Ed25519 key**: only TEST-ONLY keys exist; protected environment secret not configured. The zero-cost workflow is ready once the secret is added.
4. **macOS trusted signing / notarization**: Apple Developer ID + notarization cannot be free. Ad-hoc signing (`signingIdentity: "-"`) is configured to avoid the “app is damaged” dialog, but Gatekeeper still shows an “unverified developer” warning.
5. **Windows trusted signing**: OV/EV certificates cost money; zero-cost alternatives are self-signed (SmartScreen warning) or SignPath open-source signing (free but requires approval).
6. **Linux GPG / updater signing**: free with self-generated keys, but secrets must still be configured.
7. **GitHub owner/URLs/copyright not provided**: `config/project-metadata.example.toml` is empty; no remotes configured.
8. **No hardware/devices**: no `hardware-verified` capability.
9. **No Windows/Linux/macOS Intel smoke-test environments** beyond this arm64 host.
10. **Tauri backend still a skeleton**: tray, HID commands, settings/about/diagnostics pages not implemented.

## Documentation corrections made during this re-inspection

- `docs/evidence-status.md`: updated Rust/plugin-repo/Git status, added TEST-ONLY artifact evidence, and noted zero-cost release rework.
- `docs/assumptions-and-blockers.md`: marked plugin repository and Git initialization as resolved; recorded TEST-ONLY signing and crates.io timeout blockers.
- `docs/execution-plan.md`: updated phase statuses to reflect resolved repositories and partial plugin lock baseline.
- `docs/spec-traceability.md`: rewrote with accurate status per requirement, including TEST-ONLY verification.
- `docs/zero-cost-release.md`: new guide for zero-cost keys, workflows, and install notes; references the main `Release` / `Plugin Release` workflows.
- `README.md` and `docs/unsigned-release-security.md`: linked to the zero-cost guide.
- `mira-mouse-plugins/.github/workflows/release.yml`: rewritten to pack/sign plugins with Node `scripts/pack-sign.mjs` instead of the blocked Rust CLI.
- Removed separate `zero-cost-release.yml` and `zero-cost-unsigned-app-draft.yml` workflow files.
- This report replaced the previous inaccurate version.
