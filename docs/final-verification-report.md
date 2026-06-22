# 最终验证报告

## 1. 创建/修改的路径

### mira-mouse 主仓库
- `src-tauri/src/lib.rs`：使用 `read_device` 读取真实设备数据。
- `src-tauri/resources/plugins/mira-amaster-1.3.0.mira-plugin`：带声明式低风险写入的 amaster 插件（1.3.0，生产签名）。
- `crates/mira-plugin-runtime/src/protocol.rs`：新增 protocol-a / AM35 协议族驱动。
- `crates/mira-plugin-runtime/src/lib.rs`：暴露 protocol 模块。
- `crates/mira-plugin-runtime/Cargo.toml`：添加 `mira-core` 依赖。
- `crates/mira-core/src/lib.rs`：扩展 `DeviceSnapshot`（包括插件声明的可写 mutation）。
- `src/App.tsx`、`src/types.ts`：前端呈现并提交标准业务参数，支持回读状态与一次撤销。
- `plugins.lock.json`：更新 amaster 插件版本至 1.3.0 及 SHA-256。
- `src-tauri/tauri.conf.json`：资源路径指向 1.3.0 产物。
- `docs/execution-plan.md`、`docs/spec-traceability.md`、`docs/assumptions-and-blockers.md`、`docs/evidence-status.md`、`docs/session-checkpoint.md`、`docs/final-verification-report.md`。

### mira-mouse-plugins 插件仓库
- `plugins/amaster/plugin.json`
- `plugins/amaster/devices.json`
- `plugins/amaster/protocol/transports.json`
- `plugins/amaster/protocol/commands.json`
- `plugins/amaster/protocol/parsers.json`
- `plugins/amaster/protocol/workflows.json`
- `plugins/amaster/README.md`

## 2. 完成功能状态

| 功能 | 状态 | 证据 |
|---|---|---|
| 2.4G 无线接收器识别 | hardware-verified | hidapi 枚举匹配 VID 0x3151 / PID 0x5007 |
| 2.4G 电量读取 | hardware-verified | 代理读取 0xD6，真机返回 81%（2026-06-19） |
| 2.4G DPI 档位读取 | hardware-verified | 代理读取 0xD4，真机返回 400 DPI / 8 档 / profile 0 |
| 2.4G 回报率读取 | hardware-verified | 代理读取 0xD3，真机返回 1000Hz |
| 2.4G Profile | hardware-verified | 0xD3 / 0xD4 返回 profile=0 |
| 2.4G 灯光读取 | hardware-verified | protocol-a 0x88 返回 effect/speed/brightness/option/RGB；0x87 返回字符灯开关 |
| 2.4G 接收器状态 | hardware-verified | receiver-poll 0xF7 返回 mouseBattery=81 / receiverBattery=100 / online |
| USB 直连描述符 | source-confirmed | devices.json 已包含 protocol-a-direct |
| USB 直连真机读取 | blocked | 未插入 USB 线缆模式 |
| 蓝牙识别 | blocked | 缺少蓝牙 HID VID/PID/usage 证据 |
| AM35 协议 | source-confirmed | 反编译资料已整理，无硬件验证 |
| protocol-a 低风险写入 | fixture/build-verified；hardware-pending | 当前 DPI 档、单档 DPI、回报率、鼠标字符灯、接收器灯光均为读-改-写-回读事务；最终复检时鼠标离线，未执行 no-op 写入 |

## 2.1 2.4G 真机验证详情（2026-06-20）

验证工具：`crates/mira-plugin-runtime/examples/enumerate_hid.rs`。该工具仅验签、匹配设备并调用插件工作流，不包含 AMaster 命令或字段偏移硬编码。

```
sha256: 41e031e7ea2be84d2e9863dc3f9a7c949fec0e0cb04503e401c5ede67080d454
plugin: mira.amaster v1.3.0 signature_verified=true
target: family=protocol-a-receiver usage_page=0xffff usage=0x0002
battery=77% dpi=2400 polling_rate=1000Hz profile=0
完整 protocol-a-receiver-read 工作流通过，返回 14 组 capability。
```

验证结论：
- 插件签名（Ed25519）和 SHA-256 校验通过
- 接收器检测成功（VID 0x3151 / PID 0x5007）
- HID Feature Report 代理协议工作正常（readReady 轮询成功）
- 电量、DPI、综合设置、灯效、字符灯、FPS、DPI 快切、固件和按键映射均返回有效数据
- `transports.json` 已使用 `attempts: 20, delayMs: 20`，完整工作流已通过

## 3. 关键命令与结果

```bash
cargo test --workspace          # ok (16 tests)
npm run lint && npm run typecheck # ok
npx tauri build                 # ok -> Mira_0.1.0_aarch64.dmg
```

## 4. 构建产物

- macOS ARM64 DMG：`target/release/bundle/dmg/Mira_0.1.0_aarch64.dmg`
- DMG SHA-256：`3608265cfb5d384acfb594c4e7a4abb5fa6fda1b65934d96190856bff4e4e480`（3,008,286 bytes）
- 应用路径（挂载后）：`/Volumes/Mira/Mira.app`
- 运行方式（推荐从终端以查看日志）：`/Volumes/Mira/Mira.app/Contents/MacOS/mira`

## 5. 插件包

- 文件：`src-tauri/resources/plugins/mira-amaster-1.3.0.mira-plugin`
- SHA-256：`41e031e7ea2be84d2e9863dc3f9a7c949fec0e0cb04503e401c5ede67080d454`
- 签名：Ed25519 生产签名，publisherKeyId `mira-plugins-2026-001`，公钥 `eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3`
- 验证：运行时验签通过；DMG 挂载后内置插件 SHA-256 与 `plugins.lock.json` 完全一致

## 6. 阻塞项

1. 蓝牙 HID 设备描述符缺失。
2. AM35 真机验证缺失。
3. USB 直连模式未在当前硬件上验证。
4. macOS Input Monitoring 权限弹窗中断 UI 自动验证；需用户手动处理。
5. Apple / Windows / Linux 平台签名、公证、Updater 签名凭据缺失。
6. Windows / Linux 产物未在当前宿主机构建。
7. 社区文件、REUSE/SPDX、第三方许可清单、完整 CI Actions 尚未完成。
8. protocol-a no-op 真机写入 smoke test 待鼠标在线后执行。

## 7. 安全与分发注意事项

- 当前产物为 `unsigned-community` 构建，首次启动需在 macOS “系统设置 → 隐私与安全性”中手动允许。
- 从 DMG 直接启动可能因权限问题无法访问 HID；推荐先复制到 `/Applications` 或从终端运行验证。
- 插件使用 Ed25519 生产签名（`mira-plugins-2026-001`）；私钥仅保存在本地与 CI 密钥库中。
- 未实现遥测、账户或常驻网络服务。
