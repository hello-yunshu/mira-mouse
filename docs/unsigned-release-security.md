<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 未签名社区发布版 (Unsigned Community Releases)

`unsigned-community` 表示平台尚未建立可信的开发者身份。它并不意味着已公证 (notarized)、Windows 受信任、GPG 签名或更新器签名 (updater-signed)。

仅从已配置的官方 GitHub Release 下载，核验单独发布的 SHA-256，并查看首次启动警告 (first-launch warning)。

插件包独立使用 Ed25519 签名，并由应用程序根据固定的公钥 (pinned public key) 进行验证。该签名独立于平台级别的安装程序警告。
