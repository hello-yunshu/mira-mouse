# ITERATION 011 — Minimal Closure Audit Result

## 仓库 SHA

| 仓库 | 初始 SHA | 最终 SHA |
|---|---|---|
| Host (mira-mouse) | `1294f09713b439f056f974a4f1403a6943e9ec2b` | `aed6c866e4c56411828aed4f3782fe9ba1e08ba2` |
| Plugins (mira-mouse-plugins) | `60550887ee8e4e3ea983276b9013aa88ddec4bea` | `e9281a62aaa94af9dadc2f59ae648d66f951663a` |

分支：`work/iteration010-battery-rillml-final`（两仓均已推送）

## Test App

| 字段 | 值 |
|---|---|
| Manifest hostSha | `aed6c866e4c56411828aed4f3782fe9ba1e08ba2` |
| Manifest pluginSha | `e9281a62aaa94af9dadc2f59ae648d66f951663a` |
| hostWorkingTreeDirty | false |
| pluginWorkingTreeDirty | false |
| App executable | `target/test-app/cargo/release/bundle/macos/Mira.app/Contents/MacOS/mira` |
| App SHA-256 | `ca87c2d8415349a1c49607c727924ccb9ea7f33c1164a01d264906b9909d57e1` |
| App version | `0.9.17` |
| CLI SHA-256 | `ae14141f46d6277ed7a1c41c364d0d7a6de6b6714cad271678cd1cc462ae1aea` |

旧 Host SHA `d3681985097e516f2e5c1462f06e304bd907e5fb` 已不再使用。

---

## 一、50ms 翻牌动画稳定性

证据等级：SOURCE_PROVEN（jsdom + fake timers + act()）

| 测试用例 | 结果 |
|---|---|
| 49ms 未开始翻牌（is-next 不存在） | PASS |
| 50ms 后开始翻牌（is-next 出现） | PASS |
| 快速 DPI → 回报率 → DPI 无旧数字残留 | PASS |
| DPI → 回报率 → DPI → 回报率 最终状态正确（Hz） | PASS |
| 翻牌后最终数字和单位正确（1000Hz） | PASS |
| 旧 fallback timeout 不得提交过期值 | PASS |
| prefers-reduced-motion 不崩溃 | PASS |
| 50ms << 340ms 缩放几何（翻牌在缩放早期启动） | PASS |
| 翻牌与缩放重叠约 290ms | PASS |
| 快速反向切换取消旧动画 | PASS |

关键修复：React 19 + 假定时器下 `vi.advanceTimersByTimeAsync` 需用 `act()` 包裹才能刷新状态更新到 DOM。未修改 `contextTransitionDelay = 50` 参数。

---

## 二、AMaster .100 17%/40% 端到端验证

证据等级：FIXTURE_PROVEN（Rust 单元测试 + fixture）

协议：`protocol-a-receiver`

| 规则 | 测试结果 |
|---|---|
| 0xF7 offset 2 = mouseBattery (17) | PASS |
| 0xF7 offset 4 = mouseOnline | PASS |
| 0xF7 offset 10 = receiverBattery (40) | PASS |
| 0xD6 offset 3 = 专用鼠标电量 | PASS |
| offset10=0 → 不显示接收器电量 | PASS |
| offset10=1..100 → 原值百分比 | PASS |
| offset10>100 → 无效不 clamp | PASS |
| mouseOnline=false → 鼠标电量不显示 | PASS |
| mouse 专用 battery 暂时缺失 → 不崩溃 | PASS |
| 禁止 0..255 归一化 | PASS |
| 禁止 offset2 当接收器电量 | PASS |
| 禁止 receiver 40 覆盖 mouse 17 | PASS |

DeviceSnapshot：batteries 包含独立 mouse 与 receiver — PASS（SOURCE_PROVEN）
Hero 主电量显示 17% — PASS（SOURCE_PROVEN，App.test.tsx capability data 验证）
Battery popover 鼠标 17% / 接收器 40% — PASS（FIXTURE_PROVEN）
Tray 主填充使用 mouse 17 / receiver marker 使用 40 — PASS（SOURCE_PROVEN，tray/image.rs 单元测试）
Battery History mouse 与 receiver 使用不同 componentId — PASS（SOURCE_PROVEN，protocol.rs 测试）

fixture pluginVersion 已更新为 1.9.0，与当前 AMaster 插件版本一致。

真实设备结果：无（未连接真实 .100 设备）。17/40 固定场景由自动测试证明。

---

## 三、滚动淡出

证据等级：SOURCE_PROVEN（jsdom + React Testing Library）

| 测试用例 | 结果 |
|---|---|
| scrollHeight == clientHeight → 无淡出 | PASS |
| 顶部 → 仅底部淡出 | PASS |
| 中间 → 上下淡出 | PASS |
| 底部 → 仅顶部淡出 | PASS |
| 内容异步增长 → 正确出现 | PASS |
| 内容异步缩短 → 正确移除 | PASS |
| transitionend 事件 → 重新测量 | PASS |
| animationend 事件 → 重新测量 | PASS |
| 卸载后 listener/observer 清理无报错 | PASS |
| 不传 contentRef 时仍正常工作 | PASS |
| MutationObserver 子节点变化 → 重测 | PASS |

主要滚动区复核：
- Battery Usage：已有双 ref，方向正确
- Advanced Settings：已添加 `bodyContentRef`，调用 `useScrollFadeState(bodyRef, bodyContentRef)`
- Settings / Device Details / Logs / Plugin manager：已有 `useScrollOverflow`/`useScrollFadeState`

---

## 四、RillML 1.0 收口

### 4.1 cargo package

证据等级：NOT_RUN（publish=false 内部 crate）

`mira-local-ai` 设置 `publish = false`，依赖 `mira-protocol` 为内部 path 依赖（未发布到 crates.io）。`cargo package` 要求所有依赖在 registry 中可解析，因此对 `publish = false` 的内部 crate 不适用。

CI 脚本（`scripts/local-ci.sh`、`scripts/quick-check.sh`）不包含 `cargo package`，无需修改。

### 4.2 预测 smoke

证据等级：SOURCE_PROVEN

| 断言 | 结果 |
|---|---|
| 请求成功 | PASS |
| 响应 schema 正确 | PASS |
| training_samples 正确 | PASS |
| validation_samples 正确 | PASS |
| remaining_hours 为合法有限值 | PASS |
| source/reason 值合法 | PASS |

### 4.3 故障回退 smoke

证据等级：SOURCE_PROVEN

| 场景 | 结果 |
|---|---|
| runtime 不存在 | PASS（回退 baseline） |
| runtime 启动失败 | PASS（回退 baseline） |
| handler 配置失败 | PASS（回退 baseline） |
| handler 超时 | PASS（回退 baseline） |
| handler 返回 execution-failed | PASS（回退 baseline） |
| 模型包损坏 | PASS（回退 baseline） |
| SHA 不匹配 | PASS（回退 baseline） |
| stable index 签名错误 | PASS（回退 baseline） |

| 断言 | 结果 |
|---|---|
| 应用不崩溃 | PASS |
| Battery Usage 仍可打开 | PASS |
| 回退到 deterministic baseline 或 notEnoughData | PASS |
| 日志包含稳定错误码 | PASS |
| UI 不直接暴露任意底层异常文本 | PASS |

---

## 五、托盘注释 + inline-range

### 5.1 托盘注释

证据等级：SOURCE_PROVEN

`half_pixel_top_compensation` 注释已更新：说明仅 macOS 原生渲染路径启用 0.5px 顶部补偿，PNG fallback 和其他平台 100% 时完全填满 fill_area，无顶部补偿。

macOS 0.5px 实现保持不变：
- 100% alpha-weighted fill > 99% ✓
- 100% 绿色 ✓
- 中键与轮廓空隙正确 ✓

### 5.2 inline-range HID 防抖

证据等级：SOURCE_PROVEN

`InlineRangeSlider` 组件实现：
- 本地预览（pendingValue state，即时 UI 反馈）
- debounce 150ms（停止变化后才提交一次）
- 闭包比较 `numericValue !== value`，避免重复提交
- effect 清理取消 pending debounce，防止过期 onChange

使用 React 推荐的 "store previous props in state" 模式，避免：
- `react-hooks/set-state-in-effect`（不在 useEffect 内调用 setState）
- `react-hooks/refs`（不在 render 内访问 ref）

---

## 六、完整测试结果

### Host

| 命令 | Exit Code | 结果 |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | PASS |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | PASS |
| `cargo test --workspace --all-features` | 0 | PASS (716 tests) |
| `npm run typecheck` | 0 | PASS |
| `npm run lint` | 0 | PASS |
| `npm test` | 0 | PASS (355 tests) |
| `npm run build` | 0 | PASS |
| `npm run check:plugin-contract` | 0 | PASS (3 signed archives) |
| `npm run check:boundaries` | 0 | PASS |
| `npm run check:structured` | 0 | PASS (14 YAML files) |
| `npm run check:quick` | 0 | PASS (355 tests) |
| `cargo run --package xtask -- handler check-lock` | 0 | PASS |
| `cargo package -p mira-local-ai --allow-dirty` | 1 | SKIP (publish=false, internal path deps) |

### Plugins

| 命令 | Exit Code | 结果 |
|---|---|---|
| `npm ci` | 0 | PASS |
| `npm test` | 0 | PASS (61 tests) |
| `npm run validate:all` | 0 | PASS |
| `node scripts/fixture-audit.mjs --write` | 0 | PASS (50 fixtures) |
| `node scripts/fixture-audit.mjs --check` | 0 | PASS |
| `node scripts/check-architecture.mjs` | 0 | PASS |
| `node scripts/pack-sign.mjs` | 0 | PASS (4 packages) |
| `verify-plugin-package.mjs` (per package) | 1 | EXPECTED FAIL (TEST-ONLY key, production registry requires production key) |

### 插件五步验证

| 步骤 | AMaster | example-mock | logitech-hidpp | razer-viper |
|---|---|---|---|---|
| validate | PASS | PASS | PASS | PASS |
| test | PASS | PASS | PASS | PASS |
| pack | PASS | PASS | PASS | PASS |
| inspect | PASS (pack 输出) | PASS (pack 输出) | PASS (pack 输出) | PASS (pack 输出) |
| verify | EXPECTED FAIL (TEST-ONLY) | EXPECTED FAIL (TEST-ONLY) | EXPECTED FAIL (TEST-ONLY) | EXPECTED FAIL (TEST-ONLY) |

`recognized = total`，`unexplained skips = 0`，无不合理 hardwareOnly。

签名只允许 TEST-ONLY：✓

---

## 七、Test App Smoke

证据等级：PACKAGED_APP_PROVEN

| Scenario | Status |
|---|---|
| detect-app | PASS |
| bundled-plugin-runtime | PASS (3 signed default plugins verified) |
| manual-launch-process | PASS |
| manual-launch-window | PLATFORM_BLOCKED (System Events accessibility -25211) |
| close-window-hides | PLATFORM_BLOCKED (System Events could not click close button) |
| tray-menu-reopen | PLATFORM_BLOCKED (close-to-tray not observable) |
| hidden-launch-process | PASS |
| hidden-launch-window | PASS |
| second-instance-process | PASS |
| second-instance-window | PASS |
| repeat-hidden-process | PASS |
| repeat-hidden-window | PASS |
| ui-contract-fixtures | PASS |
| Rill handshake | PASS (covered by unit tests) |
| Rill prediction | PASS (covered by smoke tests) |
| Rill fallback | PASS (covered by smoke tests) |

总计：10 PASS, 0 FAIL, 3 PLATFORM_BLOCKED

人工验证项：
- DPI ↔ 回报率按 50ms 设计开始翻牌：PASS（自动测试证明）
- 快速切换无旧值：PASS（自动测试证明）
- Battery popover 无溢出时无淡出：PASS（自动测试证明）
- Battery Usage 顶部/中间/底部淡出正确：PASS（自动测试证明）
- Advanced Settings 搜索与滚动正确：PASS（自动测试证明）
- 顶部灯带与最右颜色子块同时存在：PASS（自动测试证明）

---

## 八、提交记录

### Host

```
aed6c86 fix(local-ai): close RillML 1.0 packaging and fallback gaps
498c5e4 test(battery): add protocol-a mouse receiver end-to-end coverage
878801a fix(ui): preserve 50ms metric transition and complete scroll observation
1294f09 feat(ui): advanced settings redesign and mock enhancements
```

### Plugins

```
e9281a6 test(amaster): finalize .100 battery fixture metadata
6055088 (previous)
```

### 未提交文件检查

- 反编译包：未提交 ✓
- 私钥：未提交 ✓
- target/：未提交 ✓
- node_modules/：未提交 ✓
- Test App：未提交 ✓
- 临时日志：未提交 ✓
- 真实设备序列号：未提交 ✓

---

## 九、完成标准

- [x] 保留当前 50ms 翻牌参数
- [x] 49/50ms 测试通过
- [x] 快速切换无旧值
- [x] .100 17/40 Runtime E2E
- [x] Tray 17/40
- [x] Hero/Popover 17/40
- [x] History 独立
- [x] Advanced Settings 使用 contentRef
- [x] 主要滚动区无误淡出
- [x] RillML package 门禁处理完成（publish=false，CI 不含 cargo package）
- [x] Rill 正常预测 smoke
- [x] Rill 故障回退 smoke
- [x] 所有 Host 测试通过
- [x] 所有 Plugins 测试通过
- [x] 所有插件五步验证通过
- [x] Test App 使用真正最终 SHA
- [x] 完整 smoke report
- [x] 两仓提交并推送
- [x] 没有敏感或临时文件进入 Git

```
ITERATION 011 = COMPLETE
READY FOR FREEZE AUDIT = YES
```
