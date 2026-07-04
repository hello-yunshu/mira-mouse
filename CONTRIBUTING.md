<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 贡献指南

欢迎为 Mira 贡献代码与文档。在开始之前，请先阅读本指南与 [`docs/comment-and-doc-style.md`](docs/comment-and-doc-style.md) 中的备注与文档约定。

## 一、仓库边界

Mira 严格区分主应用仓库与插件仓库：

- **主应用仓库**（本仓库）：负责界面、权限边界、HID 调用、更新与运行时。
- **插件仓库** [`hello-yunshu/mira-mouse-plugins`](https://github.com/hello-yunshu/mira-mouse-plugins)：负责设备事实、VID/PID 值、协议字节、型号分支与品牌 fixture。

设备事实、VID/PID 值、协议字节、型号分支与品牌 fixture 必须放在插件仓库，**不要**提交到主应用仓库。

## 二、不可提交的内容

请勿在任意仓库中包含以下内容：

- 逆向工程源码
- 厂商资产（固件、二进制、图标、截图、字体、应用资源）
- 设备稳定标识符（完整序列号、唯一设备标识）
- 凭证与签名密钥

贡献者在仓库声明的许可证下提交内容。

## 三、开发环境搭建

### 3.1 前置依赖

- **Rust**（stable 工具链，推荐通过 [rustup](https://rustup.rs/) 安装）
- **Node.js** 20+ 与 npm
- **系统依赖**：
  - macOS：Xcode Command Line Tools（`xcode-select --install`）
  - Windows：[WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) 与 MSVC 构建工具
  - Linux：`webkit2gtk-4.1`、`libgtk-3`、`libappindicator3`、`librsvg`、`patchelf` 等 Tauri 系统依赖

### 3.2 安装与运行

```bash
git clone https://github.com/hello-yunshu/mira-mouse.git
cd mira-mouse
npm install
npm run dev              # 前端开发预览（Vite）
npm exec tauri dev       # 桌面开发预览（Tauri 完整运行时）
```

## 四、代码风格

### 4.1 Rust

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

### 4.2 TypeScript / React

```bash
npm run lint          # eslint
npm run typecheck     # tsc 类型检查
npm test -- --run     # vitest 单元测试
```

### 4.3 边界与结构检查

主应用强制仓库边界，提交前请运行：

```bash
npm run check:quick       # 快速本地检查（pre-push 钩子）
npm run check:boundaries  # 边界扫描
npm run check:structured  # 结构化检查 + 版本来源检查
npm run check:ci          # 本地 CI 等价流程
```

核心改动需通过：格式化、lint、测试、边界扫描、evidence-label 更新。

## 五、备注与文档约定

详见 [`docs/comment-and-doc-style.md`](docs/comment-and-doc-style.md)。要点：

- 备注解释 **WHY** 而非 WHAT，禁止复述代码行为。
- 文档以中文为主，英文为辅；代码、命令、技术术语保留英文原词。
- 文档内容须与项目当前状态一致，禁止保留过时表述。
- 文档内 `docs/*.md` 链接须指向真实存在的文件。

## 六、提交规范

- 提交信息使用简洁描述，说明"为什么"而非仅"做了什么"。
- 一个提交聚焦一件事，避免混合无关改动。
- 版本号遵循项目规则：不含 `4` 与 `11` 数字（参见 CHANGELOG 既有版本）。
- 不要在提交中包含 secrets、凭证或签名密钥。

## 七、行为准则

参与本项目即表示你同意遵守 [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)。
