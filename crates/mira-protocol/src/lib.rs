// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mira-specific business types for battery prediction.
//!
//! These types were previously part of `rill-runtime-protocol` but have been
//! moved here to decouple the generic runtime protocol from Mira business logic.

use serde::{Deserialize, Serialize};

/// Capability string for battery usage prediction.
pub const BATTERY_USAGE_CAPABILITY: &str = "batteryUsage";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatteryModelConfig {
    pub feature_count: usize,
    pub learning_rate: f64,
    pub l2: f64,
    pub huber_delta: f64,
    pub min_training_samples: u64,
    pub min_validation_samples: u64,
    pub quality_window: usize,
    pub required_error_ratio: f64,
    pub max_drain_per_hour: f64,
    pub max_remaining_hours: f64,
    pub session_gap_minutes: i64,
    pub replacement_rise_percent: u8,
    pub min_drop_percent: f64,
    pub baseline_decay_tau_hours: f64,
}

impl Default for BatteryModelConfig {
    fn default() -> Self {
        Self {
            // 6 base features (percentage, hour sin/cos, weekday sin/cos, recent_rate)
            // + 3 context features (dpi, polling_rate, light_intensity)
            feature_count: 9,
            learning_rate: 0.03,
            l2: 0.001,
            huber_delta: 5.0,
            min_training_samples: 6,
            min_validation_samples: 8,
            quality_window: 24,
            required_error_ratio: 0.98,
            max_drain_per_hour: 50.0,
            max_remaining_hours: 9999.0,
            session_gap_minutes: 10,
            replacement_rise_percent: 5,
            min_drop_percent: 1.0,
            baseline_decay_tau_hours: 48.0,
        }
    }
}

impl BatteryModelConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        let finite = [
            self.learning_rate,
            self.l2,
            self.huber_delta,
            self.required_error_ratio,
            self.max_drain_per_hour,
            self.max_remaining_hours,
            self.min_drop_percent,
            self.baseline_decay_tau_hours,
        ]
        .into_iter()
        .all(f64::is_finite);
        if !finite {
            return Err("battery model contains non-finite parameters");
        }
        if self.feature_count != 9 {
            return Err("unsupported battery feature schema");
        }
        if self.learning_rate <= 0.0
            || self.l2 < 0.0
            || self.huber_delta <= 0.0
            || self.min_training_samples == 0
            || self.min_validation_samples == 0
            || self.quality_window < self.min_validation_samples as usize
            || !(0.0..1.0).contains(&self.required_error_ratio)
            || self.max_drain_per_hour <= 0.0
            || self.max_remaining_hours <= 0.0
            || self.session_gap_minutes <= 0
            || self.replacement_rise_percent == 0
            || self.replacement_rise_percent > 100
            || self.min_drop_percent <= 0.0
            || self.baseline_decay_tau_hours <= 0.0
        {
            return Err("invalid battery model parameters");
        }
        Ok(())
    }
}

/// 设备低频变动参数上下文：DPI、回报率、灯光模式等。
///
/// 这些字段由宿主从 `DeviceSnapshot` 缓存中提取，**不触发任何额外 HID 读取**。
/// Runtime 端将其作为预测模型的附加特征，提升剩余时间预估准确性。
/// 所有字段为 `Option`，缺失时模型回退到纯电量曲线（与历史行为一致）。
///
/// 序列化策略：字段值为 `None` 时不写入 JSON（`skip_serializing_if = "Option::is_none"`），
/// 同时配合 `#[serde(default)]`，保证旧版 runtime 反序列化时不会因 `deny_unknown_fields`
/// 报错——旧 runtime 看到的 JSON 与升级前完全一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceContextSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpi: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polling_rate_hz: Option<u16>,
    /// 灯光模式：off / static / breathing / rainbow / reactive / wave ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_mode: Option<String>,
    /// 灯光亮度（0-100）。部分插件不暴露此字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_brightness: Option<u8>,
    /// 当前活动配置文件标识。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl DeviceContextSnapshot {
    pub fn is_empty(&self) -> bool {
        self.dpi.is_none()
            && self.polling_rate_hz.is_none()
            && self.light_mode.is_none()
            && self.light_brightness.is_none()
            && self.profile.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatterySampleInput {
    pub at_unix_ms: i64,
    pub percentage: u8,
    pub charging: bool,
    /// 采样时刻的设备上下文。复用宿主缓存的 `DeviceSnapshot`，
    /// 不触发额外读取；为 `None` 时表示该样本无上下文（旧 schema 或未读取）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<DeviceContextSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatteryPredictionInput {
    pub now_unix_ms: i64,
    pub samples: Vec<BatterySampleInput>,
    /// 当前设备上下文（最近一次成功读取的快照投影）。
    /// Runtime 据此识别参数切换点，提升短时预测准确性。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_context: Option<DeviceContextSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PredictionSource {
    LocalAi,
    BaselineRecommended,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatteryPredictionOutput {
    pub remaining_hours: Option<f64>,
    pub source: PredictionSource,
    pub reason: String,
    pub training_samples: u64,
    pub validation_samples: u64,
    pub baseline_mae: Option<f64>,
    pub candidate_mae: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_config_rejects_unsafe_values() {
        assert!(BatteryModelConfig::default().validate().is_ok());
        assert!(BatteryModelConfig {
            quality_window: 0,
            ..BatteryModelConfig::default()
        }
        .validate()
        .is_err());
        assert!(BatteryModelConfig {
            learning_rate: f64::NAN,
            ..BatteryModelConfig::default()
        }
        .validate()
        .is_err());
        assert!(BatteryModelConfig {
            max_remaining_hours: 0.0,
            ..BatteryModelConfig::default()
        }
        .validate()
        .is_err());
        // 旧版 6 特征 schema 必须被拒绝，强制使用 9 特征 schema（含 DPI/回报率/灯光）
        assert!(BatteryModelConfig {
            feature_count: 6,
            ..BatteryModelConfig::default()
        }
        .validate()
        .is_err());
    }

    /// 验证 context 字段为 None 时，序列化结果不包含 context/current_context 键。
    /// 这保证旧版 runtime（deny_unknown_fields）不会因未知字段报错。
    #[test]
    fn context_none_omits_field_for_backward_compatibility() {
        let input = BatteryPredictionInput {
            now_unix_ms: 1_700_000_000_000,
            samples: vec![BatterySampleInput {
                at_unix_ms: 1_700_000_000_000,
                percentage: 80,
                charging: false,
                context: None,
            }],
            current_context: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(
            !json.contains("context"),
            "None context must not appear in JSON, got: {json}"
        );
        assert!(
            !json.contains("currentContext"),
            "None current_context must not appear in JSON, got: {json}"
        );
    }

    #[test]
    fn empty_context_is_detected_without_changing_wire_shape() {
        let context = DeviceContextSnapshot::default();
        assert!(context.is_empty());
        let populated = DeviceContextSnapshot {
            dpi: Some(800),
            ..Default::default()
        };
        assert!(!populated.is_empty());
    }

    /// 验证 context 字段有值时正确序列化，且能往返反序列化。
    #[test]
    fn context_some_roundtrips_correctly() {
        let context = DeviceContextSnapshot {
            dpi: Some(16000),
            polling_rate_hz: Some(8000),
            light_mode: Some("breathing".into()),
            light_brightness: Some(75),
            profile: Some("profile1".into()),
        };
        let input = BatteryPredictionInput {
            now_unix_ms: 1_700_000_000_000,
            samples: vec![BatterySampleInput {
                at_unix_ms: 1_700_000_000_000,
                percentage: 80,
                charging: false,
                context: Some(context.clone()),
            }],
            current_context: Some(context),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"dpi\":16000"));
        assert!(json.contains("\"pollingRateHz\":8000"));
        assert!(json.contains("\"lightMode\":\"breathing\""));
        assert!(json.contains("\"lightBrightness\":75"));
        assert!(json.contains("\"profile\":\"profile1\""));
        assert!(json.contains("\"currentContext\""));

        let decoded: BatteryPredictionInput = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, input);
    }

    /// 验证旧格式 JSON（无 context 字段）能被反序列化。
    #[test]
    fn legacy_json_without_context_deserializes() {
        let legacy = r#"{"nowUnixMs":1700000000000,"samples":[{"atUnixMs":1700000000000,"percentage":80,"charging":false}]}"#;
        let decoded: BatteryPredictionInput = serde_json::from_str(legacy).unwrap();
        assert_eq!(decoded.now_unix_ms, 1_700_000_000_000);
        assert_eq!(decoded.samples.len(), 1);
        assert_eq!(decoded.samples[0].percentage, 80);
        assert!(decoded.samples[0].context.is_none());
        assert!(decoded.current_context.is_none());
    }
}
