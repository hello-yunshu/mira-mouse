// SPDX-License-Identifier: AGPL-3.0-or-later
//! 稳健异常评分：基于 rill-ml 1.1.0 `RollingMedianMad`（Stable、有 portable
//! state）的 modified z-score。
//!
//! 实验层默认参数（仅本模块内部使用，不进入配置面）：
//! - 窗口容量 16，min_samples 8；
//! - z-score 超过 `ANOMALY_SCORE_FULL` 记 1.0，`ANOMALY_SCORE_ZERO` 及以下记 0.0，
//!   中间线性映射。窗口 MAD 为 0（恒定流）时记 0.0，不产生 NaN。

use rill_ml::stats::{ModifiedZScore, RollingMedianMad};
use rill_ml::OnlineStatistic;

/// 异常窗口容量。
const ANOMALY_CAPACITY: usize = 16;
/// 异常评分所需的最小窗口样本数。
const ANOMALY_MIN_SAMPLES: usize = 8;
/// z-score 达到该值时 anomaly_score 记满 1.0。
const ANOMALY_SCORE_FULL: f64 = 4.0;
/// z-score 低于该值时 anomaly_score 记 0.0。
const ANOMALY_SCORE_ZERO: f64 = 1.5;

/// 基于滚动中位数/MAD 的异常评分器。
#[derive(Debug, Clone)]
pub struct AnomalyScorer {
    window: RollingMedianMad,
    score: f64,
}

impl AnomalyScorer {
    pub fn new() -> Self {
        Self {
            window: RollingMedianMad::new(ANOMALY_CAPACITY, ANOMALY_MIN_SAMPLES)
                .expect("constant anomaly window config is valid"),
            score: 0.0,
        }
    }

    /// 融入一个观测并更新当前评分。评分基于"融入前"的窗口分布计算，
    /// 因此异常点本身不会进入它自己的参考分布。
    pub fn update(&mut self, value: f64) -> Result<(), rill_ml::RillError> {
        self.score = if self.window.is_ready() {
            match self.window.modified_z_score(value) {
                Ok(ModifiedZScore::Defined { score, .. }) => normalize_z(score),
                // MAD 为 0：参考窗恒定，无法建立相对尺度，按无信号处理。
                Ok(ModifiedZScore::ZeroMad { .. }) | Err(_) => 0.0,
            }
        } else {
            0.0
        };
        self.window.update(value)?;
        Ok(())
    }

    /// 最近一次更新的异常评分，范围 [0, 1]。
    pub const fn score(&self) -> f64 {
        self.score
    }
}

impl Default for AnomalyScorer {
    fn default() -> Self {
        Self::new()
    }
}

/// 将 modified z-score 线性映射到 [0, 1]。
fn normalize_z(z: f64) -> f64 {
    if !z.is_finite() {
        return 0.0;
    }
    ((z - ANOMALY_SCORE_ZERO) / (ANOMALY_SCORE_FULL - ANOMALY_SCORE_ZERO)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_stream_scores_zero() {
        let mut scorer = AnomalyScorer::new();
        for value in [5.0; 20] {
            scorer.update(value).unwrap();
        }
        assert_eq!(scorer.score(), 0.0);
    }

    #[test]
    fn spike_after_stable_window_scores_high() {
        let mut scorer = AnomalyScorer::new();
        // 有轻微噪声的稳定基线：MAD 非零，异常才有相对尺度。
        for i in 0..64 {
            scorer.update(5.0 + (i % 7) as f64 * 0.1).unwrap();
        }
        scorer.update(50.0).unwrap();
        assert!(
            scorer.score() > 0.5,
            "spike should score high: {}",
            scorer.score()
        );
        assert!(scorer.score() <= 1.0);
    }

    #[test]
    fn non_finite_value_is_rejected() {
        let mut scorer = AnomalyScorer::new();
        assert!(scorer.update(5.0).is_ok());
        assert!(scorer.update(f64::NAN).is_err());
        assert!(scorer.update(f64::INFINITY).is_err());
    }

    #[test]
    fn warmup_keeps_score_zero() {
        let mut scorer = AnomalyScorer::new();
        scorer.update(5.0).unwrap();
        scorer.update(50.0).unwrap();
        assert_eq!(scorer.score(), 0.0);
    }
}
