<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 支持

本文件说明如何获取 Mira 的帮助、反馈问题与请求设备支持。

## 反馈渠道

请通过 [GitHub Issues](https://github.com/hello-yunshu/mira-mouse/issues) 反馈问题。提交前请先搜索已有 issue，避免重复。

| 反馈类型 | 模板 | 适用场景 |
|---|---|---|
| Bug 报告 | [bug.yml](.github/ISSUE_TEMPLATE/bug.yml) | 功能异常、崩溃、行为不符合预期 |
| 功能建议 | [feature.yml](.github/ISSUE_TEMPLATE/feature.yml) | 新功能想法、体验改进建议 |
| 设备支持请求 | [device-support.yml](.github/ISSUE_TEMPLATE/device-support.yml) | 希望支持某款尚未覆盖的鼠标 |

## Bug 报告清单

提交 Bug 时请包含以下信息，以便快速定位问题：

- **Mira 版本号**：在"关于"页面或 `Mira > 关于 Mira` 中查看
- **操作系统**：macOS / Windows / Linux 及具体版本
- **连接方式**：USB 直连 或 2.4G 接收器
- **设备型号**：鼠标品牌与具体型号
- **复现步骤**：导致问题的具体操作序列
- **预期与实际行为**：期望发生什么，实际发生了什么
- **已脱敏的诊断信息**：如有错误提示，请附上文字内容

**切勿**公开完整的设备序列号或唯一设备标识。

## 常见问题

大多数常见问题已在 [README 常见问题](README.md#常见问题) 中解答，包括：

- 首次启动提示"已损坏"或"无法验证开发者"
- 蓝牙连接支持情况
- 数据收集政策
- 设备不在支持列表
- 设备被占用（0xE00002C5）

提交 issue 前请先确认问题不在常见问题列表中。

## 设备支持请求

如果 Mira 尚未支持你的鼠标，请提交设备支持请求。提交时请提供：

- 设备品牌与型号
- 连接方式（USB / 2.4G 接收器 / 蓝牙）
- 是否有官方配置工具
- 如方便，提供设备的 USB VID/PID（可在系统信息中查看）

设备支持依赖社区贡献。如你具备开发能力，可参考 [插件 SDK](docs/plugin-sdk.md) 自行适配。

## 安全漏洞报告

如发现安全漏洞，请**不要**在公开 issue 中提交。请通过仓库所有者已验证的 GitHub 联系方式私下报告，并附上：

- 漏洞描述与影响范围
- 复现步骤
- 建议的修复方向（如有）

维护者会在确认后尽快回应并协调修复。详见 [威胁模型](docs/threat-model.md) 了解项目的安全边界。

## 诊断信息

如需提供更详细的诊断信息，可运行以下命令（按平台）：

### macOS

```bash
system_profiler SPUSBDataType 2>/dev/null | grep -A 5 -i "mouse"
```

### Linux

```bash
lsusb 2>/dev/null | grep -i mouse
```

### Windows

在设备管理器中查看"鼠标和其他指针设备"，记录设备属性中的硬件 ID。

提交时请脱敏所有稳定标识符（如序列号），仅保留 VID/PID 与设备名称。
