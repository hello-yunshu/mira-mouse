<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<p align="center">
  <img src="public/app-icon.png" width="96" height="96" alt="Mira logo">
</p>

<h1 align="center">Mira</h1>

<p align="center">
  一款现代、安静、以插件驱动的鼠标设置客户端。
</p>

<p align="center">
  <a href="#特性">特性</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="#插件体系">插件体系</a> ·
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

Mira 是一个非官方、重视隐私的鼠标设置客户端，目标平台包括 macOS、Windows 和 Linux。它把设备协议放进签名的声明式 `.mira-plugin` 包里，主应用只负责稳定的界面框架、权限边界、HID 调用、主题、设置、诊断和更新。

这意味着：插件可以声明设备、协议、字段、能力和有界写入；插件不能执行原生代码、脚本、远程网页或任意 WASM。界面不是为某一个品牌硬编码的面板，而是根据插件声明的能力渲染。

## 特性

- **插件驱动的设备支持**：设备匹配、协议命令、解析器、读写流程和 UI 能力都由插件声明。
- **有边界的写入**：写入 mutation 需要声明输入范围、预读、未知字段保留策略和回读断言。
- **隐私优先**：无遥测、无账号、无广告、无常驻网络服务；诊断导出会预览并脱敏稳定标识。
- **跨平台桌面体验**：Tauri 2 + React 19，目标产物覆盖 DMG、NSIS、AppImage、Deb、RPM。
- **可审计的插件包**：插件包有锁文件、哈希、签名校验和 bundle 策略。
- **现代但克制的界面**：状态、DPI、灯光、配置控制等区域由插件能力驱动，主应用保持一致的交互骨架。

## 当前状态

Mira 仍处于预发布阶段。仓库里已经有前端、运行时、插件包加载、声明式协议执行和测试体系，但设备兼容性必须以硬件证据和插件锁定包为准。

当前默认资源中包含：

| 插件 | 作用 | 状态 |
|---|---|---|
| `mira.amaster` | AMaster / 怒喵兼容设备，Protocol A 与 AM35 研究路径 | 硬件验证中，默认打包 |
| `mira.logitech-hidpp` | Logitech HID++ 2.0 设备，特性发现、DPI、回报率、配置和灯光能力读取等 | 硬件验证中，默认打包 |
| `mira.example-mock` | 运行时和 UI 的示例插件 | 测试用途，不默认打包 |

Mira 未获得任何设备厂商授权、认可或赞助。厂商名称只用于说明兼容性研究。

## 快速开始

```bash
npm install
npm run typecheck
npm test -- --run
npm run build
```

开发预览：

```bash
npm run dev
```

完整桌面开发路径通常通过 Tauri 启动：

```bash
npm exec tauri dev
```

Vite 开发服务固定在 `http://localhost:1420`，Tauri 配置位于 [`src-tauri/tauri.conf.json`](src-tauri/tauri.conf.json)。

## 插件体系

Mira 的核心约束是：**协议归插件，界面归主应用。**

插件仓库位于 [`hello-yunshu/mira-mouse-plugins`](https://github.com/hello-yunshu/mira-mouse-plugins)。每个插件通常包含：

```text
plugin.json
devices.json
capabilities.json
protocol/commands.json
protocol/parsers.json
protocol/transports.json
protocol/workflows.json
```

主应用通过 [`plugins.lock.json`](plugins.lock.json) 锁定要打包的插件、版本、哈希、发布 key 和资源路径。运行时负责加载签名包、执行声明式协议、归一化快照，并把可写 mutation 暴露给前端。

更多细节：

- [插件包格式](docs/plugin-package-format.md)
- [插件 SDK](docs/plugin-sdk.md)
- [协议 DSL](docs/protocol-dsl.md)
- [插件安全模型](docs/plugin-security.md)
- [插件适配路线图](docs/plugin-adaptation-roadmap.md)

## 项目结构

```text
src/                         React 前端、主题、国际化、设备 UI
src-tauri/                   Tauri 壳、系统集成、插件资源
crates/mira-plugin-runtime/  声明式协议运行时
crates/mira-plugin-api/      插件 API 类型
crates/mira-core/            共享核心类型
docs/                        安全、插件、发布、适配与验证文档
schemas/                     结构化配置 schema
scripts/                     校验、图标、打包辅助脚本
```

## 开发

常用命令：

```bash
npm run lint
npm run typecheck
npm test -- --run
npm run build
npm run check:boundaries
npm run check:structured
cargo test
```

如果改动涉及插件运行时或真实 HID 路径，优先跑：

```bash
cargo run -p mira-plugin-runtime --example enumerate_hid
```

如果改动涉及插件仓库，请在插件仓库中同时运行：

```bash
npm run validate
npm test
```

## 发布与安全

社区下载使用稳定命名，资产发布在 [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases)：

- macOS: `Mira_macOS_<version>_universal.dmg`
- Windows: `Mira_Windows_<version>_x64-setup.exe`
- Linux: `Mira_Linux_<version>_amd64.AppImage`

### macOS 安装

推荐使用 Homebrew：

```bash
brew tap hello-yunshu/mira
brew install --cask mira
```

也可直接下载 DMG，详见 [macOS 安装说明](docs/install-macos.md) 与 [Homebrew 安装说明](docs/install-homebrew.md)。

未签名社区包会触发 Gatekeeper 或 SmartScreen 提示，发布时同时提供 SHA-256。参见：

- [未签名发布安全说明](docs/unsigned-release-security.md)
- [零成本发布指南](docs/zero-cost-release.md)
- [威胁模型](docs/threat-model.md)
- [安全政策](SECURITY.md)

## 许可证

代码和构建定义采用 AGPL-3.0-or-later。原创文档和非商标视觉材料采用 CC-BY-SA-4.0。详见 [`LICENSE`](LICENSE)、[`NOTICE`](NOTICE) 和 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
