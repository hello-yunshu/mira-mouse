<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 插件包格式 (Plugin Package Format)

`.mira-plugin` 是一个确定性的 ZIP 容器，包含声明式 JSON、文档、测试夹具 (fixtures)、校验和 (checksums) 以及一个可选的 Ed25519 签名 (signature)。路径必须是规范化的相对 UTF-8 路径；绝对路径、`..`、反斜杠、重复条目、符号链接、可执行/脚本/Web 扩展、远程资源以及不在白名单内的文件，在解压前都会被拒绝。

白名单允许顶层的 `plugin.json`、`checksums.json`、`devices.json`、`capabilities.json`、`README.md`、`LICENSE` 和 `META-INF/signature.ed25519`，以及 `protocol/`、`locales/`、`tests/fixtures/` 和 `models/` 前缀下的 `.json` 文件。`models/` 目录是为按模型适配器覆盖 (per-model adapter overrides) 预留的父文件夹：未来的插件可以发布针对特定模型的 JSON（例如 `models/<model>/capabilities.json`），而无需更改包格式。

`checksums.json` 的 schema 1 将除自身和 `META-INF/signature.ed25519` 之外的每个负载 (payload) 路径映射到小写的 SHA-256。覆盖范围必须精确。签名消息是 `plugin.json` 的规范 JSON (canonical JSON)，后接一个 LF 字节，再接 `checksums.json` 的规范 JSON。规范 JSON 递归排序对象键，保留数组顺序和 JSON 标量值，并输出紧凑的 UTF-8（不含无关空白字符）。密钥仅由 manifest 的密钥 ID 和已配置的信任库 (trust store) 选定。

当前限制为 512 个文件、每个文件 4 MiB、未压缩总字节数 32 MiB。在限制、schema、摘要 (digest)、覆盖范围、密钥、签名、ID、API、权限或证据错误上，验证采用失败即关闭 (fail closed) 策略。
