<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 插件适配能力增强路线图

围绕「让插件更智能、更可靠地适配设备差异」的三批功能规划。所有功能严格区分三层边界：

| 层 | 载体 | 职责 | 禁止 |
|---|---|---|---|
| 界面 (Host) | Mira 主程序 + mira-plugin-runtime | 设备发现、插件加载、UI 渲染、mutation 调度、信任校验、缓存 | 不写协议命令、不硬编码型号差异 |
| 鼠标插件 | `.mira-plugin`（品牌/协议级） | devices.json 匹配、capabilities 声明、protocol/workflows | 不渲染 UI、不感知具体型号 |
| 型号插件 | `models/<model>/`（未来） | per-model 覆盖 capabilities/workflows、固件门槛 | 不定义协议基础、不碰 UI |

## 三种边界模式

- **模式 A · Host-only**：插件不参与，纯主程序内部治理。
- **模式 B · 插件声明 + Host 执行**：插件提供声明/工作流，host 消费。
- **模式 C · 型号覆盖**：`models/<model>/` 生效，依赖合并引擎。

## 依赖关系

```
#2 型号覆盖加载 ← #1 能力动态协商（覆盖后需重新协商能力真相）
#3 #4 连接/固件分支 ← #2（型号可特化条件）
#5 写事务 ← 依赖 #7 写入排队（事务期间持锁）
#11 配置导出 ← 依赖 #1（导出字段对齐能力真相源）
```

## 总体进度

- 第一批（#6 #7 #9 #10）：✅ 已完成（#9 为协议包解析缓存，HID 探测结果缓存仍属后续优化）
- 第二批（#1 #2）：✅ 已完成
- 第三批（#3 #4 #5 #8 #11 #12）：✅ 已完成（schema + runtime 调度/执行路径均已接入）

---

## 第一批 · Host-only（无 schema 变更，低风险）

均为模式 A，不触碰插件包格式，零兼容风险。

### #9 协议包解析缓存
- **边界**：纯 Host。缓存 key = `plugin_id::model`。
- **现状**：`read_device_once` / `device_mutate_blocking` 会重复解析同一个插件包里的 protocol JSON；HID feature 探测仍随 workflow 执行。
- **实施**：`SessionState` 增加 `cached_packages: Mutex<HashMap<String, Arc<ProtocolPackage>>>`。`get_or_parse_package` 按 `plugin_id::model` 缓存解析结果，避免重复 JSON 解析；插件加载时 `invalidate_package_cache` 整体清空。
- **状态**：✅ 已完成。`Arc<ProtocolPackage>` 共享缓存，cache key 包含 model 以支持型号覆盖；不宣称缓存 HID 往返探测结果。

### #7 写入冲突排队
- **边界**：纯 Host。`begin_device_write` 当前直接拒绝并发写入。
- **现状**：`write_in_progress: Mutex<bool>`，并发 `device_mutate` 直接 `Err("A device write is still in progress")`。
- **实施**：改为 `Condvar` 排队等待——后到的写入等待前一个完成，而非立即失败。保留 30s 超时。
- **状态**：✅ 已完成。`SessionState.write_cond: Condvar` 配合 `write_in_progress` 实现排队，使用 std::sync 无新依赖。

### #6 签名降级只读
- **边界**：纯 Host 安全策略。
- **现状**：`read_device_once` 不检查签名即可读取；`device_mutate` 检查 `signature_verified && writes_enabled`。核心只读降级已成立。
- **实施**：补充 `readonly` 标记传入 `DeviceSnapshot`，UI 据此明确显示「未信任插件 · 只读模式」，而非静默隐藏写入控件。
- **状态**：✅ 已完成。`DeviceSnapshot.readonly` 下沉到前端，`App.tsx` 通过 `.readonly-notice` 显示「未信任插件 · 只读模式」。

### #10 多设备并行
- **边界**：纯 Host。插件无感（每设备独立 runtime 实例）。
- **现状**：`read_device_once` 只取 `matched.first()`；`SessionState.last_snapshot` 是单个 `Option<DeviceSnapshot>`；UI 单设备假设。
- **实施**：`last_snapshot` 改为 `Mutex<BTreeMap<String, DeviceSnapshot>>`（按 HID 路径索引）；`read_device_once` 遍历所有 matched 设备；`primary_snapshot` 优先选择非只读且有可写 mutation 的验证设备；`device_snapshots` 命令返回全部设备。
- **状态**：✅ 已完成（后端多设备遍历落地，`read_device_once` 遍历所有 matched 设备并逐个读取；`device_mutate_blocking` 用单条 `store_snapshot` 更新避免竞态；前端单设备 API 向后兼容）。

---

## 第二批 · 架构债（跨层，核心地基）

### #1 能力动态协商
- **边界**：模式 B。插件提供探测 workflow，Host 取交集渲染。
- **现状**：[plugin-sdk.md](plugin-sdk.md) 第 8 行「host does not select controls by vendor or model」。能力是插件级静态声明，靠 workflow `skipIfZero` 兜底。
- **实施**：`Capability` 增加 `probe: Option<CapabilityProbe>`（引用 workflow 输出的 `{output, field}`）；`plugin_capabilities` 函数接收 `outputs`，当 probe 引用字段值为 0 时 `available=false`；`PluginCapability.available` 字段下沉到 mira-core 与前端类型。
- **状态**：✅ 已完成。`App.tsx` 的 `compatibilityCapabilities` 按 `available !== false` 过滤。

### #2 型号覆盖加载
- **边界**：模式 C。`models/<model>/` 覆盖源，runtime 合并引擎。
- **现状**：`models/` 目录已预留（白名单已放行），但无合并逻辑。
- **实施**：`ProtocolPackage::from_files_with_model` 读取 `models/<model>/protocol/*.json` 与父插件 deep merge；`deep_merge_json` 实现 Object+Object 递归合并、其他类型覆盖替换；`MatchedDevice.model` 在 `evidence == "hardware-verified"` 且单型号时自动填充；`get_or_parse_package` 缓存键含 model。
- **状态**：✅ 已完成。合并语义：Object+Object 递归合并，其他类型 overlay 替换。

---

## 第三批 · 插件 schema 扩展

### #3 连接类型能力分支
- **边界**：模式 B+C。插件声明 `connections` 条件，型号可特化。
- **实施**：`Capability.connections: Option<Vec<String>>` 声明能力可见的连接类型（"usb"/"receiver"/"bluetooth"）；`plugin_capabilities` 接收 `connection` 参数，不在列表中则 `available=false`；`PluginCapability.connections` 下沉到 mira-core 与前端。
- **状态**：✅ 已完成。runtime 过滤逻辑已实现（`connection_available` 检查），并优先使用 workflow 实际回报的连接类型。

### #4 固件版本门槛
- **边界**：模式 B+C。插件声明 `minFirmware`，型号可特化。
- **实施**：`Capability.min_firmware: Option<String>` 声明能力所需最低固件版本（semver）；`PluginCapability.min_firmware` 下沉到 mira-core 与前端；`plugin_capabilities` 从 workflow outputs 中查找可解析固件 semver，低于门槛或无法确认时标记 `available=false`。
- **状态**：✅ 已完成。固件门槛参与 runtime 能力过滤。

### #5 写事务与回滚
- **边界**：模式 B。事务语义在 workflow DSL，回滚在 runtime。
- **实施**：`WorkflowsFile.transactions: HashMap<String, TransactionDefinition>` 声明事务边界；`TransactionDefinition` 含 `mutations`/`snapshot_workflow`/`rollback_workflow`/`timeout_ms`。
- **状态**：✅ 已完成。`mutate` 会识别包含目标 mutation 的 transaction，先执行 `snapshot_workflow`，mutation 失败后执行 `rollback_workflow`，并继承事务级 timeout。

### #8 超时统一治理
- **边界**：模式 B。默认值在 `transports.json`，Host 设硬上限。
- **实施**：`TransportDefinition` 三个变体均增加 `timeout_ms: Option<u64>`；`MutationDefinition.timeout_ms: Option<u64>` 覆盖 transport 级别；`TransactionDefinition.timeout_ms` 事务级超时。
- **状态**：✅ 已完成。`timeout_ms` 已进入 `Session` deadline；transport、mutation、transaction timeout 均受 30s 上限约束，delay/poll/input read 路径会检查剩余时间。

### #11 配置导入/导出
- **边界**：模式 B。插件声明可导出字段，Host 读写文件。
- **实施**：`PluginManifest.exportable_fields: Vec<ExportableField>` 声明可导出字段白名单；`ExportableField` 含 `id`/`export_key`/`kind`。
- **状态**：✅ 已完成。Host 提供 `device_config_export` / `device_config_import` 命令，按 `exportableFields` 白名单读写配置文件，导入时映射到插件声明 mutation。接入 `tauri-plugin-dialog`，设置页通过系统文件选择器指定导入/导出路径。

### #12 插件间依赖复用
- **边界**：模式 B。插件 manifest 声明 `dependsOn`，Host 解析。
- **实施**：`PluginManifest.depends_on: Vec<PluginDependency>` 声明插件依赖；`PluginDependency` 含 `plugin_id`/`version`/`reuse_transport`。
- **状态**：✅ 已完成。Host 解析 `dependsOn`，校验可选 semver 版本要求；`reuseTransport=true` 时 runtime 合并依赖插件的 `protocol/transports.json`，主插件同名 transport 优先。

---

## 后续增强项（非阻塞）

以下属于体验或覆盖面增强，不再阻塞本路线图完成：

1. **#9 HID 探测结果缓存**：当前完成的是协议包解析缓存；如未来需要减少 HID 往返，可在 workflow output 层增加短 TTL 缓存。
2. **#11 UI 文件选择器**：当前 Host 命令读写 app config 下的固定 `device-config.json`；未来可接入系统文件选择器做用户指定路径导入/导出。
3. **#5 事务可观测性**：当前 rollback 失败会回传错误；未来可在 UI/日志中展示 transaction id、snapshot/rollback workflow 名称。
