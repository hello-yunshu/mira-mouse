<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<p align="center">
  <img src="public/app-icon.png" width="96" height="96" alt="Mira logo">
</p>

<h1 align="center">Mira</h1>

<p align="center">
  一款现代、安静、以插件驱动的鼠标设置客户端。
</p>

<p align="center">
  <a href="#功能">功能</a> ·
  <a href="#安装">安装</a> ·
  <a href="#支持设备">支持设备</a> ·
  <a href="#开发">开发</a> ·
  <a href="README.en.md">English</a>
</p>

<p align="center">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square">
  <img alt="React" src="https://img.shields.io/badge/React-19-61DAFB?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-runtime-000000?style=flat-square">
  <img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-7C3AED?style=flat-square">
</p>

## 简介

Mira 是一个非官方、重视隐私的鼠标设置客户端，支持 macOS、Windows 和 Linux。通过插件驱动的架构适配不同品牌的鼠标，提供 DPI 调节、灯光控制、回报率设置等功能。

- 无遥测、无账号、无广告
- 跨平台桌面应用
- 设备协议与界面解耦，插件可独立扩展

## 功能

- **DPI 调节**：多档位 DPI 设置，支持 per-profile 配置
- **回报率设置**：125 / 250 / 500 / 1000 / 2000 / 4000 / 8000 Hz
- **灯光控制**：鼠标与接收器 RGB 灯效，支持颜色、效果、速度、亮度
- **夜间模式**：基于本地时间自动关闭/恢复灯光，跨午夜时段支持
- **电池状态**：实时显示鼠标与接收器电量
- **多设备管理**：同时连接多台设备，独立配置
- **主题切换**：跟随系统深浅色主题，Dock 图标同步切换
- **自动更新**：内置更新检查，无需手动下载

## 安装

### macOS

推荐使用 Homebrew：

```bash
brew tap hello-yunshu/mira
brew trust hello-yunshu/mira
brew install --cask mira
```

也可直接下载 DMG，详见 [macOS 安装说明](docs/install-macos.md) 与 [Homebrew 安装说明](docs/install-homebrew.md)。

### Windows

下载 `Mira_Windows_<version>_x64-setup.exe` 安装包运行即可。

### Linux

下载 `Mira_Linux_<version>_amd64.AppImage`，赋予执行权限后运行：

```bash
chmod +x Mira_Linux_*_amd64.AppImage
./Mira_Linux_*_amd64.AppImage
```

所有平台产物均发布在 [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases)。

> 未签名社区包会触发 Gatekeeper 或 SmartScreen 提示，发布时同时提供 SHA-256 校验值。详见 [安全说明](docs/unsigned-release-security.md)。

## 支持设备

| 品牌 | 协议 | 连接方式 | 状态 |
|---|---|---|---|
| AMaster / 怒喵 | Protocol A | USB / 2.4G 接收器 | 硬件验证中 |
| Logitech | HID++ 2.0 | USB / 2.4G 接收器 | 硬件验证中 |

Mira 未获得任何设备厂商授权、认可或赞助。厂商名称仅用于说明兼容性。

## 开发

Mira 遵循一个原则：**协议归插件，界面归主应用。** 插件以签名的声明式 `.mira-plugin` 包分发，主应用负责界面、权限边界、HID 调用和更新。

插件仓库：[`hello-yunshu/mira-mouse-plugins`](https://github.com/hello-yunshu/mira-mouse-plugins)

开发环境与命令：

```bash
npm install
npm run dev              # 前端开发预览
npm exec tauri dev       # 桌面开发预览
npm run lint && npm run typecheck && npm test -- --run
cargo test
```

更多文档：

- [插件包格式](docs/plugin-package-format.md) · [插件 SDK](docs/plugin-sdk.md) · [协议 DSL](docs/protocol-dsl.md)
- [插件安全模型](docs/plugin-security.md) · [威胁模型](docs/threat-model.md) · [安全政策](SECURITY.md)
- [macOS 安装](docs/install-macos.md) · [Homebrew 安装](docs/install-homebrew.md)

## 许可证

代码和构建定义采用 AGPL-3.0-or-later。原创文档和非商标视觉材料采用 CC-BY-SA-4.0。详见 [`LICENSE`](LICENSE)、[`NOTICE`](NOTICE) 和 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
