<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Protocol DSL

version 1 解释器仅接受已声明的写操作、有界的读操作、字节期望（byte expectation）、以及有界的延迟操作。核心拥有传输句柄（transport handles）和取消控制。默认上限为：64 步、每个 report/buffer 1,024 字节、16 次读、总延迟 2 秒。不存在表达式求值器、文件系统、网络、进程、递归、任意循环或系统调用。

未来的 checksum、field、fragmentation、forwarding、snapshot、read-modify-write 和有限状态机（finite-state-machine）操作必须保持有类型、有版本、有界且可进行 Fixture 测试。缺失某种操作是一种 capability gap，绝不意味着允许执行插件代码。
