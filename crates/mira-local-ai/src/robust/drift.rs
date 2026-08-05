// SPDX-License-Identifier: AGPL-3.0-or-later
//! 漂移信号：中位数漂移启发式。
//!
//! 长期中位数在长窗口首次就绪时锁定为参考基线，之后比较短窗口（近期）
//! 中位数与锁定基线。绝对值偏差除以短窗口 MAD（局部噪声尺度），产生
//! `DriftLevel::{None, Warning, Drift}`，方向由符号决定。检测器只报告
//! 信号；确认与滞后由 rill-ml `DriftConsensus` 在 `RobustDetector` 中完成。
//!
//! 实验层默认参数（仅本模块内部使用）：
//! - 短窗口容量 16，长窗口容量 64（参考基线在长窗口就绪时锁定一次）；
//! - ratio > `DRIFT_RATIO_DRIFT` 记 Drift，> `DRIFT_RATIO_WARNING` 记 Warning。
//!
//! 语义说明：基线锁定意味着"持续偏离初始参考"被判定为漂移。电池场景中
//! 长期改变（如用户切换配置）既算漂移，也应触发重训建议；若后续确实是
//! 新常态，重训后新基线自然建立。

use crate::robust::DriftKind;
use rill_ml::drift::DriftLevel;
use rill_ml::stats::RollingMedianMad;
use rill_ml::OnlineStatistic;

/// 短窗口容量（近期中位数）。
const SHORT_CAPACITY: usize = 16;
/// 长窗口容量（长期中位数基线）。
const LONG_CAPACITY: usize = 64;
/// 锁定基线的样本数：长窗口满窗后基线不再变化。
const LONG_MIN_SAMPLES: usize = 16;
/// 相对短窗口 MAD 的倍数：超过记 Drift。
const DRIFT_RATIO_DRIFT: f64 = 6.0;
/// 相对短窗口 MAD 的倍数：超过记 Warning。
const DRIFT_RATIO_WARNING: f64 = 3.0;
/// MAD 为零时避免除零的极小值。
const MAD_EPSILON: f64 = 1e-6;

/// 中位数漂移信号源：每个观测更新一次，`level()` 返回最近窗口的级别。
#[derive(Debug, Clone)]
pub struct MedianDriftSource {
    short: RollingMedianMad,
    long: RollingMedianMad,
    baseline: Option<f64>,
    level: DriftLevel,
    kind: DriftKind,
}

impl MedianDriftSource {
    pub fn new() -> Self {
        Self {
            short: RollingMedianMad::new(SHORT_CAPACITY, 4)
                .expect("constant short window config is valid"),
            long: RollingMedianMad::new(LONG_CAPACITY, LONG_MIN_SAMPLES)
                .expect("constant long window config is valid"),
            baseline: None,
            level: DriftLevel::None,
            kind: DriftKind::DrainRateIncrease,
        }
    }

    /// 融入一个观测。非有限值返回 Err，调用方按中性处理。
    pub fn update(&mut self, value: f64) -> Result<(), rill_ml::RillError> {
        self.short.update(value)?;
        self.long.update(value)?;
        if self.short.is_ready() && self.long.is_ready() {
            if self.baseline.is_none() {
                self.baseline = Some(self.long.summary()?.median());
            }
            let short_median = self.short.summary()?.median();
            let baseline = self.baseline.unwrap_or(short_median);
            // 以短窗口 MAD 为尺度：短窗口近似平稳，MAD 代表局部噪声水平；
            // 近期中位数相对参考基线的偏离即漂移强度。
            let short_mad = self.short.summary()?.mad();
            let ratio = (short_median - baseline).abs() / (short_mad + MAD_EPSILON);
            self.level = if ratio > DRIFT_RATIO_DRIFT {
                DriftLevel::Drift
            } else if ratio > DRIFT_RATIO_WARNING {
                DriftLevel::Warning
            } else {
                DriftLevel::None
            };
            self.kind = if short_median >= baseline {
                DriftKind::DrainRateIncrease
            } else {
                DriftKind::DrainRateDecrease
            };
        } else {
            self.level = DriftLevel::None;
        }
        Ok(())
    }

    /// 最近一次更新的原始漂移级别。
    pub const fn level(&self) -> DriftLevel {
        self.level
    }

    /// 漂移方向。
    pub const fn kind(&self) -> DriftKind {
        self.kind
    }
}

impl Default for MedianDriftSource {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_stream_reports_none() {
        let mut source = MedianDriftSource::new();
        for value in [5.0; 80] {
            source.update(value).unwrap();
        }
        assert_eq!(source.level(), DriftLevel::None);
    }

    #[test]
    fn increasing_ramp_reports_drift() {
        let mut source = MedianDriftSource::new();
        for step in 0..90 {
            source.update(5.0 + step as f64 * 0.4).unwrap();
        }
        assert_eq!(source.level(), DriftLevel::Drift);
        assert_eq!(source.kind(), DriftKind::DrainRateIncrease);
    }

    #[test]
    fn decreasing_ramp_reports_drift_downward() {
        let mut source = MedianDriftSource::new();
        for step in 0..90 {
            source.update(50.0 - step as f64 * 0.4).unwrap();
        }
        assert_eq!(source.level(), DriftLevel::Drift);
        assert_eq!(source.kind(), DriftKind::DrainRateDecrease);
    }

    #[test]
    fn spike_burst_shifts_then_recovers() {
        // 单点异常会被中位数吸收，正常的多点突发才会短暂改变中位数；
        // 回到常态后层级恢复 None。
        let mut source = MedianDriftSource::new();
        for _ in 0..48 {
            source.update(5.0).unwrap();
        }
        let single_spike = {
            source.update(50.0).unwrap();
            source.level()
        };
        assert_eq!(single_spike, DriftLevel::None);
        for _ in 0..12 {
            source.update(40.0).unwrap();
        }
        assert_eq!(source.level(), DriftLevel::Drift);
        for _ in 0..40 {
            source.update(5.0).unwrap();
        }
        assert_eq!(source.level(), DriftLevel::None);
    }

    #[test]
    fn non_finite_value_is_rejected() {
        let mut source = MedianDriftSource::new();
        assert!(source.update(5.0).is_ok());
        assert!(source.update(f64::NAN).is_err());
    }
}
