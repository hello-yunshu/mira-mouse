<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Specification Traceability

Status is deliberately conservative. `fixture-verified` is never treated as hardware evidence.

| Requirement | Implementation | Verification | Status / evidence |
|---|---|---|---|
| Fixed identity and two-repository boundary | `Cargo.toml`, `package.json`, `src-tauri/tauri.conf.json` | `scripts/check-boundaries.mjs` | partial: both repositories initialized; GitHub owner/URLs and copyright metadata still blocked |
| Six mandatory execution records | `docs/*.md` | non-empty document check | build-verified; updated during this session |
| Brand-neutral core | `crates/mira-plugin-api`, `crates/mira-plugin-runtime` | Rust unit tests and boundary scan | source complete and formatted; Rust execution blocked by crates.io fetch timeouts |
| Versioned manifest and capability contract | `crates/mira-plugin-api`, `schemas/plugin-manifest-v1.schema.json` | checked-in Rust tests + JSON Schema | source complete; Rust execution blocked by crates.io fetch timeouts |
| Safe package whitelist, hashes, signature contract | `crates/mira-plugin-runtime`, `docs/plugin-package-format.md` | checked-in malicious-package tests + TEST-ONLY Node sign/verify of `mira.example-mock` | source complete; Rust execution blocked; TEST-ONLY package verified independently |
| Bounded protocol DSL | `crates/mira-plugin-runtime/src/dsl.rs` | checked-in limit tests | source complete; Rust execution blocked |
| Capability-driven UI | `src/App.tsx`, `src/mock.ts`, `src/types.ts` | Vitest tests, Vite build | fixture-verified |
| Theme, low motion/transparency fallback | `src/styles.css`, `src/theme.ts` | Vitest theme tests | fixture-verified |
| Tray/autostart/single-instance | `src-tauri/src/lib.rs` registers plugins only | host integration test | blocked: no tray icon, no autostart state command, no real integration test |
| Plugin SDK and CLI workflow | `mira-plugin-cli`, `xtask`, plugin docs | validate → test → pack → inspect | source complete; Rust execution blocked; TEST-ONLY Node pack script produces deterministic signed `.mira-plugin` |
| AMaster protocols isolated to plugin repository | `../mira-mouse-plugins/plugins/amaster` | boundary scan | source present and validated; signed release blocked by production key |
| Locked immutable baseline | `plugins.lock.json`, `xtask/src/main.rs` | offline/cache/failure tests | partial: TEST-ONLY `mira.example-mock` entry with verified cache path and sha256; `mira.amaster` still `releaseReady: false` with `BLOCKED_*` placeholders |
| Low-risk writes with readback | plugin workflows | hardware matrix | blocked: no hardware; writes disabled by manifest evidence gate |
| Logitech/Razer | sibling experimental descriptors | schema validation | source present in plugin repository; no signed release or hardware verification |
| Platform packages and updater | Tauri config and release workflow | native package smoke tests | blocked: Rust dependency fetch fails; no installer produced |
| GitHub/community/legal files | repository roots and `.github` | YAML/CFF/JSON parse, license counts, SHA scan | build-verified; repository remotes/settings blocked |
| No false contact links | example metadata and omitted contact entries | content scan | build-verified |
| Privacy and fail-closed behavior | runtime, threat model, diagnostics/install docs | Node tests and checked-in Rust adversarial tests | partial: Node build-verified; Rust blocked; no production diagnostics path yet |
