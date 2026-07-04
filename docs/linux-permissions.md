<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Linux 设备权限

切勿以 root 身份运行 Mira，不要使用 `chmod 666 /dev/hidraw*`，也不要为所有 HID 设备安装通配规则。Mira 仅在受信任插件声明了经 schema 校验的精确 VID、PID、usage page 和 usage 时，才会提议相应规则。规则的精确目标路径与内容必须在可选的 `pkexec` 操作之前进行预览。

AppImage 用户需显式安装或移除经过审查的规则，然后重新插拔设备并重新运行访问诊断。DEB/RPM 维护者脚本必须是幂等的，且只能移除其自身拥有的规则。导入的插件会获得独立的提案，绝不自动授予权限。
