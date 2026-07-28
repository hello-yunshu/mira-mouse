# ITERATION-009 契约附录

本文件集中记录 ITERATION-009 收口的多项契约，作为源代码行为的权威文档。
后续版本若分裂为独立文档，需保持内容同步。

---

## 1. Advanced Settings 契约

Advanced Settings 是首页普通子块 overflow 的收纳区，仅展示以下 field：

- `presentation=details` 的 field；
- 首页 `selectLightingSubblocks()` 未选中的 fallback field；
- 不属于首页 6 个普通子块上限的其他可编辑 field。

### Zone-aware 去重

Advanced Settings 中相同 field 在不同 zone 下可能重复出现。Host 使用
**zone-aware 去重 key** 避免重复渲染：

```text
key = capabilityId : zoneId-or-root : fieldId
```

- `capabilityId`：capability 的 id（如 `lighting`）；
- `zoneId-or-root`：field 所属 zone 的 id；若 field 不属于任何 zone（即
  非 `LightingZone` capability），使用字符串 `root`；
- `fieldId`：field 的 id。

相同 key 的 field 在 Advanced Settings 中只渲染一次。不同 zone 下同名 field
（例如 mouse zone 与 receiver zone 都有 `color` field）会被视为不同 key，
分别渲染。

### 回报率子块上限

回报率（polling-rate）capability 在首页最多 3 个普通子块：

- 当 `fixedSlot === 2` 或 `group === "polling"` 时，limit = 3；
- 超过 3 个的 field 进入 Advanced Settings。

---

## 2. Notification Action 契约

### Action 列表（单一来源）

应用内 Toast 与系统通知共享以下 action 集合：

| Action                       | 行为                                              | 平台              |
|------------------------------|---------------------------------------------------|-------------------|
| `settings-plugin-update`     | 切换到 Settings → 聚焦插件更新区域                | All               |
| `settings-local-ai-update`   | 切换到 Settings → 聚焦 Local AI 更新区域          | All               |
| `about-app-update`           | 切换到 About → 聚焦应用更新区域                   | All               |
| `battery-usage`              | 打开 Battery Usage modal                          | All               |
| `relaunch`                   | 重启应用                                          | All               |
| `close`                      | 仅关闭 Toast / 通知，不触发跳转                   | All               |

### macOS 系统通知行为（方案 B）

macOS 系统通知**仅显示 title/body**，不存储 pending action，不在下次窗口
focus 时自动跳转。应用内 Toast 保留可点击入口。

理由：现有依赖无法可靠区分"用户点击通知"与"用户任意打开 Mira"，伪点击
机制会把任意打开误判为点击通知。方案 B 牺牲 macOS 系统通知的跳转能力，
换取语义正确性。

### Windows/Linux 系统通知行为

Windows/Linux 系统通知支持 native action：

- 点击通知 → focus 主窗口 → emit 对应事件 → 路由到 action 行为；
- `notification_action_to_event` 纯函数统一 action 到事件的映射；
- 关闭通知或超时不触发 action。

### Modal 打开时禁用跳转

任何 modal 打开时，Toast 的 onClick 不触发跳转，仅关闭 Toast。
关闭按钮永远不触发跳转。

### 托盘失败通知本地化

hidden 启动托盘构建失败的系统通知使用当前有效语言，中英文都有完整
title/body，不在 Rust 中硬编码整段英文。

---

## 3. macOS Startup Policy

### LaunchAgent plist 状态分类

Host 区分四种 plist 状态：

| State    | 含义                                                         |
|----------|--------------------------------------------------------------|
| Missing  | plist 文件不存在。                                           |
| Valid    | plist 存在且内容匹配当前 canonical exe + Label + `--hidden`。|
| Stale    | plist 存在但内容过期：exe 路径变更、缺少 `--hidden`、Label 不匹配、ProgramArguments 错误。|
| Invalid  | plist 存在但 XML 损坏（无法解析为 plist 或缺少关键 key）。   |

### Stale 自动修复

只要用户此前已启用自启（`is_enabled=true`），且 plist 存在但 Stale，
Host 自动重写为当前 canonical path。不能因为 `is_enabled=false` 就把
Stale plist 当作"用户未启用"。

### DMG 迁移状态

从 `/Volumes` 运行时：

- 不得创建无效 LaunchAgent；
- 不得先禁用旧可用自启再返回失败；
- `autostart_state()` 不得返回与真实状态不符的 `true`；
- UI 应返回可理解错误：先移动到 Applications。

### 事务安全操作顺序

启用或迁移顺序：

1. 解析 canonical executable；
2. 验证不在 `/Volumes`；
3. 构造 plist；
4. 写入临时文件；
5. 验证 plist；
6. 原子替换；
7. `launchctl bootstrap/load`；
8. 检查状态；
9. 成功后再删除 legacy。

失败时尽量保留旧可用条目。

### launchctl 结果检查

不得忽略 exit status。需要：

- 捕获 stdout/stderr；
- 记录日志；
- 失败时返回 Err；
- 必要时使用现代 `bootstrap/bootout`，并为旧系统提供兼容 fallback；
- 最后重新读取真实状态。

### `--hidden` 传递方式

通过 plist `ProgramArguments` 传递 `--hidden`，不使用 AppleScript。
这保证 hidden 启动由 launchd 直接管理，不依赖 GUI 会话。

---

## 4. Latest Test App Guide

### 一条命令可运行

```bash
npm run build:test-app
```

全新 clone（与 sibling `mira-mouse-plugins` 同级）后只需这一条命令即可完成：

1. 从当前 Host 源码构建 release mira-plugin CLI；
2. 动态发现 `plugins/*/plugin.json`；
3. 对每个插件运行 `validate / test / pack / inspect / verify`；
4. 用仓库内公开 TEST-ONLY seed 签名并验证固定测试信任根；
5. 把 Host 源码复制到临时 staging；
6. 仅在 staging 中安装测试包、生成 `releaseReady=false` lock，并启用
   `test-plugin-trust` Cargo feature；
7. 调用 `tauri build` 构建测试 App；
8. 删除 staging，源 checkout 的 lock/resources/Tauri config 保持不变；
9. 输出 `target/test-app/manifest.json`，包含 commit、dirty/source-state hash、
   CLI、App 和 plugin SHA-256。

### CLI 路径解析优先级

1. 若显式设置 `MIRA_PLUGIN_CLI`，使用该已存在的外部二进制；
2. 否则总是从当前 Host 源码构建 `target/release/mira-plugin`
   （Windows 自动追加 `.exe`），不复用可能过期的旧 CLI。

### TEST-ONLY 签名隔离

- 全新 clone 不依赖仓库外未知文件；
- 不使用生产私钥；
- 不修改生产 trusted keys；
- 不把 TEST-ONLY 包写入正式 Registry；
- `releaseReady=false`；
- Release build 必须拒绝 TEST-ONLY；
- 构建前验证生产 `trusted-keys.json` 不得包含 TEST-ONLY 密钥；
- 文件名、注释、文档明确"公开测试密钥，不是秘密"；
- 只用于显式 `test-plugin-trust` feature 的 Test App；
- 不复制进正式 App resources。

### 环境变量

| 变量                | 默认值                                          | 说明                          |
|---------------------|-------------------------------------------------|-------------------------------|
| `MIRA_PLUGIN_REPO`  | `../mira-mouse-plugins`                         | sibling 插件仓库路径          |
| `MIRA_PLUGIN_CLI`   | 未设置；默认从当前源码重建                      | 显式外部 mira-plugin 二进制    |
| `PLUGIN_KEY_ID`     | `TEST-ONLY-mira-plugins`                        | publisherKeyId                |
| `TAURI_BUNDLE`      | `app`                                            | tauri bundle 类型             |

### Smoke 测试

```bash
npm run smoke:test-app
```

自动探测刚构建的 App 路径（macOS `.app`、Linux AppImage、Windows `.exe`），
执行 12 个可观察断言：

1. 手动启动（无参数）→ 主窗口显示、可聚焦、托盘存在；
2. 关闭窗口 → 不退出、隐藏到托盘、托盘可重新打开；
3. hidden 启动（`--hidden`）→ 进程存在、主窗口不显示、不抢焦点、托盘存在；
4. 第二实例（已有 hidden 实例 + 再无参数启动）→ 复用现有进程、主窗口显示；
5. 重复 hidden（已有实例 + 再 `--hidden`）→ 不抢焦点、不强制显示。

生成详细测试报告。当前平台必须真实 smoke，不得伪造 PASS。其他平台标记
`PLATFORM_BLOCKED`。

---

## 5. Cross-repo CI Guide

### 问题

插件 CI 默认 checkout Host `main`，会导致 `Plugin branch + old Host CLI`
不匹配。

### 解决：精确 Host ref

插件 CI 支持以下 Host ref 来源，按优先级排序：

1. **workflow_dispatch input**：手动触发时通过 `mira_host_ref` 输入参数指定；
2. **repository variable**：`MIRA_HOST_REF` 仓库变量；
3. **PR matching branch**：如果插件 PR head branch name 在 Host 仓库存在，
   使用同名分支（保持 iteration 分支跨仓配对）；
4. **fixed compatible Host SHA**：已确认兼容的固定 SHA，不跟随浮动 `main`。

### 环境变量

| 变量             | 说明                                       |
|------------------|--------------------------------------------|
| `MIRA_HOST_REPO` | Host 仓库全名（如 `hello-yunshu/mira-mouse`）|
| `MIRA_HOST_REF`  | 仓库变量指定的 Host ref                    |

### CI Summary 输出

CI summary 必须输出：

```text
Plugin SHA
Host SHA
CLI SHA
Host ref source (input / variable / matching branch / fixed compatible Host SHA)
```

用于审计和故障排查。

### 工作分支验证

每个 Iteration 必须至少有一次：

- Host branch CI；
- Plugin branch CI；
- Plugin CI 使用匹配的 Iteration Host SHA；
- Linux/macOS/Windows matrix；
- fixture validate/test。

如果 GitHub 权限或 workflow 无法运行，记录 `MAINTAINER_BLOCKED`，
不得声称 independent CI passed。

---

## 6. Fixture Taxonomy

### 类型分类

| 类型                  | 典型字段                                      | 执行方式                                     |
|-----------------------|-----------------------------------------------|----------------------------------------------|
| captured-response     | `response`、`expected`、`parser`              | 加载 protocol package、调用真实 parser、深比较|
| snapshot contract     | `battery`、`dpiStages`、`mouseLight`、`expected layout` | 构造 snapshot、通过 normalization、验证字段结构 |
| fault/error contract  | `case`、`input`/`error`、`expected kind/status/fallback` | 调用错误分类、parser 或 normalization、验证 expected |
| setter sample         | `input`、`preReadResponse`、`expectedWrite`/`expectedRequestHead` | 构建 request、深比较、可选 readback 验证 |
| read sample           | `params`、`response`、`expectedParsed`        | 调用 parser、深比较                          |

### `hardwareOnly` 的唯一允许条件

只有以下真实情况可保留 `hardwareOnly: true`：

- 必须打开物理 HID 设备；
- 必须接收实时 interrupt；
- 必须观察真实灯效；
- 必须真实写入设备；
- 必须等待物理设备状态变化。

如果 fixture 已经包含完整 `response`/`expected`，通常不得 `hardwareOnly`。

### 最终门禁

`mira-plugin test` 必须：

```text
0 failed
0 unrecognized
0 unexplained skip
```

允许的 `hardwareOnly` skip 必须：

- 数量明确；
- 每个都有具体硬件理由；
- 输出 fixture 路径和理由；
- 不能只写"hardware verified"。

### Pre-read sentinel fixture（ITERATION-009 §11）

`preReadResponse` 不得使用全零填充作为 base。至少一组 fixture 必须在
未编辑位置放置非零 sentinel，证明 write 只修改目标字段而保留其他字节。

示例（Protocol A character light write）：

- `preReadResponse` offset 2-6 = `0xAA 0xBB 0xCC 0xDD 0xEE`（5 个 sentinel）；
- `preReadResponse` offset 8-11 = `0xDE 0xAD 0xBE 0xEF`（4 个 sentinel）；
- `expectedWrite` 保留所有 sentinel，只修改 offset 0（command）、
  offset 1（enabled param）、offset 7（checksum）；
- `expectedPreservedBytes` 显式声明保留位置。

---

## 7. AMaster Support Matrix

### 协议路径

| 协议        | 传输            | 用途                              | 写入  |
|-------------|-----------------|-----------------------------------|-------|
| Protocol A  | protocol-a      | AMaster 鼠标通用（DPI、灯光等）   | enabled |
| Protocol A  | protocol-a-receiver | AMaster 接收器灯光            | enabled |
| AM35        | am35-direct     | AM35 鼠标直连（灯光、DPI、按键等）| enabled |
| AM35        | am35-receiver   | AM35 接收器灯光                   | enabled |

### 灯光能力

| 协议        | 主颜色入口                          | 灯效 | 速度/亮度 | 接收器灯光 |
|-------------|-------------------------------------|------|-----------|------------|
| Protocol A  | `lightingRole=primary-color` field | ✓    | ✓         | ✓          |
| AM35        | `lightingRole=primary-color` field | ✓    | ✓         | ✓          |
| Logitech    | HID++ 主颜色 field                  | ✓    | ✓         | -          |
| Razer       | 若无可写颜色，不显示伪编辑入口       | -    | -         | -          |

### AM35 安静灯光状态标准化

AM35 原始输出 `mouseLightMode` / `mouseLightColor` 通过声明式 normalization
转换为统一 `mouseLighting` 对象。Host 不写 `if am35`，通过 plugin
normalization/state mapping/schema 处理。

关键规则：

- Mode 0 = 常亮（开启），不是关闭；
- Mode 2 = 中性语义；
- 开启但缺颜色时不用 `#000000` 覆盖；
- 关闭状态恢复时不猜原色；
- 保存 effect/speed/brightness/extraColor；
- 退出安静灯光后完整恢复。

### 已验证硬件

详见 [`mira-mouse-plugins/docs/hardware-evidence-matrix.md`](../mira-mouse-plugins/docs/hardware-evidence-matrix.md)。
