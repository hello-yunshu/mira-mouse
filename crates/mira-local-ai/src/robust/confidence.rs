// SPDX-License-Identifier: AGPL-3.0-or-later
//! 异常/漂移 → 置信度倍率映射。
//!
//! 倍率范围 [0.5, 1.0]：无信号记 1.0；异常评分或漂移级别越强，倍率越低。
//! 该倍率仅供实验消费（如 UI 展示或未来回放评估），当前不改变预测数值。

use rill_ml::drift::DriftLevel;

/// 置信度倍率下限。
const CONFIDENCE_FLOOR: f64 = 0.5;
/// 异常评分每单位对倍率的惩罚。
const ANOMALY_PENALTY: f64 = 0.2;
/// 漂移确认对倍率的惩罚。
const DRIFT_PENALTY: f64 = 0.25;
/// 漂移警告对倍率的惩罚。
const WARNING_PENALTY: f64 = 0.1;

/// 由异常评分（[0, 1]）与漂移级别推导置信度倍率。
pub fn confidence_multiplier(anomaly_score: f64, level: DriftLevel) -> f64 {
    let mut multiplier = 1.0 - anomaly_score * ANOMALY_PENALTY;
    match level {
        rill_ml::drift::DriftLevel::None => {}
        rill_ml::drift::DriftLevel::Warning => multiplier -= WARNING_PENALTY,
        rill_ml::drift::DriftLevel::Drift => multiplier -= DRIFT_PENALTY,
        _ => {}
    }
    multiplier.clamp(CONFIDENCE_FLOOR, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_signals_keeps_full_confidence() {
        assert_eq!(confidence_multiplier(0.0, DriftLevel::None), 1.0);
    }

    #[test]
    fn anomaly_and_drift_reduce_confidence() {
        let none = confidence_multiplier(0.0, DriftLevel::None);
        let drift = confidence_multiplier(0.0, DriftLevel::Drift);
        assert!(drift < none);
        assert!(confidence_multiplier(0.8, DriftLevel::Drift) < drift);
    }

    #[test]
    fn multiplier_stays_in_bounds() {
        for score in [0.0, 0.5, 1.0, 2.0] {
            for level in [DriftLevel::None, DriftLevel::Warning, DriftLevel::Drift] {
                let value = confidence_multiplier(score, level);
                assert!((0.5..=1.0).contains(&value), "value={value} score={score}");
            }
        }
    }
}
