# 为 Mira Mouse 实现雷蛇鼠标全面读取与可反馈支持

## 任务目标

请同时检查并修改以下两个仓库的当前代码，而不是仅根据本文中的路径、示例或历史设计机械实现：

- 主应用：`hello-yunshu/mira-mouse`
- 插件仓库：`hello-yunshu/mira-mouse-plugins`

目标是为 Mira 建立一套可维护的雷蛇鼠标支持：

1. 依据可信公开协议资料，尽可能读取设备能够安全返回的参数。
2. 常用参数进入 Mira 已有的主界面、状态区和配置控件。
3. 其他可读参数完整保留在“全部读数”中，不能因为暂时没有专用 UI 就丢弃。
4. 单个可选读取失败时，不应导致整台设备不可用。
5. 不持续高频读取静态、昂贵或大体积数据。
6. 未经 Mira 团队实机验证的型号清楚标记为实验性支持，不伪造硬件证据。
7. 用户遇到问题后，可以直接复制当前设备的脱敏读数与诊断日志反馈。
8. 所有实现必须保持 Mira 的声明式插件边界，不能把雷蛇型号知识散落到 React 或 Tauri 业务代码中。

最终交付必须是可运行代码、插件包、测试、文档和 CI 校验，而不只是分析报告或研究草稿。

---

# 一、开始修改前必须完成的仓库审计

先阅读两个仓库当前默认分支，不要假定本文提到的接口仍然完全一致。

至少检查：

## `mira-mouse`

- `crates/mira-core`
- `crates/mira-plugin-api`
- `crates/mira-plugin-runtime`
- `crates/mira-plugin-cli`
- `crates/mira-testkit`
- `src-tauri/src/lib.rs`
- `src-tauri/src/logging`
- `src/App.tsx`
- `src/Settings.tsx`
- `src/pluginAdapter.ts`
- `src/types.ts`
- `src/logs`
- `src/locales`
- `schemas`
- 现有设备刷新计划、投影读取、快照缓存、设备退避、插件签名和诊断导出实现

## `mira-mouse-plugins`

- 根目录 README
- `docs/plugin-sdk.md`
- `docs/plugin-testing.md`
- `docs/plugin-review-checklist.md`
- `docs/protocol-reserve-inventory.md`
- 插件校验脚本
- registry 生成逻辑
- `plugins/amaster`
- `plugins/logitech-hidpp`
- `plugins/example-mock`
- `plugins/razer-viper`
- model overlay、fixtures、protocol inventory 和打包流程

审计完成后先在实施说明中列出：

1. 可以直接复用的现有能力；
2. 当前阻碍全面读取的缺口；
3. 需要修改的通用 Host 契约；
4. 只需要修改插件数据的部分；
5. 不应该修改的稳定模块。

不要重复实现仓库已经具备的日志、脱敏、投影读取、配置导入导出、插件签名、缓存或 UI 适配能力。

---

# 二、必须遵守的现有架构

保持以下边界：

> 协议归插件，界面归主应用。

插件包只能包含声明式内容，例如：

- `plugin.json`
- `devices.json`
- `capabilities.json`
- `protocol/commands.json`
- `protocol/parsers.json`
- `protocol/transports.json`
- `protocol/workflows.json`
- model overlays
- locales
- fixtures
- 文档和许可证

插件中不得加入：

- 原生动态库；
- Python、JavaScript、TypeScript 或 Shell 脚本；
- HTML/CSS；
- WASM；
- 任意可执行程序；
- 厂商 SDK 二进制文件。

Host 负责：

- HID handle；
- 超时；
- 重试；
- 取消；
- 缓存；
- 读取调度；
- 日志；
- 脱敏；
- 权限；
- UI；
- 写入队列；
- 回读验证；
- 回滚；
- 插件签名和更新。

不要在 React、设置页或 Tauri 命令中写：

```text
if vendor == Razer
if pid == ...
if pluginId == mira.razer...
```

型号和协议差异必须由：

- `devices.json`
- family
- model ID
- workflow guard
- parser 输出
- capability probe
- connection
- firmware gate
- model overlay

共同表达。

---


# 三、子 Agent 协作与并行执行

如果当前 AI IDE、编码代理或任务运行环境支持子 Agent，必须使用子 Agent 分工完成本任务。子 Agent 不是独立随意开发者，而是由主 Agent 调度、使用统一证据和交付格式的受控执行单元。

如果运行环境不支持真正的子 Agent，则由主 Agent按相同角色顺序执行，并保留相同的中间产物、审查门和冲突处理规则；不能因为缺少子 Agent 接口而跳过研究或验证步骤。

## 3.1 主 Agent 职责

主 Agent 是唯一的最终集成负责人，负责：

- 读取两个 Mira 仓库的总体结构；
- 建立任务依赖图；
- 决定哪些工作可并行；
- 给每个子 Agent 提供明确输入、范围、禁止事项和输出格式；
- 维护共享事实表；
- 解决不同来源和不同子 Agent 之间的冲突；
- 审核所有 Host Schema 变更；
- 防止不同子 Agent 同时修改同一文件；
- 合并代码；
- 运行全量测试；
- 编写最终交付说明；
- 对“是否真正解决问题”负责。

主 Agent 不能只汇总子 Agent 的结论。所有影响协议、安全、隐私、兼容性和 Schema 的关键结论，都必须回到源码、fixture 或公开来源复核。

## 3.2 推荐子 Agent 划分

至少建立以下角色。可根据实际环境合并相近角色，但不能省略其职责。

### Agent A：Mira 主仓库审计

只读审计 `mira-mouse`，重点输出：

- 当前插件 API；
- runtime DSL；
- HID transports；
- workflow 执行方式；
- model overlay；
- package cache；
- DeviceReading 和 DeviceSnapshot；
- ReadPlan 与 projection；
- “全部读数”数据流；
- 日志、脱敏和诊断导出；
- 现有测试和 Schema；
- 本任务真正需要修改的通用缺口；
- 不应修改的稳定能力。

禁止：

- 在审计阶段直接大范围改代码；
- 根据雷蛇需求先入为主地设计专用接口；
- 重复实现仓库已经存在的能力。

输出：

```text
working/host-audit.md
working/host-schema-gaps.json
```

### Agent B：插件仓库审计

只读审计 `mira-mouse-plugins`，重点输出：

- 插件目录契约；
- devices、capabilities、protocol 和 fixtures 结构；
- AMaster 的全面读取方式；
- Logitech HID++ 的 feature discovery 和可选能力处理；
- model overlay 实际用法；
- protocol reserve；
- CI 校验；
- 打包和 registry；
- 当前 `razer-viper` 占位需要如何迁移；
- 需要新增的覆盖检查。

输出：

```text
working/plugin-repo-audit.md
working/plugin-contract-gaps.json
```

### Agent C：现代雷蛇核心协议研究

主要研究：

- OpenRazer；
- razerqdhid；
- MacRazer；
- HIDAPI。

负责：

- 90-byte report；
- report ID；
- checksum；
- status；
- transaction ID；
- command class/id；
- BUSY；
- HID interface；
- timing；
- 电量；
- DPI；
- polling；
- firmware；
- device mode；
- sleep；
- Profile 基础信息。

输出必须逐项引用具体仓库、文件、符号或文档章节。

输出：

```text
working/razer-core-protocol-evidence.md
working/razer-core-commands.json
```

### Agent D：雷蛇高级配置研究

主要研究：

- razerqdhid；
- OpenRazer；
- OpenRGB；
- 相关 Issue/PR。

负责：

- 滚轮；
- RGB 灯区；
- Profile metadata；
- 板载存储；
- 按键映射；
- HyperShift；
- 宏 metadata；
- memory layout；
- checksum/CRC；
- 哪些读取适合 core、inventory 或按需读取；
- 哪些操作属于 destructive 或 blocked。

输出：

```text
working/razer-advanced-evidence.md
working/razer-read-risk-classification.json
```

### Agent E：设备与连接矩阵研究

主要研究：

- OpenRazer 设备表；
- MacRazer 已测型号；
- RazerBatteryTaskbar；
- razerqdhid；
- OpenRGB；
- 公开 Issue/PR。

负责为每个候选型号确认：

- model；
- VID/PID；
- wired PID；
- receiver PID；
- Bluetooth 是否仅标准输入；
- usage page；
- usage；
- interface；
- transaction ID；
- report index；
- timing；
- protocol family；
- 可读能力；
- 来源数量；
- 证据等级；
- 是否允许真实匹配；
- 是否只能 detection-only；
- 是否进入 reserve。

输出：

```text
working/razer-device-matrix.json
working/razer-device-matrix.md
```

禁止根据产品营销规格猜测协议能力。

### Agent F：Host 通用能力设计与实现

在 Agent A、B、C 输出完成后开始。

负责：

- explicit model routing；
- model-aware package cache；
- optional step failure policy；
- read statuses；
- inventory workflow 契约；
- inventory cache；
- structured detail value；
- plugin locale 详情标签；
- 设备定向诊断复制；
- 通用 response policy；
- checksum 扩展；
- interface matching 扩展。

要求：

- 只增加通用能力；
- 不加入 Razer 品牌分支；
- 保持旧插件兼容；
- 先写 Schema 和测试，再接业务代码；
- 提交清晰的迁移说明。

### Agent G：雷蛇声明式插件实现

在核心 Host 契约稳定后开始。

负责：

- 新协议族插件目录；
- devices.json；
- plugin.json；
- capabilities.json；
- transports；
- commands；
- parsers；
- workflows；
- model overlays；
- locales；
- fixtures；
- protocol reserve；
- read coverage。

禁止修改 React 或 Tauri 品牌逻辑。

### Agent H：全部读数与交互实现

负责：

- 结构化对象和数组展示；
- 分组排序；
- group/field plugin locale；
- read status；
- inventory 进度；
- 重读全部；
- 重试单组；
- 复制字段；
- 复制全部读数；
- 复制当前设备诊断；
- 保持 Mira 现有设计风格；
- 可访问性和大数据量性能。

此 Agent 不决定雷蛇协议含义，只消费 Host 和插件声明。

### Agent I：日志、隐私与反馈闭环

负责：

- 通用结构化协议事件；
- correlation ID；
- 当前设备过滤；
- 临时协议诊断模式；
- command-aware payload masking；
- 诊断自动到期；
- diagnostics bundle；
- GitHub Issue Form；
- 默认不包含 HID payload；
- 宏和按键内容的敏感数据保护。

### Agent J：测试与故障注入

独立于实现 Agent，负责：

- runtime unit tests；
- model routing；
- optional failure；
- inventory partial merge；
- cache isolation；
- parser fixtures；
- UI structured value tests；
- log privacy tests；
- malformed reports；
- wrong interface；
- BUSY；
- timeout；
- checksum；
- disconnect；
- 多设备；
- 回归 AMaster、Logitech 和 mock。

测试 Agent 不应只补“让当前实现通过”的测试，还要主动寻找遗漏和错误假设。

### Agent K：许可证与来源审查

负责：

- 确认每个公开仓库当前许可证；
- 识别代码继承关系；
- 区分协议事实、文档引用、代码改写和代码复制；
- 检查 GPL/AGPL/MIT/ISC 等兼容性；
- 更新 NOTICE 和 THIRD_PARTY_NOTICES；
- 检查源码注释和文档归属；
- 防止把同一上游代码误记成多个独立来源。

### Agent L：独立反向审查

在其他实现完成后，只读审查整个变更。

审查问题：

- 是否存在 Razer 硬编码泄漏进 Host/UI；
- 是否把未经确认的型号错误归为同一 family；
- 是否有 GET 没有 fixture；
- 是否有 output 没进入全部读数；
- 是否有 optional failure 仍会终止全部读取；
- 是否高频读取了静态或大体积数据；
- 是否误记录敏感 payload；
- 是否存在 model cache 串用；
- 是否削弱正式写入 evidence 规则；
- 是否对外声称硬件已验证；
- 是否遗漏已有插件回归；
- 是否有更简单、通用的解决方法。

输出：

```text
working/final-adversarial-review.md
```

主 Agent 必须逐条处理，不得仅标记“已知问题”。

## 3.3 子 Agent 调度顺序

推荐依赖顺序：

```text
A 主仓库审计 ─┐
               ├─> F Host 通用能力 ─┬─> H 全部读数 UI
B 插件仓库审计 ┘                     ├─> I 日志隐私
                                     └─> G 雷蛇插件
C 核心协议研究 ────────────────────────┘
D 高级协议研究 ────────────────────────> G
E 设备矩阵研究 ────────────────────────> G

F + G + H + I
      ↓
J 测试与故障注入
      ↓
K 许可证与来源审查
      ↓
L 独立反向审查
      ↓
主 Agent 最终集成和全量验收
```

A、B、C、D、E 可以并行。

F 在 A、B 和 C 的最低必要结论完成前不能开始定稿。

G 可以先建立只读插件骨架，但在 F 的 Schema 稳定前不得自行创建不兼容字段。

J、K 可以在开发中提前介入，但必须在最终代码上重新执行。

## 3.4 并发限制

默认最多同时运行 4 至 6 个子 Agent，避免：

- 重复下载和阅读相同仓库；
- 同一文件并发修改；
- 证据台账出现多个不一致版本；
- 上下文过度分散；
- 大量低质量推测结果淹没关键事实。

研究 Agent 可以并行，写代码 Agent 必须按文件所有权协调。

## 3.5 文件所有权

主 Agent 在开始写代码前生成：

```text
working/file-ownership.md
```

至少包含：

| 文件/目录 | 所有 Agent | 允许只读 Agent | 合并负责人 |
|---|---|---|---|

规则：

- 同一时间只有一个 Agent可以写同一个文件；
- 公共 Schema 由 Host Agent 所有；
- 插件 JSON 由插件 Agent 所有；
- React 详情组件由 UI Agent 所有；
- 日志模型由日志 Agent 所有；
- 共享类型改动必须先由主 Agent批准；
- 子 Agent 不得通过复制文件绕过所有权；
- 发生冲突时暂停后续实现，先统一契约。

支持 Git worktree 的环境中，可以为代码 Agent 建立独立 worktree；最终仍由主 Agent检查差异并合并。

## 3.6 共享中间产物

所有 Agent 使用同一组结构化事实，不允许各自维护互不相容的私有结论。

至少维护：

```text
working/
├── task-graph.md
├── file-ownership.md
├── host-audit.md
├── plugin-repo-audit.md
├── host-schema-gaps.json
├── plugin-contract-gaps.json
├── source-evidence-ledger.json
├── razer-device-matrix.json
├── razer-command-matrix.json
├── razer-read-risk-classification.json
├── unresolved-conflicts.md
├── test-matrix.md
└── final-adversarial-review.md
```

这些是开发工作文件，不必全部发布进最终仓库。需要长期维护的内容应整理后进入正式 docs。

## 3.7 统一证据记录格式

每条协议事实至少包含：

```json
{
  "factId": "razer.basilisk-v3.get-dpi",
  "domain": "dpi",
  "model": "basilisk-v3",
  "connection": "usb",
  "vendorId": "0x1532",
  "productId": "0x0099",
  "commandClass": "0x04",
  "commandId": "0x85",
  "transactionId": "0x1F",
  "requestLayout": "reference",
  "responseLayout": "reference",
  "sources": [
    {
      "repository": "openrazer/openrazer",
      "file": "driver/...",
      "symbol": "...",
      "commit": "...",
      "license": "GPL-2.0-or-later"
    }
  ],
  "confidence": "source-confirmed",
  "independentSourceCount": 1,
  "miraCommand": "get-dpi",
  "miraParser": "dpi",
  "fixture": "tests/fixtures/...",
  "status": "enabled",
  "notes": ""
}
```

要求：

- commit 使用实际研究时的 SHA；
- 不允许只记录仓库主页；
- 同源移植项目不能虚增 independentSourceCount；
- 不确定字段写 `unknown`，不能猜测；
- 冲突必须进入 `unresolved-conflicts.md`。

## 3.8 子 Agent 输出契约

每个子 Agent结束时必须返回：

```text
任务范围
检查过的仓库和文件
确认事实
不确定事实
发现的冲突
建议修改
实际修改文件
测试结果
风险
交接给哪个 Agent
```

不得只回复“已完成”“看起来可行”或没有证据的摘要。

研究 Agent 不得直接把推测写入正式插件。

实现 Agent 不得把未经过测试的代码标记为完成。

测试 Agent 不得修改产品逻辑来掩盖失败，除非主 Agent明确重新分派。

## 3.9 冲突处理

不同 Agent 或不同公开仓库给出冲突结论时，按以下顺序处理：

1. 当前 Mira 仓库实际契约；
2. 同型号、同 PID、同连接方式的实机 fixture；
3. 同型号的原始上游实现；
4. 多个真正独立来源的一致结论；
5. 活跃项目优先于无人维护的历史项目；
6. 明确源码优先于 README、Issue 转述和博客；
7. 精确型号优先于协议族推断；
8. 无法解决时进入 reserve 或标记 unknown。

不得采用多数投票。

不得因为某个 Agent表达更确定就提高证据等级。

## 3.10 子 Agent 禁止事项

任何子 Agent都不得：

- 擅自扩大 VID-only 匹配；
- 将推测标为 source-confirmed；
- 将未实机验证型号标为 hardware-verified；
- 删除正式写入 evidence 门槛；
- 开放 destructive 命令；
- 把完整宏或序列号写入日志；
- 在 Host/UI 添加型号分支；
- 直接复制不兼容许可证代码而不说明；
- 修改无关模块；
- 跳过现有测试；
- 自动提交、推送或发布，除非主 Agent和用户明确要求；
- 用另一个 Agent的摘要替代原始来源复核；
- 通过生成大量占位代码假装覆盖完整。

## 3.11 子 Agent 完成门

子 Agent任务只有在满足以下条件时才能交回主 Agent：

- 输出文件存在且格式正确；
- 所有关键结论带来源；
- 不确定内容明确标注；
- 修改没有越过文件所有权；
- 局部测试通过；
- 没有新增未解释的 warnings；
- 对下游 Agent提供了明确输入；
- 没有把 reserve 误列为 enabled。

主 Agent最终仍需运行整个仓库的全量验收，子 Agent局部通过不能替代最终通过。

---

# 四、先修复当前阻碍多型号实验插件的 model overlay 选择

当前代码需要重点审计：

- `DeviceDescriptor`
- `MatchedDevice`
- `enumerate_matched_devices`
- `ProtocolPackage::from_files_with_model`
- model overlay 的加载与缓存键

不要再依赖“只有一个 hardware-verified 型号时才能推导 model”的逻辑。

为 `devices.json` 的每个设备描述符增加稳定、显式的型号字段，例如：

```json
{
  "family": "razer-modern-1f-usb",
  "model": "basilisk-v3",
  "vendorId": 5426,
  "productId": 153,
  "usagePage": 65280,
  "usage": 1,
  "connection": "usb",
  "evidence": "source-confirmed"
}
```

要求：

1. `model` 与 evidence 解耦。
2. 未经硬件验证的精确 PID 也可以选择自己的 model overlay。
3. `hardwareVerifiedModels` 只表示证据，不再承担运行时型号路由。
4. model 名必须通过路径安全校验。
5. package cache 的键必须包含插件版本和 model，不能让不同型号错误共用已合并包。
6. 同一 PID 存在有线、接收器或多个 HID interface 时，必须能选择对应 family/model/connection。
7. 增加旧插件兼容逻辑：未声明 model 时保持当前行为。
8. 增加单元测试，覆盖：
   - 两个未验证型号选择不同 overlay；
   - 同一插件不同 model 不串包；
   - evidence 变化不影响型号路由；
   - 非法 model 路径被拒绝；
   - 旧 devices.json 仍能加载。

如果当前 Schema 中已经有等价字段，复用现有字段，不重复增加。

---

# 五、全面读取不能等同于高频轮询

Mira 当前已经区分 Presence、Quick、BatteryOnly 和 Full 等读取计划，并支持工作流投影。保留这些优化。

为全面读取建立两层或三层读取模型：

## 4.1 核心读取

用于连接后初始化、主界面和必要刷新。

只读取：

- 设备身份；
- 当前电量和充电；
- 当前 DPI；
- DPI stages；
- 当前回报率；
- 当前 Profile；
- 当前灯光状态；
- 当前滚轮模式；
- 其他主界面直接依赖的动态参数。

核心读取必须快速、有界，不能因为读取宏、板载内存或全部按键映射而变慢。

## 4.2 详细读取 / Inventory

用于“全部读数”。

读取所有已经有可信公开协议依据、且属于安全 GET 的参数，包括静态、低频和高级参数。

建议采用通用 Host 契约，而不是雷蛇专用命令。例如：

```json
{
  "runtime": {
    "inventory": {
      "workflows": [
        "razer-modern-1f-inventory-device",
        "razer-modern-1f-inventory-performance",
        "razer-modern-1f-inventory-lighting",
        "razer-modern-1f-inventory-profiles",
        "razer-modern-1f-inventory-buttons"
      ],
      "refresh": "on-open",
      "cache": "connection",
      "maxAgeSeconds": 300
    }
  }
}
```

字段名称可以根据当前 Schema 调整，但语义必须保持通用。

行为：

1. 第一次打开“全部读数”时执行详细读取。
2. 显示读取进度，已有快照先显示，不让弹窗空白。
3. 完成后把结果合并进当前设备 `capabilities`。
4. 同一连接会话内缓存静态结果。
5. 提供“重新读取全部参数”按钮。
6. 设备断开、插件更新、model 变化时清空缓存。
7. mutation 后仅失效相关动态 group，不要清空所有静态信息。
8. 详细读取不能自动每秒执行。
9. 宏内容、整块 Flash 和大体积内存不得在普通打开弹窗时自动完整转储。
10. 允许分领域 workflow，避免为了全面读取直接无上限提高单 workflow 命令数。

如果当前 Full 读取已经只在连接初始化和明确刷新时执行，可以让核心 Full 保持现状，再新增 inventory workflow；不要破坏现有 Quick projection。

## 4.3 按需深度读取

针对可能很慢或数据量大的内容：

- 单个 Profile 内容；
- 单个宏内容；
- 完整按键层；
- 板载内存区块；
- 接收器配对详情；
- 单灯珠矩阵状态。

只在用户展开对应 group 或点击“读取更多”时执行。

---

# 六、让可选命令失败时保留其他读数

当前工作流 step 需要审计是否会在任意 transport/parser 错误时中断整个 workflow。对于跨大量型号的雷蛇插件，这种行为会导致一个不支持的可选 GET 让整台设备无法读取。

为 workflow step 增加通用、有界的失败策略。示例语义：

```json
{
  "command": "get-scroll-mode",
  "parser": "scroll-mode",
  "output": "scroll",
  "failurePolicy": {
    "action": "continue",
    "statusOutput": "scroll"
  }
}
```

具体 Schema 可以不同，但必须满足：

- 默认仍然是 `abort`，保持旧插件兼容；
- 只有插件明确声明的可选 step 才能继续；
- 必要身份命令、基础 transport 建立和安全前置命令仍然失败即终止；
- 可区分以下状态：
  - success
  - not-supported
  - skipped
  - timeout
  - busy-exhausted
  - invalid-response
  - checksum-error
  - disconnected
  - permission-denied
  - parser-error
- 继续执行时保留已成功 outputs；
- 失败状态进入结构化 `readStatuses`，不要伪装成值为 0；
- `not-supported` 不应显示成红色错误；
- timeout、checksum-error 等应在“全部读数”中显示为“读取失败”，并可重试；
- 失败 status 中不能保存未经脱敏的 HID path；
- 每个 workflow 的最大可忽略错误数、最大总耗时和最大 report 数由 Host 设置硬上限；
- 可选 step 失败不能造成无限重试。

建议在通用读取结果中加入：

```rust
DeviceReading {
    capabilities: BTreeMap<String, Value>,
    read_statuses: BTreeMap<String, ReadStatus>,
    ...
}
```

并贯通：

- runtime
- mira-core DeviceSnapshot
- Tauri serialization
- TypeScript types
- DeviceDetails UI
- diagnostics
- fixtures

不要把错误状态混入普通协议值对象，避免污染现有 `capabilities.<group>.<field>` 路径。

---

# 七、完善“全部读数”通用页面

当前“全部读数”会遍历 `device.capabilities`，这是正确方向。不要为雷蛇新建专用详情页。

但需要完善通用能力。

## 6.1 支持复杂值

当前默认字符串转换不足以正确显示对象和数组。

增加通用 `DetailValue` 渲染器，支持：

- string
- number
- boolean
- null
- RGB 颜色
- 百分比
- Hz
- 时间
- 版本号
- 数字数组
- 字符串数组
- RGB 数组
- byte 数组
- key-value object
- object array
- 嵌套对象

要求：

1. 最大递归深度，例如 3 层。
2. 每组最大可见条目和展开后的硬上限。
3. 超长数组先显示摘要，可展开。
4. byte 数组可在十六进制和十进制之间切换。
5. 颜色显示色块和文本。
6. 对象不能显示成 `[object Object]`。
7. 数组不能只显示成没有语义的逗号字符串。
8. 支持复制单个字段值。
9. 大对象使用等宽字体和折叠区域，但保持当前 Mira 视觉风格。
10. 不允许任意 HTML。

## 6.2 插件自己的详情标签

详情页 group 和 field 标签应按以下顺序解析：

1. 当前插件 locale 中的精确 group/field key；
2. Host 通用 locale；
3. 原始 key 的可读化 fallback。

建议支持：

```text
details.group.device
details.group.performance
details.field.performance.pollingRate
details.field.dpi.dpiX
details.value.deviceMode.normal
```

或者复用当前 `capability.group.*`、`capability.field.*` 命名，但必须先查当前插件 namespace。

不要要求所有雷蛇专有字段都加入 Host 的全局 locale。

## 6.3 正确排序

当前详情排序需要根据 capability 的 `details` placement 以及其字段 source 所指向的实际 capability group 计算。

例如：

```text
capability id = firmware
source = capabilities.firmwareUsb.version
```

应排序 `firmwareUsb` group，而不是只把 `firmware` 当作输出 group。

实现通用映射：

- 检查 details capability 的 fields、zones、summary 中使用的 `capabilities.<group>`；
- 将实际 group 映射到 placement order；
- 未声明 order 的 group 排在后面并按稳定字典序排列；
- 不依赖厂商和 group 名硬编码。

## 6.4 读取状态

每个 group 可以显示：

- 已读取；
- 来自缓存；
- 不支持；
- 因连接方式跳过；
- 因固件门槛跳过；
- 读取失败；
- 等待设备唤醒；
- 上次更新时间。

失败 group 提供“重试此组”，不必重新读取全部参数。

## 6.5 页面操作

在“全部读数”中增加：

- 重新读取全部参数；
- 复制全部读数；
- 复制当前设备诊断日志；
- 打开日志页；
- 报告问题。

复制全部读数时输出脱敏 JSON 或 Markdown，至少包括：

- Mira 版本；
- 插件 ID 和版本；
- model；
- evidence；
- VID/PID；
- connection；
- usage page / usage；
- 成功读数；
- read statuses；
- 更新时间。

不包含：

- 原始序列号；
- HID path；
- 用户目录；
- 机器名；
- token；
- 私钥。

---

# 八、全面读取参数清单

以下是研究范围，不代表所有型号都支持。每一项只有在存在可信命令来源、正确 parser 和型号/连接 guard 时才能启用。

原则：

- 不根据营销规格猜测协议；
- 不因为某个型号支持功能，就向所有雷蛇设备发送同一命令；
- 对每个启用的 GET 建立来源记录；
- 同时保存有意义的 raw value 和 derived value；
- 原始完整 report 仅进入临时协议诊断，不作为普通读数长期保存。

## 7.1 设备与协议身份

尽可能读取：

- 产品名称；
- model ID；
- serial number；
- serial 是否存在；
- 主控固件版本；
- USB 固件版本；
- 无线 SoC 固件版本；
- 接收器固件版本；
- LED 控制器固件版本；
- bootloader 版本，仅限安全 GET；
- device mode；
- protocol type；
- transaction ID 族；
- connection；
- interface number；
- usage page / usage；
- feature report size；
- 接收器配对状态；
- 接收器当前在线状态；
- paired device type；
- capability flags；
- 支持的灯区；
- 支持的 Profile 数量；
- 支持的按键层；
- 板载存储版本；
- 当前设备是否处于 charging/wired/wireless 状态。

普通“全部读数”显示脱敏 serial hash，不显示原始 serial。原始 serial 只允许在本地内存中用于识别，不能进入日志和导出。

## 7.2 电池与电源

尽可能读取：

- battery percentage；
- charging；
- battery present；
- battery health；
- low battery threshold；
- idle/sleep timeout；
- auto power off；
- wireless power state；
- receiver battery（若存在）；
- charging dock 状态（若协议适用）；
- 电池值是否有效；
- 最近一次可信读数；
- 设备休眠导致的暂时不可用状态。

电量必须接入现有：

- `DeviceReading.battery_percent`
- `DeviceReading.batteries`
- 充电状态
- 电量历史
- 无线唤醒恢复
- sticky display

不能把短暂 0% 或 100% 覆盖最近可信值，除非协议证明该值有效。

## 7.3 DPI 与传感器性能

尽可能读取：

- current DPI；
- X DPI；
- Y DPI；
- DPI stage count；
- active stage；
- 每个 stage 的 X/Y DPI；
- stage color；
- stage enabled；
- sensor index；
- DPI min/max/step，若设备能报告；
- supported DPI ranges；
- sensitivity stage behavior；
- DPI button enabled；
- DPI clutch/sensitivity clutch；
- polling rate；
- supported polling rates；
- polling rate raw code；
- high polling capability；
- HyperPolling receiver mode；
- angle snapping；
- motion sync；
- ripple control；
- smoothing；
- lift-off distance；
- asymmetric cut-off；
- landing distance；
- surface calibration status；
- surface profile ID；
- rotation；
- debounce；
- click response time；
- button change time；
- sensor sleep state。

将常用参数规范化到现有主界面字段；所有原始与高级字段进入 `capabilities`。

## 7.4 灯光

按实际型号和灯区读取：

- enabled；
- effect；
- effect raw；
- effect name；
- brightness；
- speed；
- primary color；
- secondary color；
- random color；
- direction；
- wave direction；
- reactive duration；
- starlight behavior；
- per-zone state；
- logo zone；
- wheel zone；
- underglow zone；
- dock/receiver zone；
- direct/temporary mode；
- onboard/persistent mode；
- supported effects；
- matrix rows/columns，仅在安全且数据量有界时；
- current profile lighting linkage。

主界面只显示设备实际报告且插件声明为可用的字段；详细原始参数进入“全部读数”。

## 7.5 滚轮

适用型号尽可能读取：

- tactile/free-spin；
- Smart-Reel；
- automatic switching；
- scroll acceleration；
- scroll resistance；
- scroll stages；
- wheel click behavior；
- wheel tilt；
- wheel LED；
- wheel mode raw；
- supported scroll modes。

无电子滚轮型号必须通过 model capability 或读取结果完全跳过。

## 7.6 Profile 与板载配置

尽可能读取：

- active profile；
- profile count；
- valid profiles；
- profile index；
- profile name；
- profile color；
- profile enabled；
- onboard/software mode；
- profile memory usage；
- memory size；
- used/free bytes；
- sector/chunk size；
- checksum/CRC；
- profile version；
- 当前 Profile 的 DPI、polling、lighting、button layer 关联。

普通打开“全部读数”只读取 metadata 和已知结构，不自动转储整块 Flash。

## 7.7 按键映射

尽可能读取：

- physical button count；
- logical button IDs；
- primary layer；
- HyperShift layer；
- per-button raw mapping；
- decoded action type；
- keyboard key；
- mouse button；
- media key；
- DPI action；
- profile action；
- scroll action；
- macro reference；
- disabled；
- layer shift；
- button mapping CRC；
- 未知 action bytes。

对于未知 action：

- 保留原始 bytes；
- 显示 `unknown`；
- 不能猜测含义；
- 不丢弃未知字段。

## 7.8 宏

默认只读取：

- macro support；
- macro count；
- macro IDs；
- macro names（若可读）；
- macro size；
- macro references；
- macro storage usage；
- macro checksum。

完整宏 payload 只在用户展开单个宏并明确读取时加载，避免：

- 大量 HID 往返；
- 快照过大；
- 日志泄漏键盘内容；
- 普通设备刷新变慢。

宏内容属于敏感数据：

- 不进入默认诊断；
- 不自动复制；
- 不写入日志；
- 导出前明确提示。

## 7.9 接收器

适用型号尽可能读取：

- receiver firmware；
- paired/unpaired；
- mouse online；
- receiver mode；
- receiver LED；
- receiver lighting；
- receiver battery（若协议存在）；
- paired PID/model；
- wireless polling capability；
- HyperPolling capability；
- receiver busy/ready；
- link quality，只有协议确实提供时；
- forwarding status；
- wake/sleep status。

不要把鼠标本体电量误标为接收器电量。

---

# 九、原始值与派生值规则

对于协议字段，尽可能同时保留：

```json
{
  "pollingRaw": 1,
  "pollingRateHz": 1000
}
```

或者：

```json
{
  "effectRaw": 3,
  "effect": "spectrum",
  "effectName": "光谱循环"
}
```

规则：

1. raw 字段有助于反馈和后续修正，但不能替代派生值。
2. 枚举未知时保留 raw，并显示“未知（0xNN）”。
3. bitfield 同时保留原始位图和已确认的布尔字段。
4. parser 不应把未知值强行映射为默认值。
5. 数值范围异常时保留 raw，但 normalized field 不应使用异常值。
6. 版本号同时保留 raw bytes 和格式化版本。
7. byte 数组在普通快照中设总大小上限。
8. 原始完整 request/response 不放进 `capabilities`。

---

# 十、雷蛇协议运行时能力

根据实际仓库现状实现通用能力，不要直接把某个上游项目的类或函数复制进 Host。

至少审计并补齐：

## 9.1 90-byte Feature Report

支持：

- 协议 payload 90 字节；
- HIDAPI buffer 可能为 report ID + 90 字节；
- report ID 0；
- write length/read length；
- 可选剥离 report ID；
- connection/model 级等待时间。

## 9.2 XOR checksum

增加通用、有边界检查的 `xor8`：

```json
{
  "algorithm": "xor8",
  "start": 2,
  "endExclusive": 88,
  "writeOffset": 88
}
```

支持构建请求和验证响应。

## 9.3 响应状态与关联

支持声明：

- status offset；
- success；
- busy；
- failure；
- not-supported；
- timeout；
- transaction ID 关联；
- command class 关联；
- command ID 关联；
- BUSY 时只重读；
- 达到上限后按策略重发；
- model/connection 级 delay；
- 总超时硬上限。

不要把雷蛇 status 逻辑硬编码进 React 或某个具体 command。

## 9.4 HID interface 选择

当前匹配只使用 VID/PID/usage page/usage 时，需要审计是否足以稳定选择雷蛇控制接口。

必要时增加通用可选条件：

- interface number；
- minimum feature report size；
- product string condition；
- serial presence；
- collection usage；
- path-independent descriptor score。

要求：

- 精确 VID/PID；
- 优先 vendor-defined control interface；
- 排除普通 mouse input 和 keyboard interface；
- 无法唯一选择时不发送命令；
- 在 HID 扫描结果中说明候选接口和拒绝原因；
- 条件必须适用于其他插件，而不是命名为 Razer 专用字段。

---

# 十一、雷蛇插件组织方式

不要继续把所有支持放进空的 `mira.razer-viper` 研究占位。

建立协议族插件，例如：

```text
plugins/razer-chroma/
├── plugin.json
├── devices.json
├── capabilities.json
├── locales/
├── protocol/
├── models/
├── tests/fixtures/
├── README.md
└── LICENSE
```

实际 ID 可根据仓库命名规范确定。

## 10.1 family

按真实协议和 transport 差异划分 family，而不是按营销系列粗暴划分。

示例：

- modern transaction 0x1F；
- legacy transaction 0x3F；
- Basilisk V3 特殊 report index；
- receiver forwarding；
- direct wired；
- wireless receiver；
- 特殊高回报率接收器。

不要假设所有 Viper、Basilisk 或 DeathAdder 使用同一协议。

## 10.2 model overlay

每个精确 PID 声明 model。

overlay 只保存真正的差异：

- transaction ID；
- delay；
- report index；
- PID；
- connection；
- DPI 范围；
- polling options；
- 电池；
- 灯区；
- 滚轮；
- Profile；
- 按键数量；
- memory layout；
- 可执行 inventory workflows；
- 禁止命令。

不要复制整套基础协议文件。

## 10.3 首批来源较完整的型号

优先实现公开协议资料较完整的型号，再扩展设备表。至少评估：

- Basilisk V3；
- Basilisk V3 Pro；
- Viper Ultimate；
- Viper V2 Pro；
- Viper V3 HyperSpeed；
- Viper V3 Pro；
- DeathAdder V2 Pro；
- DeathAdder V3 Pro；
- Cobra HyperSpeed；
- Cobra Pro；
- Atheris；
- Orochi V2；
- Naga Pro。

不要因为列表出现在本文中就直接启用。必须确认：

- 精确 PID；
- 连接方式；
- 控制接口；
- transaction ID；
- command 支持；
- 时序；
- 数据布局。

资料不足的型号进入 protocol reserve，不匹配真实设备或只做 detection-only。

---

# 十二、必须逐项研究的公开仓库与使用边界

实现前必须实际阅读以下公开仓库的当前默认分支、相关源码、协议文档、设备表、Issue 和许可证。不能只看 README，也不能仅凭二手文章整理命令。

每个仓库都要在最终实施说明中记录：仓库地址、当前许可证、阅读过的文件或目录、提取的协议事实、Mira 中对应的 command/parser/transport/workflow/fixture/model、没有直接复制的部分，以及仍未解决的不确定性。

## 11.1 OpenRazer

仓库：

```text
https://github.com/openrazer/openrazer
```

主要用途：

- 作为现代和历史雷蛇设备 VID/PID 的主要交叉参考；
- 研究 90-byte Razer report 结构；
- 研究 status、transaction ID、command class、command ID 和 XOR checksum；
- 研究电量、DPI、回报率、灯光、Profile、滚轮和固件等 GET 命令；
- 研究不同 PID 的等待时间、report index、BUSY 重试及有线/接收器差异；
- 研究 receiver forwarding 和无线设备路径；
- 防止把全部 Viper、Basilisk 或 DeathAdder 当成一个协议。

重点检查：

```text
driver/razercommon.h
driver/razercommon.c
driver/razermouse_driver.c
driver/razermouse_driver.h
daemon/
pylib/
```

使用边界：

- Linux kernel module、daemon、sysfs 和 Python 架构不能直接移植成 Mira 插件；
- 只提取可验证的协议事实、设备表和时序；
- 每条命令仍需重新表达为 Mira 声明式 command/parser/transport/workflow；
- 未经目标型号验证的命令只能进入 protocol reserve；
- 对采用或改写的代码片段进行许可证兼容审查并保留归属。

## 11.2 razerqdhid

仓库：

```text
https://github.com/geezmolycos/razerqdhid
```

主要用途：

- 作为 Basilisk V3 有线协议的重点来源；
- 研究 Basilisk V3 / Basilisk V3 Pro 的 HID interface；
- 研究 HIDAPI 的 `send_feature_report`、`get_feature_report`、report ID 0 和 90-byte payload；
- 研究 transaction ID 0x1F；
- 研究 serial、firmware、device mode、DPI、DPI stages、polling 和 scroll mode；
- 研究 onboard profiles、button mappings、macro metadata 和 flash layout；
- 研究 BUSY 后重读、响应关联以及未知按键 bytes 的保留。

重点检查：

```text
docs/basic.md
docs/cmd_basic.md
public/py/basilisk_v3/device.py
public/py/qdrazer/protocol.py
public/py/basilisk_v3/
public/py/qdrazer/
```

使用边界：

- 不能自动推广到全部雷蛇型号；
- Python、Vue 和 WebHID 代码只作为协议与行为证据；
- Profile、宏和 Flash 的破坏性操作默认禁止；
- 普通 Full/inventory 不得自动完整 dump Flash；
- 只有精确 PID、接口、连接和 fixture 对齐后才能启用真实设备匹配。

## 11.3 MacRazer

仓库：

```text
https://github.com/SorcRR/MacRazer
```

主要用途：

- 研究 macOS 下无需 kernel extension 的直接 USB HID 通信；
- 研究 MaxFeatureReportSize、vendor-defined usage page 和控制接口选择；
- 研究 Cobra HyperSpeed、Atheris 等已测试型号；
- 研究 transaction ID 0x1F、31ms 等等待时间及 BUSY 重读；
- 研究无线重连、设备唤醒和电量 0/100 暂态处理；
- 交叉验证 macOS 下 feature report 行为。

重点检查：

```text
RazerReport.swift
MacRazer/
docs/
设备枚举和 HID interface 选择代码
电量、DPI、回报率、灯光读取代码
```

使用边界：

- Swift 代码不能作为 Mira 插件依赖；
- 软件事件层按键重映射不能误称为板载映射；
- macOS 差异应抽象成 Host 通用 HID 能力；
- 不在 Host 中加入 Cobra、Atheris 等型号硬编码。

## 11.4 OpenRGB

仓库：

```text
https://github.com/CalcProgrammer1/OpenRGB
```

主要用途：

- 研究雷蛇 RGB controller；
- 交叉验证 91-byte HID buffer 和 90-byte 协议 payload；
- 研究不同 PID 的 transaction ID；
- 研究 LED ID、zone ID、matrix layout、灯区数量和特殊 RGB 规则；
- 研究灯效、方向、亮度、颜色和矩阵参数。

重点检查：

```text
Controllers/RazerController/
RGBController/
```

使用边界：

- 只作为 RGB 和设备差异补充来源，不是完整鼠标配置协议来源；
- 不用它推断电量、DPI、Profile 或按键协议；
- LED zone 规则须与 OpenRazer、目标型号资料和 fixture 交叉验证；
- 不把 OpenRGB controller 类直接移植到 Mira Host。

## 11.5 RazerBatteryTaskbar

仓库：

```text
https://github.com/Tekk-Know/RazerBatteryTaskbar
```

主要用途：

- 交叉验证无线型号 VID/PID 和 transaction ID；
- 研究 battery request；
- 研究 Windows/WebUSB control transfer 路径；
- 交叉检查 Viper、Basilisk、DeathAdder、Naga 和 Cobra 等型号的电量读取。

使用边界：

- 作为辅助来源，不作为唯一依据；
- 若项目已归档，降低证据权重；
- Electron/USB 实现不能作为插件依赖；
- PID/transaction 表必须与 OpenRazer、MacRazer 或实机日志交叉验证；
- 不能仅凭该仓库开放写入。

## 11.6 librazermacos

仓库：

```text
https://github.com/1kc/librazermacos
```

主要用途：

- 研究历史 macOS OpenRazer 协议移植；
- 研究 IOUSBDeviceInterface control request；
- 研究 SET_REPORT / GET_REPORT、90-byte report 和 checksum；
- 交叉验证旧版 macOS 雷蛇通信。

使用边界：

- 不把动态库作为 Mira 运行时依赖；
- 不复制其完整 C driver 架构；
- 作为历史交叉验证来源，不优先于活跃项目；
- 识别其与 OpenRazer 的来源关系，避免重复或错误归属。

## 11.7 razer-macos

仓库：

```text
https://github.com/1kc/razer-macos
```

主要用途：

- 研究 Electron → native addon → OpenRazer C port 的历史架构；
- 研究设备 JSON、功能声明和支持矩阵；
- 检查 Issue/PR 中的权限、断开、重连、固件和灯光差异。

使用边界：

- 主要作为历史架构和问题库；
- 不采用 Electron/native addon 方案；
- 不根据旧 release 状态声称当前型号稳定；
- 协议事实需与活跃来源交叉验证。

## 11.8 razercfg / mbuesch/razer

仓库：

```text
https://github.com/mbuesch/razer
```

主要用途：

- 研究 Naga、Krait、Copperhead、Lachesis、Diamondback、Taipan、旧 DeathAdder 和 Mamba 等历史型号；
- 辅助划分旧私有协议和现代 90-byte 协议的边界；
- 防止现代 Chroma 命令误匹配旧设备。

使用边界：

- 只用于明确的历史型号；
- 不作为现代 Viper/Basilisk/Cobra 的主要来源；
- 资料不足时保持 detection-only 或不匹配；
- 不扩大为 vendor-only 匹配。

## 11.9 HIDAPI

仓库：

```text
https://github.com/libusb/hidapi
```

主要用途：

- 确认 Mira 当前 HIDAPI 使用方式；
- 确认各平台 feature report buffer、report ID、长度和错误语义；
- 研究 macOS、Windows、Linux 的枚举与接口差异。

使用边界：

- HIDAPI 是通用传输库，不是雷蛇协议来源；
- 不能从 HIDAPI 推断 command、PID、transaction ID 或字段布局；
- 不重复封装 Host 已具备的能力。

## 11.10 libratbag

仓库：

```text
https://github.com/libratbag/libratbag
```

主要用途：

- 研究品牌无关的 Profile、DPI、button、LED 和 polling 能力模型；
- 对照 Mira capability schema 是否缺少通用语义；
- 研究能力协商和前后端分层。

使用边界：

- 不是雷蛇底层通信实现来源；
- 不引入 ratbagd 依赖；
- 不因为 libratbag 的通用抽象而声称支持 Razer。

## 11.11 Polychromatic

仓库：

```text
https://github.com/polychromatic/polychromatic
```

主要用途：

- 研究 OpenRazer 前端的功能组织、设备状态和反馈流程；
- 研究如何呈现 daemon/设备不支持和功能差异。

使用边界：

- 它是 OpenRazer 前端，不是 HID 协议来源；
- 不从 UI 推断命令；
- 不复制其样式，只参考信息架构和反馈体验。

## 11.12 应排除的低相关项目

以下项目不能作为鼠标配置协议的主要来源：

- 只做 xinput、Karabiner、AutoHotkey 等软件重映射的项目；
- 只读取标准 HID mouse input report 的项目；
- Razer Blade 笔记本控制项目；
- Razer Hydra 控制器项目；
- 只调用 Synapse、Registry 或厂商云 API 的项目；
- 无明确许可证的代码；
- 没有设备、固件和连接上下文的十六进制 Gist；
- 营销规格页；
- 没有原始来源的 AI 协议说明。

检索到新的公开仓库时，先判断其是否真的涉及“雷蛇鼠标配置通信”，再加入来源台账。

## 11.13 来源优先级

协议事实优先级：

1. 同型号、同 PID、同连接方式的 Mira 实机 fixture；
2. 同型号多个独立开源实现一致；
3. 活跃项目中的明确源码和设备表；
4. 上游协议文档；
5. 同协议族相邻型号；
6. Issue/PR 中可复现的用户日志；
7. 单一旧项目；
8. 推测。

启用读取至少需要：

- 一个明确源码来源，或两个独立交叉来源；
- 精确设备匹配；
- parser；
- fixture；
- 失败策略；
- 输出展示。

开放写入还需要 Mira 实机回读验证。

## 11.14 禁止“仓库级照搬”

不能把某个公开仓库视为整体真相。必须按协议事实拆解：

```text
设备标识来源
接口选择来源
report layout 来源
checksum 来源
command 来源
response parser 来源
timing 来源
connection 差异来源
capability 来源
```

同一型号可以组合多个来源，但必须在覆盖台账中记录。例如：

```text
Basilisk V3:
- PID/interface：razerqdhid + OpenRazer
- 90-byte report：razerqdhid + OpenRazer
- transaction ID：razerqdhid
- polling/DPI/scroll：razerqdhid
- RGB zone：OpenRGB + OpenRazer
- macOS feature report 行为：MacRazer + HIDAPI
```

最终 Mira 插件应是对多个来源的重新建模和验证，而不是某个上游项目的 JSON 翻译版。

---

# 十三、协议来源与覆盖台账

为雷蛇插件新增可审计的读取覆盖文件，例如：

```text
docs/razer-read-coverage.md
```

每个参数一行：

| Domain | Parameter | Command | Parser | Workflow | Models | Connection | Source | Fixture | UI/Details | Status |
|---|---|---|---|---|---|---|---|---|---|---|

Status：

- enabled
- source-confirmed
- fixture-verified
- reserve
- blocked
- unknown
- destructive

要求：

1. 每个启用 GET 必须有可信来源。
2. 每个启用 GET 必须有 parser。
3. 每个启用 GET 必须在核心或 inventory workflow 中被引用。
4. 每个启用 GET 必须有 fixture。
5. 每个输出字段必须有详情标签或明确 raw fallback。
6. 未引用命令必须登记为 reserve。
7. destructive 命令永远不能混入读取 workflow。
8. CI 自动检查覆盖闭环。
9. 文档必须区分“协议来源确认”和“Mira 实机验证”。

可参考公开实现中的协议事实，但必须记录许可证与来源，例如：

- OpenRazer；
- razerqdhid；
- MacRazer；
- OpenRGB；
- 其他有明确许可证和可追踪实现的来源。

不要依赖闭源 Synapse 二进制代码复制。

---

# 十四、能力展示策略

## 12.1 主界面

只放用户经常使用且已经成功读取的字段：

- battery；
- DPI；
- polling；
- lighting；
- profile；
- scroll mode；
- sleep；
- 其他少量高频设置。

使用现有：

- capability probe；
- connection；
- min firmware；
- `fieldHasReportedValue`；
- stateMapping；
- statusDisplay；
- summary；
- zones；
- placement。

没有读到的字段不出现可编辑控件。

## 12.2 全部读数

显示所有成功 output，包括没有主 UI 的：

- raw；
- derived；
- capability flags；
- firmware components；
- profile metadata；
- button mappings；
- memory metadata；
- receiver status；
- read statuses。

不要为了进入“全部读数”而给每个字段建立假的 dashboard capability。

## 12.3 实验性状态

未经 Mira 团队实机验证时显示：

```text
实验性支持
基于公开协议实现，部分型号或固件可能存在差异
```

不要显示“稳定支持”或“hardware verified”。

正式写入的原有 evidence 规则不要被削弱。

如果产品仍决定开放未经硬件验证的写入，应使用独立、明确授权、精确 PID、mutation 白名单和回读保护的实验通道；不要把 `writesEnabled` 与 evidence 校验直接删除。读取功能不应依赖是否开放实验写入。

---

# 十五、日志与用户反馈必须复用现有日志系统

Mira 已有：

- `LogService`
- buffer
- storage
- redaction
- query
- export
- diagnostics bundle
- frontend emitter
- Debug/Trace 临时诊断会话
- 单条复制
- 筛选复制
- 日志导出

在此基础上扩展，不另建雷蛇专用日志数据库。

## 13.1 稳定结构化事件

新增通用协议事件，例如：

- plugin-read-started
- plugin-read-step-succeeded
- plugin-read-step-skipped
- plugin-read-step-not-supported
- plugin-read-step-failed
- plugin-inventory-completed
- plugin-inventory-partial
- hid-feature-exchange
- hid-busy-retry
- hid-response-mismatch
- hid-checksum-failed

字段使用当前 `FieldValue` 支持的标量类型。建议包括：

- pluginId
- pluginVersion
- family
- model
- deviceKey
- vendorId
- productId
- connection
- usagePage
- usage
- interfaceNumber
- workflow
- command
- parser
- output
- status
- errorKind
- transactionId
- commandClass
- commandId
- attempt
- busyReads
- durationMs
- requestLength
- responseLength
- checksumValid
- correlationValid
- cacheHit
- readPlan
- partial
- successfulOutputs
- failedOutputs

使用 `correlationId` 关联一次设备读取、一次 inventory 和一次 mutation。

## 13.2 HID payload 隐私

当前默认诊断承诺不包含 HID payload。保持这一默认行为。

只有用户明确启动“协议诊断模式”后，才允许临时记录：

- request hex；
- response hex。

要求：

1. 只记录当前选中设备。
2. 只在有时限的 Debug/Trace 会话中记录。
3. 到期后自动关闭。
4. 对 serial、宏内容、按键文本、设备路径等敏感字节做 command-aware masking。
5. 每条 payload 长度有上限。
6. 默认 diagnostics bundle 仍不包含 payload，除非用户在导出时再次明确勾选。
7. UI 明确说明包含的内容。
8. payload 用 `FieldValue::Text` 保存，不扩大日志字段为任意递归 JSON。
9. 宏 payload 永远不自动进入日志。

## 13.3 设备定向复制

增加通用命令或 UI 行为：

```text
复制当前设备诊断
```

它应：

- 以当前 pluginId、model、deviceKey、sessionId 和 correlationId 筛选；
- 包含当前“全部读数”；
- 包含 read statuses；
- 包含最近相关 Warn/Error；
- 可选择包含临时协议诊断；
- 自动脱敏；
- 输出 Markdown 或 JSON；
- 不复制其他设备和本地 AI 的无关日志。

必要时扩展 `LogQuery`，支持：

- correlation ID；
- target prefix；
- 精确结构化字段过滤。

实现必须是通用日志过滤，不是 Razer 字符串搜索。

## 13.4 GitHub 反馈

增加雷蛇设备反馈 Issue Form，预填非敏感信息：

- model；
- VID/PID；
- connection；
- Mira version；
- plugin version；
- failed workflow/command；
- error kind。

完整日志由用户复制粘贴，不塞入 URL。

---

# 十六、读取性能和稳定性

全面读取不能损害后台体验。

要求：

1. 核心 Quick projection 保持最小 step。
2. inventory 不进入每秒 Quick。
3. 静态字段按连接会话缓存。
4. battery 等动态字段使用现有计划。
5. 每设备 HID I/O 继续串行。
6. 多设备之间不能串用 outputs、model overlay 或 handles。
7. 休眠和断开继续使用现有指数退避。
8. inventory 失败不能触发频繁自动重试。
9. inventory 支持取消。
10. 弹窗关闭后可取消尚未开始的低优先级读取。
11. Host 设置：
    - 最大 workflow 数；
    - 最大 step 数；
    - 最大 report 数；
    - 最大总耗时；
    - 最大输出字节；
    - 最大 group 数；
    - 最大字段数；
    - 最大数组长度；
    - 最大嵌套深度。
12. 达到上限时返回 partial 状态，不崩溃、不分配无界内存。
13. 不要简单把现有全局 `MAX_COMMANDS`、`MAX_REPORTS` 调到极大值。
14. 使用多个领域 workflow 和缓存解决规模问题。

---

# 十七、Fixtures 与测试

## 15.1 每个启用读取

必须有：

- 正常请求；
- 正常响应；
- parser 结果；
- normalized result；
- 全部读数展示快照；
- 来源记录。

## 15.2 失败场景

覆盖：

- success；
- busy 后成功；
- busy 耗尽；
- not supported；
- timeout；
- short response；
- extra bytes；
- wrong transaction ID；
- wrong command class；
- wrong command ID；
- checksum error；
- all-zero response；
- report ID 未剥离；
- disconnected；
- permission denied；
- wrong interface；
- unknown enum；
- out-of-range value；
- optional step failed but later step succeeded；
- required step failed and workflow aborted；
- inventory partial result merged correctly；
- cached inventory reused；
- manual refresh invalidated cache。

## 15.3 详情 UI

覆盖：

- nested object；
- numeric array；
- byte array；
- RGB array；
- empty array；
- huge array truncation；
- unknown value；
- plugin locale label；
- host fallback label；
- raw key fallback；
- per-group read status；
- copy single value；
- copy all readings；
- sensitive field masking。

## 15.4 多型号

覆盖：

- model overlay by explicit model；
- same plugin, two models；
- same PID, different connection；
- same model, wired/receiver；
- unverified model overlay；
- cache isolation；
- unsupported capability hidden；
- exact PID required。

## 15.5 日志

覆盖：

- structured fields；
- correlation ID；
- target filtering；
- device filtering；
- default no payload；
- protocol diagnostic includes masked payload；
- timeout auto-disable；
- macro payload excluded；
- clipboard output redacted；
- diagnostics bundle privacy regression。

## 15.6 回归

必须运行并修复：

- Rust tests；
- frontend tests；
- plugin validation；
- plugin fixtures；
- deterministic pack；
- existing AMaster tests；
- existing Logitech HID++ tests；
- mock plugin tests；
- Windows build；
- macOS build；
- Linux build；
- lint；
- typecheck。

---

# 十八、CI 读取覆盖检查

新增一个通用或雷蛇专用 CI 检查，确保“能读的参数不在开发中悄悄丢失”。

检查：

1. commands 中标记为 enabled GET 的命令被 workflow 引用。
2. workflow 中每个 output 有 parser。
3. parser 每个字段有 fixture 覆盖。
4. inventory 中每个 group 有 locale label 或明确 fallback。
5. enabled output 能进入 DeviceReading.capabilities。
6. 详情 UI 能渲染其类型。
7. reserve 命令登记在 protocol inventory。
8. forbidden/destructive 命令不出现在 read workflow。
9. 同一型号的 capability 声明和可执行 workflow 一致。
10. 文档覆盖表与插件文件一致。

输出机器可读 JSON 和 Markdown 摘要。

---

# 十九、交付顺序

按可审查的小步实现，不要一次提交混合所有内容。

## PR 1：通用详情读取基础

- explicit model routing；
- model-aware package cache；
- read status；
- optional workflow step；
- tests。

## PR 2：全部读数 UI

- structured DetailValue；
- plugin locale labels；
- group ordering；
- refresh/copy；
- status display；
- tests。

## PR 3：通用 inventory 契约

- runtime inventory declaration；
- inventory scheduling；
- cache；
- cancellation；
- merge；
- limits；
- diagnostics。

## PR 4：雷蛇基础协议

- transports；
- 90-byte report；
- xor8；
- status/retry/correlation；
- exact interface matching；
- fixtures。

## PR 5：雷蛇核心读取

- identity；
- firmware；
- battery；
- DPI；
- polling；
- profile；
- lighting；
- sleep；
- main UI integration。

## PR 6：雷蛇全面 inventory

- sensor；
- scroll；
- capability flags；
- receiver；
- profile metadata；
- buttons；
- macro metadata；
- memory metadata；
- detailed locales；
- read coverage report。

## PR 7：反馈闭环

- protocol diagnostic mode；
- device-targeted copy；
- Issue Form；
- privacy tests；
- documentation。

如果仓库维护方式要求单 PR，也应在提交历史中保持以上逻辑分层。

---

# 二十、验收标准

完成后必须满足：

1. 精确匹配的雷蛇型号能被插件识别。
2. 未验证型号也能正确选择自己的 model overlay。
3. 主界面只显示成功读到的常用参数。
4. “全部读数”可以主动读取并展示安全可读参数。
5. 数组和对象不显示为 `[object Object]`。
6. 插件专属字段使用插件 locale。
7. 一个可选命令失败不会清空其他成功读数。
8. 用户能看到哪些参数不支持、跳过或失败。
9. 全面读取不会进入每秒轮询。
10. 设备休眠和断开不会造成日志风暴。
11. 用户能复制当前设备的脱敏读数和相关日志。
12. 默认诊断不包含 HID payload。
13. 临时协议诊断需要明确授权并自动到期。
14. 每个启用 GET 都有来源、parser、workflow、fixture 和详情展示闭环。
15. 未知枚举和未知 bytes 被保留，不被猜测或清零。
16. destructive 命令不进入读取或普通配置路径。
17. 没有在 React/Tauri 业务层加入雷蛇型号分支。
18. AMaster、Logitech HID++ 和 mock 插件行为不回退。
19. `npm run validate`、`npm test`、Rust tests、frontend tests 和打包全部通过。
20. 文档清楚区分：
    - source-confirmed；
    - fixture-verified；
    - user-reported；
    - hardware-verified。

---

# 二十一、最终交付说明

最后输出：

1. 两个仓库分别修改了哪些文件；
2. 通用 Host 能力与雷蛇插件能力的边界；
3. 支持型号和连接方式矩阵；
4. 每个读取领域的覆盖率；
5. 哪些参数进入主界面；
6. 哪些参数只进入全部读数；
7. 哪些参数按需读取；
8. 哪些命令仍为 reserve；
9. 哪些功能永久禁止；
10. 测试结果；
11. 尚未有硬件验证的风险；
12. 用户如何复制诊断反馈；
13. 后续根据用户日志修正型号差异的方法。

不要以“没有实机”为理由只提交空插件，也不要把未经验证的行为描述为稳定支持。应通过完整、安全、可降级、可观察、可反馈的读取架构，把缺少本地硬件的风险控制在可维护范围内。
