<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# 威胁模型

## 资产

受保护的资产包括：HID 设备完整性、插件/更新 trust roots、用户设置、诊断隐私、release 凭据，以及应用程序进程。

## 不可信输入

不可信输入包括：插件 ZIP、manifest、协议工作流、Fixtures、HID 响应、未知设备字符串、导入的配置文件、更新元数据、release 下载、文件名、日志，以及用户提供的 issue 内容。

## 主要控制措施

- 在解压前校验包结构；拒绝 traversal、链接、重复、炸弹、超大条目、代码、远程内容、错误覆盖范围、digest、签名、key、API、权限和证据。
- 将 HID 句柄、时序、取消、互斥和 readback 保留在核心中；插件是声明式且有界的。
- 除非精确的硬件证据和操作策略允许，否则拒绝写入；保留未知字段并在失败时显示实际状态。
- 脱敏稳定标识符，从不隐式上传，并保持 telemetry/accounts/ads/常驻网络服务缺席。
- 使用原子化的状态/插件替换、不可变的 lock、受保护环境、显式 GitHub Action ref、最小权限，以及 clean-job 重新下载校验。

残余的硬件、平台签名、更新器签名和公开仓库风险仍被列为 `blocked`。
