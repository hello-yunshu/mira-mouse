# Logitech HID++ Public Source Matrix

This document records the cross-verified public sources for every Logitech HID++ protocol capability implemented in the `mira.logitech-hidpp` plugin. It exists so that maintainers without local Logitech hardware can confidently implement and extend support, and so that any future contributor can audit which sources back each protocol decision.

No local Logitech hardware was available during the baseline audit. All protocol implementations are grounded in the sources listed here.

## Evidence Labels

| Label | Meaning |
|---|---|
| `official-spec` | Logitech cpg-docs (official protocol documentation) explicitly describes the feature/function/field |
| `public-cross-verified` | Two or more independent mature public implementations agree on request/response/field semantics |
| `public-single-source` | Only one mature public implementation describes this capability; no conflicting evidence found |
| `fixture-verified` | Covered by a plugin test fixture (synthesized from real device captures or public traces) |
| `community-hardware-report` | Reported by community users with real hardware; not yet captured as a fixture |
| `local-hardware-sample` | Captured from local real hardware (none in current baseline) |
| `inferred` | Derived from related features or protocol structure; not directly documented |

## Vendored Sources

| Source | Repository | Submodule commit | License | Vendored path | Used for |
|---|---|---|---|---|---|
| cpg-docs | https://github.com/cvuchener/cpg-docs | `f9107a4e3190d798f9be8c5d596f255f5e5741d2` (master) | See upstream | `vendor/cpg-docs/hidpp20/features` | Official feature documentation (`.rst` files) |
| Solaar | https://github.com/pwr-Solaar/Solaar | `0ecae9f388aab0661025e169ad731eea74a2a826` (1.1.20rc3) | GPL-2.0-or-later | `vendor/solaar/lib/logitech_receiver/` | Feature constants, protocol implementation, device profiles |

### Other cross-referenced public repositories (not vendored)

| Repository | Used for | License |
|---|---|---|
| https://github.com/libratbag/libratbag | HID++ 2.0 capability/list reads, DPI stage semantics | MIT |
| https://github.com/libratbag/piper | UI-level capability cross-reference | MIT |
| https://github.com/cvuchener/hidpp | HID++ analysis, ONBOARD_PROFILES function numbers | GPL-2.0-or-later |
| https://github.com/PixlOne/logiops | HID++ 2.0 feature implementation cross-reference | MIT |

> **Note**: The `features.json` `generatedFrom` field now records `path`, `license`, `repository` URL, and `commit` SHA for each vendored source. The submodule commit SHAs (Solaar: `0ecae9f3`, cpg-docs: `f9107a4e`) are resolved automatically at generation time by `sync-hidpp-features.mjs` via the `resolveSolaarSha()` and `resolveCpgDocsSha()` functions. The submodule commit SHAs above are captured from `git submodule status` at audit time and are now pinned into `features.json` `generatedFrom`.

## Feature Registry

The plugin ships a generated `protocol/features.json` containing 195 HID++ 2.0 feature definitions. Each entry records the feature name, hex id, decimal id, the cpg-docs `.rst` filename (when documented), and the list of sources that confirm the feature. Features with `documented: null` are confirmed only via Solaar's `hidpp20_constants.py`.

## Implemented Capability Matrix

The following table maps each implemented HID++ 2.0 feature to its Mira command, workflow, read/write status, fixtures, and evidence. Feature IDs follow the HID++ 2.0 Root feature discovery convention.

### Identity and Root

#### ROOT (0x0000) — IRoot

| Field | Value |
|---|---|
| Mira command | `root-get-feature` |
| Function | GetFeature (0x00/0x01) |
| Request | `[deviceIndex, 0x00, 0x01, featureId(be-u16)]` |
| Response | `[deviceIndex, featureIndex, flags]` |
| Workflow | `hidpp2-device-read` step 1 (core, abort on failure) |
| Parser | `root-get-feature` |
| Fixture | `tests/fixtures/hidpp2-root-get-feature.json` |
| Read/Write/Readback | Read |
| Failure protection | Core step — failure aborts workflow |
| Diagnostics policy | `allow` |
| Evidence | `official-spec` + `public-cross-verified` (cpg-docs `0x0000-IRoot.rst`, Solaar, libratbag) |
| Source agreement | All sources agree on request/response layout |

Root probes `deviceIndex` candidates `1..6, 0xFF` via `paramCandidates` (first-valid mode). The first slot returning a valid `featureIndex` for `DEVICE_FW_VERSION` becomes the active device.

#### FEATURE_SET (0x0001)

| Field | Value |
|---|---|
| Mira command | `feature-set-get-count` |
| Function | GetCount (0x00) |
| Request | `[deviceIndex, featureIndex, 0x00]` |
| Response | `[deviceIndex, featureIndex, count]` |
| Workflow | `hidpp2-device-read` (optional, `onFailure: continue`) |
| Parser | `feature-set-get-count` |
| Fixture | — |
| Read/Write/Readback | Read |
| Failure protection | `skipIfZero` on `featureIndexFeatureSet`, `onFailure: continue` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (Solaar, libratbag) |
| Source agreement | Consistent |

#### DEVICE_FW_VERSION (0x0003) — Device Information

| Field | Value |
|---|---|
| Mira command | `device-info-get` |
| Function | GetDeviceInfo (0x01) |
| Request | `[deviceIndex, featureIndex, 0x01]` |
| Response | `[deviceIndex, featureIndex, unitId, modelId, transport, ...]` |
| Workflow | `hidpp2-device-read` (optional, `onFailure: continue`) |
| Parser | `device-info-get` |
| Fixture | — |
| Read/Write/Readback | Read |
| Failure protection | `skipIfZero` on `device.featureIndex`, `onFailure: continue` |
| Diagnostics policy | `mask` (contains unit ID — device-unique identifier) |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | All sources agree; unit ID masked in diagnostics per privacy policy |

#### DEVICE_NAME (0x0005)

| Field | Value |
|---|---|
| Mira commands | `device-name-get-count`, `device-name-get` |
| Functions | GetDeviceNameLength (0x00/0x01), GetDeviceName (0x10/0x11) |
| Request (count) | `[deviceIndex, featureIndex, 0x01]` |
| Request (name) | `[deviceIndex, featureIndex, 0x11, charIndex]` |
| Response (name) | `[deviceIndex, featureIndex, ...chars]` |
| Workflow | `hidpp2-device-read` (optional, `onFailure: continue`) |
| Parser | `device-name-get-count`, `device-name-get` |
| Fixture | — |
| Read/Write/Readback | Read |
| Failure protection | `skipIfZero` on `featureIndexDeviceName`, `onFailure: continue` |
| Diagnostics policy | `mask` (user-customizable name) |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent; multi-chunk read supported via `charIndex` param |

### Power and Battery

#### BATTERY_STATUS (0x1000)

| Field | Value |
|---|---|
| Mira commands | `battery-get-status`, `battery-get-capability` |
| Functions | GetStatus (0x01), GetCapability (0x10/0x11) |
| Request (status) | `[deviceIndex, featureIndex, 0x01]` |
| Response (status) | `[deviceIndex, featureIndex, currentLevel, nextLevel, status]` |
| Workflow | `hidpp2-device-read` (optional, `onFailure: continue`) |
| Parser | `battery-get-status`, `battery-get-capability` |
| Fixture | `tests/fixtures/hidpp2-battery-status.json` |
| Read/Write/Readback | Read |
| Failure protection | `skipIfZero` on `featureIndexBattery`, `onFailure: continue` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent |

#### UNIFIED_BATTERY (0x1004)

| Field | Value |
|---|---|
| Mira commands | `unified-battery-get-capabilities`, `unified-battery-get-status` |
| Functions | GetCapabilities (0x00/0x01), GetStatus (0x10/0x11) |
| Workflow | `hidpp2-device-read` (optional, `onFailure: continue`) |
| Parser | `unified-battery-get-capabilities`, `unified-battery-get-status` |
| Fixture | `tests/fixtures/g705-unified-battery.json` |
| Read/Write/Readback | Read |
| Failure protection | `skipIfZero` on `featureIndexUnifiedBattery`, `onFailure: continue`; independent of BATTERY_STATUS — one failing does not block the other |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar) + `fixture-verified` (G705) |
| Source agreement | Consistent |

### Pointer and Sensor

#### ADJUSTABLE_DPI (0x2201)

| Field | Value |
|---|---|
| Mira commands | `dpi-get-capability`, `dpi-get-list`, `dpi-get-current`, `dpi-set` |
| Functions | GetSensorCount (0x00), GetSensorDpiList (0x10), GetCurrentDpi (0x20/0x21), SetDpi (0x30) |
| Request (capability) | `[deviceIndex, featureIndex, 0x00]` |
| Request (list) | `[deviceIndex, featureIndex, 0x10, sensorIndex]` |
| Request (current) | `[deviceIndex, featureIndex, 0x21]` |
| Request (set) | `[deviceIndex, featureIndex, 0x30, 0x00, dpi(be-u16)]` |
| Response (list) | `[deviceIndex, featureIndex, ...dpiListWords]` |
| Workflow | `hidpp2-device-read` (optional, `onFailure: continue`); mutation `hidpp2-device-set-dpi-value` |
| Parser | `dpi-get-capability`, `dpi-get-list`, `dpi-get-current` |
| Fixture | `tests/fixtures/g705-adjustable-dpi.json` |
| Read/Write/Readback | Read + Write + Readback (verify `dpiValue` == param `dpi`) |
| Failure protection | `skipIfZero` on `featureIndexDpi`, `onFailure: continue`; mutation gated by `skipIfZero` on `featureIndexDpi` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag `hidpp20_adjustable_dpi_get_caps` / `hidpp20_adjustable_dpi_get_dpi_list`) + `fixture-verified` (G705) |
| Source agreement | Consistent; `dpi-get-capability` and `dpi-get-list` added in this audit cycle based on libratbag `hidpp20.c` |

The `dpi-get-capability` command reads sensor count (function 0x00), and `dpi-get-list` reads the DPI list words for a given sensor index (function 0x10). These mirror libratbag's `hidpp20_adjustable_dpi_get_caps()` and `hidpp20_adjustable_dpi_get_dpi_list()`.

#### EXTENDED_ADJUSTABLE_DPI (0x2202)

| Field | Value |
|---|---|
| Mira commands | `extended-dpi-get-current`, `extended-dpi-set` |
| Functions | GetCurrentDpi (0x50), SetDpi (0x60) |
| Request (current) | `[deviceIndex, featureIndex, 0x50, 0x00]` |
| Request (set) | `[deviceIndex, featureIndex, 0x60, 0x00, dpi(be-u16)]` |
| Workflow | `hidpp2-device-read` (optional); mutation `hidpp2-device-set-dpi-value-extended` |
| Parser | `extended-dpi-get-current` |
| Fixture | — |
| Read/Write/Readback | Read + Write + Readback (verify `dpiValue` == param `dpi`) |
| Failure protection | `skipIfZero` on `featureIndexExtendedDpi`; mutation also gated by `controlMode.hostMode` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent |

#### MOUSE_POINTER (0x2200)

| Field | Value |
|---|---|
| Mira command | `mouse-pointer-get` |
| Function | GetMousePointerInfo (0x00) |
| Request | `[deviceIndex, featureIndex, 0x00]` |
| Workflow | `hidpp2-device-read` (optional, `onFailure: continue`) |
| Parser | `mouse-pointer-get` |
| Fixture | — |
| Read/Write/Readback | Read |
| Failure protection | `skipIfZero` on `featureIndexMousePointer`, `onFailure: continue` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent |

#### POINTER_SPEED (0x2205)

| Field | Value |
|---|---|
| Mira commands | `pointer-speed-get`, `pointer-speed-set` |
| Functions | GetPointerSpeed (0x00), SetPointerSpeed (0x10) |
| Request (get) | `[deviceIndex, featureIndex, 0x00]` |
| Request (set) | `[deviceIndex, featureIndex, 0x10, speed(be-u16)]` |
| Workflow | `hidpp2-device-read` (optional); mutation `hidpp2-device-set-pointer-speed` |
| Parser | `pointer-speed-get` |
| Fixture | — |
| Read/Write/Readback | Read + Write + Readback (verify `speedRaw` == param `speed`) |
| Failure protection | `skipIfZero` on `featureIndexPointerSpeed`; mutation gated by `controlMode.hostMode` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent; input range `46..511` per Solaar/libratbag |

### Report Rate

#### REPORT_RATE (0x8060)

| Field | Value |
|---|---|
| Mira commands | `report-rate-get-list`, `report-rate-get`, `report-rate-set` |
| Functions | GetReportRateList (0x00), GetReportRate (0x10), SetReportRate (0x20) |
| Request (set) | `[deviceIndex, featureIndex, 0x20, rate(lookup-u8)]` |
| Lookup (set) | `125→8, 143→7, 167→6, 200→5, 250→4, 333→3, 500→2, 1000→1` |
| Workflow | `hidpp2-device-read` (optional); mutation `hidpp2-device-set-polling-rate` |
| Parser | `report-rate-get-list`, `report-rate-get` |
| Fixture | — |
| Read/Write/Readback | Read + Write + Readback (verify `pollingRate` == param `rate`) |
| Failure protection | `skipIfZero` on `featureIndexReportRate`; mutation uses onboard memory patch when in onboard mode |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent; standard setter lookup covers `125, 250, 500, 1000` Hz |

#### EXTENDED_ADJUSTABLE_REPORT_RATE (0x8061)

| Field | Value |
|---|---|
| Mira commands | `extended-report-rate-get-list`, `extended-report-rate-get`, `extended-report-rate-set` |
| Functions | GetReportRateList (0x10), GetReportRate (0x20), SetReportRate (0x30) |
| Request (set) | `[deviceIndex, featureIndex, 0x30, rate(lookup-u8)]` |
| Lookup (set) | `125→0, 250→1, 500→2, 1000→3, 2000→4, 4000→5, 8000→6` |
| Workflow | `hidpp2-device-read` (optional); mutation `hidpp2-device-set-polling-rate-extended` |
| Parser | `extended-report-rate-get-list`, `extended-report-rate-get` |
| Fixture | — |
| Read/Write/Readback | Read + Write + Readback (verify `pollingRate` == param `rate`) |
| Failure protection | `skipIfZero` on `featureIndexExtendedReportRate`; mutation gated by `controlMode.hostMode` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent; extended setter lookup covers `125..8000` Hz |

### Lighting

#### COLOR_LED_EFFECTS (0x8070)

| Field | Value |
|---|---|
| Mira commands | `color-led-effects-get-info`, `color-led-effects-get-zone-effect`, `color-led-effects-set-zone-effect` |
| Functions | GetInfo (0x00), GetZoneEffect (0x10), SetZoneEffect (0x30) |
| Request (set) | `[deviceIndex, featureIndex, 0x30, zone, enabled(bool), color(rgb), effect(u8), speed(u8), brightness(u8), 0x00, 0x00, 0x00, 0x00, 0x01]` |
| Workflow | `hidpp2-device-read` (optional); mutations `hidpp2-device-set-mouse-lighting`, `hidpp2-device-set-mouse-lighting-onboard` |
| Parser | `color-led-effects-get-info`, `color-led-effects-get-zone-effect` |
| Fixture | — |
| Read/Write/Readback | Read + Write + Readback (verify `effect` and `color`); postWrite `rgb-control-set` when RGB_EFFECTS present |
| Failure protection | `skipIfZero` on `featureIndexColorLed`, `onFailure: continue`; `skipIfAllZero` with `featureIndexOnboardProfiles` for mutation |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent; effect enum `{0,1,3,4,5,10,11,12}` cross-verified |

#### RGB_EFFECTS (0x8071)

| Field | Value |
|---|---|
| Mira commands | `rgb-effects-get-info`, `rgb-control-get`, `rgb-control-set` |
| Functions | GetInfo (0x00), GetRGBControl (0x50), SetRGBControl (0x50) |
| Workflow | `hidpp2-device-read` (optional); used as `postWrite` in `set-mouse-lighting` mutation |
| Parser | `rgb-effects-get-info`, `rgb-control-get` |
| Fixture | — |
| Read/Write/Readback | Read + Write (postWrite) + Readback (verify `enabled`) |
| Failure protection | `skipIfZero` on `featureIndexRgbEffects`, `onFailure: continue` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar) |
| Source agreement | Consistent; RGB host-control handoff after onboard profile patch |

### Onboard Profiles

#### ONBOARD_PROFILES (0x8100)

| Field | Value |
|---|---|
| Mira commands | `onboard-get-description`, `onboard-get-mode`, `onboard-set-mode`, `onboard-get-current-profile`, `onboard-get-current-dpi-index`, `onboard-set-current-dpi-index`, `onboard-memory-read`, `onboard-memory-write-start`, `onboard-memory-write-chunk`, `onboard-memory-write-end` |
| Functions | GetDescription (0x01), SetMode (0x11), GetMode (0x21), GetCurrentProfile (0x41), GetCurrentDpiIndex (0xb1), SetCurrentDpiIndex (0xc1), MemoryRead (0x51), MemoryWriteStart (0x61), MemoryWriteChunk (0x71), MemoryWriteEnd (0x81) |
| Workflow | `hidpp2-device-read` (mode only); `hidpp2-device-onboard-read` (full sector read with 16-byte chunks, offsets 0..sectorSize-16) |
| Parser | `onboard-get-description`, `onboard-get-mode`, `onboard-get-current-profile`, `onboard-get-current-dpi-index`, `onboard-memory-read` |
| Fixture | — |
| Read/Write/Readback | Read + Write + Readback (full sector re-read after write); CRC-CCITT-FALSE checksum verification |
| Failure protection | `skipIfZero` on `featureIndexOnboardProfiles`; memory read uses core steps (abort on failure in `hidpp2-device-onboard-read`) |
| Diagnostics policy | `allow` for metadata commands; `deny` for `onboard-memory-read/write-*` (contains user profiles, button mappings, macros) |
| Evidence | `official-spec` (cpg-docs `0x8100-OnboardProfiles.rst`) + `public-cross-verified` (Solaar, libratbag, cvuchener/hidpp) |
| Source agreement | Function numbers `0xb1`/`0xc1` for GetCurrentDpiIndex/SetCurrentDpiIndex cross-verified via libratbag `hidpp20.c` and cvuchener/hidpp |

The `onboard-set-current-dpi-index` command (function 0xc1) was added in this audit cycle to resolve the dangling `stageLayout.selectMutation = "set-dpi-stage"` reference. It encodes the 1-based UI stage to the 0-based protocol index via `lookup-u8`, with a corresponding derived `stage` field in the `onboard-get-current-dpi-index` parser for readback verification.

### Profile Management

#### PROFILE_MANAGEMENT (0x8110)

| Field | Value |
|---|---|
| Mira commands | `profile-mgmt-get-info`, `profile-mgmt-get-count`, `profile-mgmt-get-current`, `profile-mgmt-set-current`, `profile-mgmt-control` |
| Functions | GetInfo (0x00), GetCount (0x10), GetCurrent (0x20), SetCurrent (0x30), Control (0x60) |
| Workflow | `hidpp2-device-read` (optional); `hidpp2-device-profile-mgmt-read`; mutation `hidpp2-device-set-profile-mgmt-current` |
| Parser | `profile-mgmt-get-info`, `profile-mgmt-get-count`, `profile-mgmt-get-current` |
| Fixture | — |
| Read/Write/Readback | Read + Write + Readback (verify `profileIndex` == param `profileIndex`) |
| Failure protection | `skipIfZero` on `featureIndexProfileManagement`; mutation gated by `controlMode.hostMode` |
| Diagnostics policy | `allow` |
| Evidence | `public-cross-verified` (cpg-docs, Solaar, libratbag) |
| Source agreement | Consistent; dynamic range via `rangeSource: "capabilities.profileMgmtInfo.maxProfileCount"` with `rangeMaxOffset: -1` (count → 0-based index) |

### Reserve Features (Read-only, future inventory)

The following features are probed via Root `GetFeature` in `hidpp2-device-read` but do not yet have dedicated commands. Their `featureIndex` is captured for future inventory expansion:

| Feature | ID | Evidence | Notes |
|---|---|---|---|
| SURFACE_TUNING | 0x8020 | `public-single-source` (Solaar) | Probed, no command yet |
| XY_STATS | 0x8101 | `public-single-source` (Solaar) | Probed, no command yet |
| WHEEL_STATS | 0x8111 | `public-single-source` (Solaar) | Probed, no command yet |

These are listed in the `hidpp2-device-read` workflow with `onFailure: continue` so their absence on any device does not affect other reads.

## Onboard Memory Layout

The `hidpp2-device-onboard-read` workflow reads the active profile sector in 16-byte chunks from offset 0 through `sectorSize - 16`. The sector size is read dynamically from `onboard-get-description`.

### Profile Format 5 Lighting Layout

| Field | Offset | Encoding | Source |
|---|---|---|---|
| effect | 219 | u8 | libratbag `hidpp20_onboard_profiles_profile_format_v5` |
| color | 220 | rgb | libratbag |
| speed | 223 | be-u16 | libratbag |
| brightness | 225 | u8 | libratbag |
| extraColor | 226 | rgb | libratbag |
| padding | 252 | 0x00 | libratbag |

Used by mutation `hidpp2-device-set-mouse-lighting` when `onboardDescription.profileFormatId == 5`.

### Non-V5 Lighting Layout (removed in D-3)

The implicit `default: true` lighting layout has been removed from `capabilities.json` (D-3 fix). Previously this layout provided fallback offsets for `profileFormatId != 5` devices, but the `inferred` offsets had not been cross-verified against a second independent public implementation for all format IDs, risking cross-format misapplication.

Only the explicit V5 conditional layout (`when.profileFormatId === 5`) remains in `capabilities.json`. The `hidpp2-device-set-mouse-lighting-onboard` mutation is now only applicable when `onboardDescription.profileFormatId == 5`. Devices reporting other profile format IDs will not have lighting mutation support until explicit layouts for those formats are added with proper cross-verification.

> **Remaining work**: A future enhancement should add explicit layout entries (keyed by `profileFormatId`, feature version, sector size, and profile version) for non-V5 formats once real device format coverage and cross-verification against a second independent public implementation are available.

## Diagnostics Payload Policies

Every command in `commands.json` declares a `diagnostics.payload` policy. The Host runtime (`engine.rs`) loads these declaratively and the `LoggingHidEventSink` applies them before any payload reaches the log store.

| Policy | Commands | Rationale |
|---|---|---|
| `allow` | Root, FeatureSet, Battery, UnifiedBattery, DPI, ExtendedDPI, MousePointer, PointerSpeed, ReportRate, ExtendedReportRate, ColorLedEffects, RGBEffects, Onboard metadata (description/mode/profile/dpi-index), ProfileMgmt | No sensitive data — protocol headers and capability values only |
| `mask` | `device-info-get`, `device-name-get` | Contains unit ID (device-unique) or user-customizable name |
| `deny` | `onboard-memory-read`, `onboard-memory-write-start`, `onboard-memory-write-chunk`, `onboard-memory-write-end` | Contains user profiles, button mappings, macros |

When no declarative policy is present, the Host falls back to keyword-based classification (`classify_command`) for backward compatibility with AMaster and Razer plugins.

## Test Fixtures

| Fixture | Feature | Evidence label | Source |
|---|---|---|---|
| `hidpp2-root-get-feature.json` | ROOT | `fixture-verified` | Synthesized from protocol spec |
| `hidpp2-battery-status.json` | BATTERY_STATUS | `fixture-verified` | Synthesized from protocol spec |
| `g705-adjustable-dpi.json` | ADJUSTABLE_DPI | `fixture-verified` + `community-hardware-report` | Captured from G705 via 046d:c547 receiver on macOS |
| `g705-unified-battery.json` | UNIFIED_BATTERY | `fixture-verified` + `community-hardware-report` | Captured from G705 via 046d:c547 receiver on macOS |
| `no-match.json` | — | `fixture-verified` | Negative test — non-HID++ device |

### Hardware-verified models

`devices.json` declares:
- `G705 Mouse through 046d:c547 receiver on macOS`

No other local hardware samples exist in the current baseline.

## Source Conflict Resolution

No unresolvable source conflicts were identified during this audit. All cross-referenced public implementations agree on:
- Feature IDs and function numbers
- Request/response byte layouts
- Field semantics and units
- DPI stage 0-based/1-based conversion (protocol 0-based, UI 1-based)
- Polling rate lookup tables
- Onboard profile sector chunking (16 bytes)

Where cpg-docs provides official documentation, it is treated as authoritative. Where cpg-docs is silent (e.g., `FEATURE_SET`, `FEATURE_INFO`, `DEVICE_FW_VERSION`), Solaar's `hidpp20_constants.py` is the primary source, cross-verified against libratbag where applicable.

## Maintenance

When updating vendored sources:

1. Record the old and new submodule commit SHAs
2. Run `node scripts/sync-hidpp-features.mjs --check` to detect registry drift
3. Review the upstream diff before committing generated `features.json` changes
4. Update the submodule commit SHAs in the table above
5. Do not run `git pull master` and commit blindly — audit feature additions/changes first
6. Do not vendor research material into the `.mira-plugin` package

When adding new protocol capabilities:

1. Cross-verify against at least two independent public sources (or one source + official spec)
2. Record the evidence label in this matrix
3. Add a fixture when possible
4. Declare a `diagnostics.payload` policy in `commands.json`
5. Ensure the validator (`validate.mjs`) covers the new structure
