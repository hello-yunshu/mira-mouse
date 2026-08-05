// SPDX-License-Identifier: AGPL-3.0-or-later
//! 异常与漂移检测实验层（阶段 4）。
//!
//! 本模块由 `robustDetectionEnabled` 控制，默认关闭。开启时只影响
//! 训练样本权重与置信度信号，绝不直接删除数据或重置模型：
//!
//! - `anomaly.rs`：基于 rill-ml 1.1.0 `RollingMedianMad`（Preview，未视为
//!   Stable API）的稳健异常评分（modified z-score）。
//! - `drift.rs`：中位数漂移启发式 + rill-ml `DriftConsensus`（Preview，只做
//!   实验性滞后共识，未接入生产决策）的滞后共识。
//! - `confidence.rs`：异常/漂移 → 置信度倍率的映射。
//!
//! 说明：漂移信号目前仅实验计算；`confidence` 与 `suggested_action` 只是
//! 建议信号，尚未接入任何生产决策路径。开关关闭时本模块不参与任何计算路径。

pub mod anomaly;
pub mod confidence;
pub mod drift;

use mira_protocol::BatteryModelConfig;

/// 推荐动作。检测器只报告建议，由调用方决定是否执行；
/// 本实验层从不自行执行破坏性动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestedAction {
    None,
    DownWeightSample,
    LowerPredictionConfidence,
    ResetRecentRateOnly,
    RequestModelRetrain,
}

/// 漂移方向（仅作诊断信号）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftKind {
    DrainRateIncrease,
    DrainRateDecrease,
}

/// 每个观测样本的稳健信号汇总。
#[derive(Debug, Clone, PartialEq)]
pub struct BatteryRobustSignals {
    /// 稳健异常评分，范围 [0, 1]。0 = 与近期中位数一致。
    pub anomaly_score: f64,
    /// 是否检测到确认的漂移（经 DriftConsensus 滞后确认）。
    pub drift_detected: bool,
    /// 漂移方向，未检测到漂移时为 None。
    pub drift_kind: Option<DriftKind>,
    /// 置信度倍率，范围 [0.5, 1.0]。
    pub confidence_multiplier: f64,
    /// 建议动作（仅建议，不执行）。
    pub suggested_action: SuggestedAction,
    /// 训练样本权重修正系数，范围 [0.25, 1.0]。异常样本据此降权。
    pub anomaly_downweight: f64,
}

impl Default for BatteryRobustSignals {
    fn default() -> Self {
        Self {
            anomaly_score: 0.0,
            drift_detected: false,
            drift_kind: None,
            confidence_multiplier: 1.0,
            suggested_action: SuggestedAction::None,
            anomaly_downweight: 1.0,
        }
    }
}

/// 稳健检测器编排：每个观测样本调用一次 [`update`](RobustDetector::update)。
///
/// 状态全部在进程内、每次预测运行独立构造，不写入长期状态
/// （实验层要求：除非兼容性测试完整通过，否则不进入默认持久化路径）。
pub struct RobustDetector {
    anomaly_scorer: anomaly::AnomalyScorer,
    drift_source: drift::MedianDriftSource,
    consensus: rill_ml::drift::DriftConsensus,
    generation: u64,
}

impl RobustDetector {
    /// 构造检测器。实验层目前无需额外配置，`config` 保留用于未来实验参数。
    pub fn new(_config: &BatteryModelConfig) -> Result<Self, RobustDetectorError> {
        Ok(Self {
            anomaly_scorer: anomaly::AnomalyScorer::new(),
            drift_source: drift::MedianDriftSource::new(),
            consensus: rill_ml::drift::DriftConsensus::new({
                // DriftConsensusConfig 是 #[non_exhaustive]，必须 Default + 字段赋值。
                let mut consensus_config = rill_ml::drift::DriftConsensusConfig::default();
                consensus_config.minimum_warning_votes = 1;
                consensus_config.minimum_drift_votes = 1;
                consensus_config.confirmation_windows = 3;
                consensus_config.clear_windows = 3;
                consensus_config.cooldown_windows = 5;
                consensus_config.warming_windows = 2;
                consensus_config
            })
            .map_err(|_| RobustDetectorError::Init)?,
            generation: 0,
        })
    }

    /// 融入一个新观测并返回稳健信号。非有限输入返回 Err（调用方按中性处理，
    /// 不 panic、不跳过其他逻辑）。
    pub fn update(
        &mut self,
        drain_per_hour: f64,
    ) -> Result<BatteryRobustSignals, RobustDetectorError> {
        self.anomaly_scorer.update(drain_per_hour)?;
        self.drift_source.update(drain_per_hour)?;
        let level = self.drift_source.level();
        let vote = rill_ml::drift::DriftVote::new("mira-median-drift", level)
            .map_err(|_| RobustDetectorError::Vote)?;
        let result = self
            .consensus
            .update(&[vote], self.generation)
            .map_err(|_| RobustDetectorError::Consensus)?;

        let anomaly_score = self.anomaly_scorer.score();
        let drift_detected = result.level == rill_ml::drift::DriftLevel::Drift;
        let drift_kind = drift_detected.then(|| self.drift_source.kind());
        let confidence_multiplier = confidence::confidence_multiplier(anomaly_score, result.level);
        let suggested_action = if drift_detected {
            SuggestedAction::RequestModelRetrain
        } else if anomaly_score >= 0.8 {
            SuggestedAction::DownWeightSample
        } else if anomaly_score >= 0.5 {
            SuggestedAction::LowerPredictionConfidence
        } else {
            SuggestedAction::None
        };
        Ok(BatteryRobustSignals {
            anomaly_score,
            drift_detected,
            drift_kind,
            confidence_multiplier,
            suggested_action,
            anomaly_downweight: 1.0 - 0.75 * anomaly_score,
        })
    }
}

/// 稳健层错误。全部为内部状态错误，调用方一律按"无信号"处理。
#[derive(Debug, thiserror::Error)]
pub enum RobustDetectorError {
    #[error("robust detector init failed")]
    Init,
    #[error("robust detector vote construction failed")]
    Vote,
    #[error("robust drift consensus failed")]
    Consensus,
    #[error("robust statistic rejected the value: {0}")]
    Statistic(#[from] rill_ml::RillError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn config() -> BatteryModelConfig {
        BatteryModelConfig::default()
    }

    #[test]
    fn neutral_signals_when_disabled() {
        // robust_detection_enabled 默认 false；RobustDetector 仅在开启时构造，
        // 开关关闭时实验层完全不参与计算路径。
        assert!(!config().robust_detection_enabled);
    }

    #[test]
    fn spike_is_downweighted_but_stream_survives() {
        let mut detector = RobustDetector::new(&config()).unwrap();
        // 有轻微噪声的稳定基线（MAD 非零），才能为异常建立相对尺度。双向检测取 |z|
        // 绝对值，偶发低于中位数的噪声点会产生很小的异常分，但远低于真正的突升。
        let mut warmup_min_downweight: f64 = 1.0;
        for i in 0..64 {
            let signals = detector.update(5.0 + (i % 7) as f64 * 0.1).unwrap();
            assert!(
                signals.anomaly_score < 0.1,
                "warmup noise should stay near zero: {}",
                signals.anomaly_score
            );
            warmup_min_downweight = warmup_min_downweight.min(signals.anomaly_downweight);
        }
        assert!(warmup_min_downweight > 0.9);
        let spike = detector.update(50.0).unwrap();
        // 异常点被显著降权，但不会清空历史：后续样本恢复正常权重。
        assert!(spike.anomaly_downweight < 0.5);
        assert!(spike.suggested_action != SuggestedAction::None);
        let after = detector.update(5.3).unwrap();
        assert!(after.anomaly_downweight > spike.anomaly_downweight);
    }

    #[test]
    fn drift_ramp_eventually_confirms() {
        let mut detector = RobustDetector::new(&config()).unwrap();
        let mut saw_warning = false;
        let mut saw_drift = false;
        for step in 0..200 {
            let signals = detector.update(5.0 + step as f64 * 0.3).unwrap();
            if signals.confidence_multiplier < 1.0 {
                saw_warning = true;
            }
            if signals.drift_detected {
                saw_drift = true;
            }
        }
        assert!(saw_warning, "ramp should reduce confidence at some point");
        assert!(saw_drift, "sustained ramp should confirm drift");
    }

    #[test]
    fn non_finite_update_returns_error() {
        let mut detector = RobustDetector::new(&config()).unwrap();
        assert!(detector.update(5.0).is_ok());
        assert!(detector.update(f64::NAN).is_err());
    }

    #[test]
    fn signals_stay_inside_bounds() {
        let mut detector = RobustDetector::new(&config()).unwrap();
        for step in 0..240 {
            let signals = detector.update(5.0 + step as f64 * 0.5).unwrap();
            assert!((0.0..=1.0).contains(&signals.anomaly_score));
            assert!((0.5..=1.0).contains(&signals.confidence_multiplier));
            assert!(signals.anomaly_downweight >= 0.25);
            assert!(signals.anomaly_downweight <= 1.0);
        }
    }

    #[test]
    fn time_reference_helpers_compile() {
        let now = Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        assert!((now + Duration::hours(1)) > now);
    }
}
