# Mira 本地AI分析计划

## 目标与边界

“本地AI分析”是 Mira 的引擎总开关。各业务功能通过稳定的功能 ID 独立选择是否使用该引擎；总开关只负责统一停用，不直接代表任何业务功能已开启。它只消费 Mira 已经按插件契约读取并保存在本机的数据，不上传设备历史，不增加设备轮询，也不绕过插件声明的连接方式、电量语义或可写能力。

- 插件页提供独立的“本地AI分析”块，用于统一启用、查看引擎状态和后续承载更新信息。
- 电量使用设置提供“开启 AI 分析”入口，写入 `batteryUsage` 功能作用域；从电量页开启时同时启用引擎总开关与电量历史。
- 当前只有 `batteryUsage` 消费本地 AI，因此默认保留现有联动体验；未来新增功能必须使用自己的功能 ID，不因总开关打开而自动启用。
- 总开关关闭后，停止运行学习模型，并隐藏电量页等业务页面里的 AI 徽标和 AI 标题；普通历史图表与确定性洞察仍可使用。
- Mira 宿主只编译 `rill-runtime-protocol` 约定，不链接推理引擎。跨产品复用的 `rill-runtime`、签名模型包和 Mira 自有的签名 WASM handler 独立发布；客户端把三者组装为一个经过探活的部署快照后原子激活，任一不可用时无条件回退原有算法。

## 当前落地状态

- [x] 总开关"本地AI分析"与 `batteryUsage` 功能开关同步。
- [x] 插件页保持单一部署状态、检查更新、安装与回退；内部独立比较 Runtime、模型和 handler 版本。
- [x] Rill IPC v2、1 MiB 消息上限、handler 身份/API/有效能力握手验证。
- [x] `.rillpack` 固定文件白名单、全载荷校验和与 Ed25519 签名。
- [x] 发布索引签名覆盖版本、平台、URL、大小与 SHA-256，并拒绝降级。
- [x] `staging → current → previous` 原子激活、单版本 `rollback` 与激活后自检。
- [x] Runtime、模型与 handler 作为三个 release artifact 独立下载；候选组合完整探活后执行 `staging → current → previous` 原子切换。
- [x] 常驻进程控制器:`local_ai_analysis_enabled` 开关 on 启动当前平台的 `rill-runtime`，off 停止；predict 复用 IPC 通道，失败 30s 冷却后自动重启。
- [x] Sidecar 打包:`rill-runtime` 作为 Tauri `externalBin`，`model.rillpack` 与 `handler.rillhandler` 作为资源内置，首次安装无需独立下载。
- [x] Runtime 缺失、超时、异常退出、不兼容、模型包损坏和质量门控失败时回退确定性预测。
- [x] `rill-ml` 仅链接进 Mira handler 的 WASM component；Mira 主应用只依赖协议 crate，Rill runtime 保持业务无关。
- [x] 模型/索引与 handler 使用相互独立的 Ed25519 信任根；Rill 通用仓库不持有任何 Mira 私钥。
- [x] RillML 发布带签名 `stable-index.json` 的稳定 release；CI 每次构建只解析一次最新稳定版本并从 rill-ml releases 下载预编译 runtime，避免多平台构建期间版本漂移。
- [x] `Sync Latest Rill Runtime` 每 6 小时检查最新 Rill 稳定索引，只有在签名、版本、Runtime API、当前模型/handler 兼容性和 macOS ARM64/Linux/Windows 真实握手全部通过后，才复签并移动 Mira 的 `local-ai-stable` 指针。
- [x] handler 可通过 `Publish Local AI Handler` workflow 单独构建、签名和发布；不重建 Mira App、模型或 Rill runtime。客户端从专用的 `local-ai-stable` 预发布读取签名索引，避免 handler 更新占用 GitHub 的 App `latest` 指针。
- [ ] 用真实设备的长期历史校准模型参数与质量门槛。

## trust-key 设置

`rill-runtime serve` 分别接受模型 `--trust-key` 与 handler `--handler-trust-key`，格式都是
`<key-id>=<64 位十六进制 Ed25519 公钥>`。Rill IPC v2 模型/索引使用
`mira-rill-2026-002`，handler 使用 `mira-handler-2026-001`；旧模型公钥只留在模型 trust store
用于平滑识别旧安装，不能签发 v2 release index。不要把私钥写进应用或仓库。

本地 debug 要测试另一把公钥时，设置完整的安装路径并额外提供公钥参数：

```sh
RILL_RUNTIME_PATH=/path/to/rill-runtime \
RILL_MODEL_PACK_PATH=/path/to/model.rillpack \
RILL_HANDLER_PATH=/path/to/handler.rillhandler \
RILL_TRUST_KEY='dev-key-id=<64 位十六进制公钥>' \
RILL_HANDLER_TRUST_KEY='dev-handler-key=<64 位十六进制公钥>' \
cargo tauri dev
```

生产私钥只配置在 Mira 主仓库：`RILL_V2_SIGNING_KEY` 签模型与 release index，
`HANDLER_SIGNING_KEY` 只签 handler。二者都是 32 字节 Ed25519 seed（64 个十六进制字符），
必须分别对应源码中固定的两个公钥。可复用 workflow 在打包后执行签名往返验证。

## 独立发布 Mira handler

首次启用新链路时，运行一次 `Sync Latest Rill Runtime`（或完成一次正常 App release）；workflow 会从最新 App release 的已签名索引安全引导并创建固定的 `local-ai-stable` 预发布。后续只改 Mira 预测逻辑时：

1. 同步提高 `handlers/mira-battery-handler/Cargo.toml` 与 `manifest.template.json` 的版本；App 版本同步脚本不会改动这两个独立版本源，也不修改或重编译 Rill runtime。
2. 手动运行 `Publish Local AI Handler`，输入一个严格递增的稳定 semver。
3. workflow 从当前 `local-ai-stable` 保留已发布的 Rill runtime 和模型条目，仅替换 `mira.battery.handler`，分别验证 handler 签名与完整索引签名。
4. 新 handler 写入不可变的 `local-ai-handler-v<version>` release；验证完成后才原子移动 `local-ai-stable` 指针。
5. Mira 下载 handler 后会复制当前 Runtime/模型组成 staging 部署，完成真实握手再切换；失败时保留原部署并可回滚。

Rill `stable-index.json` 也必须先用 Rill 官方发布公钥验签，Mira 才会采纳其中的 runtime URL、大小和 SHA-256。handler 的最低 Runtime 版本来自自身 manifest，是兼容性下限，不跟随 Rill 最新版本机械上调。

## 第一阶段：自适应电量预测

状态：跨进程接入与安全回退已完成，仍需用真实设备历史验证质量门槛。

1. 保留原有确定性续航估算作为冷启动和故障回退。
2. 使用 RillML 在线回归，从已有放电记录学习电量、时段、星期周期和近期耗电速度之间的关系。
3. 严格采用“先预测、后学习”的渐进评估，避免拿当前答案训练后再评价当前答案。
4. 用滚动 MAE 同时评价旧预测和本地模型；至少取得 8 个验证样本，且模型误差低于旧预测后才允许接管。
5. 充电段、长时间断连、换电池、非有限结果和异常耗电速率不进入训练；任何模型错误都无条件回退旧预测。
6. 不产生额外 HID 读取。模型只在用户打开电量页、后端构建已有历史响应时重放本地样本。

验收标准：

- 开关关闭时预测结果与原有逻辑一致，业务页面没有 AI 标识。
- 开关开启但数据不足或模型未胜出时仍能正常显示旧预测。
- 模型胜出后才替代旧预测；清除历史后自然回到冷启动。
- 相同历史输入得到可重复结果，模型失败不能影响设备读取、设置修改和历史导出。

## 后续能力候选

### P1：适合优先完成

- 自适应异常耗电：用模型残差和漂移检测替代固定倍率阈值，区分偶发电量跳变与持续异常。
- 电池健康趋势：跟踪相似使用条件下的续航衰减，给出“可能老化”而不是武断的健康百分比。
- 充电习惯预测：预测常见充电时段与起充电量，在确有帮助时给出本地提醒。
- 可解释状态：显示“学习中 / 使用旧预测 / 本地模型已接管”、验证样本量和近期误差，不展示虚假置信度。

### P2：需要新增插件契约后再做

- 省电配置建议：由插件声明可调字段、合法值、功耗方向和回滚方式，Mira 只在声明范围内推荐；默认不自动写设备。
- 场景配置推荐：根据应用、连接方式和历史选择推荐 DPI、回报率或灯光配置，但必须由插件提供语义化 mutation 和安全边界。
- 设备差异学习：同一设备族可使用匿名本地特征共享初始模型，但不得由宿主硬编码厂商、协议或型号例外。
- 通知时机优化：学习用户通常何时查看或忽略提醒，只调整本地通知时机，不改变低电量事实判断。

### P3：研究项

- 插件运行异常聚类：基于错误类别、重试结果和读计划做只读诊断，帮助定位插件或连接问题。
- 自适应读取计划建议：只输出建议给现有调度器，并受插件声明、退避策略和读取预算约束；不能直接制造高频读取。
- 本地策略推荐：使用 bandit 在多个已验证、可回滚的展示或提醒策略间选择，奖励必须延迟回传且有确定性默认策略。

## 明确不做

- 不让模型猜测 HID 协议、设备身份、电量有效连接或可写命令。
- 不生成任意设备写入，不绕过插件 schema、验证器、回读确认和回滚机制。
- 不因开启本地 AI 增加后台高频读取、持续唤醒设备或缩短现有退避。
- 不上传原始电量历史、设备标识或模型状态；未来若增加云能力，必须另设独立的显式授权。
- 不在缺少质量证据时用 AI 文案包装原有固定规则。

## 工程路线

1. 完成本地总开关、同步入口、标识可见性和电量预测质量门槛。
2. 在真实设备历史上记录候选与基线误差，校准最小样本数、窗口和胜出幅度。
3. 为插件 API 设计可选的 `intelligence` 元数据，只声明特征语义、合法动作、代价和回滚，不包含厂商特例。
4. RillML 已发布预编译 runtime 二进制与签名稳定索引；CI 构建采用当时最新的已验证稳定版，在线同步任务持续采纳更新版本。客户端读取 Mira 复签的聚合索引后直接从 Rill release 下载 runtime，Mira 自身只独立发布 handler/模型。
5. 当前继续采用可重复的历史重放，让学习数据与模型包更新相互独立；只有未来改成常驻增量状态时，才引入版本化状态、迁移、损坏恢复和一键重置。
6. 增加 Runtime/模型更新可观测性：最近检查时间、失败阶段、当前/回退版本和不含设备数据的诊断导出。
7. 每项新能力先以只读建议上线，经过真机验证后再讨论可回滚的自动执行。
