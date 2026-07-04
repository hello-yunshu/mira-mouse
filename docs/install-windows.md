<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 在 Windows 上安装

从 release 资产中下载 `Mira_Windows_<version>_x64-setup.exe` 社区安装包，在运行前校验其 SHA-256。该安装包使用 Mira 品牌的 NSIS 界面，为当前用户安装，并在 `Mira` 开始菜单文件夹中放置快捷方式。

由于没有受信任的代码签名（code signing），Windows 可能会显示 SmartScreen 或 Unknown Publisher 提示。Mira 使用系统 WebView2 运行时，不声明支持 Windows ARM64。
