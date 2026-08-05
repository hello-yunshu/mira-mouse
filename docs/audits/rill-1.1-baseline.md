# Rill 1.1 integration baseline

Date: 2026-08-05 (Asia/Shanghai)

## Environment

- Mira commit: `b2b145dc5cd66ba0e18072ab185ed0d1e47fc019`
- Branch created for the work: `work/rill-ml-1.1-integration`
- OS: macOS 26.6, arm64
- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`
- Cargo: (same toolchain, `rust-toolchain.toml` pins `stable` + rustfmt/clippy)
- Node.js: `v26.5.1`

## Resolved versions before modification

| Surface | Declared or resolved version | Evidence |
|---|---|---|
| Workspace `rill-ml` | `1.0.0` | root `Cargo.toml` line 36 (`version = "1.0.0"`) and root `Cargo.lock` |
| Handler `rill-ml` | `1.1.0` | handler `Cargo.lock` (independent workspace; already drifted to 1.1.0 through the unconstrained `cargo update` in `model-pack.yml` line 129) |
| Host `rill-runtime-protocol` | `1.0.0` | root `Cargo.toml` line 35 and root `Cargo.lock` |
| Runtime release used by pack CI | latest signed Stable release at workflow execution time | `model-pack.yml` calls `resolve-latest-rill-release.mjs` without a pinned version by default; `check-version-sources.mjs` forbids fixed tags |
| Runtime IPC API | `2` | `rill-runtime-protocol` `RUNTIME_API_VERSION`, handshake validation in `src-tauri/src/local_ai_runtime.rs:248` |
| Handler API | `1` | handler manifest `HANDLER_API_VERSION = 1`, host check `local_ai_runtime.rs:251` |

Note: `rill-runtime-protocol 1.1.0` (published on crates.io) keeps
`RUNTIME_API_VERSION = 2`, `HANDLER_API_VERSION = 1`, `MODEL_PACK_FORMAT_VERSION = 1`,
`HANDLER_PACKAGE_FORMAT_VERSION = 1` — verified from the 1.1.0 crate source. The
1.0.0 → 1.1.0 protocol upgrade therefore does not change IPC V2 or the WIT
handler ABI.

## Commands and results

| Command | Result before modification |
|---|---|
| `cargo fmt --all --check` | passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | passed |
| `cargo test --workspace --locked` | passed; 541 app, 18 local AI, 179 plugin runtime (2 explicitly ignored hardware-infrastructure tests), remaining workspaces pass |
| `npm run lint` | passed |
| `npm run typecheck` | passed |
| `npm test` | passed; 20 files, 368 tests |
| `npm run build` | passed; existing bundle-size warning only |
| `npm run check:boundaries` | passed |
| `npm run check:structured` | passed |
| `cargo fmt --manifest-path handlers/mira-battery-handler/Cargo.toml --check` | passed |
| `cargo clippy --manifest-path handlers/mira-battery-handler/Cargo.toml --all-targets --locked -- -D warnings` | passed |
| `cargo test --manifest-path handlers/mira-battery-handler/Cargo.toml --locked` | passed |
| `cargo tree -p mira-local-ai --locked \| grep rill` | resolved `rill-ml v1.0.0` |
| `cargo tree --manifest-path handlers/mira-battery-handler/Cargo.toml --locked \| grep rill` | resolved `rill-ml v1.1.0` |

No original code or test failure was found. All baseline gates are green.

## Stage 0 acceptance answers

1. **Are host, handler, and runtime guaranteed to use one Rill minor?** No.
   Host protocol resolves to 1.0.0 while the handler already resolves to
   1.1.0, and pack CI resolves the runtime release dynamically. The
   checked-in sources cannot prove a common minor at build time.
2. **Can the same Mira commit resolve different dependencies on different
   dates?** The checked-in Rust lockfiles are stable when `--locked` is used,
   but the formal model-pack workflow runs an unconstrained handler
   `cargo update` (model-pack.yml line 129) and dynamically selects the latest
   Stable runtime. Its formal outputs can therefore change without changing
   the Mira commit.
3. **Does the handler recreate and retrain the model for every prediction?**
   Yes. The process is persistent, but every `invoke` calls
   `mira_local_ai::predict`, which constructs a new `LinearRegression` and
   replays the observations.
4. **Is model compatibility checked only as `feature_count == 9`?** Yes. No
   ordered feature schema or descriptor identity is present before this work.
5. **Does each call send at most about 4096 history records?** Yes. The host
   keeps only the most recent `MAX_PREDICTION_SAMPLES = 4096` records in each
   IPC V2 request (`src-tauri/src/local_ai_controller.rs:57,391`). The library
   separately rejects more than 10,000 inputs.

## Baseline risk summary

- Production fallback, IPC V2, WIT handler API v1, signed pack verification,
  and non-AI operation are present and must remain intact.
- Feature order and normalization identity are not protected.
- The deterministic baseline uses time decay, but ML training is unweighted.
- The current stateless handler repeats training and transports a large history.
- Preview APIs must remain opt-in and must not become a model-pack dependency.
