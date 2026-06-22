# 证据状态

## 协议研究

| 结论 | 来源文件 | 方法 | 状态 |
|---|---|---|---|
| protocol-a 使用 VID 0x3151 / PID 0x402A(USB) / 0x5007(2.4G) | `AMasterDriver_v1.0.6_reverse_analysis.md` §3.1 | 反编译静态分析 | source-confirmed |
| protocol-a 接口 usage_page=0xFFFF/0xFF01, usage=2/1 | `AMasterDriver_v1.0.6_reverse_analysis.md` §3.1 | 反编译 + 当前 hidapi 枚举 | hardware-verified（2.4G） |
| protocol-a Feature Report ID=0, 64 字节负载，HIDAPI 总长 65 | `AMasterDriver_v1.0.6_reverse_analysis.md` §3.2 | 反编译 | source-confirmed |
| checksum = 0xFF - (sum(bytes) & 0xFF) | `AMasterDriver_v1.0.6_reverse_analysis.md` §3.2 | 反编译 + 单元测试 | fixture-verified |
| 0xD6 电池 / 0xD4 DPI / 0xD3 综合参数 / 0xAD USB 固件 / 0x80 SoC 固件 | `AMasterDriver_v1.0.6_reverse_analysis.md` §3.3 | 反编译 | source-confirmed |
| 回报率编码表 | `AMasterDriver_v1.0.6_reverse_analysis.md` §3.3 | 反编译 + 单元测试 | fixture-verified |
| 接收器转发 F6/F7/FE/FC | `AMasterDriver_v1.0.6_reverse_analysis.md` §3.4 | 反编译 | source-confirmed |
| AM35 VID 0x0E8D / PID 0x0880 / 0x0703，RACE 协议 | `AMasterDriver_v1.0.6_reverse_analysis.md` §4 | 反编译 | source-confirmed |
| 接收器灯光无原生“跟随鼠标”字段 | `DONGLE_LIGHTING_CONFIRMATION.md` | 反编译 | source-confirmed |
| protocol-a DPI / 综合参数 / 灯光 setter | `mouseApi.py` 的 `setMouseDPI` / `setMouseInfo` / `setMDLight` | 反编译 + 数据包构造与回读断言测试 | source-confirmed / fixture-verified |

## 真机验证

| 能力 | 硬件 | 结果 | 状态 |
|---|---|---|---|
| 2.4G 接收器识别 | AM INFINITY 8K MOUSE (VID 0x3151 / PID 0x5007) | hidapi 枚举匹配，应用 UI 显示已连接 | hardware-verified |
| 2.4G 电量/回报率/DPI/Profile/真实灯效及完整只读设置 | 同上 | 签名插件完整工作流读取成功（2026-06-20） | hardware-verified |
| 2.4G 低风险写入 | 同上 | 当前值回写 smoke test 未执行：最终复检时接收器连续报告鼠标离线 | fixture/build-verified；hardware-pending |
| USB 直连 | 未插入 USB 线缆模式 | 无法验证 | blocked |
| 蓝牙 | 未获取蓝牙 HID 描述符 | 无法验证 | blocked |
| AM35 | 无 AM35 设备 | 无法验证 | blocked |

## 构建验证

- `cargo test --workspace`：通过（fixture-verified）
- `npm run lint && npm run typecheck`：通过（build-verified）
- 插件 `npm run validate && npm test`：通过，覆盖 9 个 declarative mutation/协议测试（fixture-verified）
- `npx tauri build`：成功生成 DMG（build-verified）
