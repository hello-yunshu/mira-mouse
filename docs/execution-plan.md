# Mira 执行计划（当前会话）

## 目标

在本会话内完成 AMaster / 怒喵兼容设备的 2.4G 无线接收器、USB 直连和蓝牙连接的识别与只读数据读取；补齐插件仓库结构和文档；并真实构建 macOS 产物。

## 已交付

1. **协议研究**：读取 `AMasterDriver_v1.0.6_unpacked_reverse_bundle/analysis/AMasterDriver_v1.0.6_reverse_analysis.md` 与 `DONGLE_LIGHTING_CONFIRMATION.md`，梳理 protocol-a（VID `0x3151`）与 AM35（VID `0x0E8D`）的命令、字段映射、校验和接收器转发流程。
2. **双仓库结构**：创建 `mira-mouse-plugins/` 并在其中建立 `plugins/amaster/` 插件源码。
3. **插件格式**：为 `mira.amaster` 提供 `plugin.json`、`devices.json`、`protocol/{transports,commands,parsers,workflows}.json`。
4. **运行时协议驱动**：在 `crates/mira-plugin-runtime/src/protocol.rs` 按 family 分发，实现 protocol-a 的电池、DPI 全档位、回报率、固件、灯光颜色读取，以及接收器轮询。
5. **USB / 2.4G 支持**：`devices.json` 已包含 protocol-a-direct（USB）和 protocol-a-receiver（2.4G）以及 am35-direct / am35-receiver。
6. **蓝牙**：标记为 `blocked`，因为当前反编译资料未提供蓝牙 HID 的 VID/PID 与接口证据。
7. **前端**：`DeviceSnapshot` 扩展 `dpiStages`、`evidence`；UI 按能力动态渲染电量、DPI 档位、回报率、Profile、灯光。
8. **构建**：成功生成 `target/release/bundle/dmg/Mira_0.1.0_aarch64.dmg`。
9. **测试**：`cargo test --workspace`、`npm run lint`、`npm run typecheck` 全部通过。

## 阻塞 / 后续

- 蓝牙 HID 识别缺少硬件证据；需真机捕获蓝牙配对后的 VID/PID/usage。
- AM35 协议为 `source-confirmed` 但未 `hardware-verified`；写入能力全部关闭。
- 首次启动若从 DMG 直接运行，可能需要用户授予 macOS Input Monitoring 权限；当前仅验证了从终端启动时的 HID 访问。
- Windows / Linux 产物未在当前 macOS 宿主机构建。
- 正式 Apple Developer / Windows 代码签名 / Linux GPG / Updater 签名凭据未提供，标记 `blocked`。
