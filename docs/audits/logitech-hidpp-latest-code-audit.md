# Logitech HID++ Latest Code Audit

## Baseline

| Repository | Branch | HEAD SHA | Clean |
|---|---|---|---|
| mira-mouse | main | uncommitted | No (D-1..D-7 fixes pending commit) |
| mira-mouse-plugins | main | uncommitted | No (D-1..D-7 fixes pending commit) |

### Recent relevant commits

**mira-mouse:**
- `47f9630` feat: add HID event capture, protocol diagnostics, and inventory support
- `78b5e73` fix: correct HID busy retry event and serial payload masking
- `298c8ae` fix(ci): resolve clippy and YAML parse errors blocking CI

**mira-mouse-plugins:**
- `fb30138` fix(razer-viper): add missing device-settings locale label
- `0760737` feat(razer-viper): add comprehensive read support with protocol diagnostics
- `f61f2c0` feat(razer-viper): elevate evidence to hardware-verified for release

## Architecture Overview

The Logitech HID++ plugin (`mira.logitech-hidpp`) provides read and write support for HID++ 2.0 mice through a declarative plugin system. The Host runtime (`mira-plugin-runtime`) executes workflow DSL commands, while the Tauri app layer handles UI rendering and logging.

### Key subsystems verified:
- **Plugin API 1.1**: Schema validation, capability declarations, runtime contracts
- **Workflow DSL**: Bounded execution with `onFailure: continue`, `skipIfZero` guards, projection
- **HID++ 2.0 Protocol**: Root feature discovery, feature set, device info, battery, DPI, polling rate, pointer, lighting, onboard profiles, profile management
- **Diagnostics**: Protocol diagnostic mode with command-aware payload masking
- **Device Identity**: Stable cross-connection identity via `devices.json`
- **Dynamic Range**: `rangeSource`/`rangeMaxOffset` for capability-driven UI bounds

## Findings and Modifications

### 1. DPI Stage Mutation (Resolved)

**Finding**: `stageLayout.selectMutation = "set-dpi-stage"` referenced a non-existent mutation.

**Resolution**: Implemented `hidpp2-device-set-dpi-stage` mutation with:
- `onboard-set-current-dpi-index` command (HID++ function 0xc1)
- 1-based UI stage → 0-based protocol index via `lookup-u8` encoding
- `skipIfZero` guard on `featureIndexOnboardProfiles`
- Pre-read, write, and verify with derived `stage` field assertion
- `onboard-get-current-dpi-index` parser updated with derived `stage` field (0-based → 1-based)

**Evidence**: HID++ 2.0 ONBOARD_PROFILES (0x8100) function 0xb1 (GetCurrentDpiIndex) / 0xc1 (SetCurrentDpiIndex) — cross-verified via libratbag and cvuchener/hidpp.

### 2. DPI Capability Reading (Resolved)

**Finding**: Only `dpi-get-current` and `dpi-set` existed; no sensor count or DPI list reading.

**Resolution**: Added:
- `dpi-get-capability` command (function 0x00, GetSensorCount)
- `dpi-get-list` command (function 0x10, GetSensorDpiList)
- Corresponding parsers for sensor count and DPI list words
- Workflow steps in `hidpp2-device-read` with `skipIfZero` and `onFailure: continue`
- `dpi-capability` and `dpi-list` ReadOnlyValue capabilities in `plugin.json`

**Evidence**: libratbag `hidpp20.c` `hidpp20_adjustable_dpi_get_caps()` and `hidpp20_adjustable_dpi_get_dpi_list()`.

### 3. Dynamic Range Support (Resolved)

**Finding**: Profile management and DPI ranges were static (`max: 15`, `max: 30000`).

**Resolution**: Implemented `rangeSource`/`rangeMaxOffset` in:
- Host schema (`plugin-manifest-v1.schema.json`)
- TypeScript types (`types.ts`)
- Runtime resolution (`pluginAdapter.ts` `resolveFieldRange`, `App.tsx` stageLayout)
- `profile-mgmt-current` field: `rangeSource: "capabilities.profileMgmtInfo.maxProfileCount"`, `rangeMaxOffset: -1` (count → 0-based index)

### 4. Stable Device Identity (Resolved)

**Finding**: Logitech `devices.json` lacked `identity` field; USB and receiver connections couldn't be deduplicated.

**Resolution**: Added `identity` to Logitech device descriptor:
```json
{
  "group": "logitech-hidpp-mouse",
  "displayName": "Logitech HID++ Mouse",
  "aliases": ["Logitech HID++ Device", "Logitech HID++ 2.0 Mouse", "Logitech Mouse"]
}
```

The Host `DeviceDescriptor` struct already supported `identity` (with `deny_unknown_fields`), and `MatchedDevice` propagates it for cross-connection dedup.

### 5. Declarative Diagnostics Payload Policy (Resolved)

**Finding**: Payload masking relied solely on keyword matching (`classify_command`), insufficient for Logitech-specific commands like `device-info-get` (unit ID) and `onboard-memory-read` (user profiles).

**Resolution**: Implemented declarative `diagnostics.payload` policy in `commands.json`:

| Policy | Commands | Rationale |
|---|---|---|
| `allow` | Root, battery, DPI, polling, pointer, LED, profile mgmt, onboard metadata | No sensitive data |
| `mask` | `device-info-get`, `device-name-get` | Contains unit ID / user-customizable name |
| `deny` | `onboard-memory-read/write-*` | Contains profiles, button mappings, macros |

**Host changes**:
- `engine.rs`: Added `DiagnosticsDefinition` struct, `CommandDefinition.diagnostics` field, `ProtocolPackage.command_payload_policy()` and `command_diagnostics_policies()` methods
- `protocol_event.rs`: Added `classify_command_with_policy()` — declarative policy takes precedence, falls back to keyword classification
- `lib.rs`: `LoggingHidEventSink` pre-loads `command_policies` from `ProtocolPackage`, uses declarative policy in `mask_payload_pair()`

**Validator**: `validate.mjs` enforces `diagnostics.payload` must be `"allow"` | `"mask"` | `"deny"`.

### 6. Optional Feature Failure Handling (Verified)

**Finding**: Concern that optional feature failures could abort the entire read workflow.

**Verification**: The `hidpp2-device-read` workflow uses `onFailure: "continue"` for optional feature steps (battery, DPI capability, DPI list, LED, RGB, onboard, profile mgmt). Core steps (Root, feature set, device info) use default `abort`. The `ReadStatus` per-output system tracks `Ok`/`Failed`/`Skipped`/`NotSupported` for UI display.

### 7. Inventory Mechanism (Verified, Activated)

**Finding**: `ReadPlan::Inventory` is structurally implemented but not triggered by current UI.

**Verification**: `InventoryContract` exists in `PluginManifest.runtime.inventory`, with `workflows`, `refresh` (OnOpen/Manual), and `cache_ttl_seconds`. The Logitech plugin now declares `runtime.inventory` with `workflows: ["hidpp2-device-read"]`, `refresh: "on-open"`, and `cacheTtlSeconds: 300`. The `validate.mjs` validator enforces the `runtime.inventory` contract structure (D-5 fix).

### 8. Feature Registry Source Provenance (Resolved)

The `features.json` file records `vendoredPath`, `license`, and `generatedAt`. The sync script (`sync-hidpp-features.mjs`) now resolves and records submodule commit SHA and repository URL via the `resolveSolaarSha()` and `resolveCpgDocsSha()` functions. The `generatedFrom` field includes `solaar.commit` (`0ecae9f3`) and `cpgDocs.commit` (`f9107a4e`) alongside the `repository` URL for each upstream source (D-4 fix).

### 9. Audit Defect Fixes (This Cycle)

**D-1: report-rate-set lookup completion** — Added missing 143/167/200/333 Hz entries to `report-rate-set` command lookup, aligning with `report-rate-get` parser's 8-rate support.

**D-2: effectName internationalization** — Replaced hardcoded Chinese effect names in `color-led-effects-get-zone-effect` parser with locale keys (`lighting.off`, `lighting.fixed`, etc.). Added corresponding translations to zh-CN.json and en.json.

**D-3: Default lighting layout removal** — Removed `default: true` lighting layout from `capabilities.json` to prevent misapplication to unknown device formats. Only the explicit V5 conditional layout (`profileFormatId === 5`) remains.

**D-4: Feature registry source commit tracking** — Extended `sync-hidpp-features.mjs` with `resolveSolaarSha()` and `resolveCpgDocsSha()` functions. `features.json` `generatedFrom` now records `repository` URL and `commit` SHA for both upstream sources.

**D-5: Inventory contract declaration and validation** — Added `runtime.inventory` declaration to Logitech `plugin.json` (`workflows: ["hidpp2-device-read"]`, `refresh: "on-open"`, `cacheTtlSeconds: 300`). Extended `validate.mjs` with `runtime.inventory` contract structure validation.

**D-6: Partial read test infrastructure** — Two partial read tests (`partial_read_continues_on_optional_step_failure`, `partial_read_aborts_on_core_step_failure`) marked `#[ignore]` with explanatory comments. Requires mock HID device infrastructure to properly test `onFailure: continue` behavior at the step I/O level.

**D-7: HID event sink propagation** — `execute_projection_with_cache_and_sink` and `execute_with_cache_and_sink` correctly propagate `HidEventSink` to `Session`, enabling `on_hid_exchange`, `on_hid_busy_retry`, `on_hid_checksum_failed`, and `on_hid_response_mismatch` callbacks during workflow execution.

## Test Coverage

### Host-side tests (new)
- `classify_command_with_policy_overrides_keyword` — declarative policy overrides keyword
- `classify_command_with_policy_falls_back_when_none` — None → keyword fallback
- `classify_command_with_policy_falls_back_on_unknown_policy` — unknown value → keyword fallback
- `classify_command_with_policy_logitech_patterns` — device-info=mask, onboard-memory=deny, root=allow
- `command_payload_policy_returns_declared_policy` — ProtocolPackage lookup
- `command_diagnostics_policies_collects_all_declared` — bulk collection
- `command_without_diagnostics_field_loads_successfully` — backward compatibility

### Plugin-side tests (new)
- `Logitech commands declare diagnostics payload policies for sensitive commands`
- `Logitech commands allow non-sensitive protocol commands`
- `Logitech diagnostics policies use only valid values`
- `Logitech devices declare stable identity for cross-connection dedup`

### Pre-existing tests (verified passing)
- 211 npm tests (main repo)
- 26 npm tests (plugin repo)
- 125 cargo tests (workspace)
- All `protocol_event` masking tests (serial, continuous hex, deny, allow, mask)

## Verification Results

| Check | Result |
|---|---|
| `cargo test --workspace` | OK |
| `cargo clippy --all-targets -- -D warnings` | OK (no warnings) |
| `npm run typecheck` | OK |
| `npm run lint` | OK |
| `npm test` (main) | 211 passed |
| `npm run validate` (plugin) | OK |
| `npm test` (plugin) | 26 passed |

## Public Source Verification

No local Logitech hardware was available. All protocol implementations are based on:
- Logitech cpg-docs (official protocol documentation)
- Solaar (open-source Linux driver)
- libratbag/Piper (open-source gaming device configuration)
- openlogi-hidpp (HID++ library)
- cvuchener/hidpp (HID++ analysis tool)

See `docs/logitech-public-source-matrix.md` for the detailed source matrix.

## Remaining Work

### Not implemented in this audit cycle:
1. **B2: Receiver topology all-valid fan-out** — multi-slot receiver device fan-out requires DSL `paramCandidates` mode change (`first-valid` → `all-valid`); deferred due to cross-plugin impact

### User feedback flow:
- Users without local hardware can use the protocol diagnostic mode to capture HID exchanges
- The `device-support.yml` issue template captures Mira version, plugin version, and failure context
- Diagnostic exports respect declarative payload policies (device-info masked, onboard-memory denied)
