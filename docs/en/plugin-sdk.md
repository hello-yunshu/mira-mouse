<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin SDK

Plugins declare device matching, topology, capabilities, host-controlled UI placement hints, fields, protocol workflows, localized labels, narrow Linux permission metadata, and offline Fixtures. Plugins cannot provide CSS, HTML, JavaScript, native code, scripts, network access, or filesystem access.

## Declarative controls

The host renders `plugin.json.capabilities`; it does not select controls by vendor or model. Built-in complex renderers cover `DpiStages` and `LightingZone`. The generic renderer supports `Segmented`, `Select`, `Toggle`, `Slider`, `Number`, `Color`, `GradientStops`, `ReadOnlyValue`, and `Action`.

Capability `metadata` may contain only declarative data interpreted by the host. Each capability describes its accent source, editable fields, option sources, visibility conditions, and runtime state mappings through declarative fields under `metadata` (`accentSource`, `fields`, `zones`, `stageLayout`, `statusDisplay`, `stateMapping`, `visibleWhen`). The host renders driven by these declarations and never reads any vendor- or model-specific hardcoded logic.

Each capability should also declare one or more `placements`:

```json
"placements": [
  { "region": "control", "group": "performance", "order": 10, "span": 1, "icon": "gauge" },
  { "region": "status", "order": 20, "span": 2, "icon": "gauge" }
]
```

- `region`: `hero`, `control`, `status`, or `details`.
- `group`: capabilities with the same value share one control tab.
- `order`: ascending order within the region.
- `span`: retained for manifest compatibility; the equal-row dashboard surface normalizes visible items to the same width.
- `icon`: a host-controlled token: `battery`, `gauge`, `wave`, `lightbulb`, `timer`, `profile`, `info`, or `settings`.

The host keeps page sections stable and distributes every visible item at equal width across one full row. Dashboard control groups and status items are capped at six per region; extra declarations are rejected and defensively hidden by the host. Plugins cannot inject components or styles.

Metadata never supplies executable code or CSS. A control is writable only when the signed plugin is trusted, the connected device exposes the declared mutation, and the runtime validates its input schema.

## Declarative fields

Declarative fields are the core of capability `metadata`. Plugins declare UI structure, data sources, edit modes, and visibility conditions through these fields. The host reads the declarations and drives rendering without any vendor- or model-specific hardcoded logic.

### fields

`fields` is a top-level capability field declaring the list of subfields for that capability (up to 32). Each field describes an editable or read-only device state item with the following properties:

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `id` | string | yes | Unique field identifier matching `^[a-z][a-z0-9-]*$`, max 64 chars. |
| `source` | string | yes | Dotted read path within the device state, e.g. `state.pollingRate`, `battery`. |
| `editor` | string | yes | Editor type determining UI rendering; see the enum table below. |
| `mutation` | string \| string[] | no | Mutation id invoked when writing this field, or an ordered array of candidates. The host picks the first candidate supported by the device's `writableMutations`. |
| `param` | string | no | Single input parameter name for the mutation. |
| `params` | object | no | Composite input parameters for the mutation, as key-value pairs. |
| `paramSources` | object | no | Fills remaining parameters from the current snapshot during composite writes; live readings override fallback values in `params`. |
| `labelKey` | string | no | i18n key for the field label. |
| `labelSource` | string | no | Runtime friendly-name source path for the current value; takes precedence over `options` value labels. |
| `editTitleKey` | string | no | i18n key for the edit modal title; supports `{{label}}` / `{{field}}` interpolation. |
| `editLabelKey` | string | no | i18n key for the input label inside the editor; falls back to `labelKey` when not declared. |
| `options` | array | no | Static option list, each item is `{ value, labelKey }`, up to 32. |
| `optionSource` | string | no | Runtime option source path; merged with `options` (runtime wins). |
| `range` | object | no | Numeric range `{ min, max, step? }`, used with the `modal-range` editor. |
| `format` | string | no | Field value format; see the format section. |
| `visibleWhen` | object | no | Visibility condition `{ path, eq?, ne? }`; see the visibleWhen section. |
| `switch` | object | no | Switch toggle declaration; see the switch section. Only used with the `inline-toggle` editor. |

### editor type enum

`editor` determines how the host renders the field. All valid values:

| editor | Purpose | Notes |
|--------|---------|-------|
| `inline-toggle` | Switch toggle | Renders a switch directly in the control area, reading the `field.switch` declaration. |
| `inline-segmented` | Segmented control | Renders segmented buttons directly in the control area. |
| `inline-value` | Read-only value | Displays a formatted value directly in the control area. |
| `inline-action` | Action button | Renders a button in the control area; click executes `field.mutation` + `field.params`. |
| `modal-select` | Modal select | Opens a modal to select from `field.options`/`field.optionSource`. |
| `modal-color` | Modal color | Opens a modal to pick a color. |
| `modal-range` | Modal range | Opens a modal to adjust a range value; requires `field.range`. |
| `modal-number` | Modal number | Opens a modal to input a number. |
| `modal-dpi-stage` | DPI stage editor | Used for DPI stage editing (driven by `stageLayout`). |
| `modal-gradient` | Gradient editor | Used for GradientStops rendering. |
| `static-readonly` | Static read-only | Displays a value only, not editable. |

### switch

`switch` declares switch toggle behavior and is only used with the `inline-toggle` editor. When the field is turned off, the host writes `offValue` to `mutation`; when turned back on, the host reads the restore value from `restoreField` and writes it.

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `source` | string | yes | Switch state read path. |
| `offValue` | string \| number \| boolean \| null | yes | Value written when turned off. |
| `restoreField` | string | no | Field id to read the restore value from when turned back on. |

The host determines switch state: `readPath(switch.source) !== switch.offValue` means on.

### zones

`zones` is used by the `LightingZone` capability to declare lighting subregions (up to 8). Each zone is a collection of related fields:

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `id` | string | yes | Unique zone identifier matching `^[a-z][a-z0-9-]*$`, max 32 chars. |
| `labelKey` | string | yes | i18n key for the zone label. |
| `fields` | array | yes | Field list for this zone; same structure as top-level `fields`, up to 32. |
| `visibleWhen` | object | no | Zone-level visibility condition. The host filters out zones that do not match. |

### stageLayout

`stageLayout` is used by the `DpiStages` capability to declare DPI stage layout and mutations:

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `dotsSource` | string | yes | Read path for the stage dots array (e.g. `state.dpiStages`). |
| `selectMutation` | string \| string[] | yes | Mutation to switch the active stage. |
| `setMutation` | string \| string[] | yes | Mutation to modify a stage's DPI value. |
| `valueSource` | string | yes | Read path for the stage value. |
| `colorSource` | string | no | Read path for the stage color. |
| `range` | object | yes | DPI value range `{ min, max, step? }`. |
| `selectParam` | string | no | Parameter name for the stage switch mutation; defaults to `value`. |
| `stageParam` | string | no | Stage parameter name for the stage write mutation; defaults to `stage`. |
| `valueParam` | string | no | Value parameter name for the stage write mutation; defaults to `value`. |

### statusDisplay

`statusDisplay` declares status bar display, mapping field values to the dashboard status strip:

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `valueSource` | string | yes | Status value read path. |
| `valueFormat` | string | no | Value format; same values as `field.format`. |
| `valueOptions` | array | no | Option mapping, each item is `{ value, labelKey }`, mapping raw values to readable labels. |
| `onClickField` | string | no | Field id to jump to when the status strip is clicked. |

### stateMapping

`stateMapping` is a map from UI state fields to snapshot read paths. Keys become field names under `DeviceState.state`, and values are dotted read paths on the `DeviceSnapshot` (e.g. `capabilities.settings.wirelessSleepValue`). The host aggregates mappings from all capabilities and normalizes `state.*` values from the snapshot.

```json
"stateMapping": {
  "pollingRate": "pollingRateHz",
  "supportedPollingRates": "supportedPollingRatesHz"
}
```

### accentSource

`accentSource` declares the snapshot read path for the host's global accent color. Devices with both mouse and receiver lighting should explicitly point to the mouse lighting color to avoid the receiver color or capability ordering affecting the theme:

```json
"accentSource": "capabilities.mouseLighting.color"
```

When not declared, the host falls back to the first lighting color for legacy plugin compatibility, then to the current DPI stage color. New plugins should not rely on this fallback order.

### visibleWhen

`visibleWhen` declares the visibility condition for a field or zone:

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `path` | string | yes | Read path within the snapshot. |
| `eq` | any | no | Visible when `readPath(path) === eq`. |
| `ne` | any | no | Visible when `readPath(path) !== ne`. |

Evaluation rules: if `eq` is present, compare equality; otherwise if `ne` is present, compare inequality; if neither is present, visible as long as `readPath(path) != null`. Fields without `visibleWhen` are always visible.

### optionSource

`optionSource` declares the runtime option source path. The host reads the option array via `readPath` and merges it with `field.options` (runtime wins, deduplicated, capped at 8). Runtime options can be `{ value, labelKey }` objects or raw values (in which case the host tries to match `labelKey` in `options`, falling back to `String(value)`).

### labelSource

`labelSource` describes the runtime friendly name of the current value, while `labelKey` always describes the field title. Value label resolution priority is: `labelSource` (returns `String(value)` when non-null) > `labelKey` of the matching item in `options` for the current `field.source` value > the formatted raw value via `format`. This keeps rendering stable as "Lighting effect / Neon" instead of mistaking "Neon" for the field title.

### format

`format` declares how the field value is formatted:

| format | Description |
|--------|-------------|
| `sleep` | Sleep time formatting (seconds/minutes conversion). |
| `percent` | Percentage. |
| `hertz` | Frequency (Hz). |
| `connection` | Connection type. |
| `color` | Color value. |
| `default` | No formatting; displays the raw value. |

## Legacy metadata fields removed

After the declarative UI refactor, the following legacy metadata fields have all been removed and are incompatible with old plugins. Plugins must be rewritten against the new schema:

`effectOptions`, `receiverLightingOptions`, `lightingRole`, `switchSource`, `bindings`, `mutation`, `mutations`, `section`, `status`, `source`, `param`, `summary`, `options`, `range`, `format`, `unit`, `actionLabel`, `params`, `label`

These fields were previously declared directly at the capability top level or in a flat structure under `metadata`. They have now migrated to declarative fields (`fields`/`switch`/`zones`/`stageLayout`/`statusDisplay`/`stateMapping`/`visibleWhen`/`optionSource`/`labelSource`/`format`). When upgrading legacy plugins, the flat fields must be rewritten as field declarations within the `fields` array.

Additionally, the `Info` control type has been removed. The original `Info` control was used for pure informational display; it is now replaced by the `ReadOnlyValue` control with the `static-readonly` editor.

## Complete examples

The following examples reference the capability declarations in `src/mock.ts` and cover common scenarios.

### battery/status read-only capability

Read-only battery display, using `ReadOnlyValue` + `static-readonly` editor, reading from the device's top-level `battery` field and formatting as a percentage:

```json
{
  "id": "battery",
  "control": "ReadOnlyValue",
  "labelKey": "capability.battery",
  "readOnly": true,
  "placements": [{ "region": "hero", "order": 10, "span": 1, "icon": "battery" }],
  "metadata": {
    "fields": [
      {
        "id": "value",
        "source": "battery",
        "editor": "static-readonly",
        "format": "percent",
        "labelKey": "capability.battery"
      }
    ],
    "stateMapping": {
      "battery": "batteryPercent",
      "charging": "charging"
    }
  }
}
```

### polling-rate select capability

Polling rate selection, using `Select` + `modal-select` editor, with options dynamically read from the device runtime via `optionSource`, formatted as Hz:

```json
{
  "id": "polling-rate",
  "control": "Select",
  "labelKey": "capability.polling-rate",
  "readOnly": false,
  "placements": [{ "region": "control", "group": "polling", "order": 20, "span": 1, "icon": "wave" }],
  "metadata": {
    "fields": [
      {
        "id": "value",
        "source": "state.pollingRate",
        "mutation": "set-polling-rate",
        "param": "value",
        "editor": "modal-select",
        "optionSource": "state.supportedPollingRates",
        "format": "hertz",
        "labelKey": "capability.polling-rate"
      }
    ],
    "stateMapping": {
      "pollingRate": "pollingRateHz",
      "supportedPollingRates": "supportedPollingRatesHz"
    }
  }
}
```

### dpi stage capability

DPI stage editing, using the `DpiStages` control + `stageLayout` to declare stage layout, active mutation, set-value mutation, value/color sources, and range:

```json
{
  "id": "dpi",
  "control": "DpiStages",
  "labelKey": "capability.dpi",
  "readOnly": false,
  "placements": [{ "region": "control", "group": "performance", "order": 10, "span": 1, "icon": "gauge" }],
  "metadata": {
    "stageLayout": {
      "dotsSource": "state.dpiStages",
      "selectMutation": "set-active-dpi-stage",
      "setMutation": "set-dpi-stage-value",
      "valueSource": "state.dpiStages",
      "colorSource": "state.dpiStages",
      "range": { "min": 100, "max": 32000, "step": 50 }
    },
    "stateMapping": {
      "dpiStages": "dpiStages"
    }
  }
}
```

### lighting multi-zone capability

Multi-zone lighting editing, using the `LightingZone` control + `zones` to declare mouse and receiver zones. Each zone contains a switch (`inline-toggle` + `switch`), effect select (`modal-select` + `options`/`labelSource`), speed/brightness (`modal-range` + `range`), and color (`modal-color`). Both zones and fields use `visibleWhen` to control visibility:

```json
{
  "id": "lighting",
  "control": "LightingZone",
  "labelKey": "capability.lighting",
  "readOnly": false,
  "placements": [
    { "region": "control", "group": "lighting", "order": 30, "span": 1, "icon": "lightbulb" },
    { "region": "status", "order": 30, "span": 1, "icon": "lightbulb" }
  ],
  "metadata": {
    "zones": [
      {
        "id": "mouse",
        "labelKey": "lighting.mouse",
        "fields": [
          {
            "id": "status",
            "source": "state.mouseLightEffect",
            "mutation": "set-mouse-lighting",
            "param": "effect",
            "editor": "inline-toggle",
            "switch": { "source": "state.mouseLightEffect", "offValue": 0, "restoreField": "effect" },
            "labelKey": "dashboard.status"
          },
          {
            "id": "effect",
            "source": "state.mouseLightEffect",
            "mutation": "set-mouse-lighting",
            "param": "effect",
            "editor": "modal-select",
            "labelKey": "receiverLighting.field.effect",
            "labelSource": "capabilities.mouseLighting.effectName",
            "options": [
              { "value": 0, "labelKey": "lighting.effect.off" },
              { "value": 1, "labelKey": "lighting.effect.fixed" },
              { "value": 2, "labelKey": "lighting.effect.breathing" },
              { "value": 3, "labelKey": "lighting.effect.neon" },
              { "value": 4, "labelKey": "lighting.effect.rainbow" }
            ],
            "visibleWhen": { "path": "state.mouseLightEffect", "ne": null }
          },
          {
            "id": "speed",
            "source": "state.mouseLightSpeed",
            "mutation": "set-mouse-lighting",
            "param": "speed",
            "editor": "modal-range",
            "labelKey": "capability.field.speed",
            "range": { "min": 0, "max": 10, "step": 1 },
            "visibleWhen": { "path": "state.mouseLightEffect", "ne": null }
          },
          {
            "id": "brightness",
            "source": "state.mouseLightBrightness",
            "mutation": "set-mouse-lighting",
            "param": "brightness",
            "editor": "modal-range",
            "labelKey": "capability.field.brightness",
            "range": { "min": 0, "max": 100, "step": 1 },
            "visibleWhen": { "path": "state.mouseLightEffect", "ne": null }
          },
          {
            "id": "color",
            "source": "state.mouseLightColor",
            "mutation": "set-mouse-lighting",
            "param": "color",
            "editor": "modal-color",
            "labelKey": "capability.field.color",
            "visibleWhen": { "path": "state.mouseLightEffect", "ne": null }
          }
        ]
      },
      {
        "id": "receiver",
        "labelKey": "lighting.receiver",
        "visibleWhen": { "path": "capabilities.receiverLighting", "ne": null },
        "fields": [
          {
            "id": "status",
            "source": "state.receiverLightEffect",
            "mutation": "set-receiver-lighting",
            "param": "effect",
            "editor": "inline-toggle",
            "switch": { "source": "state.receiverLightEffect", "offValue": 0, "restoreField": "effect" },
            "labelKey": "dashboard.status"
          },
          {
            "id": "effect",
            "source": "state.receiverLightEffect",
            "mutation": "set-receiver-lighting",
            "param": "effect",
            "editor": "modal-select",
            "labelKey": "receiverLighting.field.effect",
            "labelSource": "capabilities.receiverLighting.effectName",
            "options": [
              { "value": 0, "labelKey": "lighting.effect.off" },
              { "value": 1, "labelKey": "lighting.effect.fixed" },
              { "value": 2, "labelKey": "lighting.effect.breathing" },
              { "value": 3, "labelKey": "lighting.effect.neon" },
              { "value": 4, "labelKey": "lighting.effect.rainbow" }
            ],
            "visibleWhen": { "path": "state.receiverLightEffect", "ne": null }
          },
          {
            "id": "speed",
            "source": "state.receiverLightSpeed",
            "mutation": "set-receiver-lighting",
            "param": "speed",
            "editor": "modal-range",
            "labelKey": "receiverLighting.field.speed",
            "range": { "min": 0, "max": 10, "step": 1 },
            "visibleWhen": { "path": "state.receiverLightEffect", "ne": null }
          },
          {
            "id": "brightness",
            "source": "state.receiverLightBrightness",
            "mutation": "set-receiver-lighting",
            "param": "brightness",
            "editor": "modal-range",
            "labelKey": "receiverLighting.field.brightness",
            "range": { "min": 0, "max": 100, "step": 1 },
            "visibleWhen": { "path": "state.receiverLightEffect", "ne": null }
          },
          {
            "id": "color",
            "source": "state.receiverLightColor",
            "mutation": "set-receiver-lighting",
            "param": "color",
            "editor": "modal-color",
            "labelKey": "receiverLighting.field.color",
            "visibleWhen": { "path": "state.receiverLightEffect", "ne": null }
          }
        ]
      }
    ],
    "stateMapping": {
      "mouseLightEnabled": "capabilities.settings.mouseLightEnabled",
      "mouseLightColor": "capabilities.mouseLighting.color",
      "mouseLightEndColor": "capabilities.settings.mouseLightEndColor",
      "mouseLightEffect": "capabilities.mouseLighting.effect",
      "mouseLightSpeed": "capabilities.mouseLighting.speed",
      "mouseLightBrightness": "capabilities.mouseLighting.brightness",
      "receiverLightEnabled": "capabilities.receiverLighting.enabled",
      "receiverLightEffect": "capabilities.receiverLighting.effect",
      "receiverLightSpeed": "capabilities.receiverLighting.speed",
      "receiverLightBrightness": "capabilities.receiverLighting.brightness",
      "receiverLightColor": "capabilities.receiverLighting.color"
    }
  }
}
```

### sleep-time conditionally visible capability

Sleep time editing can use the `Number` control + `modal-number` editor, with `range` to constrain the value, `format: "sleep"` for formatting, and `editTitleKey` / `editLabelKey` to keep the modal title and input label consistent. `statusDisplay` shows the current value in the status bar and supports click-to-edit; `stateMapping` maps snapshot fields to device capabilities paths:

```json
{
  "id": "sleep-time",
  "control": "Number",
  "labelKey": "capability.sleep-time",
  "readOnly": false,
  "placements": [{ "region": "status", "order": 10, "span": 1, "icon": "timer" }],
  "metadata": {
    "fields": [
      {
        "id": "value",
        "source": "state.wirelessSleepValue",
        "mutation": "set-sleep",
        "param": "value",
        "editor": "modal-number",
        "format": "sleep",
        "range": { "min": 0, "max": 1800, "step": 30 },
        "labelKey": "sleep.wireless",
        "editTitleKey": "dashboard.setSleepTitle",
        "editLabelKey": "dashboard.timeoutSeconds",
        "visibleWhen": { "path": "connection", "ne": "usb" }
      }
    ],
    "statusDisplay": {
      "valueSource": "state.wirelessSleepValue",
      "valueFormat": "sleep",
      "onClickField": "value"
    },
    "stateMapping": {
      "wirelessSleepValue": "capabilities.settings.wirelessSleepValue"
    }
  }
}
```

In the example above, `visibleWhen` declares that the field is only visible when the connection is not USB, reflecting the semantics that sleep time only matters for wireless/Bluetooth connections. `statusDisplay.onClickField` points to the `value` field id; clicking the status strip jumps to that field's editor.

```bash
cargo xtask plugin new mira.example ./example
cargo xtask plugin validate ./example
cargo xtask plugin test ./example
cargo xtask plugin pack ./example --output example.mira-plugin
cargo xtask plugin inspect example.mira-plugin
```

Production signing requires an externally protected key and a release review. The CLI intentionally refuses `sign` when no configured signing provider exists.
