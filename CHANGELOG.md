<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 更新日志

## 0.9.7 - 2026-07-20

- 补全 0.6.0 至 0.9.6 的历史更新日志，统一版本号来源校验。
- 大幅扩展声明式插件 SDK 文档：补充 fields、editor、switch、zones、stageLayout、statusDisplay、stateMapping、accentSource、visibleWhen 等字段说明与示例。
- 新增英文文档目录索引（docs/en/README.md），说明翻译策略。
- 安装文档补充 Homebrew 4.x `brew trust` 步骤，Linux 安装改为 AppImage 说明。
- 精简冗余代码注释，保留关键防回归说明（chunk 边界、锁作用域、快照写入等）。
- 修复 BatteryUsage 弹窗时间范围切换区的 JSX 缩进对齐。
- 为 comment-and-doc-style、local-ai-analysis-plan、mira-material-design-guidelines 文档补充 SPDX 许可头。

## 0.9.6 - 2026-07-19

- 修复日志页样式与模态弹窗标题重复问题，恢复被误删的 overlay/log CSS。
- 完善声明式控件渲染，优化仪表盘动效与跨视图切换的过渡体验。
- 加固 mira-battery-handler 的 Cargo.lock 校验，CI 新增 `xtask handler check-lock` 防止 lockfile 漂移。

## 0.9.0 - 2026-07-18

- 新增统一的本地日志与诊断系统：记录设备 mutation、采集诊断上下文、清理告警。
- 新增统一 overlay root（Modal/Portal）层，支持堆栈跟踪；所有模态弹窗统一路由到 overlay root。
- 优化日志页与背景毛玻璃效果，修复 backdrop-filter 与 transform 冲突。
- 同步 CITATION.cff 与 ROADMAP.md 到 0.9.0。

## 0.8.6 - 2026-07-17

- 新增 demo 模式下的本地 mutation 模拟，无需真实设备即可演示配置变更流程。
- 修复 macOS 毛玻璃效果，解决 backdrop-filter 与 transform 冲突导致的局部失效。
- 修复 skipIfZero 布尔处理、接收器电量解析与弹窗样式问题。
- 升级 ESLint、typescript-eslint、actions/setup-node 等依赖。

## 0.8.3 - 2026-07-16

- 从 rill-ml v0.7.1 releases 下载预编译 rill-runtime，移除 CI 中过时的 RillML symlink 步骤。
- 将 Mira InvokeHandler 抽取为独立签名的 WASM 插件。
- 统一应用版本来源至 sync-version 脚本。
- 更新生产验证公钥。

## 0.8.1 - 2026-07-15

- 内部版本对齐与发布链调整。

## 0.8.0 - 2026-07-15

- 新增基于签名模型包的本地 AI 电量预测。
- 优化电量使用对比图表。

## 0.6.10 - 2026-07-13

- 完善设备刷新机制与电量洞察。
- 同步插件 release/v2026-07-13。

## 0.6.9 - 2026-07-13

- 修复窄屏对齐、设备切换器间距与电量图表布局。
- 电量区间切换按钮改用基于主题色的色相渐变。

## 0.6.8 - 2026-07-13

- 重构为按需刷新模型，修复 macOS vibrancy 渗透问题。

## 0.6.7 - 2026-07-12

- 新增读取计划调度器与工作流投影，提升设备读取效率。
- 新增 AM Infinity .97 RACE transport 与 motion language，解耦运行时。
- 为下载链路添加 GitHub mirror 回退以提升可靠性。
- 暴露 normalize_device_outputs_with_package 并追踪 reports_executed。

## 0.6.6 - 2026-07-12

- 优化电量图表网格、mock 数据与 X 轴刻度。

## 0.6.5 - 2026-07-11

- 重构为声明式插件架构，扩展声明式插件 SDK 并抽取插件更新器。
- 从声明式 zones 解析灯光角色，优化 UI 细节。
- 将 TypeScript 回退至 6.0.3 以保持 typescript-eslint 兼容性。
- 升级 ed25519-dalek 至 3.0.0；忽略 RUSTSEC-2024-0429（glib unsoundness）。

## 0.6.3 - 2026-07-09

- 修复电量上报并精细化灯光开关 source。
- 新增基于插件身份的历史合并与电量 UI 细节。

## 0.6.2 - 2026-07-09

- 调整电量图标生成与插件图标。
- 修复插件更新区焦点样式，与关于页更新区保持一致。

## 0.6.1 - 2026-07-09

- 重构托盘模块，修复 Protocol A 电量问题。
- 修复 Linux CI 上 macOS-only 托盘 helper 的 dead_code 警告。

## 0.6.0 - 2026-07-08

- 新增电量使用历史追踪与洞察分析。
- 优化电量使用弹窗细节。
- CI 移除 dependency-review-action，改用 Dependabot security updates。

## 0.5.7 - 2026-07-04

- 文档清理：删除面向维护者的发布/版本管理文档（zero-cost-release、plugin-publishing、plugin-versioning），保留面向用户和开发者的文档。
- README 改进：新增常见问题版块，更新设备状态为"已支持"，添加设备支持引导。
- 文档结构化：中文版作为主语言放在 docs/ 根目录，英文版放在 docs/en/。
- 删除过时文档：GOVERNANCE、SECURITY、video-script-amaster 等。
- 新增文档约定：docs/comment-and-doc-style.md。

## 0.5.3 - 2026-07-02

- 修复 Settings 中已禁用开关的显示：当选项不可配置时（例如接收器灯光不受支持），开关现在显示为关闭状态而非开启。
- 新增 `supportsMouseLighting` 检查：当设备不支持鼠标灯光写入时，鼠标灯光开关将被禁用并显示为关闭。
- 在设备读取器中加入系统唤醒检测：长轮询间隔被切分为 10 秒窗口并配合 `SystemTime` 跳变检测，使 Mac/PC 从睡眠唤醒后能快速重新枚举设备。
- 恢复 macOS Dock 图标主题切换，通过 `setApplicationIconImage` 配合适当留白的 `.icns` 资源（内容区占比 81.6%），图标不再显得比其他应用更大。
- 重新生成全平台图标：macOS `.icns` 文件包含 Apple HIG 留白，Windows/Linux/Web 资源按各自规范保持满版。

## 0.3.10 - 2026-07-01

- 修复 macOS 更新重启逻辑，改为重新打开 `.app` bundle 而非直接启动 bundle 内部可执行文件，避免自 0.3.6 左右引入的系统"选择应用打开"提示。
- 运行时保持使用 bundle 自带的 Dock 图标，避免 Mira 切到前台时图标出现视觉跳变。
- macOS 更新通知不再使用 `notify-rust` 后端，避免在更新检查与下载失败重叠时 macOS 弹出查找 `use_default` 应用的提示。
- 将"开机自启"与"登录时隐藏窗口"拆分为独立设置；仅当两者同时启用时才在后台启动 Mira。
- Windows 更新器安装改为 quiet 模式，自动更新过程保持静默。

## 0.3.7 - 2026-07-01

- 新增多设备支持，并刷新全应用的 UI 文案。
- 修复 Linux udev 热插拔监控 API，并新增本地 pre-push 守卫。
- 精化 Material Design UI，新增专门的设计指南文档，优化样式、tooltip 与仪表盘布局。
- 简化 macOS Dock 图标主题切换：Dock 图标改由打包的 `icon.icns`（配对 `icon-dark.icns`）提供，移除运行时 `setApplicationIconImage` 覆盖，消除视觉差异。
- 清理仓库内容：停止跟踪 Tauri 生成的 schema 与本地测试配置，加固 `.gitignore`，刷新参考文档。

## 0.3.1 - 2026-06-28

- 为 Windows NSIS 安装器与卸载器打造 Mira 液态玻璃风格：配套 header/sidebar 资产、共享应用图标、简体中文 + 英文支持，以及独立的 `Mira` 开始菜单文件夹。
- 新增可复现的 Windows 安装器资产生成器与 npm 脚本，使 NSIS 位图资产与 Mira 默认玻璃主题保持一致。
- 微调 macOS DMG 背景亮度，使其更贴合当前安装器视觉方向。
- 在打开外部链接时隐藏 Windows 辅助命令窗口，并将 Windows 系统主题检测改为直接读取注册表。

## 0.3.0 - 2026-06-28

- 移除 Windows 系统标题栏（最小化/最大化/关闭），改为由前端渲染的自定义 `WindowsWindowControls` 组件（最小化 + 关闭），并配套 `.windows-window-controls` 样式。
- 在 Windows 上隐藏托盘电池百分比与接收器电池开关，因为 Windows 系统托盘无法在图标旁渲染文字。
- 在 `DpiEditModal` 中加入 DPI 步进不匹配守卫：当输入值不在声明的步进网格上时，Apply 按钮保持禁用。
- 当写入操作因 "is not available on this device" 失败时，弹出友好的"操作不可用"通知，提示用户关闭可能占用设备的官方软件（Logi Options+、G HUB、AMaster）。
- 扩展 `plugin_capabilities`，从 `protocol/workflows.json` 写入输入中丰富能力元数据（DPI min/max/step 与回报率 `allowed` 值），插件不再需要在 `plugin.json` 中硬编码数值范围。新增 Rust 单元测试覆盖 DPI 范围丰富化与 Select 选项丰富化（基于当前可写 mutation）。
- 使 `pluginRange` 在类型化 `metadata.range` 之外，也接受遗留的顶层 `min/max/step` 元数据；使 `pluginOptions` 在保留插件标签的同时，通过插件声明的选项列表过滤设备回报的回报率。
- Windows 背景切换为 Acrylic 优先（半透明磨砂玻璃），Mica 作为回退；在 Windows 启动时应用 `set_decorations(false)` 移除系统 chrome。
- 通过 `--floating-glass-bg`、`--floating-glass-blur`、`--floating-glass-shadow` CSS 变量统一浮空玻璃样式，覆盖 tooltip、通知、编辑弹窗与设备详情面板；提高 `--glass-popup` 不透明度以增强可读性。
- 将 `nav-links` 重定位为 `top-nav` 的绝对定位兄弟元素，按平台右对齐（Windows：通过 `right: max(16px, calc(50% - 234px))` 对齐到仪表盘右边缘；macOS：固定 `right: 16px`），并为 `<option>` 添加浅色/深色主题背景色。
- 加固 `prefers-reduced-transparency: reduce`：禁用所有 backdrop 滤镜，用纯色背景替换玻璃弹窗。
- 在 CI 流水线中加入 `Clean stale bundle outputs` 步骤，在每次 Tauri 构建前清理缓存的 bundle 产物，防止过期文件混入新发布。
- 添加 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`，在 Windows release 构建中抑制命令行窗口。

## 0.2.0 - 2026-06-27

- 首次公开发布品牌中立的 Mira 应用外壳，包含声明式插件契约、HID++ 与 AMaster 插件 manifest，以及有界 workflow 运行时。
- 未声明硬件兼容性或签名发布。
