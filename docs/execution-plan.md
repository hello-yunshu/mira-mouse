<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Mira Execution Plan

Updated: 2026-06-18

This plan applies the evidence labels required by the implementation prompt. A phase continues when an external dependency is blocked.

| Phase | Deliverable | Exit evidence | Status |
|---|---|---|---|
| 0 | Two repositories, identity, research inventory, threat and license notes | Files and environment probes | in-progress: both repositories initialized; GitHub owner/URLs and copyright metadata still blocked |
| 1 | Core contracts, package verifier, bounded DSL, mock UI | Unit and malicious-package tests | in-progress: source present and formatted; Rust tests blocked by crates.io fetch timeouts |
| 2 | AMaster read-only plugin and fixtures in plugin repository | Offline fixture tests only | in-progress: plugin repository and fixtures exist; signed release blocked by production key |
| 3 | Locked plugin sync and bundled baseline | Deterministic package and offline sync tests | partial: TEST-ONLY signed `mira.example-mock` in lock; `mira.amaster` still has `BLOCKED_*` placeholders; `cargo xtask` cannot build locally |
| 4 | Low-risk writes | Read-modify-write plus hardware readback | blocked: no hardware; writes disabled |
| 5 | Profiles, notifications, tray favorites, night mode | Core unit/UI tests | in-progress: only core primitives and mock UI exist |
| 6 | Logitech/Razer experimental descriptors | Read-only, model-specific evidence | in-progress: descriptors exist in plugin repository; no signed release or hardware verification |
| 7 | CI, packages, release verification, legal/community docs | Host build plus CI configuration checks | in-progress: CI/YAML/docs present; Rust/native build blocked by network; no remotes configured |

## Execution order

1. Establish repository and evidence boundaries.
2. Implement versioned, brand-neutral contracts and deterministic tooling.
3. Build a capability-driven React shell using mock data only at an explicit test boundary.
4. Put all device facts and fixtures in `mira-mouse-plugins`.
5. Exercise validate, test, pack, inspect, and locked sync failure paths.
6. Run every locally available formatter, test, parser, boundary, and build check.
7. Re-read the specification and publish the exact residual blockers in the final report.
