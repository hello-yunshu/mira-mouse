<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Protocol DSL

version 1 解释器仅接受已声明的写操作、有界的读操作、字节期望（byte expectation）、以及有界的延迟操作。核心拥有传输句柄（transport handles）和取消控制。默认上限为：64 步、每个 report/buffer 1,024 字节、16 次读、总延迟 2 秒。不存在表达式求值器、文件系统、网络、进程、递归、任意循环或系统调用。

未来的 checksum、field、fragmentation、forwarding、snapshot、read-modify-write 和有限状态机（finite-state-machine）操作必须保持有类型、有版本、有界且可进行 Fixture 测试。缺失某种操作是一种 capability gap，绝不意味着允许执行插件代码。

## Engine 层会话预算

DSL 上限之上，运行时（`crates/mira-plugin-runtime/src/engine.rs`）对单次 workflow 执行还施加以下会话级预算，确保任何插件 workflow 都不会无界占用 HID 通道：

| 常量 | 值 | 含义 |
|------|----|----|
| `MAX_COMMANDS` | 56 | 单个 workflow 内 step 数量上限（与 DSL 64 步留出 8 步余量） |
| `MAX_REPORTS` | 128 | 单次 workflow 执行允许的 HID report 读取次数上限 |
| `MAX_DELAY_MS` | 5,000 | 单次 workflow 内所有 `Delay` 步骤累计延迟上限（5 秒） |
| `MAX_OPERATION_TIMEOUT_MS` | 30,000 | 单次 HID 读/写操作的最大超时（30 秒，防止 UI 冻结） |

这些预算在 `engine.rs` 中以常量形式定义，运行时强制执行，超出时返回 `Err`。`reports per second` 由插件 manifest 的 `permissions.reportsPerSecond` 字段声明，Host 调度器按设备作用域聚合以避免超过设备能力。inventory workflow 与普通读取 workflow 共享同一预算，不享受额外配额。

mutation、postWrite、memory read/write 步骤均计入同一 workflow 的 `MAX_COMMANDS` 与 `MAX_REPORTS` 预算，无独立通道。这意味着 onboard sector 完整回读（16 chunk × sector_size）必须与主读取 workflow 拆分为独立 workflow 调用，由 Host 按需触发。
