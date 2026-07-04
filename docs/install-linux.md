<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 在 Linux 上安装

Mira 已发布 Linux AppImage 产物，可从 [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases) 下载 `Mira_Linux_<version>_amd64.AppImage`。赋予执行权限后即可运行：

```bash
chmod +x Mira_Linux_<version>_amd64.AppImage
./Mira_Linux_<version>_amd64.AppImage
```

AppImage 不会静默安装设备权限；请查阅 [Linux 权限说明](linux-permissions.md)。DEB/RPM 包目前仍为规划中的产物，未来若发布，将仅安装由锁定的内置插件生成的最小化规则。Snap、Flatpak 以及 Linux ARM64 在现阶段不属于支持目标。
