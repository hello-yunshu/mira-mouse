<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 在 macOS 上安装

社区 DMG 发布在 [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases) 页面，文件名固定为 `Mira_macOS_<version>_aarch64.dmg`。Mira 的 macOS 版本仅支持 Apple Silicon（ARM64），不提供 Intel 构建。

## 方案 A：Homebrew（推荐）

```bash
brew tap hello-yunshu/mira
brew trust hello-yunshu/mira
brew install --cask mira
```

Homebrew 4.x 要求对第三方 tap 执行 `brew trust`，将 tap 标记为受信任来源；若不执行此步骤，`brew install --cask mira` 会以 `Refusing to load cask ... from untrusted tap` 失败。

升级遵循标准 Homebrew 流程：

```bash
brew upgrade --cask mira
```

关于 tap、未签名应用注意事项以及可用变量的详细信息，请参阅 [install-homebrew.md](install-homebrew.md)。

## 方案 B：直接下载 DMG

1. 从最新 release 下载 `Mira_macOS_<version>_aarch64.dmg`。
2. 校验 release 页面上资产旁公布的 SHA-256。
3. 挂载 DMG，将 `Mira.app` 拖入 `/Applications`。

## 首次启动警告

Mira 使用 ad-hoc 签名（`signingIdentity: "-"`）构建，**未**经过公证（notarized）。首次启动会被 Gatekeeper 拦截。如需继续，请选择以下任一方式：

- 右键点击 `Mira.app` → **打开（Open）** → 在 Gatekeeper 对话框中确认。
- 打开 **系统设置（System Settings） → 隐私与安全性（Privacy & Security）** → 在 Mira 的拦截提示旁点击 **仍要打开（Open Anyway）**。
- 在终端中执行一次：

  ```bash
  xattr -dr com.apple.quarantine /Applications/Mira.app
  ```

## 权限

启动后，如果 Mira 需要与鼠标通信，请在 **系统设置 → 隐私与安全性 → 输入监控（Input Monitoring）** 中授予 HID 访问权限。
