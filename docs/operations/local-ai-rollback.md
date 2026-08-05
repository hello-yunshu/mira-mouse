# Mira Local AI 回退与故障处理操作文档

> 状态：已实现（阶段 0–6）
> 配套：`docs/architecture/local-ai-rill-compatibility.md` · `docs/audits/rill-1.1-battery-replay-report.md`

本文档说明本轮 rill-ml 1.1.0 适配中，各实验能力如何**单独关闭**，以及发布后如何**回退**到稳定状态。

---

## 1. 三个实验开关（相互独立）

在 `BatteryModelConfig` 中，三个实验能力分别可关，**不共用一个总开关**：

```text
weighted_learning_enabled   = false   # 阶段 3 加权学习（默认关）
robust_detection_enabled    = false   # 阶段 4 稳健检测（默认关）
stateful_handler_enabled    = false   # 阶段 5 状态化 handler（默认关）
```

| 能力 | 关闭后行为 |
|---|---|
| 加权学习 | 训练权重恒为 1.0，行为与冻结路径逐位一致 |
| 稳健检测 | 检测分支完全不参与，等价性测试保证与未加层一致 |
| 状态化 handler | 走现有 IPC V2 + WIT v1 + 确定性 fallback |

> 默认三项均为 `false`，因此**默认发布不受任何实验能力影响**。

---

## 2. 功能回退分类

### 2.1 加权学习

- 关闭：`weighted_learning_enabled = false`。
- 加权模式下质量门使用 `weighted_mae`；不满足时回退 `candidateNotBetter` / `insufficientEffectiveWeight`。
- 回放结论显示加权模型在既有 fixture 上未优于普通模型，因此保持默认关闭。

### 2.2 稳健检测

- 关闭：`robust_detection_enabled = false`。
- 检测器只提供辅助信号（`anomaly_downweight` / 建议动作），**从不**删除数据或重置模型。

### 2.3 状态化 handler

- 关闭：`stateful_handler_enabled = false`。
- 开启时仅尝试实验路径，handshake 失败自动回退到 IPC V2。

### 2.4 legacy 模型兼容

- 无独立布尔开关；旧模型包（无 schema 身份字段）自动进入 legacy 路径，仍检查 `feature_count` 并记录 warning。

---

## 3. 运行期回退链

```text
Stateful Handler v2
   ↓（关闭或 handshake 失败）
Stateless IPC V2 Handler v1
   ↓（model 质量门不通过 / schema 不匹配 / handler 调用失败）
确定性预测（Deterministic prediction）
```

结构化回退原因（`fallback_reason`）：

```text
RuntimeUnavailable / RuntimeVersionMismatch / HandlerLoadFailed
HandlerInvocationFailed / SchemaMismatch / InsufficientSamples
ModelQualityGateFailed / InvalidModelOutput / StatefulRestoreFailed
```

任何回退都不崩溃、不删除用户历史、不影响无 AI 模式。

---

## 4. 发布回退步骤

若新版本发布后出现问题：

1. **关闭实验功能**：将上述三个开关置为 `false`（默认值），重新发布。
2. **继续使用 IPC V2**：默认路径不受影响。
3. **继续使用确定性预测**：模型质量门回退的兜底始终可用。
4. **回滚 handler / model pack**：signature 验证保证可替换回上一版，不发生隐式升级（见 CI 可复现改造）。
5. **历史数据格式不迁移**：本轮不修改原始电量历史存储格式；新增字段全部 optional，旧记录可正常读取，新记录被旧版本忽略时不影响核心数据。

---

## 5. 数据与状态迁移约束

- 尽量不修改原始电量历史存储格式。
- 如必须增加字段：新增字段 optional、带 `serde(default)`、旧记录可读、新记录被旧版本忽略不破坏核心数据、提供序列化往返测试。
- stateful snapshot 恢复时任何关键身份不匹配（schema / descriptor / rill 版本 / state format / device identity / generation）→ 拒绝 restore，新建空状态，不影响原始历史。

---

## 6. 验证命令

回退相关验证：

```bash
cargo test -p mira-local-ai --workspace --locked
cargo test -p mira-local-ai --test replay_fixtures --locked
cargo run -p mira-local-ai --bin replay_report -- table
```

复现构建：

```bash
cargo build --workspace --locked
cargo build --manifest-path handlers/mira-battery-handler/Cargo.toml --locked
```