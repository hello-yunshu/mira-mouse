# Mira 设备稳定性审计与修复：阶段一结果

## 0. 基线与范围

- 主仓库：`/Users/yunshu/Documents/GitHub/mira-mouse`，分支 `main`，HEAD `9d37ef2858307bb242350e56845518a4663f544d`。
- 本地 `origin/main` 跟踪分支已不存在（`git status` 显示 `[gone]`）；未执行 fetch/ls-remote，远端最新状态**未验证**。
- 开工前工作区干净，无他人未提交改动。
- 插件仓库：`/Users/yunshu/Documents/GitHub/mira-mouse-plugins`，分支 `main`，HEAD `9a5d9a1`，工作区干净；主仓库通过 `bundled-plugins.lock.json` 固定使用 `mira.amaster` 1.9.2 的二进制插件包。
- 提示词没有显式的「阶段一」编号。本报告把「阶段一」定义为：**定位证据链 + 对已证实的 P0 做先失败后通过的修复 + 针对性验证 + 证据报告**。P1、插件仓库改动、真实设备复测、渲染层空白专项、提交/推送均未进入本阶段。

## 1. 证据链与结论

| 提示词风险点 | 实际代码路径 | 结论 | 本阶段处理 |
|---|---|---|---|
| burst 分支吞掉手动 Full | [lib.rs](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L11481)`choose_read_plan` 调用点；旧代码在 `mouse_pending_burst` 时无条件返回 Quick，而 `forced_read_plan` 已在循环开头被 `take()` | **已确认 bug** | 已修复；新增 `manual_full_survives_mouse_pending_burst` |
| 枚举失败被当作断开 | [lib.rs](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L10528)`refresh_devices()` 失败被忽略，旧代码 `matched.is_empty()` 直接 `DeviceReadOutcome::Clear` | **已确认 bug** | 已修复；枚举失败改为 `PreserveLast` 并记录日志 |
| 前端初始化/选择响应覆盖新事件 | [App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4666) 初次 `device_snapshots`；[App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4737) `device_select` | **已确认 bug** | 已修复；新增版本号与选择序号 |
| 降级 Full 自愈 | `SnapshotReadiness::DegradedFull`、`update_degraded_full_retry`、`mark_wake_recovery_observation`；AMaster 声明 `wakeRecovery`（`componentId: mouse`） | 存在设计路径；**未在真实设备上验证** | 未改动；列为本阶段剩余风险 |
| `mouseReady=false` 全页替换 | [App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4873)`device.mouseReady === false` 直接渲染 `AwaitingMouseState`；`mouse_ready` 来源见 [protocol.rs](file:///Users/yunshu/Documents/GitHub/mira-mouse/crates/mira-plugin-runtime/src/protocol.rs#L862) | 行为确认；是否应改为「接收器功能可见 + 鼠标控制只读」需要产品决策 | 未改动；属于 P1/后续阶段 |
| 插件与多 HID 接口 | AMaster `devices.json` 对 `.100` 有 2 个直连描述、2 个接收器描述；`enumerate_matched_devices` 按 `(plugin_id, family, path)` 去重；`interface_number` 未在 AMaster 描述中设置 | 未复现选错/污染；存在多接口生成多条快照的风险 | 仅只读核对，未改插件 |
| 纯渲染空白 | `src/main.tsx` 无 ErrorBoundary；`Suspense` 仅包裹 Settings/About/Logs 懒加载页；Dashboard 不走 Suspense；另有 DOM 快照交叉过渡 | **未证实**；需要 WebView/真机复现 | 未改动；本阶段无证据 |
| 错误可观测性 | `record_device_read_error` + `DeviceReadDiagnostic`；HID 路径在对外字段中已脱敏 | 基本具备；枚举失败此前完全静默 | 已为枚举失败增加 stderr 日志 |

## 2. 改动清单

### 2.1 Rust：保留 burst 期间的强请求

- 新增 [choose_read_plan](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L9717)。
- [spawn_device_reader 调用点](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L11481)改为统一走该函数。
- 规则：`mouse_pending_burst` 时仍默认 Quick，但 `forced_plan` 为 Full/Inventory 时保留；非 burst 行为不变（PresenceOnly 在鼠标未就位时升级 BatteryOnly）。

### 2.2 Rust：区分枚举失败与真实断开

- 新增 [enumeration_read_outcome](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L10487)。
- [read_device_once 集成点](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L10528)：`refresh_devices()` 返回错误时记录 `[mira] background: HID enumeration failed: ...`，且空列表只返回 `PreserveLast`，不执行 `Clear`。
- 真实断开（枚举成功但列表为空）仍返回 `Clear`，保留原有清缓存行为。

### 2.3 前端：事件优先于旧请求响应

- 新增 `snapshotRevisionRef`、`deviceSelectRequestRef`，见 [App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4406)。
- 初次 `device_snapshots` 返回时若版本号已变化则丢弃响应，见 [App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4666)。
- `device-updated`、`device-snapshots-updated` 事件递增版本号，见 [App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4683) 与 [App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4702)。
- `device_select` 返回时丢弃旧选择序号；若期间有新事件，则用最新 entries 渲染，不用旧响应，见 [App.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.tsx#L4737)。

### 2.4 新增测试

- [src/App.device-race.test.tsx](file:///Users/yunshu/Documents/GitHub/mira-mouse/src/App.device-race.test.tsx)：2 个前端竞态用例。
- [read_plan_tests](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L16106)：5 个 Rust 用例。
  - `manual_full_survives_mouse_pending_burst`
  - `mouse_pending_burst_without_stronger_request_uses_quick`
  - `mouse_pending_upgrades_presence_to_battery_outside_burst`
  - `transient_enumeration_failure_preserves_last_snapshot`
  - `confirmed_empty_enumeration_clears_snapshot_cache`

## 3. 先失败、后通过的真实证据

### 3.1 修复前失败

1. 前端：`npx vitest run src/App.device-race.test.tsx`
   - 结果：`2 failed`。
   - 旧代码结论：事件已显示 `Newest Mouse`，随后迟到的 `device_snapshots` 响应把界面覆盖回 `Stale Mouse`；迟到的 `device_select` 响应覆盖更新的 `device-snapshots-updated`。
   - 完整输出：`/Users/yunshu/Library/Application Support/Trae CN/ModularData/ai-agent/command-execution/07f0fa4655bee27268bf643df8568d94/output.log`
2. Rust：先按旧语义抽取判定函数后运行
   - `cargo test -p mira-app --lib manual_full_survives_mouse_pending_burst`
   - 失败断言：`left: Quick, right: Full`
   - `cargo test -p mira-app --lib transient_enumeration_failure_preserves_last_snapshot`
   - 失败断言：旧判定返回 `Clear`，不是 `PreserveLast`
   - 完整输出：`/Users/yunshu/Library/Application Support/Trae CN/ModularData/ai-agent/command-execution/898ac32f86e52cbbf76fc503cb6b96d3/output.log`

### 3.2 修复后通过

| 命令 | 真实结果 | 输出路径 |
|---|---|---|
| `npx vitest run src/App.device-race.test.tsx` | 1 file / **2 tests passed** | 本次命令输出 |
| `cargo test -p mira-app --lib <5 个新增用例>` | 每个用例 `1 passed; 0 failed` | `.../bdda5ca3d166e6dcd0c3951807313d0c/output.log` |
| `npm run check:quick` | lint + typecheck + **574 tests passed** + `cargo fmt --all --check` + `git diff --check` 全部通过 | `.../61444c4a0bca7804a1182380b154646a/output.log` |
| `cargo test -p mira-app --lib` | **574 passed; 0 failed; 0 ignored** | `.../bb589b93e7197f5ac65e2b7bd2bbd287/output.log` |
| `cargo clippy -p mira-app --lib --locked -- -D warnings` | 退出码 0，无 warning | `.../4cd644c110c2ce7e0e9005e9e78b5a8b/output.log` |
| `npm run check:boundaries && npm run check:structured` | architecture lint clean；structured files PASS | `.../bbe21b527ac59ed2eee6f0c9c5883023/output.log` |
| `git diff --check` | 退出码 0 | 本次命令输出 |

说明：首次 `cargo test` 时本地缓存缺少依赖，Cargo 从 USTC registry 下载了缺失 crate；未安装系统软件，未触发任何远程 Action。

## 4. 未执行 / 未验证

以下项目**不得**写成通过：

1. **真实 AMaster `.100` 接收器/鼠标复测**：未执行。当前环境无设备会话，无法验证休眠/唤醒、插拔、多 HID 接口和真实 `receiver-poll` 时序。
2. **纯渲染空白（只剩背景光效）**：未执行。需要构建并运行 Tauri WebView、抓取崩溃日志或截图；当前只有静态代码证据，不能把数据层空态与渲染层崩溃混为一谈。
3. **`AwaitingMouseState` 遮蔽接收器功能**：未修复。需要产品确认「等待鼠标时接收器灯光/电量是否可操作」，并决定鼠标写入能力如何只读化。
4. **多 HID 接口/多设备快照污染**：未执行真机验证。仅确认 `enumerate_matched_devices` 的去重键与 `interface_number` 过滤逻辑。
5. **插件仓库改动**：未执行。没有证据指向 AMaster 协议偏移/工作流错误；本次只读核对了 `devices.json`、`workflows.json`、`commands.json`、`parsers.json`、`transports.json`。
6. **`npm run check:ci` / `scripts/local-ci.sh` / `cargo test --workspace --locked`**：未执行。`local-ci.sh` 包含 `cargo run --package xtask -- plugins sync --locked`（会下载已发布插件资产）和完整 workspace 构建，超出本阶段允许的验证范围。
7. **`npm run build` / Tauri 构建 / 安装包**：未执行。
8. **提交与推送**：未提交、未推送。当前分支是 `main`，与提示词「提交到当前工作分支」及「不擅自合并主分支」存在冲突；需要用户决定是否新建分支后提交。
9. **远端最新状态**：未验证。`origin/main` 本地跟踪分支为 `[gone]`，且未执行 fetch/ls-remote。

## 5. 剩余风险

- `mouse_ready` 是「上次成功读取」的粘性值；`receiverIdle` 读取失败时旧 `Some(false)` 可能保留，导致等待页持续显示。当前未加时间戳/新鲜度门禁。
- `update_degraded_full_retry` 每设备只自动重试一次；若这次重试发生在鼠标仍离线时，后续恢复仍依赖唤醒恢复或用户手动刷新。
- AMaster `.100` 的直连/接收器描述均未设置 `interface_number`；若同一物理设备的不同 HID interface 产生不同 `path`，仍可能生成多条快照。未复现。
- `src-tauri/src/lib.rs` 仍有 3 处 `let _ = refresh_devices()`（[8847](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L8847)、[11637](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L11637)、[11887](file:///Users/yunshu/Documents/GitHub/mira-mouse/src-tauri/src/lib.rs#L11887)）；本次只修复后台主读取路径。
- burst 期间现在允许一次强 Full 执行；需在真机确认不会因接收器持续失败而形成重复 Full（现有 `DegradedFullRetryState::Attempted` 理论上限制了次数）。
- `device-updated` 在多设备时仍以本地 entries 的选中项为准，而不是事件 payload；若后端选中路径与本地 entries 短暂不一致，仍可能显示旧设备。

## 6. 提示词含糊/矛盾与处理

1. **「阶段一」未定义**：提示词只有「第一步定位 / 第二步修复 / 第三步验证」，没有阶段编号。本报告按「首个修复闭环」执行，并在开头声明范围。
2. **当前分支就是 `main`**：提示词要求「提交到当前工作分支」，同时要求「不擅自合并主分支」。直接提交等于写 `main`。本阶段未提交，等待用户决定是否新建工作分支。
3. **quick-check 不含 Rust 单测**：`scripts/quick-check.sh` 只跑 `cargo fmt --check`，不跑 `cargo test`。因此额外执行了 `cargo test -p mira-app --lib`；不能只用 quick-check 宣称 Rust 修复已验证。
4. **`receiver-poll` 定义位置**：提示词只列了 `workflows.json`/`parsers.json`/`transports.json`，实际命令定义在 `plugins/amaster/protocol/commands.json`（`receiver-poll` 为 `0xF7`）。审计时必须同时看 `commands.json`。
5. **主仓库插件是二进制包**：主仓库 `src-tauri/resources/plugins/mira-amaster-1.9.2.mira-plugin` 与 `bundled-plugins.lock.json` 固定 1.9.2；插件源码在独立仓库。修改插件源码不会自动进入应用，必须重新打包/签名/发布并更新 lock。
6. **基线版本**：提示词给出的 `9d37ef28...` 与当前 HEAD 完全一致；但 `origin/main` 已丢失，不能据此断言远端仍是最新。
