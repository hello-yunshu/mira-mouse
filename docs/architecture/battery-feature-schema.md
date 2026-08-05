# Mira 电量特征 Schema（Feature Schema / Model Descriptor）

> 状态：已实现（阶段 2）
> 唯一权威实现：[`crates/mira-local-ai/src/battery_features.rs`](file:///Users/yunshu/Documents/GitHub/mira-mouse/crates/mira-local-ai/src/battery_features.rs)
> schema id：`mira-battery-feature-schema-v1` · schema version：`1`

本文档是电量模型特征的一份**人类可读契约**；代码（`battery_feature_schema()` 与 `build_battery_features`）才是唯一权威。两者不一致时以代码为准，并应更新本文档。

---

## 1. 为什么需要 schema 身份

适配前，模型兼容性只检查 `feature_count == 9`，无法识别：特征顺序变化、某特征归一化公式变化、单位变化、新旧模型恰好都是 9 个特征、handler 与 host 对同一索引理解不同。

引入 `FeatureSchema` / `FeatureDescriptor` / `ModelDescriptor` 后，特征顺序、名称、单位、归一化公式与约束全部参与确定性 hash，任何定义变化都会改变 hash，从而**拒绝**按旧定义训练的模型，而不是静默错载。

---

## 2. 特征顺序（身份的一部分）

直接改动顺序会改变 schema hash，并导致旧模型包被拒绝。

| # | 特征名 | 类型 | raw 范围 | normalized | 归一化公式 | 缺失策略 |
|---|---|---|---|---|---|---|
| 0 | `battery_percentage` | f64 | 0..=100 | 0.0..=1.0 | `percentage / 100` | 拒绝样本 |
| 1 | `local_hour_sin` | f64 | 本地小时角 | -1.0..=1.0 | `sin(hour/24×2π)` | 拒绝样本 |
| 2 | `local_hour_cos` | f64 | 本地小时角 | -1.0..=1.0 | `cos(hour/24×2π)` | 拒绝样本 |
| 3 | `weekday_sin` | f64 | 周一=0..=6 | -1.0..=1.0 | `sin(weekday/7×2π)` | 拒绝样本 |
| 4 | `weekday_cos` | f64 | 周一=0..=6 | -1.0..=1.0 | `cos(weekday/7×2π)` | 拒绝样本 |
| 5 | `recent_drain_rate` | f64 | 0..=max_drain（%/h） | /10（不 clamp） | `recent_drain_per_hour / 10` | 取 1.0/10=0.1，标记 context 缺失 |
| 6 | `dpi_normalized` | f64 | 100..=100000 | 0.0..=1.0 | `clamp(dpi/60000, 0, 1)` | 0.0 + 降低 context quality |
| 7 | `polling_rate_normalized` | f64 | 1..=16000 | 0.0..=1.0 | `clamp(polling/16000, 0, 1)` | 0.0 + 降低 context quality |
| 8 | `lighting_intensity` | f64 | mode∈[0,1]×brightness∈[0,100] | 0.0..=1.0 | `mode_intensity × (brightness/100)` | 0.0 + 降低 context quality |

归一化上界预留：`MAX_DPI = 60000`、`MAX_POLLING_RATE_HZ = 16000`（为未来高分辨率/高回报率设备预留 2x 空间）。

> 归一化公式的修改必须创建新的 schema 版本，不得原地改动 v1 语义。

---

## 3. 上下文完整度

用 `FeatureContextQuality` 表示 DPI / 回报率 / 灯光三项上下文的可用性，并作为样本权重修正：

| 状态 | 含义 | 权重系数 |
|---|---|---|
| `FullContext` | 三项上下文全部可用 | 1.0 |
| `PartialContext` | 至少一项缺失 | 0.8 |
| `NoContext` | 完全无上下文 | 0.6 |

---

## 4. 身份计算

### 4.1 schema hash

`battery_schema_hash()` 对 `FeatureSchema`（含顺序、名称、unit、transform、constraint、metadata）取 rill-ml 的确定性 hash，进程内缓存（`OnceLock`），跨进程稳定。

### 4.2 model descriptor hash

`battery_model_descriptor_hash_hex(config)` 对 `ModelDescriptor`（算法 `linear-regression-sgd` + schema hash + 配置摘要）做规范 JSON 序列化后取 SHA-256。

配置摘要覆盖影响训练行为的数值参数（`learning_rate`、`l2`、`huber_delta`、`min_training_samples`、`min_validation_samples`、`required_error_ratio`、`max_drain_per_hour`、`quality_window`、衰减 tau 等），**不含 schema 身份字段自身**。超参数变化 → descriptor hash 变化 → 拒绝按旧身份训练的模型状态。

### 4.3 golden 文件

`tests/fixtures/battery_schema_v1.json` 锁定当前 schema hash。测试 `hash_is_identical_across_processes_and_rebuilds` 断言当前 hash 与 golden 一致，防止无意的 schema 漂移。

---

## 5. 模型加载兼容策略

`check_schema_identity(config)` 返回三态：

| 状态 | 判定 | 行为 |
|---|---|---|
| `Matched` | 声明身份与当前实现一致 | 正常加载 |
| `LegacyModelPack` | 未声明任何身份字段 | 走 legacy 兼容路径：仍检查 `feature_count`，记录 warning，不允许把旧模型重新标记为新 schema |
| `Mismatch` | 声明了身份但不一致（`SchemaId` / `SchemaHash` / `ModelDescriptorHash`） | **拒绝使用**，记录 expected/actual，回退确定性预测，不崩溃、不删除历史 |

---

## 6. 测试覆盖

| 测试 | 验证点 |
|---|---|
| `feature_names_and_count_are_frozen` | 9 个特征、名称固定 |
| `feature_order_change_changes_schema_hash` | 顺序变化 → hash 变化 |
| `normalization_description_change_changes_descriptor` | 归一化说明变化 → descriptor 变化 |
| `hash_is_identical_across_processes_and_rebuilds` | 跨进程 + golden 文件一致 |
| `matching_identity_is_accepted` | 声明匹配 → 正常加载 |
| `wrong_schema_hash_is_rejected` / `wrong_schema_id_is_rejected` | schema 不匹配 → 拒绝 |
| `legacy_config_without_identity_enters_legacy_path` | 旧模型无身份 → legacy 路径 |
| `same_feature_count_but_different_order_is_rejected` | 数量相同但顺序不同 → 拒绝 |
| `descriptor_json_roundtrips_stably` | JSON 序列化往返稳定 |
| `feature_vector_order_matches_names` | 向量顺序与名称一致 |