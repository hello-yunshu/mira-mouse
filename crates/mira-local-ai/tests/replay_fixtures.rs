// SPDX-License-Identifier: AGPL-3.0-or-later
//! 回放兼容性测试（阶段 6）。
//!
//! 读取提交的 `tests/fixtures/replay/*.json`，在 plain / weighted / robust 三种配置下运行
//! `predict`，验证：
//! - 所有场景与配置都不 panic、不返回错误；
//! - 所有指标均为有限值（无 NaN / inf）；
//! - 输出 source 只可能是 `LocalAi` 或 `BaselineRecommended`；
//! - 确定性：同一 fixture 在同一配置下重复运行结果逐位一致。
//!
//! 这些 fixture 是提交的权威数据，`cargo run -p mira-local-ai --bin replay_report -- gen`
//! 只在需要重建时运行；本测试始终读取已提交文件，保证 CI 可复现。

use std::path::{Path, PathBuf};

use mira_local_ai::predict;
use mira_protocol::{
    BatteryModelConfig, BatteryPredictionInput, BatteryPredictionOutput, BatterySampleInput,
    DeviceContextSnapshot, PredictionSource,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplayFixture {
    name: String,
    description: String,
    events: Vec<String>,
    now_unix_ms: i64,
    now_timezone_offset_minutes: i32,
    expected_behavior: String,
    samples: Vec<FixtureSample>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureSample {
    at_unix_ms: i64,
    percentage: u8,
    charging: bool,
    #[serde(default)]
    dpi: Option<u16>,
    #[serde(default)]
    polling_rate_hz: Option<u16>,
    #[serde(default)]
    light_mode: Option<String>,
    #[serde(default)]
    light_brightness: Option<u8>,
}

impl FixtureSample {
    fn to_input(&self) -> BatterySampleInput {
        BatterySampleInput {
            at_unix_ms: self.at_unix_ms,
            timezone_offset_minutes: 0,
            percentage: self.percentage,
            charging: self.charging,
            context: self.context(),
        }
    }

    fn context(&self) -> Option<DeviceContextSnapshot> {
        if self.dpi.is_none()
            && self.polling_rate_hz.is_none()
            && self.light_mode.is_none()
            && self.light_brightness.is_none()
        {
            return None;
        }
        Some(DeviceContextSnapshot {
            dpi: self.dpi,
            polling_rate_hz: self.polling_rate_hz,
            light_mode: self.light_mode.clone(),
            light_brightness: self.light_brightness,
            profile: None,
        })
    }
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/replay")
}

fn load_fixtures() -> Vec<ReplayFixture> {
    let dir = fixtures_dir();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("replay fixtures dir must exist (run `gen` first)")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    assert!(
        names.len() >= 10,
        "expected the 10 committed replay fixtures, got {names:?}"
    );
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(&name)).expect("read fixture");
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
        })
        .collect()
}

fn run(config: &BatteryModelConfig, fixture: &ReplayFixture) -> BatteryPredictionOutput {
    let input = BatteryPredictionInput {
        now_unix_ms: fixture.now_unix_ms,
        now_timezone_offset_minutes: fixture.now_timezone_offset_minutes,
        samples: fixture
            .samples
            .iter()
            .map(FixtureSample::to_input)
            .collect(),
        current_context: None,
    };
    predict(&input, config).expect("predict must not error on valid committed fixture")
}

fn weighted_config() -> BatteryModelConfig {
    BatteryModelConfig {
        weighted_learning_enabled: true,
        learning_recency_tau_hours: Some(48.0),
        ..BatteryModelConfig::default()
    }
}

fn robust_config() -> BatteryModelConfig {
    BatteryModelConfig {
        robust_detection_enabled: true,
        ..BatteryModelConfig::default()
    }
}

fn assert_finite_metrics(output: &BatteryPredictionOutput, label: &str) {
    for (name, value) in [
        ("baseline_mae", output.baseline_mae),
        ("candidate_mae", output.candidate_mae),
        ("weighted_mae", output.weighted_mae),
        ("recent_mae", output.recent_mae),
        ("effective_sample_weight", output.effective_sample_weight),
    ] {
        assert!(
            value.is_none_or(f64::is_finite),
            "{label}: {name} must be finite or None, got {value:?}"
        );
    }
    assert!(
        matches!(
            output.source,
            PredictionSource::LocalAi | PredictionSource::BaselineRecommended
        ),
        "{label}: unexpected source {:?}",
        output.source
    );
}

#[test]
fn every_committed_fixture_runs_under_all_configs_without_panic_or_nan() {
    let fixtures = load_fixtures();
    let configs: [(&str, BatteryModelConfig); 3] = [
        ("plain", BatteryModelConfig::default()),
        ("weighted", weighted_config()),
        ("robust", robust_config()),
    ];
    for fixture in &fixtures {
        for (label, config) in &configs {
            let run_label = format!("{} / {label}", fixture.name);
            let output = run(config, fixture);
            assert_finite_metrics(&output, &run_label);
        }
    }
}

#[test]
fn replay_is_deterministic_across_repeated_runs() {
    let fixtures = load_fixtures();
    for fixture in &fixtures {
        let a = run(&BatteryModelConfig::default(), fixture);
        let b = run(&BatteryModelConfig::default(), fixture);
        assert_eq!(a.remaining_hours, b.remaining_hours, "{}", fixture.name);
        assert_eq!(a.source, b.source, "{}", fixture.name);
        assert_eq!(a.reason, b.reason, "{}", fixture.name);
    }
}

#[test]
fn weighted_mode_never_emits_non_finite_weighted_metrics() {
    let fixtures = load_fixtures();
    for fixture in &fixtures {
        let output = run(&weighted_config(), fixture);
        assert_finite_metrics(&output, &format!("weighted / {}", fixture.name));
    }
}
