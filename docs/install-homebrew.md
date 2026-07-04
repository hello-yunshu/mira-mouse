<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 通过 Homebrew 安装（macOS）

Mira 通过自托管的 tap [`hello-yunshu/homebrew-mira`](https://github.com/hello-yunshu/homebrew-mira) 以 Homebrew Cask 形式分发。该 Cask 封装了主仓库 GitHub Releases 页面上发布的未签名社区 DMG。

## 安装

```bash
brew tap hello-yunshu/mira
brew trust hello-yunshu/mira
brew install --cask mira
```

Homebrew 4.x 要求对第三方 tap 执行 `brew trust`。该命令将 tap 标记为受信任来源，以便加载其 Cask。若不执行此步骤，`brew install --cask mira` 会以 `Refusing to load cask ... from untrusted tap` 失败。

`brew install --cask mira` 会挂载 DMG，将 `Mira.app` 拷贝到 `/Applications`，并提示下方所述的未签名应用注意事项。

## 升级

```bash
brew update
brew upgrade --cask mira
```

CI 流水线会在每次发布后几分钟内将新的 `Casks/mira.rb` 推送到 tap 仓库，因此 `brew upgrade` 无需人工干预即可获取最新版本。

## 卸载

```bash
brew uninstall --cask mira
brew untap hello-yunshu/mira
```

## 首次启动警告（未签名且未经公证）

Mira 使用 ad-hoc 签名（`signingIdentity: "-"`）构建，**未**经过公证（notarized）。Homebrew 在安装应用时会设置 macOS 隔离属性（quarantine attribute），因此首次启动会被 Gatekeeper 拦截。如需继续，请选择以下任一方式：

- 右键点击 `Mira.app` → **打开（Open）** → 在 Gatekeeper 对话框中确认。
- 打开 **系统设置（System Settings） → 隐私与安全性（Privacy & Security）** → 在 Mira 的拦截提示旁点击 **仍要打开（Open Anyway）**。
- 在终端中执行一次：

  ```bash
  xattr -dr com.apple.quarantine /Applications/Mira.app
  ```

你也可以在安装时传入 `--no-quarantine` 以彻底跳过该属性（仅在信任来源时使用）：

```bash
brew install --cask --no-quarantine mira
```

## 校验 SHA-256

Cask 锁定了 `Mira_macOS_<version>_universal.dmg` 的 SHA-256。Homebrew 会在安装时自动校验。如需手动核对：

```bash
brew info --cask mira
shasum -a 256 /Applications/Mira.app/..  # 与 brew info 输出的值对比
```

也可以与 [release 页面](https://github.com/hello-yunshu/mira-mouse/releases) 上资产旁公布的校验值进行对比。

## HID 权限

启动后，如果 Mira 需要与鼠标通信，请在 **系统设置 → 隐私与安全性 → 输入监控（Input Monitoring）** 中授予 HID 访问权限。

## tap 如何保持同步

[`.github/workflows/pipeline.yml`](../.github/workflows/pipeline.yml) 中的 `homebrew-tap` 任务在每次成功发布后运行。它会下载由 `release-publish` 任务发布的 DMG，计算其 SHA-256，使用新的 `version` 和 `sha256` 渲染 [`homebrew/Casks/mira.rb`](../homebrew/Casks/mira.rb)，并将结果推送到 `hello-yunshu/homebrew-mira`。

该任务使用 `HOMEBREW_TAP_TOKEN` 仓库密钥进行认证，该密钥必须是具有 `hello-yunshu/homebrew-mira` 上 `repo` 权限的 Personal Access Token（classic）。如果密钥缺失，该任务会被跳过，tap 不会更新，直到下一次配置了该密钥的发布。

## 手动更新 tap（维护者）

如果 CI 任务不可用，维护者可以手动更新 tap：

```bash
git clone https://github.com/hello-yunshu/homebrew-mira.git
cd homebrew-mira
VERSION=0.5.2  # 替换为目标版本
SHA256=$(curl -sSL "https://github.com/hello-yunshu/mira-mouse/releases/download/app/v${VERSION}/Mira_macOS_${VERSION}_universal.dmg" | shasum -a 256 | awk '{print $1}')
sed -i.bak -e "s/^  version .*/  version \"${VERSION}\"/" \
           -e "s/^  sha256 .*/  sha256 \"${SHA256}\"/" Casks/mira.rb
rm Casks/mira.rb.bak
git commit -am "Bump mira to ${VERSION}"
git push
```
