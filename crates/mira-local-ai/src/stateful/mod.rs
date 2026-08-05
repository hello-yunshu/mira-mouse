// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stateful Handler ABI v2 实验骨架（阶段 5）。
//!
//! 双栈实验：本模块**不替换**默认的 IPC V2 + WIT Handler API v1 链路。
//! 它提供 `observe / decide / snapshot / restore / reset` 原语，以及一个
//! 可配置开关 `config.stateful_handler_enabled`（默认关闭）。默认生产路径
//! 继续使用无状态的重放训练（`predict`），本模块仅在显式开启时被尝试，
//! 任何失败都回退到确定性预测，不影响现有电量历史与模型包。
//!
//! 状态身份（`StateIdentity`）记录 schema id / schema hash / model
//! descriptor hash / rill-ml 版本 / 状态格式版本 / 设备身份 / generation id /
//! 样本数 / 最新观测时间。`restore` 时任何关键身份不匹配都会拒绝恢复并
//! 重建空状态，绝不危害原始电量历史。

use chrono::{DateTime, Utc};
use mira_protocol::{BatteryModelConfig, DeviceContextSnapshot};
use rill_ml::loss::{HuberLoss, RegressionLoss};
use rill_ml::models::{LinearRegression, LinearRegressionConfig};
use rill_ml::optim::{Optimizer, SgdConfig};
use rill_ml::persistence::Snapshot;
use rill_ml::OnlineRegressor;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::battery_features::{
    self, battery_model_descriptor_hash_hex, battery_schema_hash_hex, BATTERY_SCHEMA_ID,
};
use crate::BatteryPredictionError;

/// 与本仓库根 Cargo.lock 解析到的 rill-ml 版本一致（major.minor）。
/// 记录在状态身份中，用于拒绝跨版本恢复。
pub const RILL_ML_VERSION: &str = "1.1";
/// 本模块（identity + model）状态信封的格式版本。
pub const MIRA_STATE_FORMAT_VERSION: u32 = 1;

/// 状态重置原因。任何原因都只清空内存中的模型状态，不触碰原始电量历史。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetReason {
    SchemaChanged,
    DeviceChanged,
    BatteryReplaced,
    HistoryCleared,
    CorruptedState,
    ExplicitDebugReset,
}

/// 状态化模型的输入观测（等价于无状态路径的 `DrainObservation`）。
#[derive(Debug, Clone)]
pub struct StatefulObservation {
    pub at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub timezone_offset_minutes: i32,
    pub percentage: u8,
    pub drain_per_hour: f64,
    pub context: Option<DeviceContextSnapshot>,
}

/// 状态身份：任何字段用于恢复时的一致性校验。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateIdentity {
    pub schema_id: String,
    pub schema_hash: String,
    pub model_descriptor_hash: String,
    pub rill_ml_version: String,
    pub state_format_version: u32,
    pub device_identity: String,
    pub generation_id: u64,
    pub sample_count: u64,
    pub latest_observation_at_ms: i64,
}

/// 增量 EWMA 近期耗电率累加器。是模型状态的一部分：`decide` 依赖它计算
/// `recent_drain_rate` 特征，因此必须随快照持久化，否则 restore 后无法复现决策。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatefulRecentState {
    pub weighted_rate_sum: f64,
    pub total_weight_sum: f64,
    pub prev_ended_at_ms: Option<i64>,
}

/// 可持久化的完整状态信封：身份 + 近期累加器 + rill-ml 的 `Snapshot<LinearRegression>`。
/// 使用 rill-ml 自带的 format version 与 `ValidateState` 校验，不重复发明哈希。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatefulSnapshot {
    pub identity: StateIdentity,
    pub recent: StatefulRecentState,
    pub model: Snapshot<LinearRegression>,
}

/// 恢复失败的具体原因，供日志与回退决策使用（不暴露底层异常文本）。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RestoreError {
    #[error("state format version mismatch: expected {expected}, got {actual}")]
    StateVersionMismatch { expected: u32, actual: u32 },
    #[error("rill-ml version mismatch: expected {expected}, got {actual}")]
    RillVersionMismatch { expected: String, actual: String },
    #[error("schema id mismatch: expected {expected}, got {actual}")]
    SchemaIdMismatch { expected: String, actual: String },
    #[error("schema hash mismatch")]
    SchemaHashMismatch,
    #[error("model descriptor hash mismatch")]
    ModelDescriptorHashMismatch,
    #[error("device identity mismatch: expected {expected}, got {actual}")]
    DeviceIdentityMismatch { expected: String, actual: String },
    #[error("restored model state validation failed: {0}")]
    InvalidModelState(String),
}

impl RestoreError {
    /// 稳定的回退码，供 host 日志/观测使用。
    pub fn fallback_code(&self) -> &'static str {
        match self {
            RestoreError::StateVersionMismatch { .. } => "statefulStateVersionMismatch",
            RestoreError::RillVersionMismatch { .. } => "statefulRillVersionMismatch",
            RestoreError::SchemaIdMismatch { .. } => "statefulSchemaIdMismatch",
            RestoreError::SchemaHashMismatch => "statefulSchemaHashMismatch",
            RestoreError::ModelDescriptorHashMismatch => "statefulDescriptorMismatch",
            RestoreError::DeviceIdentityMismatch { .. } => "statefulDeviceMismatch",
            RestoreError::InvalidModelState(_) => "statefulInvalidModelState",
        }
    }
}

fn new_linear_regression(
    config: &BatteryModelConfig,
) -> Result<LinearRegression, BatteryPredictionError> {
    let mut sgd = SgdConfig::default();
    sgd.learning_rate = config.learning_rate;
    sgd.l2 = config.l2;
    let optimizer = Optimizer::sgd(config.feature_count, sgd)
        .map_err(|_| BatteryPredictionError::InvalidModel)?;
    let mut lr_config = LinearRegressionConfig::default();
    lr_config.optimizer = optimizer;
    lr_config.loss = RegressionLoss::Huber(
        HuberLoss::new(config.huber_delta).map_err(|_| BatteryPredictionError::InvalidModel)?,
    );
    LinearRegression::new(config.feature_count, lr_config)
        .map_err(|_| BatteryPredictionError::InvalidModel)
}

/// 状态化电池模型：持有 `LinearRegression` 与增量 EWMA 近期耗电率累加器。
///
/// 增量训练与无状态路径在**同一有序数据、未加权**的情况下逐位等价
/// （均为 SGD 顺序学习）；因此 `observe` 按 `prediction_time` 未知的约束
/// 使用 weight = 1.0（无状态路径的 recency 权重相对最终 `now` 计算，无法在
/// 增量时刻预先知道）。这是骨架的已知限制，未来可在 `observe` 引入
/// 相对自身时刻的权重策略。
#[derive(Debug)]
pub struct StatefulBatteryModel {
    model: LinearRegression,
    config: BatteryModelConfig,
    identity: StateIdentity,
    // 增量 EWMA：维护 Σ w·rate 与 Σ w，锚定 prev_ended_at。
    weighted_rate_sum: f64,
    total_weight_sum: f64,
    prev_ended_at: Option<DateTime<Utc>>,
}

impl StatefulBatteryModel {
    pub fn new(
        config: &BatteryModelConfig,
        device_identity: String,
        generation_id: u64,
    ) -> Result<Self, BatteryPredictionError> {
        let model = new_linear_regression(config)?;
        let identity = StateIdentity {
            schema_id: BATTERY_SCHEMA_ID.to_owned(),
            schema_hash: battery_schema_hash_hex()?,
            model_descriptor_hash: battery_model_descriptor_hash_hex(config)?,
            rill_ml_version: RILL_ML_VERSION.to_owned(),
            state_format_version: MIRA_STATE_FORMAT_VERSION,
            device_identity,
            generation_id,
            sample_count: 0,
            latest_observation_at_ms: 0,
        };
        Ok(Self {
            model,
            config: config.clone(),
            identity,
            weighted_rate_sum: 0.0,
            total_weight_sum: 0.0,
            prev_ended_at: None,
        })
    }

    pub fn identity(&self) -> &StateIdentity {
        &self.identity
    }

    pub fn samples_seen(&self) -> u64 {
        self.model.samples_seen()
    }

    /// 增量观察一个放电观测：更新 EWMA 累加器并学习一个样本。
    pub fn observe(
        &mut self,
        observation: &StatefulObservation,
    ) -> Result<(), BatteryPredictionError> {
        // 1. 从 prev_ended_at 衰减到 observation.at。
        if let Some(prev) = self.prev_ended_at {
            let dt_hours = (observation.at - prev).num_seconds().max(0) as f64 / 3600.0;
            if dt_hours > 0.0 {
                let decay = (-dt_hours / self.config.baseline_decay_tau_hours).exp();
                self.weighted_rate_sum *= decay;
                self.total_weight_sum *= decay;
            }
        }
        let recent_rate =
            (self.total_weight_sum > 0.0).then_some(self.weighted_rate_sum / self.total_weight_sum);
        let features = battery_features::build_battery_features(
            observation.percentage,
            observation.at,
            observation.timezone_offset_minutes,
            recent_rate,
            observation.context.as_ref(),
        )?;
        self.model
            .learn_weighted(&features.values, observation.drain_per_hour, 1.0)
            .map_err(|_| BatteryPredictionError::InvalidModel)?;

        // 2. 从 observation.at 衰减到 ended_at，再以 weight=1.0 加入当前观测。
        let inner_dt_hours =
            (observation.ended_at - observation.at).num_seconds().max(0) as f64 / 3600.0;
        if inner_dt_hours > 0.0 {
            let decay = (-inner_dt_hours / self.config.baseline_decay_tau_hours).exp();
            self.weighted_rate_sum *= decay;
            self.total_weight_sum *= decay;
        }
        self.weighted_rate_sum += observation.drain_per_hour;
        self.total_weight_sum += 1.0;
        self.prev_ended_at = Some(observation.ended_at);

        self.identity.sample_count += 1;
        self.identity.latest_observation_at_ms = observation.ended_at.timestamp_millis();
        Ok(())
    }

    /// 基于当前状态做出决策，返回预测的放电率（不套安全门，由调用方决定）。
    pub fn decide(
        &self,
        current_percentage: u8,
        now: DateTime<Utc>,
        now_timezone_offset_minutes: i32,
        current_context: Option<&DeviceContextSnapshot>,
    ) -> Result<f64, BatteryPredictionError> {
        let (mut rate_sum, mut weight_sum) = (self.weighted_rate_sum, self.total_weight_sum);
        if let Some(prev) = self.prev_ended_at {
            let dt_hours = (now - prev).num_seconds().max(0) as f64 / 3600.0;
            if dt_hours > 0.0 {
                let decay = (-dt_hours / self.config.baseline_decay_tau_hours).exp();
                rate_sum *= decay;
                weight_sum *= decay;
            }
        }
        let recent_rate = (weight_sum > 0.0).then_some(rate_sum / weight_sum);
        let features = battery_features::build_battery_features(
            current_percentage,
            now,
            now_timezone_offset_minutes,
            recent_rate,
            current_context,
        )?;
        self.model
            .predict(&features.values)
            .map_err(|_| BatteryPredictionError::InvalidModel)
    }

    /// 生成可持久化的快照（身份 + 近期累加器 + 模型状态）。
    pub fn snapshot(&self) -> StatefulSnapshot {
        StatefulSnapshot {
            identity: self.identity.clone(),
            recent: StatefulRecentState {
                weighted_rate_sum: self.weighted_rate_sum,
                total_weight_sum: self.total_weight_sum,
                prev_ended_at_ms: self.prev_ended_at.map(|t| t.timestamp_millis()),
            },
            model: Snapshot::new(self.model.clone()),
        }
    }

    /// 从快照恢复模型。任何关键身份不匹配都会拒绝恢复并返回具体原因。
    pub fn restore(
        snapshot: StatefulSnapshot,
        config: &BatteryModelConfig,
        current_device_identity: &str,
        current_generation_id: u64,
    ) -> Result<Self, RestoreError> {
        let identity = &snapshot.identity;
        if identity.state_format_version != MIRA_STATE_FORMAT_VERSION {
            return Err(RestoreError::StateVersionMismatch {
                expected: MIRA_STATE_FORMAT_VERSION,
                actual: identity.state_format_version,
            });
        }
        if identity.rill_ml_version != RILL_ML_VERSION {
            return Err(RestoreError::RillVersionMismatch {
                expected: RILL_ML_VERSION.to_owned(),
                actual: identity.rill_ml_version.clone(),
            });
        }
        if identity.schema_id != BATTERY_SCHEMA_ID {
            return Err(RestoreError::SchemaIdMismatch {
                expected: BATTERY_SCHEMA_ID.to_owned(),
                actual: identity.schema_id.clone(),
            });
        }
        let expected_schema_hash = battery_schema_hash_hex()
            .map_err(|error| RestoreError::InvalidModelState(error.to_string()))?;
        if identity.schema_hash != expected_schema_hash {
            return Err(RestoreError::SchemaHashMismatch);
        }
        let expected_descriptor_hash = battery_model_descriptor_hash_hex(config)
            .map_err(|error| RestoreError::InvalidModelState(error.to_string()))?;
        if identity.model_descriptor_hash != expected_descriptor_hash {
            return Err(RestoreError::ModelDescriptorHashMismatch);
        }
        if identity.device_identity != current_device_identity {
            return Err(RestoreError::DeviceIdentityMismatch {
                expected: current_device_identity.to_owned(),
                actual: identity.device_identity.clone(),
            });
        }
        if identity.generation_id != current_generation_id {
            return Err(RestoreError::DeviceIdentityMismatch {
                expected: current_generation_id.to_string(),
                actual: identity.generation_id.to_string(),
            });
        }

        let model = snapshot
            .model
            .into_validated_model()
            .map_err(|error| RestoreError::InvalidModelState(error.to_string()))?;
        let prev_ended_at = snapshot
            .recent
            .prev_ended_at_ms
            .and_then(DateTime::from_timestamp_millis);
        Ok(Self {
            model,
            config: config.clone(),
            identity: identity.clone(),
            weighted_rate_sum: snapshot.recent.weighted_rate_sum,
            total_weight_sum: snapshot.recent.total_weight_sum,
            prev_ended_at,
        })
    }

    /// 重置内存状态。不触碰原始电量历史。
    pub fn reset(&mut self, reason: ResetReason) {
        let device_identity = self.identity.device_identity.clone();
        let generation_id = self.identity.generation_id.wrapping_add(1);
        let config = self.config.clone();
        let _ = reason;
        if let Ok(replacement) = Self::new(&config, device_identity, generation_id) {
            *self = replacement;
        }
    }
}

/// 双栈决策结果：`Ok(Some(rate))` 表示状态化路径成功；
/// `Ok(None)` 表示未启用状态化路径（走原有 IPC V2 + WIT v1 / 确定性）。
#[derive(Debug)]
pub enum StatefulDecision {
    /// 状态化路径产出有效放电率。
    Used(f64),
    /// 未启用状态化路径，回退到无状态路径。
    Disabled,
}

/// 一批观测的增量建模入口（双栈实验的宿主侧原语）。
/// 生成新模型 → 依次 observe → decide，返回状态化决策。
pub fn decide_stateful_batch(
    observations: &[StatefulObservation],
    current_percentage: u8,
    now: DateTime<Utc>,
    now_timezone_offset_minutes: i32,
    current_context: Option<&DeviceContextSnapshot>,
    device_identity: &str,
    config: &BatteryModelConfig,
) -> Result<StatefulDecision, BatteryPredictionError> {
    if !config.stateful_handler_enabled {
        return Ok(StatefulDecision::Disabled);
    }
    let mut model = StatefulBatteryModel::new(config, device_identity.to_owned(), 1)?;
    for observation in observations {
        model.observe(observation)?;
    }
    let rate = model.decide(
        current_percentage,
        now,
        now_timezone_offset_minutes,
        current_context,
    )?;
    if !rate.is_finite() || rate <= 0.0 || rate > config.max_drain_per_hour {
        return Ok(StatefulDecision::Disabled);
    }
    Ok(StatefulDecision::Used(rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Timelike};

    fn test_config() -> BatteryModelConfig {
        BatteryModelConfig::default()
    }

    fn observation(start: DateTime<Utc>, index: usize, drain: f64) -> StatefulObservation {
        let at = start + Duration::hours(index as i64);
        StatefulObservation {
            at,
            ended_at: at + Duration::minutes(30),
            timezone_offset_minutes: 0,
            percentage: 80,
            drain_per_hour: drain,
            context: None,
        }
    }

    #[test]
    fn observe_then_decide_produces_finite_rate() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        for index in 0..120 {
            model.observe(&observation(start, index, 5.0)).unwrap();
        }
        let rate = model
            .decide(80, start + Duration::hours(121), 0, None)
            .unwrap();
        assert!(rate.is_finite() && rate > 0.0);
        assert_eq!(model.samples_seen(), 120);
        assert_eq!(model.identity().sample_count, 120);
    }

    #[test]
    fn snapshot_restore_roundtrip_preserves_decision() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        for index in 0..120 {
            model.observe(&observation(start, index, 5.0)).unwrap();
        }
        let snapshot = model.snapshot();
        let restored =
            StatefulBatteryModel::restore(snapshot, &test_config(), "device-a", 1).unwrap();
        let now = start + Duration::hours(121);
        let before = model.decide(80, now, 0, None).unwrap();
        let after = restored.decide(80, now, 0, None).unwrap();
        assert!((before - after).abs() < 1e-9);
        assert_eq!(restored.identity().sample_count, 120);
    }

    #[test]
    fn restore_rejects_state_version_mismatch() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        model.observe(&observation(start, 0, 5.0)).unwrap();
        let mut snapshot = model.snapshot();
        snapshot.identity.state_format_version = 999;
        let error =
            StatefulBatteryModel::restore(snapshot, &test_config(), "device-a", 1).unwrap_err();
        assert!(matches!(error, RestoreError::StateVersionMismatch { .. }));
    }

    #[test]
    fn restore_rejects_rill_version_mismatch() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        model.observe(&observation(start, 0, 5.0)).unwrap();
        let mut snapshot = model.snapshot();
        snapshot.identity.rill_ml_version = "0.9".into();
        let error =
            StatefulBatteryModel::restore(snapshot, &test_config(), "device-a", 1).unwrap_err();
        assert!(matches!(error, RestoreError::RillVersionMismatch { .. }));
    }

    #[test]
    fn restore_rejects_schema_hash_mismatch() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        model.observe(&observation(start, 0, 5.0)).unwrap();
        let mut snapshot = model.snapshot();
        snapshot.identity.schema_hash = "0".repeat(64);
        let error =
            StatefulBatteryModel::restore(snapshot, &test_config(), "device-a", 1).unwrap_err();
        assert!(matches!(error, RestoreError::SchemaHashMismatch));
    }

    #[test]
    fn restore_rejects_device_identity_mismatch() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        model.observe(&observation(start, 0, 5.0)).unwrap();
        let snapshot = model.snapshot();
        let error =
            StatefulBatteryModel::restore(snapshot, &test_config(), "device-b", 1).unwrap_err();
        assert!(matches!(error, RestoreError::DeviceIdentityMismatch { .. }));
    }

    #[test]
    fn restore_rejects_generation_mismatch() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        model.observe(&observation(start, 0, 5.0)).unwrap();
        let snapshot = model.snapshot();
        let error =
            StatefulBatteryModel::restore(snapshot, &test_config(), "device-a", 5).unwrap_err();
        assert!(matches!(error, RestoreError::DeviceIdentityMismatch { .. }));
    }

    #[test]
    fn reset_clears_state_and_bumps_generation() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        for index in 0..120 {
            model.observe(&observation(start, index, 5.0)).unwrap();
        }
        model.reset(ResetReason::BatteryReplaced);
        assert_eq!(model.samples_seen(), 0);
        assert_eq!(model.identity().generation_id, 2);
        assert_eq!(model.identity().device_identity, "device-a");
    }

    /// 增量状态化训练与无状态批量训练在未加权数据上给出相同最终预测。
    /// 这证明 V2（每次重放）与 V3（增量 observe）在相同历史上的输出误差≈0。
    #[test]
    fn stateful_incremental_matches_batch_training() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = test_config();
        let observations: Vec<StatefulObservation> = (0..120)
            .map(|index| {
                let at = start + Duration::hours(index as i64);
                let angle = at.hour() as f64 / 24.0 * std::f64::consts::TAU;
                StatefulObservation {
                    at,
                    ended_at: at + Duration::minutes(30),
                    timezone_offset_minutes: 0,
                    percentage: 80,
                    drain_per_hour: 5.0 + 3.0 * angle.sin() + 1.5 * angle.cos(),
                    context: None,
                }
            })
            .collect();
        let now = start + Duration::hours(121);

        // 状态化：增量 observe
        let mut stateful = StatefulBatteryModel::new(&config, "device-a".into(), 1).unwrap();
        for obs in &observations {
            stateful.observe(obs).unwrap();
        }
        let stateful_rate = stateful.decide(80, now, 0, None).unwrap();

        // 无状态：从头批量训练（同一有序数据、weight=1.0）
        let mut batch = new_linear_regression(&config).unwrap();
        let mut acc = (0.0f64, 0.0f64);
        let mut prev: Option<DateTime<Utc>> = None;
        for obs in &observations {
            if let Some(p) = prev {
                let dt = (obs.at - p).num_seconds().max(0) as f64 / 3600.0;
                let decay = (-dt / config.baseline_decay_tau_hours).exp();
                acc.0 *= decay;
                acc.1 *= decay;
            }
            let recent_rate = (acc.1 > 0.0).then_some(acc.0 / acc.1);
            let features = battery_features::build_battery_features(
                obs.percentage,
                obs.at,
                obs.timezone_offset_minutes,
                recent_rate,
                obs.context.as_ref(),
            )
            .unwrap();
            batch.learn(&features.values, obs.drain_per_hour).unwrap();
            let inner = (obs.ended_at - obs.at).num_seconds().max(0) as f64 / 3600.0;
            let decay = (-inner / config.baseline_decay_tau_hours).exp();
            acc.0 *= decay;
            acc.1 *= decay;
            acc.0 += obs.drain_per_hour;
            acc.1 += 1.0;
            prev = Some(obs.ended_at);
        }
        let mut b_acc = acc;
        if let Some(p) = prev {
            let dt = (now - p).num_seconds().max(0) as f64 / 3600.0;
            let decay = (-dt / config.baseline_decay_tau_hours).exp();
            b_acc.0 *= decay;
            b_acc.1 *= decay;
        }
        let recent_rate = (b_acc.1 > 0.0).then_some(b_acc.0 / b_acc.1);
        let batch_features =
            battery_features::build_battery_features(80, now, 0, recent_rate, None).unwrap();
        let batch_rate = batch.predict(&batch_features.values).unwrap();

        assert!(
            (stateful_rate - batch_rate).abs() < 1e-9,
            "stateful {stateful_rate} vs batch {batch_rate}"
        );
    }

    #[test]
    fn snapshot_json_roundtrip_restores_across_processes() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let mut model = StatefulBatteryModel::new(&test_config(), "device-a".into(), 1).unwrap();
        for index in 0..120 {
            model.observe(&observation(start, index, 5.0)).unwrap();
        }
        let snapshot = model.snapshot();
        // 模拟跨进程传输：序列化为 JSON 再反序列化。
        let json = serde_json::to_string(&snapshot).unwrap();
        let decoded: StatefulSnapshot = serde_json::from_str(&json).unwrap();
        let restored =
            StatefulBatteryModel::restore(decoded, &test_config(), "device-a", 1).unwrap();
        let now = start + Duration::hours(121);
        let before = model.decide(80, now, 0, None).unwrap();
        let after = restored.decide(80, now, 0, None).unwrap();
        assert!((before - after).abs() < 1e-9);
        assert_eq!(restored.identity().sample_count, 120);
    }

    #[test]
    fn disabled_flag_returns_disabled_decision() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig {
            stateful_handler_enabled: false,
            ..BatteryModelConfig::default()
        };
        let observations = vec![observation(start, 0, 5.0)];
        let decision = decide_stateful_batch(
            &observations,
            80,
            start + Duration::hours(1),
            0,
            None,
            "device-a",
            &config,
        )
        .unwrap();
        assert!(matches!(decision, StatefulDecision::Disabled));
    }

    #[test]
    fn enabled_flag_uses_stateful_path() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig {
            stateful_handler_enabled: true,
            ..BatteryModelConfig::default()
        };
        let observations: Vec<StatefulObservation> = (0..120)
            .map(|index| observation(start, index, 5.0))
            .collect();
        let decision = decide_stateful_batch(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            None,
            "device-a",
            &config,
        )
        .unwrap();
        match decision {
            StatefulDecision::Used(rate) => assert!(rate.is_finite() && rate > 0.0),
            other => panic!("expected Used, got {other:?}"),
        }
    }
}
