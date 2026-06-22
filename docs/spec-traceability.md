# 需求追踪表

| 提示词章节 | 要求摘要 | 实现位置 | 测试/证据 | 状态 |
|---|---|---|---|---|
| 0.1 真实性优先 | 使用 `source-confirmed` / `hardware-verified` 等标签 | `plugins.lock.json`, `devices.json`, `docs/evidence-status.md` | 手动标记 | fixture-verified |
| 1.2 双仓库 | `mira-mouse` + `mira-mouse-plugins` | `mira-mouse-plugins/plugins/amaster/` | 目录存在 | build-verified |
| 1.3 硬边界 | 品牌 VID/PID 只在插件仓库 | `mira-mouse-plugins/plugins/amaster/devices.json` | 代码扫描无品牌常量 | build-verified |
| 4.1 `.mira-plugin` 容器 | ZIP 容器、白名单、校验和、签名 | `crates/mira-plugin-runtime/src/package.rs` | `cargo test --workspace` | fixture-verified |
| 4.3 `plugins.lock.json` | 固定插件 SHA-256 | `plugins.lock.json` | SHA 与资源文件一致 | build-verified |
| 5.2 标准能力 | 电量、DPI、回报率、Profile、灯光及受约束修改 | `src/App.tsx`, `src/types.ts` | UI 与 mutation 测试 | build-verified |
| 6.2 protocol-a | VID 0x3151 / PID 0x402A(USB) / 0x5007(2.4G)，Feature Report，校验，命令 | `crates/mira-plugin-runtime/src/protocol.rs` | 单元测试 + 真机枚举 | hardware-verified（2.4G） |
| 6.3 AM35 | VID 0x0E8D / PID 0x0880 / 0x0703，Output/Input Report | `mira-mouse-plugins/plugins/amaster/protocol/*.json` | 无硬件 | source-confirmed / blocked |
| 8.1 主窗口 | 未连接提示、连接后状态 | `src/App.tsx` | 截图 | build-verified |
| 8.2 图标和文字 | 关键值配文字 | `src/App.tsx` | 截图 | build-verified |
| 9.1 托盘 | 菜单栏图标 | `src-tauri/src/lib.rs` setup | 应用启动 | build-verified |
| 十三、测试 | 单元/契约测试 | `cargo test --workspace` | 全部通过 | fixture-verified |

## 未完成

- 蓝牙 HID 设备描述符：缺少证据，标记 `blocked`。
- AM35 真机读取：标记 `blocked`。
- Plugin DSL 已执行声明式读取与读-改-写-回读 mutation；插件持有命令、字段、范围和断言，宿主仅提供受约束解释器。
- protocol-a 当前 DPI 档、DPI 数值、回报率、鼠标字符灯和接收器灯光已开放；真机写入 smoke test 因鼠标离线待补。
- 社区文件、Actions、REUSE、第三方许可清单：部分缺失，标记 `blocked`。
