# 假设与阻塞

## 硬阻塞

1. **蓝牙 HID 设备识别**
   - 原因：反编译资料与当前枚举均未确认 AMaster 鼠标在蓝牙配对后的 VID/PID/usage page/usage。
   - 影响：无法在 `devices.json` 中声明蓝牙匹配规则；UI 中显示“蓝牙”连接方式但无法识别设备。
   - 解除：需要真机在不同蓝牙主机上枚举 HID 设备并记录 VID/PID/usage。

2. **AM35 硬件验证**
   - 原因：当前插入的硬件为 protocol-a（VID 0x3151 / PID 0x5007），没有 AM35 设备。
   - 影响：AM35 协议实现为 `source-confirmed`，未 `hardware-verified`；读取返回错误，不会显示假数据。
   - 解除：获取 AM35 鼠标进行真机测试。

3. **平台签名与公证**
   - 原因：未提供 Apple Developer ID、Windows 受信任代码签名证书、Linux GPG、Updater 签名密钥。
   - 影响：只能发布 `unsigned-community` 资产，首次启动需用户手动允许 Gatekeeper / SmartScreen。
   - 解除：用户提供凭据并在 CI 中配置受保护 Environment。

4. **跨平台构建**
   - 原因：当前宿主机为 macOS ARM64，没有 Windows / Linux Runner 或容器。
   - 影响：无法在本会话内实际构建 Windows NSIS EXE 和 Linux AppImage/DEB/RPM。
   - 解除：在对应平台的 CI Runner 上运行 `npm exec tauri build`。

## 假设

- 用户当前插入的硬件是 2.4G 接收器模式（VID 0x3151 / PID 0x5007）。
- protocol-a 校验算法 `0xFF - (sum & 0xFF)` 适用于所有读取命令。
- 鼠标在线标志在接收器轮询响应偏移 4 处，且程序逻辑中为反相（0 = online）。
- DPI X/Y 值在 0xD4 响应中以 2 字节小端存放，与反编译分析一致。
