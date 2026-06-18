<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Assumptions and Blockers

Updated: 2026-06-18

| Item | Status | Evidence / impact | Unblock action |
|---|---|---|---|
| GitHub owner and repository URLs | blocked | Not provided; no remote is fabricated | Populate `config/project-metadata.toml` from the example |
| Developer name and copyright owner | blocked | Not provided | Supply metadata before release |
| GitHub/X/Telegram links | blocked | Not provided; UI hides them | Supply verified URLs |
| Sibling plugin repository | resolved | `/Users/yunshu/Documents/GitHub/mira-mouse-plugins` exists; `npm run validate` and `npm test` pass; four plugins and fixtures are present | Configure remotes and release environments |
| Plugin release hash and publisher key | partial | `mira.example-mock` is signed with a TEST-ONLY key and recorded in `plugins.lock.json`; `mira.amaster` still has `BLOCKED_*` placeholders | Complete protected plugin release for `mira.amaster` and replace placeholders |
| Plugin production Ed25519 key | blocked | Only TEST-ONLY keys exist; test keys are prohibited from release | Configure protected environment secret and update `publisherKeyId`; zero-cost workflow is ready |
| Tauri updater signing key | blocked | No production key supplied; updater remains disabled | Configure updater key and signed metadata |
| Apple signing/notarization | blocked | No Apple Developer identity/notary credentials | Configure protected release environment |
| Windows trusted code signing | blocked | No certificate or Windows host | Configure protected release environment and Windows runner |
| Linux optional GPG signing | blocked | No GPG identity | Configure protected release environment |
| Hardware verification | blocked | No model, firmware, transport, or device supplied | Record controlled hardware matrix runs |
| Rust/Tauri host build | partial | `cargo` and `rustc` installed via Homebrew; `cargo fmt` applied; `cargo fetch`/`clippy`/`test` are blocked by crates.io timeouts in this environment | Resolve network/registry access or build on a runner with cached dependencies |
| Windows/Linux native smoke tests | blocked | Current host is macOS 26.5.1 arm64 only | Run checked-in CI on genuine runners/containers |
| Official-document live retrieval | blocked | Not attempted in this session | Re-run page-content audit with working browser retrieval |
| Git repository initialization | resolved | Both `mira-mouse` and `mira-mouse-plugins` are now Git repositories (`main` branch) | Add official remotes when owner/URLs are supplied |

No blocked item authorizes a placeholder in a production release asset.
