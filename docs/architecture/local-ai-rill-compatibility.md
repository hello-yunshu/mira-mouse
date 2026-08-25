# Mira Local AI × Rill 1.5 Stable 兼容性架构文档

> 状态：已实现（阶段 0–6）
本文档说明 Mira 当前如何**可复现、可回退、可审计**地消费 Rill 1.5 Stable，以及各实验能力所处的路径与开关。

当前生产契约：签名 Stable release、release-index schema v3、Runtime API 2、Handler API 1；Preview IPC v3 不进入生产。当前 Local AI 发布目标为 macOS ARM64。

---

## 1. 版本与构建可复现性

### 1.1 依赖声明

依赖声明不做永久精确锁死，允许后续兼容升级：

```toml
rill-ml = { version = "1.5", default-features = false }
rill-runtime-protocol = "1.5"
```

实际可复现版本由**已提交的锁文件**保证；当前 committed lock 为 `1.5.1`，而不是永久 `=1.5.1` pin。

### 1.2 锁文件

| 位置 | 作用 |
|---|---|
| 根 `Cargo.lock` | workspace（含 `mira-local-ai`）解析到 `rill-ml 1.5.1` |
| `handlers/mira-battery-handler/Cargo.lock` | 独立 workspace 的 handler 解析到 `rill-ml 1.5.1` |

### 1.3 CI 可复现

- 所有正式 Rust 构建使用 `--locked`，不再在 release 构建中执行无约束 `cargo update`。
- 升级 rill 小版本时：独立 PR 更新 `Cargo.toml` + 锁文件 → 重新走 CI，而不是在打包时临时升级。
- 模型包 / handler 包 CI 读取**已解析的 Stable release**，并执行版本一致性检查（见下）。

### 1.4 版本一致性检查

`scripts/check-rill-compatibility.mjs`（或等价工具）比较：

- 根锁文件 `rill-ml`
- 根锁文件 `rill-runtime-protocol`
- handler 锁文件 `rill-ml`
- 已解析的 runtime release 版本

约束：

```text
host rill-runtime-protocol major.minor == runtime release major.minor
handler rill-ml major.minor          == runtime release major.minor
```

当前 Stable 接口契约：

```text
IPC API = 2
handler API = 1
```

脚本失败时以非零状态退出，作为 CI 门禁。

> 说明：Rill 1.5.1 保持 `RUNTIME_API_VERSION = 2`、`HANDLER_API_VERSION = 1`；Mira 不把兼容策略改成永久 patch pin。

---

## 2. 生产链路（保持不变）

以下能力**不得删除或弱化**，本次适配未改其行为：

- HID 电量历史采集
- 当前确定性电量预测（IPC V2 调用 `predict` 的 baseline 路径）
- 本地 AI 开关
- runtime 启动失败后的自动回退
- handler 调用失败后的自动回退
- model pack / handler pack 的签名验证
- IPC V2
- WIT handler API v1
- 现有插件协议
- 现有用户界面和设置结构

默认生产路径：

```text
IPC V2  +  WIT Handler API v1  +  确定性 fallback
```

---

## 3. Feature Schema 与 Model Descriptor（阶段 2）

### 3.1 唯一权威定义

特征定义收敛到单一模块：[`crates/mira-local-ai/src/battery_features.rs`](../../crates/mira-local-ai/src/battery_features.rs)。

- `BATTERY_SCHEMA_ID = "mira-battery-feature-schema-v1"`
- `BATTERY_SCHEMA_VERSION = 1`
- `BATTERY_FEATURE_NAMES`：9 个特征，顺序即身份的一部分。

每个特征的语义契约（单位 / 范围 / 归一化公式 / 缺失策略）集中记录在模块注释与 `FeatureDescriptor::transform/metadata` 中，见 [battery-feature-schema.md](battery-feature-schema.md)。

### 3.2 schema 身份

- `battery_schema_hash()`：确定性 SHA-256，跨进程稳定（golden 文件 `tests/fixtures/battery_schema_v1.json` 锁定）。
- `battery_model_descriptor(config)`：算法 + schema hash + 配置摘要（影响训练行为的数值参数）。
- `battery_model_descriptor_hash_hex(config)`：对 `ModelDescriptor` 规范 JSON 取 SHA-256。

### 3.3 模型加载兼容策略

通过 `check_schema_identity` 判定，返回三态：

| 状态 | 行为 |
|---|---|
| `Matched` | schema / descriptor 身份一致，正常加载。 |
| `LegacyModelPack` | 未声明任何身份字段 → 走 legacy 兼容路径；仍检查 `feature_count`，记录 warning；不允许把旧模型重新标记为新 schema。 |
| `Mismatch` | 声明了身份但与当前实现不一致（schema id / schema hash / descriptor hash 任一不符）→ **拒绝使用该模型**，记录 expected/actual，回退确定性预测，不崩溃、不删除历史。 |

### 3.4 协议字段

`BatteryModelConfig` 新增（全部 `Option` + `serde(default)`，对旧模型包兼容）：

```rust
pub schema_id: Option<String>,
pub schema_hash: Option<String>,
pub model_descriptor_hash: Option<String>,
```

---

## 4. 加权学习（阶段 3，默认关闭）

### 4.1 权重函数

`mira_local_ai::recency_weight(sample_time, prediction_time, config)`：

```text
weight = max(exp(-age_hours / tau), MIN_RECENCY_WEIGHT)
```

- `tau` 优先取 `learning_recency_tau_hours`，未设置时复用 `baseline_decay_tau_hours`（48h），不引入新参数。
- 最新样本权重 = 1.0；未来时间戳被 clamp；极旧样本有下限 `MIN_RECENCY_WEIGHT`；非法 tau（NaN / 非正）回退基线 tau。绝不产生 NaN。

### 4.2 训练与评价

- 开启时调用 Rill 1.5.1 的 `learn_weighted(features, target, weight)`。
- 评价指标同步维护 `weighted_mae`、`recent_mae`（最近 `quality_window` 小时验证窗口）、`effective_sample_weight`，同时保留普通 `candidate_mae` 便于比较长期稳定性。
- 加权模式下质量门使用 `weighted_mae` 作为候选质量指标。

### 4.3 保护

- 训练样本数 < `min_training_samples` → 回退。
- 加权模式下 `training_weight_sum < min_training_samples`（有效权重不足）→ 回退。
- 候选质量不优于确定性 baseline（`candidate < baseline × required_error_ratio`）→ 回退 `candidateNotBetter`。

### 4.4 回放结论（重要）

历史阶段结论（非当前依赖状态）：`docs/audits/rill-1.1-battery-replay-report.md` 在 10 个固定 fixture 上对比 plain / weighted / robust：

- 加权模型在**全部 10 个场景**的 `recent MAE` 均不优于普通模型（0 提升、10 退化、0 相当）。
- 因此 `weighted_learning_enabled` 保持**默认关闭**，符合阶段 3 验收标准（未满足“适应速度明显快于旧模型”时不擅自默认启用）。
- 该结论仅针对当前 fixture 与 `tau = 48h`；调整 tau 或补充真实遥测回放后需重新评估，不能据此断言加权学习在真实数据上无效。

---

## 5. 稳健异常 / 漂移检测（阶段 4，默认关闭）

### 5.1 开关

```rust
pub robust_detection_enabled: bool  // 默认 false
```

### 5.2 职责边界

新检测器只提供**辅助**信号，绝不直接删除数据或重置模型：

- 异常样本只通过 `anomaly_downweight` 影响训练权重。
- 输出统一为建议动作（`None / DownWeightSample / LowerPredictionConfidence / ResetRecentRateOnly / RequestModelRetrain`）。

> **现状（不夸大）**：漂移信号目前仅实验计算；`confidence` 与
> `suggested_action` 只是建议信号，**尚未接入任何生产决策路径**（生产只消费
> `anomaly_downweight` 训练权重）。`RobustDetector` 由
> `robust_detection_enabled` 控制，默认关闭。

### 5.3 模块结构

```text
crates/mira-local-ai/src/robust/
  mod.rs        # RobustDetector + BatteryRobustSignals + SuggestedAction
```

现有固定规则（电量突升 / 大幅下降 / 重连 / 充电换电 / 重校准 / 明显错误读数）继续作为第一层，检测器只作补充。

### 5.4 兼容性要求

- 开关关闭时行为与未加检测层逐位一致（有等价性测试）。
- portable state 序列化失败时可安全重建。
- Preview 状态格式不得成为正式模型包唯一依赖。

---

## 6. 状态化模型实验骨架（阶段 5，默认关闭）

> **重要**：本阶段只实现了**库内**的状态化模型骨架（`observe / decide /
> snapshot / restore / reset` 原语），**并未**真正接入 IPC V3、WIT Handler
> ABI v2、host 双栈握手或 runtime 的 stateful 调用。因此：
>
> - 不把阶段 5 计为生产功能完成；
> - 不声称"已完成 Stateful Handler v2 双栈"；
> - 不存在"handshake 失败自动回退"的运行时路径（尚无握手逻辑）。

### 6.1 开关

```rust
pub stateful_handler_enabled: bool  // 默认 false
```

### 6.2 当前实现边界

- 提供 `StatefulBatteryModel` 库内原语与 `decide_stateful_batch` 宿主侧入口。
- `predict` 训练路径**不**调用本骨架；`stateful_handler_enabled` 仅影响
  `decide_stateful_batch` 是否走状态化路径。
- IPC V3 / WIT ABI v2 / host 双栈握手 / runtime stateful 调用均**未实现**，
  属于转正前的预留方向，不做任何实现承诺。
- 默认生产路径保持不变：`IPC V2 + WIT Handler API v1 + 确定性 fallback`。

### 6.3 原语

[`crates/mira-local-ai/src/stateful/mod.rs`](../../crates/mira-local-ai/src/stateful/mod.rs) 提供实验骨架：

```text
observe(sample) / decide(context) / snapshot() / restore(snapshot) / reset(reason)
```

### 6.4 状态身份

snapshot 记录 schema id、schema hash、model descriptor hash、rill-ml version、state format version、device identity、generation id、sample count、latest observation timestamp。恢复时任何关键字段不匹配 → 拒绝 restore、新建空状态、不影响原始历史、记录具体原因。

### 6.5 状态操作

`reset` 支持：schema changed / device changed / battery replaced / user cleared history / model state corrupted / explicit debug reset。器重连**不等同于**换电池。

### 6.6 转正条件

IPC 字节 / 时延 / 内存 / snapshot 大小对比留待状态化路径转正前的专项测试，本轮**不**声称 IPC V3 性能更好。

---

## 7. 可观测性

新增日志不输出用户隐私与完整历史。建议记录：

```text
rill_runtime_version / rill_protocol_version
handler_rill_ml_version / handler_api_version / ipc_api_version
feature_schema_id / feature_schema_hash_prefix / model_descriptor_hash_prefix
training_sample_count / effective_sample_weight / weighted_mae
fallback_reason / robust_detection_enabled / stateful_handler_enabled
```

hash 只输出短前缀；fallback reason 使用结构化枚举。

---

## 8. 相关文件

| 类别 | 路径 |
|---|---|
| 特征 schema 唯一权威 | `crates/mira-local-ai/src/battery_features.rs` |
| 预测 / 加权 / 回退 | `crates/mira-local-ai/src/lib.rs` |
| 稳健检测 | `crates/mira-local-ai/src/robust/mod.rs` |
| 状态化骨架 | `crates/mira-local-ai/src/stateful/mod.rs` |
| 协议配置 | `crates/mira-protocol/src/lib.rs` |
| 回放工具 | `crates/mira-local-ai/src/bin/replay_report.rs` |
| 回放测试 | `crates/mira-local-ai/tests/replay_fixtures.rs` |
| 回放 fixtures | `crates/mira-local-ai/tests/fixtures/replay/` |
| schema golden | `crates/mira-local-ai/tests/fixtures/battery_schema_v1.json` |
| 回放对比报告（历史阶段） | `docs/audits/rill-1.1-battery-replay-report.md` |
| 基线审计（历史阶段） | `docs/audits/rill-1.1-baseline.md` |
| 回退操作 | `docs/operations/local-ai-rollback.md` |
