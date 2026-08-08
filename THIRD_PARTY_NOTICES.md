<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 第三方声明

依赖声明必须由 release CI 从 `Cargo.lock` 与 `package-lock.json` 生成。本项目不再分发本地逆向工程 bundle、厂商二进制、图标、截图、字体或应用资源。

`Cargo.lock` 与 `package-lock.json` 已纳入仓库，SBOM 与最终第三方清单正在 release 流程中由锁文件生成并校验。在 SBOM 与最终第三方清单正式发布前，不声称已完成的第三方清单。

Mira 主应用依赖 `rill-runtime-protocol`（MIT），仅用于跨进程 JSON 约定；主应用本身不链接 RillML 推理引擎。通用 `rill-runtime` sidecar 来自 RillML，Mira 自有的沙箱化 WASM handler 使用 `rill-ml`（均为 MIT）。上游来源：<https://github.com/hello-yunshu/rill-ml>。

Mira 通过 Tauri `externalBin` 打包启用 WASM 功能的通用 `rill-runtime`，并随应用携带独立签名的 `model.rillpack` 与 `handler.rillhandler`。三者可独立发布，客户端只在完整组合通过握手后原子激活。

Thinking Orbs（`thinking-orbs` 0.2.0，MIT）仅用于设备初始化、手动检查、设备读写与导入导出中的短时过程反馈。Mira 通过精确版本锁定和 `package-lock.json` 固定实际分发内容；该组件不接触 HID、插件协议、文件内容、网络、模型或用户数据。上游来源：<https://github.com/Jakubantalik/thinking-orbs>。

Mira 的 Attention Beam 边缘扫光视觉语言参考了 Jakub Antalik 的 `border-beam`（MIT）概念。Mira 使用自有精简实现，不包含 `border-beam` 作为运行时依赖，也不直接使用上游组件。上游来源: <https://github.com/Jakubantalik/border-beam>。
