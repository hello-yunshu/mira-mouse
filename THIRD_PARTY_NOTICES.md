<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 第三方声明

依赖声明必须由 release CI 从 `Cargo.lock` 与 `package-lock.json` 生成。本项目不再分发本地逆向工程 bundle、厂商二进制、图标、截图、字体或应用资源。

`Cargo.lock` 与 `package-lock.json` 已纳入仓库，SBOM 与最终第三方清单正在 release 流程中由锁文件生成并校验。在 SBOM 与最终第三方清单正式发布前，不声称已完成的第三方清单。

Mira 主应用依赖 `rill-runtime-protocol`（MIT），仅用于跨进程 JSON 约定；主应用本身不链接 RillML 推理引擎。通用 `rill-runtime` sidecar 来自 RillML，Mira 自有的沙箱化 WASM handler 使用 `rill-ml`（均为 MIT）。上游来源：<https://github.com/hello-yunshu/rill-ml>。

Mira 通过 Tauri `externalBin` 打包启用 WASM 功能的通用 `rill-runtime`，并随应用携带独立签名的 `model.rillpack` 与 `handler.rillhandler`。三者可独立发布，客户端只在完整组合通过握手后原子激活。
