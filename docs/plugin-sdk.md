<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin SDK

插件声明设备匹配、拓扑、能力（capabilities）、Host 侧控制的 UI 放置提示、字段、协议工作流、本地化标签、狭窄的 Linux 权限元数据，以及离线 Fixtures。插件不能提供 CSS、HTML、JavaScript、原生代码、脚本、网络访问或文件系统访问。

## 声明式控件

Host 渲染 `plugin.json.capabilities`；它不会按厂商或型号选择控件。内置的复杂渲染器覆盖 `DpiStages` 和 `LightingZone`。通用渲染器支持 `Segmented`、`Select`、`Toggle`、`Slider`、`Number`、`Color`、`GradientStops`、`ReadOnlyValue` 和 `Action`。

Capability `metadata` 只能包含由 Host 解释的声明式数据。每个 capability 通过 `metadata` 下的声明式字段（`fields`、`zones`、`stageLayout`、`statusDisplay`、`stateMapping`、`visibleWhen`）描述其可编辑项、选项来源、可见性条件与运行时状态映射。Host 按声明驱动渲染，不读取任何厂商或型号相关的硬编码逻辑。

每个 capability 还应声明一个或多个 `placements`：

```json
"placements": [
  { "region": "control", "group": "performance", "order": 10, "span": 1, "icon": "gauge" },
  { "region": "status", "order": 20, "span": 2, "icon": "gauge" }
]
```

- `region`：`hero`、`control`、`status` 或 `details`。
- `group`：相同值的 capability 共享一个 control 标签页。
- `order`：在该 region 内的升序排列。
- `span`：为 manifest 兼容性保留；等行仪表盘界面会将可见条目归一化为相同宽度。
- `icon`：由 Host 控制的 token：`battery`、`gauge`、`wave`、`lightbulb`、
  `timer`、`profile`、`info` 或 `settings`。

Host 保持页面各 section 稳定，并将每个可见条目以等宽分布在一整行内。仪表盘 control group 和 status 条目在每个 region 内最多 6 个；多出的声明会被拒绝并由 Host 防御性隐藏。插件无法注入组件或样式。

Metadata 永不提供可执行代码或 CSS。仅当签名插件受信任、连接的设备暴露了所声明的 mutation、且运行时校验其输入 schema 时，控件才可写。

## 声明式字段

声明式字段是 capability `metadata` 的核心。插件通过这些字段声明 UI 结构、数据来源、编辑方式与可见性条件，Host 读取声明后驱动渲染，无需任何厂商或型号相关的硬编码逻辑。

### fields

`fields` 是 capability 顶级字段，声明该 capability 的子字段列表（最多 32 个）。每个 field 描述一个可编辑或只读的设备状态项，含以下属性：

| 属性 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | 是 | 字段唯一标识，匹配 `^[a-z][a-z0-9-]*$`，最长 64 字符。 |
| `source` | string | 是 | 在设备状态中的点分读取路径，如 `state.pollingRate`、`battery`。 |
| `editor` | string | 是 | 编辑器类型，决定 UI 渲染方式，见下方枚举表。 |
| `mutation` | string \| string[] | 否 | 写入该字段时调用的 mutation id，或按优先级排列的候选数组。Host 取设备 `writableMutations` 中首个支持的候选。 |
| `param` | string | 否 | mutation 的单一输入参数名。 |
| `params` | object | 否 | mutation 的复合输入参数，键值对形式。 |
| `labelKey` | string | 否 | 字段标签的 i18n key。 |
| `labelSource` | string | 否 | 运行时标签来源路径，优先于 `options` 匹配与 `labelKey`。 |
| `options` | array | 否 | 静态选项列表，每项为 `{ value, labelKey }`，最多 32 个。 |
| `optionSource` | string | 否 | 设备运行时选项来源路径，与 `options` 合并（运行时优先）。 |
| `range` | object | 否 | 数值范围 `{ min, max, step? }`，配合 `modal-range` 编辑器使用。 |
| `format` | string | 否 | 字段值格式化方式，见 format 小节。 |
| `visibleWhen` | object | 否 | 可见性条件 `{ path, eq?, ne? }`，见 visibleWhen 小节。 |
| `switch` | object | 否 | 开关切换声明，见 switch 小节。仅配合 `inline-toggle` 编辑器使用。 |

### editor 类型枚举

`editor` 决定 Host 如何渲染字段。所有合法取值如下：

| editor | 用途 | 说明 |
|--------|------|------|
| `inline-toggle` | 开关切换 | 在控件区直接渲染开关，读 `field.switch` 声明。 |
| `inline-segmented` | 分段选择 | 在控件区直接渲染分段按钮。 |
| `inline-value` | 只读值显示 | 在控件区直接显示格式化值。 |
| `inline-action` | 动作按钮 | 在控件区渲染按钮，点击执行 `field.mutation` + `field.params`。 |
| `modal-select` | 弹窗选择 | 点击打开弹窗，从 `field.options`/`field.optionSource` 选择。 |
| `modal-color` | 弹窗颜色 | 点击打开弹窗选择颜色。 |
| `modal-range` | 弹窗范围 | 点击打开弹窗调整范围值，需配合 `field.range`。 |
| `modal-number` | 弹窗数字 | 点击打开弹窗输入数字。 |
| `modal-dpi-stage` | DPI 档位编辑 | 用于 DPI 档位编辑（由 `stageLayout` 驱动）。 |
| `modal-gradient` | 渐变色编辑 | 用于 GradientStops 控件渲染。 |
| `static-readonly` | 静态只读 | 仅显示值，不可编辑。 |

### switch

`switch` 声明开关切换行为，仅配合 `inline-toggle` 编辑器使用。当字段被关闭时，Host 向 `mutation` 写入 `offValue`；当字段被重新开启时，Host 从 `restoreField` 读取恢复值并写入。

| 属性 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `source` | string | 是 | 开关状态读取路径。 |
| `offValue` | string \| number \| boolean \| null | 是 | 关闭时写入的值。 |
| `restoreField` | string | 否 | 重新开启时读取恢复值的字段 id。 |

Host 判定开关状态：`readPath(switch.source) !== switch.offValue` 即为开启。

### zones

`zones` 用于 `LightingZone` capability，声明灯光子区域列表（最多 8 个）。每个区域是一组相关字段的集合：

| 属性 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | 是 | 区域唯一标识，匹配 `^[a-z][a-z0-9-]*$`，最长 32 字符。 |
| `labelKey` | string | 是 | 区域标签的 i18n key。 |
| `fields` | array | 是 | 该区域的字段列表，结构同顶级 `fields`，最多 32 个。 |
| `visibleWhen` | object | 否 | 区域级可见性条件。Host 过滤不满足条件的区域。 |

### stageLayout

`stageLayout` 用于 `DpiStages` capability，声明 DPI 档位布局与 mutation：

| 属性 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `dotsSource` | string | 是 | 档位点数组读取路径（如 `state.dpiStages`）。 |
| `selectMutation` | string \| string[] | 是 | 切换当前激活档位的 mutation。 |
| `setMutation` | string \| string[] | 是 | 修改某档位 DPI 值的 mutation。 |
| `valueSource` | string | 是 | 档位值读取路径。 |
| `colorSource` | string | 否 | 档位颜色读取路径。 |
| `range` | object | 是 | DPI 值范围 `{ min, max, step? }`。 |
| `selectParam` | string | 否 | 切换档位 mutation 的参数名，默认 `value`。 |
| `stageParam` | string | 否 | 写入档位 mutation 的档位参数名，默认 `stage`。 |
| `valueParam` | string | 否 | 写入档位 mutation 的数值参数名，默认 `value`。 |

### statusDisplay

`statusDisplay` 声明状态栏显示，将字段值映射到仪表盘状态条：

| 属性 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `valueSource` | string | 是 | 状态值读取路径。 |
| `valueFormat` | string | 否 | 值格式化方式，取值同 `field.format`。 |
| `valueOptions` | array | 否 | 选项映射，每项为 `{ value, labelKey }`，将原始值映射为可读标签。 |
| `onClickField` | string | 否 | 点击状态条时跳转到的字段 id。 |

### stateMapping

`stateMapping` 是 UI 状态字段到 snapshot 读取路径的映射表。键会成为 `DeviceState.state` 下的字段名，值是 `DeviceSnapshot` 上的点分读取路径（如 `capabilities.settings.wirelessSleepValue`）。Host 聚合所有 capability 的映射后，从 snapshot 归一化出 `state.*` 值。

```json
"stateMapping": {
  "pollingRate": "pollingRateHz",
  "supportedPollingRates": "supportedPollingRatesHz"
}
```

### visibleWhen

`visibleWhen` 声明字段或区域的可见性条件：

| 属性 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | snapshot 中的读取路径。 |
| `eq` | any | 否 | 当 `readPath(path) === eq` 时可见。 |
| `ne` | any | 否 | 当 `readPath(path) !== ne` 时可见。 |

求值规则：有 `eq` 时比较相等；否则有 `ne` 时比较不等；两者都没有时只要 `readPath(path) != null` 即可见。未声明 `visibleWhen` 的字段始终可见。

### optionSource

`optionSource` 声明设备运行时选项来源路径。Host 通过 `readPath` 读取该路径得到选项数组，与 `field.options` 合并（运行时优先，去重，限制在 8 个内）。运行时选项可以是 `{ value, labelKey }` 对象，也可以是原始值（此时 Host 尝试匹配 `options` 中的 `labelKey`，回退为 `String(value)`）。

### labelSource

`labelSource` 声明运行时标签来源路径。标签解析优先级为：`labelSource`（非空时返回 `String(value)`）> `options` 中匹配项的 `labelKey` > `field.labelKey`。

### format

`format` 声明字段值的格式化方式：

| format | 说明 |
|--------|------|
| `sleep` | 休眠时间格式化（秒/分钟换算）。 |
| `percent` | 百分比。 |
| `hertz` | 频率（Hz）。 |
| `connection` | 连接类型。 |
| `color` | 颜色值。 |
| `default` | 不做格式化，直接显示原始值。 |

## 旧 metadata 字段已移除

声明式 UI 重构后，以下旧 metadata 字段已全部移除，不兼容旧插件。插件必须按新 schema 重写：

`effectOptions`、`receiverLightingOptions`、`lightingRole`、`switchSource`、`bindings`、`mutation`、`mutations`、`section`、`status`、`source`、`param`、`summary`、`options`、`range`、`format`、`unit`、`actionLabel`、`params`、`label`

这些字段原先直接声明在 capability 顶级或 `metadata` 下的扁平结构中，现已迁移到声明式字段（`fields`/`switch`/`zones`/`stageLayout`/`statusDisplay`/`stateMapping`/`visibleWhen`/`optionSource`/`labelSource`/`format`）。旧插件升级时需将扁平字段改写为 `fields` 数组中的 field 声明。

同时，`Info` control 类型已移除。原 `Info` 控件用于纯信息展示，现由 `ReadOnlyValue` control 配合 `static-readonly` editor 替代。

## 完整示例

以下示例参考 `src/mock.ts` 中的 capability 声明，覆盖常见场景。

### battery/status 只读 capability

电量只读展示，使用 `ReadOnlyValue` + `static-readonly` editor，从设备顶层 `battery` 字段读取并以百分比格式化：

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

### polling-rate 选择 capability

回报率选择，使用 `Select` + `modal-select` editor，选项通过 `optionSource` 从设备运行时动态读取，并以 Hz 格式化：

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

### dpi 档位 capability

DPI 分档编辑，使用 `DpiStages` control + `stageLayout` 声明档位布局、激活 mutation、设值 mutation、值/颜色来源与范围：

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

### lighting 多区域 capability

灯光多区域编辑，使用 `LightingZone` control + `zones` 声明 mouse 与 receiver 两个区域。每个区域含开关（`inline-toggle` + `switch`）、效果选择（`modal-select` + `options`/`labelSource`）、速度/亮度（`modal-range` + `range`）、颜色（`modal-color`）。区域与字段均通过 `visibleWhen` 控制可见性：

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

### sleep-time 条件可见 capability

休眠时间编辑，使用 `Number` control + `modal-range` editor，配合 `range` 限定范围、`format: "sleep"` 格式化。通过 `statusDisplay` 在状态栏展示当前值并支持点击进入编辑，通过 `stateMapping` 将 snapshot 字段映射到设备 capabilities 路径：

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
        "editor": "modal-range",
        "format": "sleep",
        "range": { "min": 0, "max": 1800, "step": 30 },
        "labelKey": "capability.sleep-time",
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

上例中 `visibleWhen` 声明该字段仅在非 USB 连接时可见，体现休眠时间只对无线/蓝牙连接有意义的语义。`statusDisplay.onClickField` 指向 `value` 字段 id，点击状态条即跳转到该字段编辑。

```bash
cargo xtask plugin new mira.example ./example
cargo xtask plugin validate ./example
cargo xtask plugin test ./example
cargo xtask plugin pack ./example --output example.mira-plugin
cargo xtask plugin inspect example.mira-plugin
```

生产签名需要外部受保护的密钥和一次 release review。CLI 在没有配置签名提供方时会故意拒绝 `sign`。
