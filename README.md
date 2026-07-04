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
  <a href="#常见问题">常见问题</a> ·
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

Mira 是一个非官方的鼠标设置工具，支持 macOS、Windows 和 Linux。无需账号、无需联网、无需厂商驱动，装完即用。

- 无遥测、无账号、无广告
- 跨平台，macOS / Windows / Linux 同步更新
- 插件化架构，新设备支持通过插件扩展，不依赖主应用更新

## 功能

- **DPI 调节**：多档位 DPI，逐配置独立设置
- **回报率设置**：125 ~ 8000 Hz
- **灯光控制**：鼠标与接收器 RGB 灯效，颜色 / 效果 / 速度 / 亮度自由调
- **夜间模式**：按本地时间自动关闭和恢复灯光，跨午夜无忧
- **电池状态**：实时显示鼠标与接收器电量
- **多设备管理**：同时连接多台设备，各自独立配置
- **主题切换**：跟随系统深浅色，macOS Dock 图标同步
- **自动更新**：内置更新检查，新版本自动提醒

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

下载 `Mira_Windows_<version>_x64-setup.exe` 双击安装即可。

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

## 常见问题

- **首次打开提示"已损坏"或"无法验证开发者"？** 这是未签名应用的正常提示。macOS 用户可在"系统设置 > 隐私与安全性"点击"仍要打开"，或执行 `xattr -cr /Applications/Mira.app`。详见 [安全说明](docs/unsigned-release-security.md)。
- **支持蓝牙连接吗？** 目前仅支持 USB 直连和 2.4G 接收器，暂不支持蓝牙。
- **会收集我的数据吗？** 不会。Mira 无遥测、无账号、无联网上报，所有操作在本地完成。
- **我的鼠标不在支持列表里怎么办？** 可到 [Issues](https://github.com/hello-yunshu/mira-mouse/issues) 提交设备支持请求，或参考 [插件 SDK](docs/plugin-sdk.md) 自行适配。

## 开发

Mira 遵循一个原则：**协议归插件，界面归主应用。** 插件以签名的声明式 `.mira-plugin` 包分发，主应用负责界面、权限边界、HID 调用和更新。

插件仓库：[`hello-yunshu/mira-mouse-plugins`](https://github.com/hello-yunshu/mira-mouse-plugins)

```bash
npm install
npm run dev              # 前端开发预览
npm exec tauri dev       # 桌面开发预览
npm run lint && npm run typecheck && npm test -- --run
cargo test
```

更多文档：

- [插件包格式](docs/plugin-package-format.md) · [插件 SDK](docs/plugin-sdk.md) · [协议 DSL](docs/protocol-dsl.md)
- [插件安全模型](docs/plugin-security.md) · [威胁模型](docs/threat-model.md)
- [macOS 安装](docs/install-macos.md) · [Homebrew 安装](docs/install-homebrew.md) · [Linux 权限](docs/linux-permissions.md)

## 许可证

代码和构建定义采用 AGPL-3.0-or-later。原创文档和非商标视觉材料采用 CC-BY-SA-4.0。详见 [`LICENSE`](LICENSE)、[`NOTICE`](NOTICE) 与 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
