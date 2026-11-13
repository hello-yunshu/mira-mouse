# Mira 设备稳定性真机验证与剩余风险推进

## 0. 结论

- 阶段一 `fix/device-stability-stage1` 提交：`e2e58874`。
- 真机：AM INFINITY MOUSE .100（接收器 `VID 0x3151 / PID 0x5007`），macOS 27.2。
- 真机确认有效：**burst 期间强制 Full 不再被降级**；**迟到的初始化空快照不再覆盖新事件**。
- 真机未能确认：`device_select` 乱序（只有一个匹配设备，无法切换）；真实拔插 `Clear`；`refresh_devices()` 失败分支（无法安全制造枚举失败）。
- 新发现并已修复：**首屏空白 / 只剩背景**。根因是 `thinking-orbs` 嵌套依赖携带 React 18，与宿主 React 19 形成双运行时，`ThinkingOrb` 调用另一份 React 的 hooks，dispatcher 为 null。
- 仍未修复：等待页整体遮蔽接收器功能、`mouse_ready` 可能保持旧值、多 HID 接口并列场景。

## 1. 真机环境

- `hidutil list` 显示设备 3 个 HID 接口：
  - UsagePage 1 / Usage 2（鼠标输入）
  - UsagePage 12 / Usage 1（消费类）
  - UsagePage 65535 / Usage 2（厂商控制）
- 插件 `devices.json` 的接收器描述匹配 `65535/2`；运行日志稳定为 `matched 1 device(s)`。
- 另两个接口不匹配当前插件描述，因此**没有出现多设备污染**。
- 原始输出：[mira-hid-3151.txt](evidence/mira-device-stability/mira-hid-3151.txt)

## 2. 三项修复的真机验证

### 2.1 burst 期间 Full 不被降级：PASS（真机）

方法：鼠标自然休眠使 `receiverIdle.mouseOnline=false`；调试构建通过 `MIRA_DIAG_AUTO_OFFLINE_REFRESH_MS` 在检测到 `mouse_ready=false` 后调用与 `device_refresh` 相同的路径（`reset_all_backoff` + `request_refresh_with_plan(Full)` + 唤醒读取线程）。

关键日志摘录（[mira-burst-real.log](evidence/mira-device-stability/mira-burst-real.log)）：

```text
[mira-diag] injected BatteryOnly (mouse_ready=Some(true))
[mira-diag] plan=BatteryOnly connected=true burst=false mouse_pending=false forced=Some(BatteryOnly)
[mira-diag] emit device-updated name=AM INFINITY MOUSE .100 mouse_ready=Some(false)
[mira-diag] plan=Quick connected=true burst=true mouse_pending=true forced=None
[mira-diag] injected Full (mouse_ready=Some(false))
[mira-diag] plan=Full connected=true burst=true mouse_pending=true forced=Some(Full)
[mira] plugin workflow protocol-a-receiver-read: 5 outputs: [receiverFirmwareLed, receiverFirmwareSoc, receiverFirmwareUsb, receiverIdle, receiverLighting]
[mira-diag] outcome=ready entries=1 fresh=1 names=["AM INFINITY MOUSE .100"]
```

结论：`burst=true` 时 `plan=Full`，不再被旧逻辑改成 Quick；读取实际执行并产出快照。
限制：鼠标休眠时插件按 `skipIfZero` 只执行接收器相关 5 个输出，因此这是接收器范围内的完整读取，不是鼠标参数完整读取。鼠标被物理唤醒后的 14 输出补全未能复测（见第 5 节）。

### 2.2 迟到初始化快照不覆盖新事件：PASS（真机 + 注入延迟）

方法：调试构建延迟 `device_snapshots` 响应 5.5s，并延迟读取线程启动 3s，使初始化请求先捕获空列表；随后真实设备事件到达；最后才返回空响应。

关键日志（[mira-order-green.log](evidence/mira-device-stability/mira-order-green.log)）：

```text
[mira-diag] device_snapshots response entries=0 names=[] mouse_ready=[]
[mira-diag] emit device-updated name=AM INFINITY MOUSE .100 mouse_ready=None
[mira-diag] emit device-updated name=AM INFINITY MOUSE .100 mouse_ready=Some(false)
```

延迟空响应返回后，界面仍停留在“等待鼠标就位”页，而不是回退到“未找到设备”空态：
![迟到快照后的界面](evidence/mira-device-stability/late-snapshot-fixed.png)

限制：延迟由调试构建注入，设备数据本身来自真实设备；生产构建没有该延迟。

### 2.3 `device_select` 乱序：BLOCKED（真机）

- 当前只有一个匹配设备，UI 不渲染“切换鼠标”入口，无法在真机执行设备切换。
- 日志证据：`matched 1 device(s)`（[mira-burst-real.log](evidence/mira-device-stability/mira-burst-real.log)）。
- 该守卫仅有单元测试覆盖：[App.device-race.test.tsx](../../src/App.device-race.test.tsx) 的第二个用例；不能写成真机通过。

### 2.4 真实断开 `Clear` / 枚举失败 `PreserveLast`

- 鼠标休眠不是断开：真机日志中 `matched` 仍为 1，`mouse_ready=false`，结果为 `outcome=ready`，**没有** `outcome=clear`。
- 真实拔插未执行：无法物理拔掉接收器，因此 `DeviceReadOutcome::Clear` 未在真机验证。
- `refresh_devices()` 枚举失败未执行：macOS 本地枚举无法在不破坏系统的前提下稳定注入失败；仅有单元测试 `transient_enumeration_failure_preserves_last_snapshot`。

## 3. 新缺陷：首屏空白（red → green）

### 3.1 Red

调试构建启动后 WebView 空白，仅保留窗口背景与标题栏按钮。前端信标记录：

```text
[mira-diag] frontend event=mounted detail=None
[mira-diag] frontend event=window-error detail=Some("null is not an object (evaluating 'T.current.useRef')
@tauri://localhost/assets/react-vendor-CCosFbUU.js:9:43095
M@tauri://localhost/assets/runtime-data-cache-ClcH_MJQ.js:1:15424 ...")
```

证据：[mira-beacon.log](evidence/mira-device-stability/mira-beacon.log)、[mira-reactfix.log](evidence/mira-device-stability/mira-reactfix.log)。

### 3.2 根因

- 宿主：`node_modules/react` 版本 `19.2.8`。
- `thinking-orbs` 的 git 依赖自带嵌套 `node_modules/react` 版本 `18.3.1`。
- Vite 解析到两份 React；`ThinkingOrb` 调用 React 18 的 `useRef`，但渲染由 React 19 驱动，React 19 的 dispatcher 对 React 18 的 hooks 为 null，运行时异常导致首屏空白。
- 这不是 CSS/vibrancy 问题；禁用 vibrancy 后仍空白。

### 3.3 Green

修复：

- [vite.config.ts](../../vite.config.ts)：`resolve.dedupe: ['react', 'react-dom', 'react/jsx-runtime']`，并新增单一 `react-vendor` manualChunk。
- 修复后构建产物只有 `react-vendor-BNmTiJ-I.js` 包含 React 运行时符号；不再出现第二份 React。

验证：

- 启动日志只有 `frontend event=mounted`，无 `window-error`：[mira-green.log](evidence/mira-device-stability/mira-green.log)。
- 界面正常渲染“等待鼠标就位”页：[awaiting-mouse-fixed.png](evidence/mira-device-stability/awaiting-mouse-fixed.png)。

## 4. 唤醒后只有灯光（本轮根因与修复）

### 4.1 Red

真机：鼠标物理唤醒后，页面只有灯光，DPI/回报率/配置等能力缺失。
旧数据链路日志（[wake-only-lighting-red.log](evidence/mira-device-stability/wake-only-lighting-red.log)）：

```text
[mira-diag] reading plan=BatteryOnly mouse_ready=Some(true) caps=["battery", "receiver", "receiverIdle"]
[mira-diag] wake-recovery requests Full after mouse component returned
[mira-diag] snapshot plan=BatteryOnly mouse_ready=Some(true) readiness=Quick
[mira-diag] plan=Full connected=true burst=false forced=Some(Full)
[mira-diag] outcome=ready entries=1 fresh=0
```

`fresh=0` 表示这次 Full 被 500ms `READ_DEBOUNCE_TTL` 缓存跳过：强制 Full 被取出后直接复用旧快照，没有再执行 HID 工作流。旧 UI 截图：[only-lighting-red.png](evidence/mira-device-stability/only-lighting-red.png)。

### 4.2 根因

1. `mark_wake_recovery_observation` 在 BatteryOnly/Quick 读到鼠标组件返回后请求 Full。
2. 下一轮 `spawn_device_reader` 取到 `forced=Some(Full)`，但 `read_device_once` 先命中 500ms `cache_hit`，直接复用旧快照并 `continue`。
3. 请求被消费掉，没有第二次 Full；`mouse_ready` 已由 BatteryOnly 快照发布为 true，UI 从等待页切到 Dashboard，但静态能力仍是旧的接收器快照 → 只剩灯光。
4. UI 侧不是灯光 section 条件渲染单独错误，而是数据面没有完成 Full 补全。

### 4.3 修复

- [lib.rs](../../src-tauri/src/lib.rs)：`should_use_cached_read` 让 Full/Inventory 穿透 500ms 防抖缓存。
- [lib.rs](../../src-tauri/src/lib.rs)：`should_withhold_mouse_ready_until_full` 在 `false → true` 时先保持等待页，等 Full 完成后才发布 `mouse_ready=true`，避免用户看到半快照。
- 新增单测：`full_and_inventory_bypass_read_debounce`、`mouse_ready_transition_waits_for_full`。

### 4.4 Green

真机唤醒后日志（[wake-recovery-green.log](evidence/mira-device-stability/wake-recovery-green.log)）：

```text
[mira-diag] reading plan=BatteryOnly mouse_ready=Some(true) caps=["battery", "receiver", "receiverIdle"]
[mira-diag] wake-recovery requests Full after mouse component returned
[mira-diag] withholding mouse_ready=true until Full completes
[mira-diag] plan=Full connected=true burst=false forced=Some(Full)
[mira-diag] reading plan=Full mouse_ready=Some(true) caps=[battery, ..., settings] (15 输出)
[mira-diag] snapshot plan=Full ... readiness=Full
[mira-diag] emit device-updated ... mouse_ready=Some(true)
```

唤醒后完整 Dashboard：[wake-recovery-green.png](evidence/mira-device-stability/wake-recovery-green.png)。

`mouse_ready` 旧值滞留与本次“只有灯光”是同一根因链：BatteryOnly 先发布 true，但 Full 被缓存吞掉；已一并修复。

## 5. 物理拔插验证

用户已实际拔掉/插回接收器。

### 5.1 拔掉

```text
[mira] background: matched 0 device(s)
[mira-diag] outcome=clear
```

UI 进入空态，无旧设备残留：[physical-unplugged.png](evidence/mira-device-stability/physical-unplugged.png)。进程未崩溃。

### 5.2 插回

```text
[mira] background: matched 1 device(s)
[mira] background read failed ... battery: condition at offset 0 timed out
...
[mira-diag] reading plan=Full mouse_ready=Some(true) caps=[15 输出]
[mira-diag] snapshot plan=Full ... readiness=Full
```

插回后先显示“正在识别并读取鼠标…”占位，没有旧数据残留：[physical-replugged-loading.png](evidence/mira-device-stability/physical-replugged-loading.png)；退避重试完成后恢复完整 Dashboard：[physical-replugged-recovered.png](evidence/mira-device-stability/physical-replugged-recovered.png)。完整日志：[physical-replug.log](evidence/mira-device-stability/physical-replug.log)。

残余体验风险：接收器插回时若鼠标仍休眠，首次 Full 可能因鼠标电池超时失败并进入退避。首轮退避已由 60 秒缩短为 10 秒，且鼠标组件从缺失恢复到存在时会清除该路径的旧退避并补发 Full，避免唤醒后仍被旧退避拦截。退避常量与唤醒补 Full 分别由 `backoff_schedule_matches_spec`、`wake_recovery_full_request_clears_stale_device_backoff` 覆盖；本轮真机拔插复测因未完成物理操作，未取得新的插回耗时数据。

### 5.3 可控枚举失败

调试构建通过 `MIRA_DIAG_ENUM_FAIL_FILE`（debug-only）在运行中制造一次“枚举失败 + 空 matched”：

```text
[mira] background: matched 0 device(s)
[mira-diag] outcome=preserve-last
```

UI 保留设备、未清空：[enumeration-failure-preserve.png](evidence/mira-device-stability/enumeration-failure-preserve.png)。移除文件后恢复 `matched 1`、`outcome=ready`。日志：[enumeration-failure.log](evidence/mira-device-stability/enumeration-failure.log)。

说明：这是对 `refresh_devices()` 失败分支的可控故障注入，不是真实 OS 枚举失败；发布构建不包含该钩子。

## 6. 剩余风险状态

| 风险 | 状态 | 证据 / 说明 |
|---|---|---|
| 唤醒后只有灯光 / Full 被缓存吞掉 | **已修复，真机 PASS** | red/green 日志与截图 |
| 首屏空白 / 只剩背景 | **已修复，真机 PASS** | React 双运行时 dedupe |
| 真实拔插 `Clear` / 插回恢复 | **真机 PASS** | 空态、占位、完整恢复；无崩溃 |
| 枚举失败 `PreserveLast` | **可控验证 PASS** | debug fault injection；真实 OS 失败未制造 |
| 设备切换乱序 | BLOCKED | 只有一个匹配设备 |
| 等待页遮蔽接收器功能 | 已确认，未修复 | 需产品决定；接收器灯光/电量在鼠标离线时不可操作 |
| 多 HID / 直连+接收器 | 部分验证 | 3 接口仅 1 匹配；直连+接收器并列未测 |
| 插件仓库 | 通过，未改动 | 64 tests passed；真机在线 14 outputs、离线 5 outputs |
| 完整 CI | 部分执行 | quick-check、cargo 576、clippy、boundaries、structured、plugin-contract 通过；`local-ci.sh` 未执行 |
| 标准发布构建 precheck | **BLOCKED** | `check-local-ai-assets.mjs` 仍报 Local AI handler pack 无效；使用 config override 仅用于 debug 真机验证 |

## 7. 分支与测试

- 分支：`fix/device-stability-stage1`
- 阶段一提交：`e2e58874`
- 首屏空白修复：`406b2381`
- 真机诊断：`eebaee67`
- 真机报告/证据：`e1122ac1`、`b981fffd`
- 本轮唤醒修复与拔插证据：见 `git log`
- 前端：`npm run check:quick` → lint/typecheck/**574 tests passed**/fmt/diff-check 通过
- Rust：`cargo test -p mira-app --lib` → **576 passed; 0 failed**
- Rust lint：`cargo clippy -p mira-app --lib --locked -- -D warnings` → 通过
- 架构/结构/插件契约：通过
- 插件仓库：`npm test` → **64 passed**
- 前端构建：`npm run build` 通过
- 未 push、未合并 `main`、未改写历史
- 所有诊断 env/延迟/强制枚举失败钩子均在 `#[cfg(debug_assertions)]`；前端调试信标已移除，不进入发布产物

## 8. 给非工程用户的物理拔插操作卡

1. 保持当前 Mira 窗口可见。
2. 拔掉接收器，等待 3 秒。页面应变成“还没找到支持的鼠标呢”。
3. 等 5 秒，确认 Mira 仍在前台、没有闪退。
4. 插回接收器，等待 5 秒。页面应先出现“正在识别并读取鼠标…”，随后恢复设备页。
5. 若插回后 30 秒仍显示“正在识别并读取鼠标…”，点击“刷新”一次；仍不恢复则保持接收器插着并记录时间。
6. 不要在此期间关闭 Mira 或移除接收器。
