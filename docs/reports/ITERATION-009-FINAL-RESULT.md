# ITERATION-009 Final Result

> 核对基准：`Mira_AMaster_Engineering_Docs_v9/03-implementation-tasks/ITERATION-009_FINAL_CLOSURE.md`
>
> 核对时间：2026-07-28（Asia/Shanghai）

## 结论

**ITERATION 009 PARTIAL**

实现、本地全量门禁、可复现 Test App 和当前 macOS 实机 smoke 已收口。尚不能写
`COMPLETE`，因为两个仓库的本轮修复仍未提交/推送，Host branch CI、Plugin branch
CI 及 Linux/macOS/Windows 独立矩阵尚未在这些最终改动上运行。

## A. 两仓信息

| Repo | Branch | 当前基线 commit | 工作树 |
|---|---|---|---|
| `mira-mouse` | `work/iteration009-final-closure` | `5848a3e595437bd8b82e5f1a44c6b6ed6ae8b80f` | 含本轮未提交修复 |
| `mira-mouse-plugins` | `work/iteration009-final-closure` | `5ef79d2f883d4d353165d0e499b5edcd0ee7c1d4` | 含本轮未提交修复 |

未伪造 final commit SHA。`target/test-app/manifest.json` 记录两个基线 commit、
dirty 状态、变更文件，以及排除纯报告/smoke 驱动后的实际构建输入状态 SHA-256。

## B. 本轮补齐的缺口

### macOS LaunchAgent

- 使用真正的 plist XML parser 分类 `Missing / Valid / Stale / Invalid`。
- stale、invalid 和 legacy enabled 状态会进入修复，不再被当作“已禁用”。
- `/Volumes` 启动不会写无效 plist，也不会先删除旧可用自启。
- plist 先写临时文件并同步，再原子替换；load/verify 失败会回滚。
- `bootstrap/bootout/print` 有旧系统 fallback，所有 exit status 都被检查。
- 只有新 LaunchAgent 成功加载并复核为 Valid 后才禁用 legacy 项。

实机状态：`~/Library/LaunchAgents/Mira.plist` 指向当前 Test App 的真实 Mach-O，
参数为 `--hidden`；`launchctl print gui/501/Mira` 可见已注册，最近退出码 0。

### Fixture executor

- captured-response：调用真实 parser；`expected` 中每个字段必须存在并匹配。
  仅用于证据的 wire 字段移到 `wireEvidence`。
- snapshot：构造标准 outputs，调用
  `normalize_device_outputs_with_package`，比较电量、充电、DPI stages、
  polling rate、mouse lighting 和 receiver gradient。
- fault：每个 case 提供结构化 `input`，调用 runtime
  `classify_contract_fault`，不再只检查 result 是否在字符串白名单里。
- 新增可生成、可 CI 检查的 45 文件审计表：
  `mira-mouse-plugins/docs/ITERATION-009-FIXTURE-AUDIT.md`。

| Plugin | Passed | Skipped | Failed |
|---|---:|---:|---:|
| amaster | 70 | 0 | 0 |
| example-mock | 2 | 0 | 0 |
| logitech-hidpp | 5 | 0 | 0 |
| razer-viper | 9 | 0 | 0 |
| **Total** | **86** | **0** | **0** |

审计结果：45/45 recognized，0 hardwareOnly，0 unexplained skip。

### Latest Test App

- 固定 TEST-ONLY seed 已提交在 `tests/keys/`，对应既有测试公钥。
- release App 仅在显式 Cargo feature `test-plugin-trust` 下信任该测试根。
- 默认总是从当前 Host 源码重建 release CLI；仅显式
  `MIRA_PLUGIN_CLI` 才允许复用外部二进制。
- 所有临时 plugin packages、lock、resources、Tauri feature 都写入隔离 staging；
  源 checkout 不被测试构建污染。
- 动态发现并 validate/test/sign/verify 全部四个插件；正式生产信任库未修改。
- 测试锁项逐字段校验并从正式锁继承 `repository`；缺失运行时必填字段会在构建前失败。
- 测试二进制提供 `--test-bundled-plugins` 自检，实际读取内嵌锁与 App Resources，
  校验三个默认包的 SHA、签名、身份和 `devices.json`。
- macOS 重复构建后清理非产品 xattr，并在写 manifest 前执行严格 codesign 验证。
- 默认产出 macOS `.app`；可用 `TAURI_BUNDLE` 显式选择其他 bundle。

最终构建证据：

| Item | Value |
|---|---|
| App version | `0.9.17` |
| CLI SHA-256 | `8b85627a4faf443a0650d062a6d59e9e85d2357097c421f31d7d361840b28ace` |
| App executable | `target/test-app/cargo/release/bundle/macos/Mira.app/Contents/MacOS/mira` |
| App executable SHA-256 | `088829217910667aaf53607e62ef12304051676f039aa6577c85d9d2476409e1` |
| Manifest | `target/test-app/manifest.json` |

该 `.app` 为 ad-hoc signed Test App，未 notarize，不是生产发布包。

### Packaged smoke

最终结果：**13 PASS / 0 FAIL / 0 PLATFORM_BLOCKED**。

- 精确定位并启动打包后的 executable。
- 同一打包二进制读取内嵌锁并验证三个默认鼠标插件的资源、SHA、签名、身份和设备描述。
- 手动启动显示窗口。
- 原生关闭按钮关闭窗口后进程仍存活。
- 原生托盘菜单可重新打开窗口。
- `--hidden` 启动无可见窗口。
- 交互式第二实例复用单一进程并显示窗口。
- 重复 hidden 启动复用单一进程且不显示窗口。
- App device/UI contract fixture tests 通过。
- 最终原生界面显示 `mira.amaster`、`mira.logitech-hidpp`、
  `mira.razer-viper` 均为“签名已验证 / 默认内置”，并识别当前连接的
  `AM INFINITY MOUSE .100`。

详见 `docs/reports/ITERATION-009-SMOKE-RESULT.md`。

### Cross-repo CI

- Host ref 优先级为：dispatch input → repository variable → matching branch →
  固定兼容 Host SHA。
- fallback 不再跟随浮动 `main`，当前固定为
  `5848a3e595437bd8b82e5f1a44c6b6ed6ae8b80f`。
- CI summary 输出实际 Plugin SHA、Host SHA、Host ref source、CLI SHA-256。
- fixture 审计表加入 Plugin CI 防漂移。

## C. 本地门禁

### Host

- ESLint：通过。
- TypeScript typecheck：通过。
- Vitest：19 files / 315 tests 通过。
- Vite production build：通过。
- boundary / structured / bundled plugin contract：通过。
- `cargo fmt --all --check`：通过。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `cargo test --workspace --locked`：全部通过；runtime 中 2 个既有
  mock-HID infrastructure tests 保持 ignored。

版本门禁同时修正了一个既有矛盾：App crates 继续继承 workspace version；
`mira-plugin-cli` 作为独立发布物保留自身 SemVer，并被单独校验。

### Plugins

- validate / protocol inventory：通过。
- Node tests：61/61 通过。
- architecture lint：通过。
- fixture audit：通过。
- CLI validate/test 四个插件：86 passed / 0 skipped / 0 failed。

## D. 未完成与阻塞

| Item | Status | Reason |
|---|---|---|
| 本轮改动提交 | `MAINTAINER_BLOCKED` | 用户未要求创建 commits |
| 两仓分支推送 | `MAINTAINER_BLOCKED` | 用户未授权 push |
| Host branch GitHub CI | `MAINTAINER_BLOCKED` | 最终工作树尚未形成远端 commit |
| Plugin branch GitHub CI | `MAINTAINER_BLOCKED` | 同上 |
| Linux/Windows/macOS 独立远端矩阵 | `MAINTAINER_BLOCKED` | 必须基于推送后的精确 SHA 运行 |
| AM35 真实硬件写入矩阵 | `NOT_RUN` | 最终 App 已识别并读取当前 `AM INFINITY MOUSE .100`；本次插件列表修复未主动改写用户设备设置 |

除上述需要提交、远端 CI 或实体硬件的项目外，本次审计确认的实现缺口均已修复。
