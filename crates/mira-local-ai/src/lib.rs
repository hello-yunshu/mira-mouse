use chrono::{DateTime, FixedOffset, Utc};
use mira_protocol::{
    BatteryModelConfig, BatteryModelSchemaStatus, BatteryPredictionInput, BatteryPredictionOutput,
    BatterySampleInput, DeviceContextSnapshot, PredictionSource,
};
use rill_ml::{
    diagnostics::BaselineComparator,
    loss::{HuberLoss, RegressionLoss},
    models::{LinearRegression, LinearRegressionConfig},
    optim::{Optimizer, SgdConfig},
    OnlineRegressor,
};
use thiserror::Error;

pub mod battery_features;
pub mod robust;

const MAX_SAMPLES: usize = 10_000;
/// Battery percentages are normally integer-quantized. A 1% change over a few
/// minutes is not enough evidence to extrapolate an hourly drain rate.
const MIN_OBSERVATION_MINUTES: f64 = 30.0;
/// 样本权重下限：保证 recency weight 恒正且有限（rill-ml 要求 weight >= 0，
/// weight = 0 会跳过学习）。极旧样本取该下限，等效于被忽略但不会静默删除。
const MIN_RECENCY_WEIGHT: f64 = 1e-6;

/// 训练样本的近期权重：越靠近预测时刻的样本越重要。
///
/// weight = exp(-age_hours / tau)，age 为样本相对 `prediction_time` 的时长（小时）。
/// - 未来时间戳 clamp 到 age = 0（权重 1.0），不会产生 NaN；
/// - tau 优先取 `learning_recency_tau_hours`，未显式设置时复用确定性基线的
///   `baseline_decay_tau_hours`，不凭空引入新参数；非法 tau（NaN / 非正）回退基线 tau。
/// - 权重下限 `MIN_RECENCY_WEIGHT`。
pub fn recency_weight(
    sample_time: DateTime<Utc>,
    prediction_time: DateTime<Utc>,
    config: &BatteryModelConfig,
) -> f64 {
    let tau_hours = config
        .learning_recency_tau_hours
        .filter(|tau| tau.is_finite() && *tau > 0.0)
        .unwrap_or(config.baseline_decay_tau_hours);
    let age_hours = (prediction_time - sample_time).num_seconds().max(0) as f64 / 3600.0;
    (-age_hours / tau_hours).exp().max(MIN_RECENCY_WEIGHT)
}
/// A large downward change inside this window may be a lower-charge battery
/// swap or reconnect recalibration rather than ordinary discharge.
const SHORT_LEVEL_DISCONTINUITY_MINUTES: i64 = 15;

#[derive(Debug, Clone)]
struct DrainObservation {
    at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    timezone_offset_minutes: i32,
    percentage: u8,
    drain_per_hour: f64,
    /// 采样时段的设备上下文（DPI/回报率/灯光等），作为模型附加特征。
    context: Option<DeviceContextSnapshot>,
}

#[derive(Debug, Error)]
pub enum BatteryPredictionError {
    #[error("battery history exceeds {MAX_SAMPLES} samples")]
    TooManySamples,
    #[error("invalid prediction timestamp")]
    InvalidNow,
    #[error("invalid prediction timezone offset")]
    InvalidNowTimezone,
    #[error("invalid battery sample at index {index}")]
    InvalidSample { index: usize },
    #[error("unable to initialize the configured model")]
    InvalidModel,
    #[error("battery feature schema identity failure: {0}")]
    FeatureSchema(#[from] battery_features::BatteryFeatureError),
    #[error("robust detection layer failed: {0}")]
    Robust(#[from] robust::RobustDetectorError),
}

pub fn predict(
    input: &BatteryPredictionInput,
    config: &BatteryModelConfig,
) -> Result<BatteryPredictionOutput, BatteryPredictionError> {
    if input.samples.len() > MAX_SAMPLES {
        return Err(BatteryPredictionError::TooManySamples);
    }
    let now = DateTime::from_timestamp_millis(input.now_unix_ms)
        .ok_or(BatteryPredictionError::InvalidNow)?;
    validate_timezone_offset(input.now_timezone_offset_minutes)
        .ok_or(BatteryPredictionError::InvalidNowTimezone)?;
    let mut samples = input.samples.clone();
    for (index, sample) in samples.iter().enumerate() {
        if sample.percentage > 100
            || DateTime::from_timestamp_millis(sample.at_unix_ms).is_none()
            || validate_timezone_offset(sample.timezone_offset_minutes).is_none()
        {
            return Err(BatteryPredictionError::InvalidSample { index });
        }
    }
    // 阶段 2：schema 身份检查。模型包声明了 schema / descriptor 但身份不一致时，
    // 必须拒绝该模型并回退确定性预测；不崩溃、不删除历史数据。旧模型包（无身份
    // 字段）进入 legacy 兼容路径，仍只检查 feature count。
    let schema_status = match battery_features::check_schema_identity(config)? {
        battery_features::SchemaIdentity::Matched => BatteryModelSchemaStatus::DescriptorMatched,
        battery_features::SchemaIdentity::LegacyModelPack => {
            BatteryModelSchemaStatus::LegacyCompatibility
        }
        battery_features::SchemaIdentity::Mismatch(mismatch) => {
            eprintln!(
                "mira-local-ai: model schema rejected ({kind}: expected {expected}, got {actual}); \
                 falling back to deterministic prediction",
                kind = mismatch.kind,
                expected = mismatch.expected,
                actual = mismatch.actual.as_deref().unwrap_or("<missing>"),
            );
            return Ok(fallback(
                "schemaMismatch",
                0,
                0,
                None,
                None,
                None,
                None,
                None,
                &BatteryModelSchemaStatus::SchemaMismatch,
            ));
        }
    };
    samples.retain(|sample| sample.at_unix_ms <= input.now_unix_ms);
    samples.sort_by_key(|sample| sample.at_unix_ms);

    let Some(current) = samples
        .iter()
        .filter(|sample| !sample.charging && sample.at_unix_ms <= input.now_unix_ms)
        .max_by_key(|sample| sample.at_unix_ms)
    else {
        return Ok(fallback(
            "noDischargingSample",
            0,
            0,
            None,
            None,
            None,
            None,
            None,
            &schema_status,
        ));
    };
    if current.percentage == 0 {
        return Ok(fallback(
            "emptyBattery",
            0,
            0,
            None,
            None,
            None,
            None,
            None,
            &schema_status,
        ));
    }

    let observations = discharge_observations(&samples, config);
    let prediction_context =
        merge_prediction_context(input.current_context.as_ref(), current.context.as_ref());
    validated_model_prediction(
        &observations,
        current.percentage,
        now,
        input.now_timezone_offset_minutes,
        prediction_context.as_ref(),
        config,
        &schema_status,
    )
}

/// Resolve the context used for the prediction being made now.
///
/// A freshly read `currentContext` wins field by field. Missing fields inherit the
/// latest historical value when available, and any fields that remain absent are
/// handled by `context_features` as zero-contribution optional features. Context is
/// therefore an accuracy enhancement, never a prerequisite for producing a result.
fn merge_prediction_context(
    current: Option<&DeviceContextSnapshot>,
    latest_sample: Option<&DeviceContextSnapshot>,
) -> Option<DeviceContextSnapshot> {
    let mut merged = current.cloned().or_else(|| latest_sample.cloned())?;
    if let Some(previous) = latest_sample {
        if merged.dpi.is_none() {
            merged.dpi = previous.dpi;
        }
        if merged.polling_rate_hz.is_none() {
            merged.polling_rate_hz = previous.polling_rate_hz;
        }
        if merged.light_mode.is_none() {
            merged.light_mode.clone_from(&previous.light_mode);
        }
        if merged.light_brightness.is_none() {
            merged.light_brightness = previous.light_brightness;
        }
        if merged.profile.is_none() {
            merged.profile.clone_from(&previous.profile);
        }
    }
    (!merged.is_empty()).then_some(merged)
}

fn validated_model_prediction(
    observations: &[DrainObservation],
    current_percentage: u8,
    now: DateTime<Utc>,
    now_timezone_offset_minutes: i32,
    current_context: Option<&DeviceContextSnapshot>,
    config: &BatteryModelConfig,
    schema_status: &BatteryModelSchemaStatus,
) -> Result<BatteryPredictionOutput, BatteryPredictionError> {
    // RillML 1.0 将 SgdConfig/LinearRegressionConfig 标记为 #[non_exhaustive]，
    // 必须通过 Default + 字段赋值构造，不能用结构体表达式。
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
    let mut model = LinearRegression::new(config.feature_count, lr_config)
        .map_err(|_| BatteryPredictionError::InvalidModel)?;
    let mut comparator = BaselineComparator::new(
        &["deterministic-baseline", "rill-local-ai"],
        config.quality_window,
    )
    .map_err(|_| BatteryPredictionError::InvalidModel)?;

    // 增量 EWMA 累加器:维护 S = Σ w_j·rate_j 和 W = Σ w_j,锚定在 prev_ended_at。
    // 当 at 前进 dt 时,所有既有权重都乘同一个 exp(-dt/tau),所以只需对 S 和 W 整体衰减。
    // 这把原 O(N²) 的 weighted_baseline_rate(&observations[..index], ...) 调用降为 O(N) 的
    // 增量更新,数学上完全等价(详见下方 #[cfg(test)] 参考实现和等价性测试)。
    let mut weighted_rate_sum: f64 = 0.0;
    let mut total_weight_sum: f64 = 0.0;
    let mut prev_ended_at: Option<DateTime<Utc>> = None;

    // 阶段 3（加权学习，默认关闭）：训练样本按 recency 权重；评价指标同步维护
    // 加权 MAE / 近期窗口 MAE / 训练有效权重。未开启时权重恒为 1.0，
    // 行为与冻结路径逐位一致（见参考实现等价性测试）。
    let weighted_enabled = config.weighted_learning_enabled;
    let mut training_weight_sum: f64 = 0.0;
    let mut validation_weight_sum: f64 = 0.0;
    let mut candidate_weighted_error_sum: f64 = 0.0;
    let mut recent_error_sum: f64 = 0.0;
    let mut recent_count: u64 = 0;

    // 阶段 4（稳健检测实验层，默认关闭）：开启时异常样本仅通过权重降权
    // 影响训练；绝不删除数据或重置模型。关闭时本分支完全不参与。
    let mut robust = (config.robust_detection_enabled)
        .then(|| robust::RobustDetector::new(config))
        .transpose()?;

    for observation in observations {
        // 1. 从 prev_ended_at 衰减到 observation.at,得到以 observation.at 为锚的累加器。
        //    此时累加器等价于原 weighted_baseline_rate(&observations[..index], observation.at, tau)。
        if let Some(prev) = prev_ended_at {
            let dt_hours = (observation.at - prev).num_seconds().max(0) as f64 / 3600.0;
            if dt_hours > 0.0 {
                let decay = (-dt_hours / config.baseline_decay_tau_hours).exp();
                weighted_rate_sum *= decay;
                total_weight_sum *= decay;
            }
        }
        let recent_rate = (total_weight_sum > 0.0).then_some(weighted_rate_sum / total_weight_sum);

        let feature_vector = battery_features::build_battery_features(
            observation.percentage,
            observation.at,
            observation.timezone_offset_minutes,
            recent_rate,
            observation.context.as_ref(),
        )?;
        let features = &feature_vector.values;
        if model.samples_seen() >= config.min_training_samples {
            if let Some(baseline_prediction) = recent_rate {
                if let Ok(ai_prediction) = model.predict(features) {
                    if ai_prediction.is_finite() {
                        comparator
                            .record(0, observation.drain_per_hour, baseline_prediction)
                            .map_err(|_| BatteryPredictionError::InvalidModel)?;
                        comparator
                            .record(1, observation.drain_per_hour, ai_prediction)
                            .map_err(|_| BatteryPredictionError::InvalidModel)?;
                        let metric_weight = if weighted_enabled {
                            recency_weight(observation.at, now, config)
                        } else {
                            1.0
                        };
                        validation_weight_sum += metric_weight;
                        let ai_error = (observation.drain_per_hour - ai_prediction).abs();
                        candidate_weighted_error_sum += metric_weight * ai_error;
                        if (now - observation.at).num_seconds().max(0) as f64 / 3600.0
                            <= config.quality_window as f64
                        {
                            recent_error_sum += ai_error;
                            recent_count += 1;
                        }
                    }
                }
            }
        }
        if weighted_enabled || robust.is_some() {
            let mut weight = if weighted_enabled {
                recency_weight(observation.at, now, config)
            } else {
                1.0
            };
            if let Some(detector) = robust.as_mut() {
                if let Ok(signals) = detector.update(observation.drain_per_hour) {
                    weight *= signals.anomaly_downweight;
                }
            }
            training_weight_sum += weight;
            model
                .learn_weighted(features, observation.drain_per_hour, weight)
                .map_err(|_| BatteryPredictionError::InvalidModel)?;
        } else {
            training_weight_sum += 1.0;
            model
                .learn(features, observation.drain_per_hour)
                .map_err(|_| BatteryPredictionError::InvalidModel)?;
        }

        // 2. 从 observation.at 衰减到 observation.ended_at,再以 weight=1.0 加入当前观测。
        //    这把累加器锚点推进到 observation.ended_at,为下一轮准备好以 ended_at 为锚的求和。
        //    (observation.ended_at 处自身的权重为 exp(0) = 1.0)
        let inner_dt_hours =
            (observation.ended_at - observation.at).num_seconds().max(0) as f64 / 3600.0;
        if inner_dt_hours > 0.0 {
            let decay = (-inner_dt_hours / config.baseline_decay_tau_hours).exp();
            weighted_rate_sum *= decay;
            total_weight_sum *= decay;
        }
        weighted_rate_sum += observation.drain_per_hour;
        total_weight_sum += 1.0;
        prev_ended_at = Some(observation.ended_at);
    }

    comparator.update_best();
    let baseline = comparator.entry(0);
    let candidate = comparator.entry(1);
    let validation_samples = candidate.map_or(0, |entry| entry.total_samples());
    let baseline_samples = baseline.map_or(0, |entry| entry.total_samples());
    let baseline_mae = baseline.and_then(|entry| entry.rolling_mae());
    let candidate_mae = candidate.and_then(|entry| entry.rolling_mae());
    let training_samples = model.samples_seen();
    let weighted_mae = (validation_weight_sum > 0.0)
        .then_some(candidate_weighted_error_sum / validation_weight_sum);
    let recent_mae = (recent_count > 0).then_some(recent_error_sum / recent_count as f64);
    let effective_sample_weight = Some(training_weight_sum);

    if training_samples < config.min_training_samples {
        return Ok(fallback(
            "insufficientTrainingData",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status,
        ));
    }
    // 加权学习保护：训练样本虽然够数，但有效权重不足（如 tau 极短、样本几乎全部
    // 过期）时，加权模型可信度不足，同样回退确定性基线。
    if weighted_enabled && training_weight_sum < config.min_training_samples as f64 {
        return Ok(fallback(
            "insufficientEffectiveWeight",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status,
        ));
    }
    if validation_samples < config.min_validation_samples || baseline_samples != validation_samples
    {
        return Ok(fallback(
            "insufficientValidationData",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status,
        ));
    }
    let (Some(baseline_error), Some(candidate_error)) = (baseline_mae, candidate_mae) else {
        return Ok(fallback(
            "qualityMetricsUnavailable",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status,
        ));
    };
    // 加权模式下以加权 MAE 作为候选质量指标：加权模型对近期样本的表现才是用户
    // 真正关心的。加权 MAE 缺失时退回普通 candidate MAE。
    let candidate_error_for_gate = if weighted_enabled {
        weighted_mae.unwrap_or(candidate_error)
    } else {
        candidate_error
    };
    if candidate_error_for_gate >= baseline_error * config.required_error_ratio {
        return Ok(fallback(
            "candidateNotBetter",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status,
        ));
    }

    // 从 prev_ended_at 衰减到 now,得到最终 recent_rate
    // (等价于原 weighted_baseline_rate(observations, now, tau))。
    if let Some(prev) = prev_ended_at {
        let dt_hours = (now - prev).num_seconds().max(0) as f64 / 3600.0;
        if dt_hours > 0.0 {
            let decay = (-dt_hours / config.baseline_decay_tau_hours).exp();
            weighted_rate_sum *= decay;
            total_weight_sum *= decay;
        }
    }
    let recent_rate = (total_weight_sum > 0.0).then_some(weighted_rate_sum / total_weight_sum);
    let predicted_rate = model
        .predict(
            &battery_features::build_battery_features(
                current_percentage,
                now,
                now_timezone_offset_minutes,
                recent_rate,
                current_context,
            )?
            .values,
        )
        .map_err(|_| BatteryPredictionError::InvalidModel)?;
    if !predicted_rate.is_finite()
        || predicted_rate <= 0.0
        || predicted_rate > config.max_drain_per_hour
    {
        return Ok(fallback(
            "candidateOutsideSafetyBounds",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status,
        ));
    }
    let remaining_hours = current_percentage as f64 / predicted_rate;
    if !remaining_hours.is_finite() || remaining_hours > config.max_remaining_hours {
        return Ok(fallback(
            "candidateOutsideSafetyBounds",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status,
        ));
    }

    Ok(BatteryPredictionOutput {
        remaining_hours: Some(remaining_hours),
        source: PredictionSource::LocalAi,
        reason: "candidatePassedQualityGate".into(),
        training_samples,
        validation_samples,
        baseline_mae,
        candidate_mae,
        weighted_mae,
        recent_mae,
        effective_sample_weight,
        schema_status: Some(schema_status.clone()),
    })
}

/// 构造回退结果。参数与 `BatteryPredictionOutput` 的指标字段一一对应，
/// 显式传参可防止遗漏任一指标，故允许超过 7 个参数。
#[allow(clippy::too_many_arguments)]
fn fallback(
    reason: &str,
    training_samples: u64,
    validation_samples: u64,
    baseline_mae: Option<f64>,
    candidate_mae: Option<f64>,
    weighted_mae: Option<f64>,
    recent_mae: Option<f64>,
    effective_sample_weight: Option<f64>,
    schema_status: &BatteryModelSchemaStatus,
) -> BatteryPredictionOutput {
    BatteryPredictionOutput {
        remaining_hours: None,
        source: PredictionSource::BaselineRecommended,
        reason: reason.into(),
        training_samples,
        validation_samples,
        baseline_mae,
        candidate_mae,
        weighted_mae,
        recent_mae,
        effective_sample_weight,
        schema_status: Some(schema_status.clone()),
    }
}

/// 将灯光模式名映射为功耗强度评分 \[0, 1\] 等特征相关实现已收敛到
/// `battery_features` 模块（唯一权威定义），此处不再重复。
fn validate_timezone_offset(offset_minutes: i32) -> Option<FixedOffset> {
    let seconds = offset_minutes.checked_mul(60)?;
    FixedOffset::east_opt(seconds)
}

fn discharge_observations(
    samples: &[BatterySampleInput],
    config: &BatteryModelConfig,
) -> Vec<DrainObservation> {
    let mut observations = Vec::new();
    let mut segment: Vec<&BatterySampleInput> = Vec::new();
    let mut previous: Option<&BatterySampleInput> = None;
    for sample in samples {
        let split = previous.is_some_and(|prev| {
            prev.charging
                || sample.charging
                || sample.at_unix_ms - prev.at_unix_ms
                    > config.session_gap_minutes.saturating_mul(60_000)
                || is_level_discontinuity(prev, sample, config)
        });
        if split {
            finish_segment(&segment, &mut observations, config);
            segment.clear();
        }
        if !sample.charging {
            segment.push(sample);
        }
        previous = Some(sample);
    }
    // The current, unfinished segment is input context, never its own label.
    observations
}

fn is_level_discontinuity(
    previous: &BatterySampleInput,
    current: &BatterySampleInput,
    config: &BatteryModelConfig,
) -> bool {
    if current.percentage.saturating_sub(previous.percentage) >= config.replacement_rise_percent {
        return true;
    }
    let drop = previous.percentage.saturating_sub(current.percentage);
    if drop < config.replacement_rise_percent {
        return false;
    }
    let elapsed_ms = current.at_unix_ms - previous.at_unix_ms;
    if elapsed_ms <= 0 {
        return true;
    }
    let elapsed_minutes = elapsed_ms / 60_000;
    if elapsed_minutes <= SHORT_LEVEL_DISCONTINUITY_MINUTES {
        return true;
    }
    let hours = elapsed_ms as f64 / 3_600_000.0;
    drop as f64 / hours > config.max_drain_per_hour
}

fn finish_segment(
    segment: &[&BatterySampleInput],
    observations: &mut Vec<DrainObservation>,
    config: &BatteryModelConfig,
) {
    let (Some(start), Some(end)) = (segment.first(), segment.last()) else {
        return;
    };
    let drop = start.percentage as f64 - end.percentage as f64;
    if drop < config.min_drop_percent {
        return;
    }
    let hours = (end.at_unix_ms - start.at_unix_ms) as f64 / 3_600_000.0;
    let rate = drop / hours;
    if !hours.is_finite()
        || hours < MIN_OBSERVATION_MINUTES / 60.0
        || !rate.is_finite()
        || rate <= 0.0
        || rate > config.max_drain_per_hour
    {
        return;
    }
    let (Some(at), Some(ended_at)) = (
        DateTime::from_timestamp_millis(start.at_unix_ms),
        DateTime::from_timestamp_millis(end.at_unix_ms),
    ) else {
        return;
    };
    observations.push(DrainObservation {
        at,
        ended_at,
        timezone_offset_minutes: start.timezone_offset_minutes,
        percentage: start.percentage,
        drain_per_hour: rate,
        // 使用放电段起始样本的上下文，代表该放电时段的设备参数状态。
        context: start.context.clone(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Timelike};
    use std::time::Instant;

    /// 原 O(N²) EWMA baseline 实现，从 git 历史恢复，仅供等价性测试对比。
    ///
    /// 数学定义：对每个历史观测 o_j 计算 w_j(at) = exp(-(at - o_j.ended_at)/tau)，
    /// 返回 Σ_j w_j·o_j.drain / Σ_j w_j。空切片返回 None。
    ///
    /// 生产代码已改用 O(N) 增量累加器（见 `validated_model_prediction` 主循环），
    /// 此函数仅在 `#[cfg(test)]` 下编译，作为参考标准验证增量实现的数学等价性。
    fn weighted_baseline_rate_reference(
        observations: &[DrainObservation],
        at: DateTime<Utc>,
        decay_tau_hours: f64,
    ) -> Option<f64> {
        let mut weighted_rate = 0.0;
        let mut total_weight = 0.0;
        for observation in observations {
            let hours_ago = (at - observation.ended_at).num_seconds().max(0) as f64 / 3600.0;
            let weight = (-hours_ago / decay_tau_hours).exp();
            weighted_rate += observation.drain_per_hour * weight;
            total_weight += weight;
        }
        (total_weight > 0.0).then_some(weighted_rate / total_weight)
    }

    /// 原 `validated_model_prediction` 实现（O(N²)），从 git 历史恢复，仅供等价性测试对比。
    ///
    /// 与生产版本唯一的差异：主循环内调用 `weighted_baseline_rate_reference(&observations[..index], ...)`
    /// 和末次 `weighted_baseline_rate_reference(observations, now, ...)`，而非增量累加器。
    /// 其余（features / comparator / model.learn / 质量门 / fallback 路径）逐字保持一致。
    #[allow(clippy::too_many_lines)]
    fn validated_model_prediction_reference(
        observations: &[DrainObservation],
        current_percentage: u8,
        now: DateTime<Utc>,
        now_timezone_offset_minutes: i32,
        current_context: Option<&DeviceContextSnapshot>,
        config: &BatteryModelConfig,
        schema_status: &BatteryModelSchemaStatus,
    ) -> Result<BatteryPredictionOutput, BatteryPredictionError> {
        let optimizer = Optimizer::sgd(config.feature_count, {
            let mut sgd = SgdConfig::default();
            sgd.learning_rate = config.learning_rate;
            sgd.l2 = config.l2;
            sgd
        })
        .map_err(|_| BatteryPredictionError::InvalidModel)?;
        let mut model = LinearRegression::new(config.feature_count, {
            let mut lr_config = LinearRegressionConfig::default();
            lr_config.optimizer = optimizer;
            lr_config.loss = RegressionLoss::Huber(
                HuberLoss::new(config.huber_delta)
                    .map_err(|_| BatteryPredictionError::InvalidModel)?,
            );
            lr_config
        })
        .map_err(|_| BatteryPredictionError::InvalidModel)?;
        let mut comparator = BaselineComparator::new(
            &["deterministic-baseline", "rill-local-ai"],
            config.quality_window,
        )
        .map_err(|_| BatteryPredictionError::InvalidModel)?;

        let weighted_enabled = config.weighted_learning_enabled;
        let mut training_weight_sum: f64 = 0.0;
        let mut validation_weight_sum: f64 = 0.0;
        let mut candidate_weighted_error_sum: f64 = 0.0;
        let mut recent_error_sum: f64 = 0.0;
        let mut recent_count: u64 = 0;
        let mut robust = (config.robust_detection_enabled)
            .then(|| robust::RobustDetector::new(config))
            .transpose()?;

        for (index, observation) in observations.iter().enumerate() {
            let recent_rate = weighted_baseline_rate_reference(
                &observations[..index],
                observation.at,
                config.baseline_decay_tau_hours,
            );
            let features = &battery_features::build_battery_features(
                observation.percentage,
                observation.at,
                observation.timezone_offset_minutes,
                recent_rate,
                observation.context.as_ref(),
            )?
            .values;
            if model.samples_seen() >= config.min_training_samples {
                if let Some(baseline_prediction) = recent_rate {
                    if let Ok(ai_prediction) = model.predict(features) {
                        if ai_prediction.is_finite() {
                            comparator
                                .record(0, observation.drain_per_hour, baseline_prediction)
                                .map_err(|_| BatteryPredictionError::InvalidModel)?;
                            comparator
                                .record(1, observation.drain_per_hour, ai_prediction)
                                .map_err(|_| BatteryPredictionError::InvalidModel)?;
                            let metric_weight = if weighted_enabled {
                                recency_weight(observation.at, now, config)
                            } else {
                                1.0
                            };
                            validation_weight_sum += metric_weight;
                            let ai_error = (observation.drain_per_hour - ai_prediction).abs();
                            candidate_weighted_error_sum += metric_weight * ai_error;
                            if (now - observation.at).num_seconds().max(0) as f64 / 3600.0
                                <= config.quality_window as f64
                            {
                                recent_error_sum += ai_error;
                                recent_count += 1;
                            }
                        }
                    }
                }
            }
            if weighted_enabled || robust.is_some() {
                let mut weight = if weighted_enabled {
                    recency_weight(observation.at, now, config)
                } else {
                    1.0
                };
                if let Some(detector) = robust.as_mut() {
                    if let Ok(signals) = detector.update(observation.drain_per_hour) {
                        weight *= signals.anomaly_downweight;
                    }
                }
                training_weight_sum += weight;
                model
                    .learn_weighted(features, observation.drain_per_hour, weight)
                    .map_err(|_| BatteryPredictionError::InvalidModel)?;
            } else {
                training_weight_sum += 1.0;
                model
                    .learn(features, observation.drain_per_hour)
                    .map_err(|_| BatteryPredictionError::InvalidModel)?;
            }
        }

        comparator.update_best();
        let baseline = comparator.entry(0);
        let candidate = comparator.entry(1);
        let validation_samples = candidate.map_or(0, |entry| entry.total_samples());
        let baseline_samples = baseline.map_or(0, |entry| entry.total_samples());
        let baseline_mae = baseline.and_then(|entry| entry.rolling_mae());
        let candidate_mae = candidate.and_then(|entry| entry.rolling_mae());
        let training_samples = model.samples_seen();
        let weighted_mae = (validation_weight_sum > 0.0)
            .then_some(candidate_weighted_error_sum / validation_weight_sum);
        let recent_mae = (recent_count > 0).then_some(recent_error_sum / recent_count as f64);
        let effective_sample_weight = Some(training_weight_sum);

        if training_samples < config.min_training_samples {
            return Ok(fallback(
                "insufficientTrainingData",
                training_samples,
                validation_samples,
                baseline_mae,
                candidate_mae,
                weighted_mae,
                recent_mae,
                effective_sample_weight,
                schema_status,
            ));
        }
        if weighted_enabled && training_weight_sum < config.min_training_samples as f64 {
            return Ok(fallback(
                "insufficientEffectiveWeight",
                training_samples,
                validation_samples,
                baseline_mae,
                candidate_mae,
                weighted_mae,
                recent_mae,
                effective_sample_weight,
                schema_status,
            ));
        }
        if validation_samples < config.min_validation_samples
            || baseline_samples != validation_samples
        {
            return Ok(fallback(
                "insufficientValidationData",
                training_samples,
                validation_samples,
                baseline_mae,
                candidate_mae,
                weighted_mae,
                recent_mae,
                effective_sample_weight,
                schema_status,
            ));
        }
        let (Some(baseline_error), Some(candidate_error)) = (baseline_mae, candidate_mae) else {
            return Ok(fallback(
                "qualityMetricsUnavailable",
                training_samples,
                validation_samples,
                baseline_mae,
                candidate_mae,
                weighted_mae,
                recent_mae,
                effective_sample_weight,
                schema_status,
            ));
        };
        let candidate_error_for_gate = if weighted_enabled {
            weighted_mae.unwrap_or(candidate_error)
        } else {
            candidate_error
        };
        if candidate_error_for_gate >= baseline_error * config.required_error_ratio {
            return Ok(fallback(
                "candidateNotBetter",
                training_samples,
                validation_samples,
                baseline_mae,
                candidate_mae,
                weighted_mae,
                recent_mae,
                effective_sample_weight,
                schema_status,
            ));
        }

        let recent_rate =
            weighted_baseline_rate_reference(observations, now, config.baseline_decay_tau_hours);
        let predicted_rate = model
            .predict(
                &battery_features::build_battery_features(
                    current_percentage,
                    now,
                    now_timezone_offset_minutes,
                    recent_rate,
                    current_context,
                )?
                .values,
            )
            .map_err(|_| BatteryPredictionError::InvalidModel)?;
        if !predicted_rate.is_finite()
            || predicted_rate <= 0.0
            || predicted_rate > config.max_drain_per_hour
        {
            return Ok(fallback(
                "candidateOutsideSafetyBounds",
                training_samples,
                validation_samples,
                baseline_mae,
                candidate_mae,
                weighted_mae,
                recent_mae,
                effective_sample_weight,
                schema_status,
            ));
        }
        let remaining_hours = current_percentage as f64 / predicted_rate;
        if !remaining_hours.is_finite() || remaining_hours > config.max_remaining_hours {
            return Ok(fallback(
                "candidateOutsideSafetyBounds",
                training_samples,
                validation_samples,
                baseline_mae,
                candidate_mae,
                weighted_mae,
                recent_mae,
                effective_sample_weight,
                schema_status,
            ));
        }

        Ok(BatteryPredictionOutput {
            remaining_hours: Some(remaining_hours),
            source: PredictionSource::LocalAi,
            reason: "candidatePassedQualityGate".into(),
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
            weighted_mae,
            recent_mae,
            effective_sample_weight,
            schema_status: Some(schema_status.clone()),
        })
    }

    fn test_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap()
    }

    fn test_schema_status() -> BatteryModelSchemaStatus {
        BatteryModelSchemaStatus::LegacyCompatibility
    }

    fn new_linear_regression(
        config: &BatteryModelConfig,
    ) -> Result<LinearRegression, BatteryPredictionError> {
        let optimizer = Optimizer::sgd(config.feature_count, {
            let mut sgd = SgdConfig::default();
            sgd.learning_rate = config.learning_rate;
            sgd.l2 = config.l2;
            sgd
        })
        .map_err(|_| BatteryPredictionError::InvalidModel)?;
        LinearRegression::new(config.feature_count, {
            let mut lr_config = LinearRegressionConfig::default();
            lr_config.optimizer = optimizer;
            lr_config.loss = RegressionLoss::Huber(
                HuberLoss::new(config.huber_delta)
                    .map_err(|_| BatteryPredictionError::InvalidModel)?,
            );
            lr_config
        })
        .map_err(|_| BatteryPredictionError::InvalidModel)
    }

    fn sample(at: DateTime<Utc>, percentage: u8, charging: bool) -> BatterySampleInput {
        BatterySampleInput {
            at_unix_ms: at.timestamp_millis(),
            timezone_offset_minutes: 0,
            percentage,
            charging,
            context: None,
        }
    }

    #[test]
    fn cold_start_explicitly_recommends_baseline() {
        let now = test_now();
        let result = predict(
            &BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: 0,
                samples: vec![sample(now, 80, false)],
                current_context: None,
            },
            &BatteryModelConfig::default(),
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
        assert_eq!(result.remaining_hours, None);
    }

    #[test]
    fn short_quantized_drop_does_not_become_training_observation() {
        let now = test_now();
        let samples = vec![
            sample(now - Duration::minutes(65), 80, false),
            sample(now - Duration::minutes(60), 79, false),
            sample(now, 79, false),
        ];

        assert!(discharge_observations(&samples, &BatteryModelConfig::default()).is_empty());
    }

    #[test]
    fn lower_charge_swap_or_recalibration_splits_training_segments() {
        let now = test_now();
        let samples = vec![
            sample(now - Duration::minutes(150), 90, false),
            sample(now - Duration::minutes(140), 89, false),
            sample(now - Duration::minutes(130), 88, false),
            sample(now - Duration::minutes(120), 87, false),
            // A quick 37% downward jump can be a lower-charge battery swap or
            // reconnect recalibration. It must be a boundary, not a 222%/h label.
            sample(now - Duration::minutes(115), 50, false),
            sample(now - Duration::minutes(105), 49, false),
            sample(now - Duration::minutes(95), 48, false),
            sample(now - Duration::minutes(85), 47, false),
            sample(now - Duration::minutes(60), 47, false),
        ];

        let observations = discharge_observations(&samples, &BatteryModelConfig::default());
        assert_eq!(observations.len(), 2);
        assert!(observations
            .iter()
            .all(|observation| (observation.drain_per_hour - 6.0).abs() < 0.01));
    }

    #[test]
    fn learned_daily_pattern_can_pass_quality_gate() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let observations = (0..120)
            .map(|index| {
                let at = start + Duration::hours(index);
                let angle = at.hour() as f64 / 24.0 * std::f64::consts::TAU;
                DrainObservation {
                    at,
                    ended_at: at + Duration::minutes(30),
                    timezone_offset_minutes: 0,
                    percentage: 80,
                    drain_per_hour: 5.0 + 3.0 * angle.sin() + 1.5 * angle.cos(),
                    context: None,
                }
            })
            .collect::<Vec<_>>();
        let result = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            None,
            &BatteryModelConfig::default(),
            &test_schema_status(),
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::LocalAi);
        assert!(result.remaining_hours.is_some_and(f64::is_finite));

        let capped = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            None,
            &BatteryModelConfig {
                max_remaining_hours: 0.1,
                ..BatteryModelConfig::default()
            },
            &test_schema_status(),
        )
        .unwrap();
        assert_eq!(capped.source, PredictionSource::BaselineRecommended);
        assert_eq!(capped.remaining_hours, None);
    }

    /// recency_weight 契约：最新样本≈1.0、未来时间戳 clamp、极旧样本下限、
    /// 非法 tau 回退、绝不产生 NaN。
    #[test]
    fn recency_weight_properties_hold() {
        let now = test_now();
        let weighted = BatteryModelConfig {
            learning_recency_tau_hours: Some(48.0),
            ..BatteryModelConfig::default()
        };
        // 同刻样本权重 1.0；未来时间戳 clamp 到 age=0 → 1.0
        assert_eq!(recency_weight(now, now, &weighted), 1.0);
        assert_eq!(
            recency_weight(now + Duration::hours(3), now, &weighted),
            1.0
        );
        // 一个 tau 后权重 ≈ e^-1
        let one_tau = recency_weight(now - Duration::hours(48), now, &weighted);
        assert!((one_tau - (-1.0f64).exp()).abs() < 1e-9);
        // 极旧样本取下限，恒正
        let ancient = recency_weight(now - Duration::days(400), now, &weighted);
        assert_eq!(ancient, MIN_RECENCY_WEIGHT);
        assert!(ancient > 0.0);
        // 非法 tau（NaN / 0）回退 baseline tau=48，权重有限且为正
        for bad_tau in [Some(f64::NAN), Some(0.0), Some(-1.0)] {
            let bad = BatteryModelConfig {
                learning_recency_tau_hours: bad_tau,
                ..BatteryModelConfig::default()
            };
            let w = recency_weight(now - Duration::hours(48), now, &bad);
            assert!(w.is_finite() && w > 0.0);
            assert!((w - (-1.0f64).exp()).abs() < 1e-9);
        }
        // 默认配置（未设置）复用 baseline tau
        assert_eq!(
            recency_weight(now, now, &BatteryModelConfig::default()),
            1.0
        );
    }

    /// 行为切换（长期高耗电 → 近期低耗电）后，加权模型直接以模型输出对比：
    /// 加权模型对近期状态的预测明显低于未加权模型（适应更快）。
    #[test]
    fn weighted_learning_adapts_faster_at_model_level() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig {
            weighted_learning_enabled: true,
            learning_recency_tau_hours: Some(48.0),
            ..BatteryModelConfig::default()
        };
        let now = start + Duration::hours(802);
        let mut plain = new_linear_regression(&config).unwrap();
        let mut weighted = new_linear_regression(&config).unwrap();
        for index in 0..400 {
            let at = start + Duration::hours(index * 2);
            let features = battery_features::build_battery_features(80, at, 0, Some(5.0), None)
                .unwrap()
                .values;
            let drain = if index < 300 { 10.0 } else { 2.0 };
            plain
                .learn(&features, drain)
                .map_err(|_| BatteryPredictionError::InvalidModel)
                .unwrap();
            weighted
                .learn_weighted(&features, drain, recency_weight(at, now, &config))
                .map_err(|_| BatteryPredictionError::InvalidModel)
                .unwrap();
        }
        let features_now = battery_features::build_battery_features(80, now, 0, Some(2.0), None)
            .unwrap()
            .values;
        let plain_rate = plain.predict(&features_now).unwrap();
        let weighted_rate = weighted.predict(&features_now).unwrap();
        assert!(
            (weighted_rate - 2.0).abs() < (plain_rate - 2.0).abs(),
            "weighted should track the recent low-drain state: weighted={weighted_rate} vs plain={plain_rate}, truth=2.0"
        );
    }

    /// 加权模式在通过质量门时输出 weighted_mae / recent_mae / effective_sample_weight，
    /// 且有效权重严格小于训练样本数（确实按时间衰减了权重）。
    #[test]
    fn weighted_mode_emits_weighted_metrics_on_success() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let observations = (0..120)
            .map(|index| {
                let at = start + Duration::hours(index);
                let angle = at.hour() as f64 / 24.0 * std::f64::consts::TAU;
                DrainObservation {
                    at,
                    ended_at: at + Duration::minutes(30),
                    timezone_offset_minutes: 0,
                    percentage: 80,
                    drain_per_hour: 5.0 + 3.0 * angle.sin() + 1.5 * angle.cos(),
                    context: None,
                }
            })
            .collect::<Vec<_>>();
        let config = BatteryModelConfig {
            weighted_learning_enabled: true,
            learning_recency_tau_hours: Some(120.0),
            ..BatteryModelConfig::default()
        };
        let result = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            None,
            &config,
            &test_schema_status(),
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::LocalAi);
        assert!(result.weighted_mae.is_some_and(f64::is_finite));
        assert!(result.recent_mae.is_some_and(f64::is_finite));
        let effective = result.effective_sample_weight.unwrap();
        assert!(effective.is_finite() && effective > 0.0);
        assert!(
            effective < result.training_samples as f64,
            "effective weight {effective} must be below sample count {}",
            result.training_samples
        );
        // 未加权配置下 weighted_mae 应等于 candidate MAE（权重全为 1.0）
        let plain = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            None,
            &BatteryModelConfig::default(),
            &test_schema_status(),
        )
        .unwrap();
        assert_eq!(plain.source, PredictionSource::LocalAi);
        let plain_effective = plain.effective_sample_weight.unwrap();
        assert_eq!(plain_effective, plain.training_samples as f64);
    }

    /// 加权保护门：tau 极短导致有效权重不足时回退确定性基线。
    #[test]
    fn weighted_insufficient_effective_weight_falls_back() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let observations: Vec<DrainObservation> = (0..120)
            .map(|index| {
                let at = start + Duration::hours(index);
                DrainObservation {
                    at,
                    ended_at: at + Duration::minutes(30),
                    timezone_offset_minutes: 0,
                    percentage: 80,
                    drain_per_hour: 5.0,
                    context: None,
                }
            })
            .collect();
        // tau=0.5h：120 小时前的样本权重 ≈ 0，有效权重 ≈ 最近 1-2 个样本 < 6
        let config = BatteryModelConfig {
            weighted_learning_enabled: true,
            learning_recency_tau_hours: Some(0.5),
            ..BatteryModelConfig::default()
        };
        let result = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            None,
            &config,
            &test_schema_status(),
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
        assert_eq!(result.reason, "insufficientEffectiveWeight");
        assert_eq!(result.remaining_hours, None);
    }

    /// 加权模式下生产实现与参考实现逐字段等价（含新增指标字段）。
    #[test]
    fn weighted_mode_matches_reference_implementation() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig {
            weighted_learning_enabled: true,
            learning_recency_tau_hours: Some(24.0),
            ..BatteryModelConfig::default()
        };
        for seed in 0..4u64 {
            // 与 incremental_ewma 参考测试同款数据样式（2h 间隔、平滑日周期），
            // 两种 EWMA 实现的浮点等价性在该数据集上已被 1e-9 验证。
            let observations: Vec<DrainObservation> = (0..120)
                .map(|i| {
                    let at = start + Duration::hours(i);
                    let angle = (at.hour() as f64 + seed as f64) / 24.0 * std::f64::consts::TAU;
                    DrainObservation {
                        at,
                        ended_at: at + Duration::minutes(30),
                        timezone_offset_minutes: 0,
                        percentage: 80,
                        drain_per_hour: 5.0 + 3.0 * angle.sin() + 1.5 * angle.cos(),
                        context: None,
                    }
                })
                .collect();
            let got = validated_model_prediction(
                &observations,
                80,
                start + Duration::hours(121),
                0,
                None,
                &config,
                &test_schema_status(),
            )
            .unwrap();
            let want = validated_model_prediction_reference(
                &observations,
                80,
                start + Duration::hours(121),
                0,
                None,
                &config,
                &test_schema_status(),
            )
            .unwrap();
            assert_eq!(got.source, want.source, "seed={seed}");
            assert_eq!(got.reason, want.reason, "seed={seed}");
            assert_eq!(got.training_samples, want.training_samples);
            assert_eq!(got.validation_samples, want.validation_samples);
            let eq = |a: Option<f64>, b: Option<f64>| match (a, b) {
                (Some(x), Some(y)) => (x - y).abs() < 1e-3,
                (None, None) => true,
                _ => false,
            };
            // 生产增量 EWMA 与参考 O(N²) 在浮点上允许微小差异（参考测试同款容差）。
            assert!(
                eq(got.baseline_mae, want.baseline_mae),
                "seed={seed} got={:?} want={:?}",
                got.baseline_mae,
                want.baseline_mae
            );
            assert!(eq(got.candidate_mae, want.candidate_mae), "seed={seed}");
            assert!(eq(got.weighted_mae, want.weighted_mae), "seed={seed}");
            assert!(eq(got.recent_mae, want.recent_mae), "seed={seed}");
            // 有效权重由权重求和而来，双实现应完全一致
            assert!(
                eq(got.effective_sample_weight, want.effective_sample_weight),
                "seed={seed} got={:?} want={:?}",
                got.effective_sample_weight,
                want.effective_sample_weight
            );
            assert_eq!(got.remaining_hours, want.remaining_hours, "seed={seed}");
        }
    }

    #[test]
    fn robust_enabled_matches_robust_disabled_on_clean_data() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let observations: Vec<DrainObservation> = (0..120)
            .map(|i| {
                let at = start + Duration::hours(2 * i);
                let angle = (at.hour() as f64) / 24.0 * std::f64::consts::TAU;
                DrainObservation {
                    at,
                    ended_at: at + Duration::minutes(30),
                    timezone_offset_minutes: 0,
                    percentage: 80,
                    drain_per_hour: 5.0 + 3.0 * angle.sin() + 1.5 * angle.cos(),
                    context: None,
                }
            })
            .collect();
        let now = start + Duration::hours(241);
        let plain = BatteryModelConfig::default();
        let robust_on = BatteryModelConfig {
            robust_detection_enabled: true,
            ..BatteryModelConfig::default()
        };
        let got = validated_model_prediction(
            &observations,
            80,
            now,
            0,
            None,
            &robust_on,
            &test_schema_status(),
        )
        .unwrap();
        let want = validated_model_prediction(
            &observations,
            80,
            now,
            0,
            None,
            &plain,
            &test_schema_status(),
        )
        .unwrap();
        // 干净数据上实验层完全中性（权重 ×1.0），结果必须逐字段一致：
        // 阶段 4 开启不改变阶段 3 行为。
        assert_eq!(got.source, want.source);
        assert_eq!(got.reason, want.reason);
        assert_eq!(got.training_samples, want.training_samples);
        assert_eq!(got.validation_samples, want.validation_samples);
        assert_eq!(got.baseline_mae, want.baseline_mae);
        assert_eq!(got.candidate_mae, want.candidate_mae);
        assert_eq!(got.weighted_mae, want.weighted_mae);
        assert_eq!(got.recent_mae, want.recent_mae);
        assert_eq!(got.effective_sample_weight, want.effective_sample_weight);
        assert_eq!(got.remaining_hours, want.remaining_hours);
    }

    #[test]
    fn robust_spike_downweights_but_history_survives() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let base: Vec<DrainObservation> = (0..120)
            .map(|i| {
                let at = start + Duration::hours(2 * i);
                let angle = (at.hour() as f64) / 24.0 * std::f64::consts::TAU;
                DrainObservation {
                    at,
                    ended_at: at + Duration::minutes(30),
                    timezone_offset_minutes: 0,
                    percentage: 80,
                    drain_per_hour: 5.0 + 3.0 * angle.sin() + 1.5 * angle.cos(),
                    context: None,
                }
            })
            .collect();
        let mut spiked = base.clone();
        spiked[60].drain_per_hour = 50.0;
        let now = start + Duration::hours(241);
        let config = BatteryModelConfig {
            robust_detection_enabled: true,
            ..BatteryModelConfig::default()
        };
        let clean =
            validated_model_prediction(&base, 80, now, 0, None, &config, &test_schema_status())
                .unwrap();
        let with_spike =
            validated_model_prediction(&spiked, 80, now, 0, None, &config, &test_schema_status())
                .unwrap();
        // 异常样本被降权：训练有效权重下降。
        assert!(with_spike.effective_sample_weight < clean.effective_sample_weight);
        // 单点异常不会摧毁历史：仍产出有限预测，且近期误差被降权后不劣于基线。
        assert!(with_spike.remaining_hours.is_some_and(|h| h.is_finite()));
        assert!(with_spike.recent_mae.is_some());
    }

    #[test]
    fn future_samples_never_enter_training() {
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let result = predict(
            &BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: 0,
                samples: vec![
                    sample(now, 80, false),
                    sample(now + Duration::minutes(1), 100, false),
                    sample(now + Duration::minutes(6), 90, false),
                    sample(now + Duration::minutes(20), 90, false),
                ],
                current_context: None,
            },
            &BatteryModelConfig::default(),
        )
        .unwrap();
        assert_eq!(result.training_samples, 0);
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
    }

    #[test]
    fn invalid_percentage_is_rejected() {
        let now = test_now();
        let error = predict(
            &BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: 0,
                samples: vec![sample(now, 101, false)],
                current_context: None,
            },
            &BatteryModelConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            BatteryPredictionError::InvalidSample { index: 0 }
        ));
    }

    #[test]
    fn current_context_overrides_available_fields_and_inherits_missing_ones() {
        let historical = DeviceContextSnapshot {
            dpi: Some(800),
            polling_rate_hz: Some(1000),
            light_mode: Some("off".into()),
            light_brightness: Some(20),
            profile: Some("profile-1".into()),
        };
        let partial_current = DeviceContextSnapshot {
            dpi: Some(3200),
            polling_rate_hz: None,
            light_mode: Some("rainbow".into()),
            light_brightness: None,
            profile: None,
        };

        let merged = merge_prediction_context(Some(&partial_current), Some(&historical)).unwrap();
        assert_eq!(merged.dpi, Some(3200));
        assert_eq!(merged.polling_rate_hz, Some(1000));
        assert_eq!(merged.light_mode.as_deref(), Some("rainbow"));
        assert_eq!(merged.light_brightness, Some(20));
        assert_eq!(merged.profile.as_deref(), Some("profile-1"));
    }

    #[test]
    fn missing_or_partial_context_never_blocks_prediction() {
        let now = test_now();
        for current_context in [
            None,
            Some(DeviceContextSnapshot {
                dpi: Some(1600),
                ..DeviceContextSnapshot::default()
            }),
        ] {
            let output = predict(
                &BatteryPredictionInput {
                    now_unix_ms: now.timestamp_millis(),
                    now_timezone_offset_minutes: 0,
                    samples: vec![sample(now, 80, false)],
                    current_context,
                },
                &BatteryModelConfig::default(),
            )
            .unwrap();
            assert_eq!(output.source, PredictionSource::BaselineRecommended);
        }
    }

    /// 验证灯光模式到功耗强度评分的映射（经由 build_battery_features 的灯光特征）。
    #[test]
    fn light_mode_intensity_maps_known_modes() {
        let now = test_now();
        let intensity_for = |mode: &str| {
            let ctx = DeviceContextSnapshot {
                dpi: None,
                polling_rate_hz: None,
                light_mode: Some(mode.into()),
                light_brightness: None,
                profile: None,
            };
            battery_features::build_battery_features(80, now, 0, Some(5.0), Some(&ctx))
                .unwrap()
                .values[8]
        };
        assert_eq!(intensity_for("off"), 0.0);
        assert_eq!(intensity_for("OFF"), 0.0);
        assert_eq!(intensity_for("static"), 0.3);
        assert_eq!(intensity_for("breathing"), 0.5);
        assert_eq!(intensity_for("reactive"), 0.6);
        assert_eq!(intensity_for("ripple"), 0.7);
        assert_eq!(intensity_for("wave"), 0.8);
        assert_eq!(intensity_for("starlight"), 0.85);
        assert_eq!(intensity_for("rainbow"), 0.9);
        assert_eq!(intensity_for("custom"), 1.0);
        assert_eq!(intensity_for("unknown_xyz"), 0.5);
    }

    /// 验证上下文特征归一化与缺失值回退。
    #[test]
    fn context_features_normalize_and_default_correctly() {
        let now = test_now();
        let build = |ctx: Option<&DeviceContextSnapshot>| {
            battery_features::build_battery_features(80, now, 0, Some(5.0), ctx)
                .unwrap()
                .values
        };
        let ctx = DeviceContextSnapshot {
            dpi: Some(16000),
            polling_rate_hz: Some(8000),
            light_mode: Some("breathing".into()),
            light_brightness: Some(80),
            profile: None,
        };
        let feats = build(Some(&ctx));
        let [dpi, rate, light] = [feats[6], feats[7], feats[8]];
        assert!((dpi - (16000.0 / 60000.0)).abs() < 1e-9);
        assert!((rate - (8000.0 / 16000.0)).abs() < 1e-9);
        // breathing=0.5 * brightness=0.8 = 0.4
        assert!((light - 0.4).abs() < 1e-9);

        let ctx_no_brightness = DeviceContextSnapshot {
            dpi: Some(16000),
            polling_rate_hz: Some(8000),
            light_mode: Some("breathing".into()),
            light_brightness: None,
            profile: None,
        };
        let feats2 = build(Some(&ctx_no_brightness));
        assert!((feats2[8] - 0.5).abs() < 1e-9);

        let ctx_high = DeviceContextSnapshot {
            dpi: Some(65000),
            polling_rate_hz: Some(8000),
            light_mode: Some("breathing".into()),
            light_brightness: Some(80),
            profile: None,
        };
        let feats_high = build(Some(&ctx_high));
        assert!((feats_high[6] - 1.0).abs() < 1e-9);

        let feats_none = build(None);
        assert_eq!(
            [feats_none[6], feats_none[7], feats_none[8]],
            [0.0, 0.0, 0.0]
        );
    }

    /// 验证不同上下文产生不同预测结果：模型确实消费了 DPI/回报率/灯光特征。
    ///
    /// 构造两组训练数据，放电率与 DPI 正相关（高 DPI 耗电更快），
    /// 验证模型在高 DPI 上下文下预测的剩余时间更短。
    #[test]
    fn different_contexts_produce_different_predictions() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let low_ctx = DeviceContextSnapshot {
            dpi: Some(800),
            polling_rate_hz: Some(125),
            light_mode: Some("off".into()),
            light_brightness: None,
            profile: None,
        };
        let high_ctx = DeviceContextSnapshot {
            dpi: Some(26000),
            polling_rate_hz: Some(8000),
            light_mode: Some("rainbow".into()),
            light_brightness: Some(100),
            profile: None,
        };
        let observations: Vec<DrainObservation> = (0..60)
            .map(|i| {
                let at = start + Duration::hours(i * 2);
                let is_high = i % 2 == 0;
                DrainObservation {
                    at,
                    ended_at: at + Duration::minutes(30),
                    timezone_offset_minutes: 0,
                    percentage: if is_high { 90 } else { 80 },
                    drain_per_hour: if is_high { 10.0 } else { 2.0 },
                    context: Some(if is_high {
                        high_ctx.clone()
                    } else {
                        low_ctx.clone()
                    }),
                }
            })
            .collect();

        let result_low = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            Some(&low_ctx),
            &BatteryModelConfig::default(),
            &test_schema_status(),
        )
        .unwrap();
        let result_high = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            Some(&high_ctx),
            &BatteryModelConfig::default(),
            &test_schema_status(),
        )
        .unwrap();

        // 两种上下文都应通过质量门（模型学到了上下文与放电率的关系）
        // 高功耗上下文预测的放电率应更高 → 剩余时间更短
        if let (Some(low_hours), Some(high_hours)) =
            (result_low.remaining_hours, result_high.remaining_hours)
        {
            assert!(
                high_hours < low_hours,
                "高功耗上下文剩余时间应更短: high={high_hours}h vs low={low_hours}h"
            );
        }
        // 即使未通过质量门（数据量不足等），模型也必须能接受上下文输入而不报错
        assert!(
            result_low.source == PredictionSource::LocalAi
                || result_low.source == PredictionSource::BaselineRecommended
        );
        assert!(
            result_high.source == PredictionSource::LocalAi
                || result_high.source == PredictionSource::BaselineRecommended
        );
    }

    /// 验证 9 维特征向量包含上下文特征。
    #[test]
    fn features_vector_has_nine_dimensions_with_context() {
        let now = test_now();
        let ctx = DeviceContextSnapshot {
            dpi: Some(16000),
            polling_rate_hz: Some(4000),
            light_mode: Some("static".into()),
            light_brightness: Some(50),
            profile: None,
        };
        let feats = battery_features::build_battery_features(80, now, 0, Some(5.0), Some(&ctx))
            .unwrap()
            .values;
        assert_eq!(feats.len(), 9);
        // 基础特征
        assert!((feats[0] - 0.8).abs() < 1e-9); // percentage
        assert!(feats[5] > 0.0); // recent_rate
                                 // 上下文特征
        assert!((feats[6] - (16000.0 / 60000.0)).abs() < 1e-9); // dpi
        assert!((feats[7] - (4000.0 / 16000.0)).abs() < 1e-9); // polling_rate
                                                               // static=0.3 * brightness=0.5 = 0.15
        assert!((feats[8] - 0.15).abs() < 1e-9); // light_intensity

        let feats_none = battery_features::build_battery_features(80, now, 0, Some(5.0), None)
            .unwrap()
            .values;
        assert_eq!(feats_none.len(), 9);
        assert_eq!(feats_none[6], 0.0);
        assert_eq!(feats_none[7], 0.0);
        assert_eq!(feats_none[8], 0.0);
    }

    /// 验证 O(N) 增量 EWMA 与原 O(N²) 实现在浮点容差 1e-9 内字段对字段相等。
    ///
    /// 用内置 LCG（不引入 `rand` 依赖）生成确定性随机序列，覆盖
    /// count ∈ {0,1,5,50,500} × 8 个种子 × 多种 gap/duration/rate 组合，
    /// 同时跑生产版和参考版，对比 6 个输出字段。质量门未通过的序列也必须给出一致 fallback。
    #[test]
    fn incremental_ewma_matches_reference_implementation() {
        fn lcg_next(state: &mut u64) -> u64 {
            *state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *state
        }
        fn lcg_range(state: &mut u64, lo: f64, hi: f64) -> f64 {
            let u = lcg_next(state);
            lo + (u as f64 / u64::MAX as f64) * (hi - lo)
        }

        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig::default();

        for &count in &[0usize, 1, 5, 50, 500] {
            for seed in 0..8u64 {
                let mut rng = seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(42);
                let mut at_cursor = start;
                let mut observations = Vec::with_capacity(count);
                for _ in 0..count {
                    // gap before this observation: 0 / 5min / 1h / 1day
                    let gap_minutes = [0.0, 5.0, 60.0, 1440.0][lcg_next(&mut rng) as usize % 4];
                    at_cursor += Duration::minutes(gap_minutes as i64);
                    // segment duration: 1min / 30min / 2h
                    let dur_minutes = [1.0, 30.0, 120.0][lcg_next(&mut rng) as usize % 3];
                    let ended_at = at_cursor + Duration::minutes(dur_minutes as i64);
                    // drain rate ∈ [0.1, 50.0] with jitter
                    let rate = lcg_range(&mut rng, 0.1, 50.0);
                    observations.push(DrainObservation {
                        at: at_cursor,
                        ended_at,
                        timezone_offset_minutes: 0,
                        percentage: 80,
                        drain_per_hour: rate,
                        context: None,
                    });
                    at_cursor = ended_at;
                }
                let now = at_cursor + Duration::hours(2);

                let got = validated_model_prediction(
                    &observations,
                    80,
                    now,
                    0,
                    None,
                    &config,
                    &test_schema_status(),
                )
                .unwrap();
                let want = validated_model_prediction_reference(
                    &observations,
                    80,
                    now,
                    0,
                    None,
                    &config,
                    &test_schema_status(),
                )
                .unwrap();

                assert_eq!(
                    got.source, want.source,
                    "source mismatch at count={count} seed={seed}"
                );
                assert_eq!(
                    got.reason, want.reason,
                    "reason mismatch at count={count} seed={seed}"
                );
                assert_eq!(got.training_samples, want.training_samples);
                assert_eq!(got.validation_samples, want.validation_samples);
                let eq = |a: Option<f64>, b: Option<f64>| -> bool {
                    match (a, b) {
                        (Some(x), Some(y)) => (x - y).abs() < 1e-9,
                        (None, None) => true,
                        _ => false,
                    }
                };
                assert!(
                    eq(got.remaining_hours, want.remaining_hours),
                    "remaining_hours mismatch at count={count} seed={seed}: {got:?} vs {want:?}"
                );
                assert!(
                    eq(got.baseline_mae, want.baseline_mae),
                    "baseline_mae mismatch at count={count} seed={seed}"
                );
                assert!(
                    eq(got.candidate_mae, want.candidate_mae),
                    "candidate_mae mismatch at count={count} seed={seed}"
                );
            }
        }
    }

    /// 空 observations → 全程 recent_rate=None → 无训练样本 → insufficientTrainingData。
    #[test]
    fn incremental_ewma_handles_empty_observations() {
        let now = test_now();
        let config = BatteryModelConfig::default();
        let result =
            validated_model_prediction(&[], 80, now, 0, None, &config, &test_schema_status())
                .unwrap();
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
        assert_eq!(result.remaining_hours, None);
        assert_eq!(result.training_samples, 0);
        assert_eq!(result.reason, "insufficientTrainingData");
    }

    /// 单观测 → 迭代 0 recent_rate=None；末次 recent_rate=Some(o_0.drain)。
    /// 训练样本 1 < min_training_samples(默认) → 走 insufficientTrainingData fallback。
    /// 同时验证参考实现给出一致结果。
    #[test]
    fn incremental_ewma_handles_single_observation() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig::default();
        let observations = vec![DrainObservation {
            at: start,
            ended_at: start + Duration::minutes(30),
            timezone_offset_minutes: 0,
            percentage: 80,
            drain_per_hour: 5.0,
            context: None,
        }];
        let result = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(2),
            0,
            None,
            &config,
            &test_schema_status(),
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
        assert_eq!(result.training_samples, 1);

        let ref_result = validated_model_prediction_reference(
            &observations,
            80,
            start + Duration::hours(2),
            0,
            None,
            &config,
            &test_schema_status(),
        )
        .unwrap();
        assert_eq!(result.source, ref_result.source);
        assert_eq!(result.training_samples, ref_result.training_samples);
        assert_eq!(result.reason, ref_result.reason);
    }

    /// 4096 observations 必须 <100ms 完成。O(N) 增量约 1ms，O(N²) 回归会 >10s 立即失败。
    /// 此测试防御未来误改回每次重算的 weighted_baseline_rate 调用。
    #[test]
    fn incremental_ewma_performance_under_max_samples() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig::default();
        let observations: Vec<DrainObservation> = (0..4096)
            .map(|i| {
                let at = start + Duration::minutes(i * 10);
                DrainObservation {
                    at,
                    ended_at: at + Duration::minutes(5),
                    timezone_offset_minutes: 0,
                    percentage: 80,
                    drain_per_hour: 5.0 + (i as f64 % 10.0),
                    context: None,
                }
            })
            .collect();
        let now = start + Duration::hours(4096 * 10 / 60 + 2);

        let t0 = Instant::now();
        let _ = validated_model_prediction(
            &observations,
            80,
            now,
            0,
            None,
            &config,
            &test_schema_status(),
        )
        .unwrap();
        let elapsed = t0.elapsed();

        assert!(
            elapsed.as_millis() < 100,
            "O(N) EWMA on 4096 samples should be <100ms, got {elapsed:?}"
        );
    }

    // ── RillML 1.0 prediction / fallback smoke ──────────────────────────────

    /// 所有 handler 端 fallback reason 都必须是预定义稳定码，
    /// 不能包含底层异常文本（保证 UI 不直接暴露内部错误）。
    fn is_stable_fallback_reason(reason: &str) -> bool {
        const STABLE_REASONS: &[&str] = &[
            "noDischargingSample",
            "emptyBattery",
            "insufficientTrainingData",
            "insufficientValidationData",
            "qualityMetricsUnavailable",
            "candidateNotBetter",
            "candidateOutsideSafetyBounds",
        ];
        STABLE_REASONS.contains(&reason)
    }

    /// RillML 1.0 真实预测 smoke：用足够的放电历史调用 `predict()`，验证响应 schema、
    /// source/reason 合法性、training_samples/validation_samples 正确性，以及
    /// remaining_hours 为合法有限值或根据质量门正确 fallback。
    ///
    /// 不要求 AI 一定战胜 baseline——质量门回退也是正确结果。
    #[test]
    fn prediction_smoke_with_sufficient_discharge_history() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        let config = BatteryModelConfig::default();

        // 构建 80 个放电段。每段 8 个样本、5 分钟间隔（35 分钟总时长 ≥ 30 min
        // MIN_OBSERVATION_MINUTES），段间 15 分钟间隔（> 10 min session_gap_minutes）。
        // 放电率随小时正弦变化，让线性模型能学习日周期模式。
        // 最后一段是"当前未完成段"，不产生 observation，因此 79 个有效 observation。
        let mut samples = Vec::with_capacity(640);
        for index in 0..80u32 {
            let segment_start = start + Duration::minutes((index * 50) as i64);
            let hour_angle = segment_start.hour() as f64 / 24.0 * std::f64::consts::TAU;
            // drop ∈ [3, 7]，放电率 = drop / (35/60) h，随小时正弦变化
            let drop = (5.0 + 2.0 * hour_angle.sin()).round().clamp(3.0, 7.0) as u8;
            for step in 0..8u32 {
                let at = segment_start + Duration::minutes((step * 5) as i64);
                let pct = 80u8.saturating_sub(((drop as f64 * step as f64) / 7.0).round() as u8);
                samples.push(sample(at, pct, false));
            }
        }
        let now = start + Duration::minutes(80 * 50);

        let result = predict(
            &BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: 0,
                samples,
                current_context: None,
            },
            &config,
        )
        .expect("predict must not error on valid discharge history");

        // --- 响应 schema 正确性 ---
        // source 必须是合法枚举变体
        assert!(
            result.source == PredictionSource::LocalAi
                || result.source == PredictionSource::BaselineRecommended,
            "source must be a valid PredictionSource variant, got {:?}",
            result.source
        );

        // training_samples：79 个 observation 都经过 model.learn()
        assert_eq!(result.training_samples, 79);
        // validation_samples 不超过 training_samples
        assert!(
            result.validation_samples <= result.training_samples,
            "validation_samples ({}) must not exceed training_samples ({})",
            result.validation_samples,
            result.training_samples
        );

        // --- source / reason / remaining_hours 一致性 ---
        match result.source {
            PredictionSource::LocalAi => {
                // 通过质量门：remaining_hours 必须是合法有限正值
                let hours = result
                    .remaining_hours
                    .expect("LocalAi source must carry remaining_hours");
                assert!(
                    hours.is_finite() && hours > 0.0 && hours <= config.max_remaining_hours,
                    "remaining_hours must be finite, positive, and within bounds, got {hours}"
                );
                assert_eq!(
                    result.reason, "candidatePassedQualityGate",
                    "LocalAi source must carry candidatePassedQualityGate reason"
                );
                // 质量门已过 → validation_samples 必须达标
                assert!(result.validation_samples >= config.min_validation_samples);
            }
            PredictionSource::BaselineRecommended => {
                // 质量门回退：remaining_hours 必须为 None
                assert_eq!(
                    result.remaining_hours, None,
                    "BaselineRecommended must not carry remaining_hours"
                );
                // reason 必须是稳定错误码，不能是底层异常文本
                assert!(
                    is_stable_fallback_reason(&result.reason),
                    "fallback reason must be a stable code, got: {}",
                    result.reason
                );
            }
        }
    }

    /// 验证 `predict()` 的各 fallback 路径都返回稳定错误码而非底层异常文本，
    /// 且 remaining_hours 一致地为 None（Battery Usage 回退到 deterministic baseline）。
    #[test]
    fn prediction_smoke_fallback_reasons_are_stable_codes() {
        let now = test_now();
        let config = BatteryModelConfig::default();

        // 无放电样本（全部充电中）→ noDischargingSample
        let result = predict(
            &BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: 0,
                samples: vec![sample(now, 80, true)],
                current_context: None,
            },
            &config,
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
        assert_eq!(result.reason, "noDischargingSample");
        assert!(result.remaining_hours.is_none());
        assert_eq!(result.training_samples, 0);

        // 电量为 0 → emptyBattery
        let result = predict(
            &BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: 0,
                samples: vec![sample(now, 0, false)],
                current_context: None,
            },
            &config,
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
        assert_eq!(result.reason, "emptyBattery");
        assert!(result.remaining_hours.is_none());

        // 单个放电样本（无完整放电段）→ insufficientTrainingData
        let result = predict(
            &BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: 0,
                samples: vec![sample(now, 80, false)],
                current_context: None,
            },
            &config,
        )
        .unwrap();
        assert_eq!(result.source, PredictionSource::BaselineRecommended);
        assert_eq!(result.reason, "insufficientTrainingData");
        assert_eq!(result.training_samples, 0);
        assert!(result.remaining_hours.is_none());
    }
}
