<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin SDK

插件声明设备匹配、拓扑、能力（capabilities）、Host 侧控制的 UI 放置提示、字段、协议工作流、本地化标签、狭窄的 Linux 权限元数据，以及离线 Fixtures。插件不能提供 CSS、HTML、JavaScript、原生代码、脚本、网络访问或文件系统访问。

## 声明式控件

Host 渲染 `plugin.json.capabilities`；它不会按厂商或型号选择控件。内置的复杂渲染器覆盖 `DpiStages` 和 `LightingZone`。通用渲染器支持 `Segmented`、`Select`、`Toggle`、`Slider`、`Number`、`Color`、`ReadOnlyValue` 和 `Action`。

Capability `metadata` 只能包含由 Host 解释的数据：

- `label`：本地化的回退标签（fallback label）。
- `section`：`control` 或 `status`。
- `status`：同时在仪表盘状态条中暴露当前值。
- `source`：在归一化的 `DeviceSnapshot` 状态中的点分路径。
- `mutation` 和 `param`：稳定的 mutation id 及其单一输入名。
- `options`：最多 8 个有界的 `{ value, label }` 条目，用于 segmented/select 控件。
- `summary`：最多 4 个有界的 `{ label, source, unit?, format?, options? }`
  条目，渲染在 Host 拥有的控件骨架（skeleton）中。声明的条目数量
  控制等宽 summary 列；插件无法改变该骨架。
- `bindings`：有序的 `{ when: { path, eq }, source, mutation, param, label }`
  变体，用于依赖连接状态的控件，例如无线/蓝牙休眠。
- `min`、`max`、`step`、`unit`、`format`、`description`、`actionLabel`、`params`。

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

Host 保持页面各 section 稳定，并将每个可见条目以等宽分布在一整行内。仪表盘 control group 和 status 条目在每个 region 内最多 6 个；多出的声明会被拒绝并由 Host 防御性隐藏。插件无法注入组件或样式。没有 `placements` 的旧版 manifest 使用传统的 `metadata.section` 兼容适配器。

Metadata 永不提供可执行代码或 CSS。仅当签名插件受信任、连接的设备暴露了所声明的 mutation、且运行时校验其输入 schema 时，控件才可写。

```bash
cargo xtask plugin new mira.example ./example
cargo xtask plugin validate ./example
cargo xtask plugin test ./example
cargo xtask plugin pack ./example --output example.mira-plugin
cargo xtask plugin inspect example.mira-plugin
```

生产签名需要外部受保护的密钥和一次 release review。CLI 在没有配置签名提供方时会故意拒绝 `sign`。
