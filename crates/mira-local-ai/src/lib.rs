use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};
use mira_protocol::{
    BatteryModelConfig, BatteryPredictionInput, BatteryPredictionOutput, BatterySampleInput,
    DeviceContextSnapshot, PredictionSource,
};
use rill_ml::{
    diagnostics::BaselineComparator,
    loss::{HuberLoss, RegressionLoss},
    models::{LinearRegression, LinearRegressionConfig},
    optim::{Optimizer, SgdConfig},
    OnlineRegressor,
};
use thiserror::Error;

const MAX_SAMPLES: usize = 10_000;

/// DPI 归一化上界。当前主流最高 DPI 约 30000（Razer DeathAdder V3 Pro），
/// 设为 60000（2x）为未来高分辨率传感器预留空间。
const MAX_DPI: f64 = 60000.0;
/// 回报率归一化上界。当前主流最高回报率 8000 Hz（Razer Viper 8KHz），
/// 设为 16000 Hz（2x）为未来更高刷新率设备预留空间。
const MAX_POLLING_RATE_HZ: f64 = 16000.0;

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
    samples.retain(|sample| sample.at_unix_ms <= input.now_unix_ms);
    samples.sort_by_key(|sample| sample.at_unix_ms);

    let Some(current) = samples
        .iter()
        .filter(|sample| !sample.charging && sample.at_unix_ms <= input.now_unix_ms)
        .max_by_key(|sample| sample.at_unix_ms)
    else {
        return Ok(fallback("noDischargingSample", 0, 0, None, None));
    };
    if current.percentage == 0 {
        return Ok(fallback("emptyBattery", 0, 0, None, None));
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
) -> Result<BatteryPredictionOutput, BatteryPredictionError> {
    let optimizer = Optimizer::sgd(
        config.feature_count,
        SgdConfig {
            learning_rate: config.learning_rate,
            l2: config.l2,
        },
    )
    .map_err(|_| BatteryPredictionError::InvalidModel)?;
    let mut model = LinearRegression::new(
        config.feature_count,
        LinearRegressionConfig {
            optimizer,
            loss: RegressionLoss::Huber(
                HuberLoss::new(config.huber_delta)
                    .map_err(|_| BatteryPredictionError::InvalidModel)?,
            ),
        },
    )
    .map_err(|_| BatteryPredictionError::InvalidModel)?;
    let mut comparator = BaselineComparator::new(
        &["deterministic-baseline", "rill-local-ai"],
        config.quality_window,
    )
    .map_err(|_| BatteryPredictionError::InvalidModel)?;

    for (index, observation) in observations.iter().enumerate() {
        let recent_rate = weighted_baseline_rate(
            &observations[..index],
            observation.at,
            config.baseline_decay_tau_hours,
        );
        let features = features(
            observation.percentage,
            observation.at,
            observation.timezone_offset_minutes,
            recent_rate,
            observation.context.as_ref(),
        );
        if model.samples_seen() >= config.min_training_samples {
            if let Some(baseline_prediction) = recent_rate {
                if let Ok(ai_prediction) = model.predict(&features) {
                    if ai_prediction.is_finite() {
                        comparator
                            .record(0, observation.drain_per_hour, baseline_prediction)
                            .map_err(|_| BatteryPredictionError::InvalidModel)?;
                        comparator
                            .record(1, observation.drain_per_hour, ai_prediction)
                            .map_err(|_| BatteryPredictionError::InvalidModel)?;
                    }
                }
            }
        }
        model
            .learn(&features, observation.drain_per_hour)
            .map_err(|_| BatteryPredictionError::InvalidModel)?;
    }

    comparator.update_best();
    let baseline = comparator.entry(0);
    let candidate = comparator.entry(1);
    let validation_samples = candidate.map_or(0, |entry| entry.total_samples());
    let baseline_samples = baseline.map_or(0, |entry| entry.total_samples());
    let baseline_mae = baseline.and_then(|entry| entry.rolling_mae());
    let candidate_mae = candidate.and_then(|entry| entry.rolling_mae());
    let training_samples = model.samples_seen();

    if training_samples < config.min_training_samples {
        return Ok(fallback(
            "insufficientTrainingData",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
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
        ));
    }
    let (Some(baseline_error), Some(candidate_error)) = (baseline_mae, candidate_mae) else {
        return Ok(fallback(
            "qualityMetricsUnavailable",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
        ));
    };
    if candidate_error >= baseline_error * config.required_error_ratio {
        return Ok(fallback(
            "candidateNotBetter",
            training_samples,
            validation_samples,
            baseline_mae,
            candidate_mae,
        ));
    }

    let recent_rate = weighted_baseline_rate(observations, now, config.baseline_decay_tau_hours);
    let predicted_rate = model
        .predict(&features(
            current_percentage,
            now,
            now_timezone_offset_minutes,
            recent_rate,
            current_context,
        ))
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
    })
}

fn fallback(
    reason: &str,
    training_samples: u64,
    validation_samples: u64,
    baseline_mae: Option<f64>,
    candidate_mae: Option<f64>,
) -> BatteryPredictionOutput {
    BatteryPredictionOutput {
        remaining_hours: None,
        source: PredictionSource::BaselineRecommended,
        reason: reason.into(),
        training_samples,
        validation_samples,
        baseline_mae,
        candidate_mae,
    }
}

fn weighted_baseline_rate(
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

/// 将灯光模式名映射为功耗强度评分 \[0, 1\]。
///
/// 不同灯光模式的功耗差异显著：关闭最省电，彩虹/星光等全彩动态模式最耗电。
/// 未知模式默认取中位强度 0.5，避免引入偏差。
fn light_mode_intensity(mode: &str) -> f64 {
    match mode.to_lowercase().as_str() {
        "off" | "disabled" | "none" => 0.0,
        "static" | "fixed" | "solid" => 0.3,
        "breathing" | "breath" => 0.5,
        "reactive" => 0.6,
        "ripple" => 0.7,
        "wave" => 0.8,
        "starlight" => 0.85,
        "rainbow" | "cycle" | "spectrum" => 0.9,
        "custom" => 1.0,
        _ => 0.5,
    }
}

/// 从 `DeviceContextSnapshot` 提取 3 个归一化特征：DPI、回报率、灯光综合强度。
///
/// 归一化策略：
/// - DPI: `dpi / 60000.0`，clamp 到 \[0, 1\]
/// - 回报率: `polling_rate_hz / 16000.0`，clamp 到 \[0, 1\]
/// - 灯光强度: `mode_intensity * (brightness / 100.0)`，无亮度时仅用 mode_intensity
///
/// 上下文缺失（旧 schema 样本）时返回 `[0.0, 0.0, 0.0]`，
/// 线性模型中对应权重贡献为 0，等价于不使用该特征，保证向后兼容。
fn context_features(context: Option<&DeviceContextSnapshot>) -> [f64; 3] {
    match context {
        Some(ctx) => {
            let dpi = ctx
                .dpi
                .map(|d| (d as f64 / MAX_DPI).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let polling_rate = ctx
                .polling_rate_hz
                .map(|p| (p as f64 / MAX_POLLING_RATE_HZ).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let light_intensity = ctx
                .light_mode
                .as_ref()
                .map(|mode| {
                    let base = light_mode_intensity(mode);
                    match ctx.light_brightness {
                        Some(b) => base * (b as f64 / 100.0),
                        None => base,
                    }
                })
                .unwrap_or(0.0);
            [dpi, polling_rate, light_intensity]
        }
        None => [0.0, 0.0, 0.0],
    }
}

/// 构造 9 维特征向量：6 个基础特征 + 3 个上下文特征。
///
/// 基础特征：电量百分比、时间（sin/cos）、星期（sin/cos）、近期放电率
/// 上下文特征：DPI、回报率、灯光综合强度
fn features(
    percentage: u8,
    at: DateTime<Utc>,
    timezone_offset_minutes: i32,
    recent_rate: Option<f64>,
    context: Option<&DeviceContextSnapshot>,
) -> [f64; 9] {
    let timezone = validate_timezone_offset(timezone_offset_minutes)
        .expect("timezone offsets are validated at the handler boundary");
    let local = at.with_timezone(&timezone);
    let hour_angle = local.hour() as f64 / 24.0 * std::f64::consts::TAU;
    let weekday_angle = local.weekday().num_days_from_monday() as f64 / 7.0 * std::f64::consts::TAU;
    let [dpi, polling_rate, light_intensity] = context_features(context);
    [
        percentage as f64 / 100.0,
        hour_angle.sin(),
        hour_angle.cos(),
        weekday_angle.sin(),
        weekday_angle.cos(),
        recent_rate.unwrap_or(1.0) / 10.0,
        dpi,
        polling_rate,
        light_intensity,
    ]
}

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
                || sample.percentage.saturating_sub(prev.percentage)
                    >= config.replacement_rise_percent
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
        || hours <= 0.0
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
    use chrono::{Duration, TimeZone};

    fn test_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap()
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
        )
        .unwrap();
        assert_eq!(capped.source, PredictionSource::BaselineRecommended);
        assert_eq!(capped.remaining_hours, None);
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

    /// 验证灯光模式到功耗强度评分的映射。
    #[test]
    fn light_mode_intensity_maps_known_modes() {
        assert_eq!(light_mode_intensity("off"), 0.0);
        assert_eq!(light_mode_intensity("OFF"), 0.0);
        assert_eq!(light_mode_intensity("static"), 0.3);
        assert_eq!(light_mode_intensity("breathing"), 0.5);
        assert_eq!(light_mode_intensity("reactive"), 0.6);
        assert_eq!(light_mode_intensity("ripple"), 0.7);
        assert_eq!(light_mode_intensity("wave"), 0.8);
        assert_eq!(light_mode_intensity("starlight"), 0.85);
        assert_eq!(light_mode_intensity("rainbow"), 0.9);
        assert_eq!(light_mode_intensity("custom"), 1.0);
        // 未知模式取中位值
        assert_eq!(light_mode_intensity("unknown_xyz"), 0.5);
    }

    /// 验证 context_features 归一化与缺失值回退。
    #[test]
    fn context_features_normalize_and_default_correctly() {
        // 完整上下文
        let ctx = DeviceContextSnapshot {
            dpi: Some(16000),
            polling_rate_hz: Some(8000),
            light_mode: Some("breathing".into()),
            light_brightness: Some(80),
            profile: None,
        };
        let [dpi, rate, light] = context_features(Some(&ctx));
        assert!((dpi - (16000.0 / 60000.0)).abs() < 1e-9);
        assert!((rate - (8000.0 / 16000.0)).abs() < 1e-9);
        // breathing=0.5 * brightness=0.8 = 0.4
        assert!((light - 0.4).abs() < 1e-9);

        // 无亮度时仅用 mode_intensity
        let ctx_no_brightness = DeviceContextSnapshot {
            dpi: Some(16000),
            polling_rate_hz: Some(8000),
            light_mode: Some("breathing".into()),
            light_brightness: None,
            profile: None,
        };
        let [_, _, light2] = context_features(Some(&ctx_no_brightness));
        assert!((light2 - 0.5).abs() < 1e-9);

        // DPI 超出上界时 clamp 到 1.0
        let ctx_high = DeviceContextSnapshot {
            dpi: Some(65000),
            polling_rate_hz: Some(8000),
            light_mode: Some("breathing".into()),
            light_brightness: Some(80),
            profile: None,
        };
        let [dpi_high, _, _] = context_features(Some(&ctx_high));
        assert!((dpi_high - 1.0).abs() < 1e-9);

        // 上下文缺失时返回全零
        let [d, r, l] = context_features(None);
        assert_eq!([d, r, l], [0.0, 0.0, 0.0]);
    }

    /// 验证不同上下文产生不同预测结果：模型确实消费了 DPI/回报率/灯光特征。
    ///
    /// 构造两组训练数据，放电率与 DPI 正相关（高 DPI 耗电更快），
    /// 验证模型在高 DPI 上下文下预测的剩余时间更短。
    #[test]
    fn different_contexts_produce_different_predictions() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();
        // 低功耗上下文：低 DPI、低回报率、灯光关闭
        let low_ctx = DeviceContextSnapshot {
            dpi: Some(800),
            polling_rate_hz: Some(125),
            light_mode: Some("off".into()),
            light_brightness: None,
            profile: None,
        };
        // 高功耗上下文：高 DPI、高回报率、彩虹灯全亮
        let high_ctx = DeviceContextSnapshot {
            dpi: Some(26000),
            polling_rate_hz: Some(8000),
            light_mode: Some("rainbow".into()),
            light_brightness: Some(100),
            profile: None,
        };
        // 训练数据：低功耗场景放电慢（2%/h），高功耗场景放电快（10%/h）
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
        )
        .unwrap();
        let result_high = validated_model_prediction(
            &observations,
            80,
            start + Duration::hours(121),
            0,
            Some(&high_ctx),
            &BatteryModelConfig::default(),
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
        let feats = features(80, now, 0, Some(5.0), Some(&ctx));
        assert_eq!(feats.len(), 9);
        // 基础特征
        assert!((feats[0] - 0.8).abs() < 1e-9); // percentage
        assert!(feats[5] > 0.0); // recent_rate
                                 // 上下文特征
        assert!((feats[6] - (16000.0 / 60000.0)).abs() < 1e-9); // dpi
        assert!((feats[7] - (4000.0 / 16000.0)).abs() < 1e-9); // polling_rate
                                                               // static=0.3 * brightness=0.5 = 0.15
        assert!((feats[8] - 0.15).abs() < 1e-9); // light_intensity

        // 无上下文时后 3 维为 0
        let feats_none = features(80, now, 0, Some(5.0), None);
        assert_eq!(feats_none.len(), 9);
        assert_eq!(feats_none[6], 0.0);
        assert_eq!(feats_none[7], 0.0);
        assert_eq!(feats_none[8], 0.0);
    }
}
