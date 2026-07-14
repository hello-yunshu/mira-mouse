<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 第三方声明

依赖声明必须由 release CI 从 `Cargo.lock` 与 `package-lock.json` 生成。本项目不再分发本地逆向工程 bundle、厂商二进制、图标、截图、字体或应用资源。

`Cargo.lock` 与 `package-lock.json` 已纳入仓库，SBOM 与最终第三方清单正在 release 流程中由锁文件生成并校验。在 SBOM 与最终第三方清单正式发布前，不声称已完成的第三方清单。

Mira 主应用通过 crates.io 依赖 `rill-runtime-protocol`（MIT），仅用于跨进程 JSON 约定；主应用本身不链接 RillML 推理引擎。独立的 `mira-runtime` sidecar 则依赖 `rill-runtime` 与 `rill-ml`（均为 MIT）。上游来源：<https://github.com/hello-yunshu/rill-ml>。

Mira 通过 Tauri `externalBin` 打包自建的 `mira-runtime` 二进制（基于 `rill-runtime` crate 构建）与签名模型包 `model.rillpack`，作为本地 AI bundle 随 Mira 主程序分发。
