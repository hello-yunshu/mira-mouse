<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<p align="center">
  <img src="public/app-icon.png" width="128" height="128" alt="Mira logo">
</p>

<h1 align="center">Mira</h1>

<p align="center">
  一款现代、安静、以插件驱动的鼠标设置客户端。<br>
  支持 macOS、Windows 与 Linux，无需账号、无需云端服务、无需厂商驱动，装完即用。
</p>

<p align="center">
  <a href="https://github.com/hello-yunshu/mira-mouse/releases"><img alt="Release" src="https://img.shields.io/github/v/release/hello-yunshu/mira-mouse?style=flat-square&color=7C3AED"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-24C8DB?style=flat-square">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square">
  <img alt="React" src="https://img.shields.io/badge/React-19-61DAFB?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-runtime-000000?style=flat-square">
  <img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-7C3AED?style=flat-square">
</p>

<p align="center">
  <a href="#简介">简介</a> ·
  <a href="#功能">功能</a> ·
  <a href="#截图">截图</a> ·
  <a href="#安装">安装</a> ·
  <a href="#支持设备">支持设备</a> ·
  <a href="#架构">架构</a> ·
  <a href="#常见问题">常见问题</a> ·
  <a href="#开发">开发</a> ·
  <a href="README.en.md">English</a>
</p>

---

## 简介

Mira 是一个非官方的鼠标设置工具，支持 macOS、Windows 和 Linux。无需账号、无需云端服务、无需厂商驱动，装完即用；网络仅用于用户选择的应用、插件与本地 AI 组件更新。

## 特点

### 插件驱动，声明式协议

Mira 不硬编码任何设备逻辑。每款鼠标的支持以签名插件包形式独立分发，新增设备无需更新主应用。插件只描述设备事实与协议工作流，不包含可执行代码，安全边界清晰可审计。

### 完全本地，零数据收集

无遥测、无账号、无联网上报。所有设置与设备交互在本地完成，不向任何服务器发送数据。唯一网络请求是更新检查，且可关闭。

### 跨平台，同步发布

macOS、Windows、Linux 三端共享同一套核心代码与插件生态，版本同步发布。无论在哪个平台，体验一致。

### 有界协议，行为可预测

协议 DSL 默认上限 64 步、16 次读、总延迟 2 秒，不存在表达式求值、文件系统、网络或任意循环。每个工作流可被 Fixture 测试，行为可预测、可审计。

### 本地 AI 续航预测

基于设备上下文（DPI、回报率、灯光模式）的 9 维特征向量，在本地完成续航估算。无需上传数据，模型随主程序分发。

### 装完即用

无需厂商驱动、无需注册账号、无需联网。下载安装后即可使用。

## 功能

| 类别 | 能力 |
|---|---|
| **DPI 调节** | 多档位 DPI，逐配置独立设置 |
| **回报率** | 125 ~ 8000 Hz，按设备能力动态读取 |
| **灯光控制** | 鼠标与接收器 RGB 灯效，颜色 / 效果 / 速度 / 亮度自由调节 |
| **夜间模式** | 按本地时间自动关闭和恢复灯光，支持跨午夜 |
| **电池状态** | 实时显示鼠标与接收器电量，托盘图标随电量变化 |
| **多设备管理** | 同时连接多台设备，各自独立配置 |
| **主题切换** | 跟随系统深浅色，macOS Dock 图标同步切换 |
| **自动更新** | 内置更新检查，新版本自动提醒 |
| **本地 AI 续航预测** | 基于设备上下文（DPI、回报率、灯光）的本地续航估算 |

## 截图

<!-- 截图待补充：建议提供以下场景的截图，放置于 docs/screenshots/ 目录
- 主仪表盘（深色 / 浅色各一张）
- DPI 调节面板
- 灯光控制面板
- 电池状态与托盘图标
- 多设备切换
-->

> 截图即将补充。

## 安装

### macOS

推荐 Homebrew：

```bash
brew tap hello-yunshu/mira
brew trust hello-yunshu/mira
brew install --cask mira
```

也可直接下载 DMG：[macOS 安装说明](docs/install-macos.md) · [Homebrew 安装说明](docs/install-homebrew.md)

### Windows

下载 `Mira_Windows_<version>_x64-setup.exe`，双击安装即可。

### Linux

下载 `Mira_Linux_<version>_amd64.AppImage`，赋权后运行：

```bash
chmod +x Mira_Linux_*_amd64.AppImage
./Mira_Linux_*_amd64.AppImage
```

所有产物发布在 [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases)。

> 未签名社区包首次启动会触发 Gatekeeper 或 SmartScreen 提示，属正常现象。各发布均附带 SHA-256 校验值，详见 [安全说明](docs/unsigned-release-security.md)。

## 支持设备

| 品牌 | 协议 | 连接方式 | 状态 |
|---|---|---|---|
| AMaster / 怒喵 | Protocol A | USB / 2.4G 接收器 | 已支持 |
| Logitech | HID++ 2.0 | USB / 2.4G 接收器 | 已支持 |

Mira 未获得任何设备厂商授权、认可或赞助。厂商名称仅用于说明兼容性。

想让自己的设备被支持？查看 [插件 SDK](docs/plugin-sdk.md) 或到 [GitHub Issues](https://github.com/hello-yunshu/mira-mouse/issues) 提交设备支持请求。

## 架构

Mira 遵循一个原则：**协议归插件，界面归主应用。**

```
┌─────────────────────────────────────────────────┐
│                  Mira 主应用                     │
│  ┌───────────┐  ┌──────────┐  ┌──────────────┐  │
│  │  界面层    │  │ 权限边界  │  │  HID 调用    │  │
│  │  (React)  │  │  与签名   │  │  与更新      │  │
│  └───────────┘  └──────────┘  └──────────────┘  │
│                        │                         │
│            ┌───────────┴───────────┐             │
│            │   插件运行时（有界）    │             │
│            └───────────┬───────────┘             │
└────────────────────────┼────────────────────────┘
                         │ 签名的声明式 .mira-plugin 包
         ┌───────────────┴───────────────┐
         │                               │
  ┌──────┴──────┐                ┌───────┴──────┐
  │  AMaster    │                │   Logitech   │
  │  插件       │                │   插件       │
  └─────────────┘                └──────────────┘
```

- **主应用**负责界面渲染、权限边界、HID 调用与自动更新。
- **插件**以签名的声明式 `.mira-plugin` 包分发，只描述设备事实与协议工作流，不包含可执行代码。
- **协议 DSL** 有界且可测试：默认上限 64 步、16 次读、总延迟 2 秒，不存在表达式求值、文件系统、网络或任意循环。

更多架构与安全细节：[插件 SDK](docs/plugin-sdk.md) · [协议 DSL](docs/protocol-dsl.md) · [插件安全模型](docs/plugin-security.md) · [威胁模型](docs/threat-model.md)

插件仓库：[`hello-yunshu/mira-mouse-plugins`](https://github.com/hello-yunshu/mira-mouse-plugins)

## 常见问题

<details>
<summary><b>首次打开提示"已损坏"或"无法验证开发者"？</b></summary>

这是未签名应用的正常提示。macOS 用户可在"系统设置 > 隐私与安全性"点击"仍要打开"，或执行：

```bash
xattr -cr /Applications/Mira.app
```

详见 [安全说明](docs/unsigned-release-security.md)。

</details>

<details>
<summary><b>支持蓝牙连接吗？</b></summary>

目前仅支持 USB 直连和 2.4G 接收器，暂不支持蓝牙。

</details>

<details>
<summary><b>会收集我的数据吗？</b></summary>

不会。Mira 无遥测、无账号、无联网上报，所有操作在本地完成。

</details>

<details>
<summary><b>我的鼠标不在支持列表里怎么办？</b></summary>

可到 [Issues](https://github.com/hello-yunshu/mira-mouse/issues) 提交设备支持请求，或参考 [插件 SDK](docs/plugin-sdk.md) 自行适配。

</details>

<details>
<summary><b>提示设备被占用（0xE00002C5）？</b></summary>

官方配置工具（如 AMasterLauncher、Logi Options+、G HUB）会独占 HID 设备。请先关闭这些工具再使用 Mira。

</details>

## 开发

```bash
git clone https://github.com/hello-yunshu/mira-mouse.git
cd mira-mouse
npm install
npm run dev              # 前端开发预览（Vite）
npm run sidecar:build    # 从相邻的 RillML 0.7 源码构建通用 rill-runtime sidecar
npm exec tauri dev       # 桌面开发预览（Tauri 完整运行时）
```

代码质量检查：

```bash
npm run lint && npm run typecheck && npm test -- --run   # 前端
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test  # Rust
npm run check:boundaries  # 仓库边界扫描
npm run check:ci          # 本地 CI 等价流程
```

贡献指南详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

### 文档索引

| 文档 | 说明 |
|---|---|
| [插件包格式](docs/plugin-package-format.md) | `.mira-plugin` 包结构与 manifest 规范 |
| [插件 SDK](docs/plugin-sdk.md) | capability 声明、placement、metadata 契约 |
| [协议 DSL](docs/protocol-dsl.md) | 有界工作流语法与上限 |
| [插件安全模型](docs/plugin-security.md) | 签名、信任根、权限边界 |
| [插件测试](docs/plugin-testing.md) | Fixture 证据与测试约定 |
| [威胁模型](docs/threat-model.md) | 资产、不可信输入与控制措施 |
| [本地 AI Runtime](docs/local-ai-analysis-plan.md) | Rill runtime、模型与 WASM handler 的边界和发布链 |
| [macOS 安装](docs/install-macos.md) | DMG 与 Gatekeeper 说明 |
| [Homebrew 安装](docs/install-homebrew.md) | tap 变量与升级流程 |
| [Linux 权限](docs/linux-permissions.md) | udev 规则与热插拔 |
| [Windows 安装](docs/install-windows.md) | NSIS 安装器说明 |
| [许可证说明](docs/license-notes.md) | AGPL 与 CC-BY-SA 适用范围 |
| [安全说明](docs/unsigned-release-security.md) | 未签名发布的安全考量 |
| [备注与文档约定](docs/comment-and-doc-style.md) | 代码注释与文档撰写规范 |

## 项目状态

Mira 处于活跃开发中。

- 已发布 macOS、Windows、Linux 三端安装包
- 已支持 AMaster（Protocol A）与 Logitech（HID++ 2.0）两套协议
- 本地 AI 续航预测已集成，基于设备上下文的 9 维特征向量
- 路线图详见 [ROADMAP.md](ROADMAP.md)

## 许可证

代码和构建定义采用 AGPL-3.0-or-later。原创文档和非商标视觉材料采用 CC-BY-SA-4.0。详见 [`LICENSE`](LICENSE)、[`NOTICE`](NOTICE) 与 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。

Mira 名称与徽标受 [TRADEMARKS.md](TRADEMARKS.md) 约束。
