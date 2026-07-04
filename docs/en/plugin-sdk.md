<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin SDK

Plugins declare device matching, topology, capabilities, host-controlled UI placement hints, fields, protocol workflows, localized labels, narrow Linux permission metadata, and offline Fixtures. They cannot provide CSS, HTML, JavaScript, native code, scripts, network access, or filesystem access.

## Declarative controls

The host renders `plugin.json.capabilities`; it does not select controls by
vendor or model. Built-in complex renderers cover `DpiStages` and
`LightingZone`. The generic renderer supports `Segmented`, `Select`, `Toggle`,
`Slider`, `Number`, `Color`, `ReadOnlyValue`, and `Action`.

Capability `metadata` may contain only data interpreted by the host:

- `label`: localized fallback label.
- `section`: `control` or `status`.
- `status`: also expose the current value in the dashboard status strip.
- `source`: dotted path within the normalized `DeviceSnapshot` state.
- `mutation` and `param`: stable mutation id and its single input name.
- `options`: up to eight bounded `{ value, label }` entries for segmented/select controls.
- `summary`: up to four bounded `{ label, source, unit?, format?, options? }`
  entries rendered inside a host-owned control skeleton. The declared item count
  controls the equal-width summary columns; plugins cannot change the skeleton.
- `bindings`: ordered `{ when: { path, eq }, source, mutation, param, label }`
  variants for connection-dependent controls such as wireless/Bluetooth sleep.
- `min`, `max`, `step`, `unit`, `format`, `description`, `actionLabel`, `params`.

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
- `span`: retained for manifest compatibility; equal-row dashboard surfaces
  normalize visible items to the same width.
- `icon`: a host-controlled token: `battery`, `gauge`, `wave`, `lightbulb`,
  `timer`, `profile`, `info`, or `settings`.

The host keeps page sections stable and distributes every visible item at equal
width across one full row. Dashboard control groups and status items are capped
at six per region; extra declarations are rejected and defensively hidden by
the host. Plugins cannot inject components or styles. Older manifests without
`placements` use the legacy `metadata.section` compatibility adapter.

Metadata never supplies executable code or CSS. A control is writable
only when the signed plugin is trusted, the connected device exposes the
declared mutation, and the runtime validates its input schema.

```bash
cargo xtask plugin new mira.example ./example
cargo xtask plugin validate ./example
cargo xtask plugin test ./example
cargo xtask plugin pack ./example --output example.mira-plugin
cargo xtask plugin inspect example.mira-plugin
```

Production signing requires an externally protected key and a release review. The CLI intentionally refuses `sign` when no configured signing provider exists.
