<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 更新日志

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
