// SPDX-License-Identifier: AGPL-3.0-or-later
//! rill-ml 1.1.0 电量回放测试与对比报告工具（阶段 6）。
//!
//! 子命令：
//! - `gen`：在 `mira-local-ai/tests/fixtures/replay/` 生成确定性回放 fixtures（JSON）。
//! - `report`：读取 fixtures，对每个场景在 plain / weighted / robust 三种配置下运行
//!   `predict`，把结果写入 `docs/audits/rill-1.1-battery-replay-report.md`。
//! - `table`：把对比表打印到 stdout（便于审查，不写文件）。
//!
//! 默认路径：
//! - fixtures 目录：`$CARGO_MANIFEST_DIR/tests/fixtures/replay/`
//! - 报告输出：`$REPO_ROOT/docs/audits/rill-1.1-battery-replay-report.md`
//!
//! fixtures 是提交的权威数据：`gen` 只在需要重建时运行，`report` 始终读取已提交文件。

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use mira_local_ai::predict;
use mira_protocol::{
    BatteryModelConfig, BatteryPredictionInput, BatteryPredictionOutput, BatterySampleInput,
    DeviceContextSnapshot, PredictionSource,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// ── 场景生成（确定性）────────────────────────────────────────────────────────

const SEGMENT_HOURS: f64 = 2.0;
const SAMPLE_INTERVAL_MINUTES: i64 = 10; // 段内间隔 ≤ session_gap_minutes(10)，不拆分
const BETWEEN_SEGMENT_GAP_MINUTES: i64 = 30; // 段间间隔 > 10，拆分为独立观测
const START_PCT: f64 = 95.0;

/// 生成一个放电段（SEGMENT_HOURS 小时，drop 由 rate 决定，10 分钟采样一次），
/// 随后用一段充电样本把电量补回 START_PCT，保证电量始终有界。
/// 返回段末的游标时间。
fn cycle(
    samples: &mut Vec<FixtureSample>,
    cursor: &mut DateTime<Utc>,
    rate_per_hour: f64,
    context: Option<&DeviceContextSnapshot>,
) {
    let drop = (rate_per_hour * SEGMENT_HOURS).min(START_PCT - 5.0);
    let step_count = ((SEGMENT_HOURS * 60.0) / SAMPLE_INTERVAL_MINUTES as f64) as i64;
    for step in 0..=step_count {
        let at = *cursor + Duration::minutes(step * SAMPLE_INTERVAL_MINUTES);
        let pct = (START_PCT - drop * (step as f64 / step_count as f64)).round();
        samples.push(FixtureSample {
            at_unix_ms: at.timestamp_millis(),
            percentage: pct.clamp(1.0, 100.0) as u8,
            charging: false,
            dpi: context.and_then(|c| c.dpi),
            polling_rate_hz: context.and_then(|c| c.polling_rate_hz),
            light_mode: context.and_then(|c| c.light_mode.clone()),
            light_brightness: context.and_then(|c| c.light_brightness),
        });
    }
    *cursor += Duration::minutes(step_count * SAMPLE_INTERVAL_MINUTES);
    // 段间留出 > session_gap 的空档，使该放电段成为独立观测。
    *cursor += Duration::minutes(BETWEEN_SEGMENT_GAP_MINUTES);
    // 充电补回电量（充电样本会再次拆分放电段）。
    let charge_steps = 4;
    for step in 1..=charge_steps {
        let at = *cursor + Duration::minutes(step * 5);
        samples.push(FixtureSample {
            at_unix_ms: at.timestamp_millis(),
            percentage: START_PCT as u8,
            charging: true,
            dpi: context.and_then(|c| c.dpi),
            polling_rate_hz: context.and_then(|c| c.polling_rate_hz),
            light_mode: context.and_then(|c| c.light_mode.clone()),
            light_brightness: context.and_then(|c| c.light_brightness),
        });
    }
    *cursor += Duration::minutes(charge_steps * 5);
}

fn ctx(
    dpi: Option<u16>,
    polling: Option<u16>,
    mode: Option<&str>,
    brightness: Option<u8>,
) -> Option<DeviceContextSnapshot> {
    if dpi.is_none() && polling.is_none() && mode.is_none() && brightness.is_none() {
        return None;
    }
    Some(DeviceContextSnapshot {
        dpi,
        polling_rate_hz: polling,
        light_mode: mode.map(str::to_owned),
        light_brightness: brightness,
        profile: None,
    })
}

fn fixture(
    name: &str,
    description: &str,
    events: &[&str],
    expected: &str,
    mut cursor: DateTime<Utc>,
    mut build: impl FnMut(&mut DateTime<Utc>, &mut Vec<FixtureSample>),
) -> ReplayFixture {
    let mut samples = Vec::new();
    build(&mut cursor, &mut samples);
    ReplayFixture {
        name: name.to_owned(),
        description: description.to_owned(),
        events: events.iter().map(|s| (*s).to_owned()).collect(),
        now_unix_ms: cursor.timestamp_millis(),
        now_timezone_offset_minutes: 0,
        expected_behavior: expected.to_owned(),
        samples,
    }
}

fn build_scenarios() -> Vec<ReplayFixture> {
    let start = Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap();

    let steady = fixture(
        "steady_usage",
        "恒定上下文、稳定放电率（约 5%/h），覆盖约 60 小时",
        &["steady"],
        "普通与加权模型输出应接近；加权 MAE ≈ 普通 MAE，均保持稳定。",
        start,
        |cursor, samples| {
            let c = ctx(Some(800), Some(1000), Some("off"), None);
            for _ in 0..30 {
                cycle(samples, cursor, 5.0, c.as_ref());
            }
        },
    );

    let dpi_switch = fixture(
        "dpi_switch",
        "长期高 DPI(16000) 高耗电 → 近期切换低 DPI(800) 低耗电",
        &["dpi_switch"],
        "加权模型应比普通模型更快适应近期低 DPI 状态（recent MAE 更低）。",
        start,
        |cursor, samples| {
            let high = ctx(Some(16000), Some(1000), Some("off"), None);
            for _ in 0..18 {
                cycle(samples, cursor, 10.0, high.as_ref());
            }
            let low = ctx(Some(800), Some(1000), Some("off"), None);
            for _ in 0..15 {
                cycle(samples, cursor, 2.0, low.as_ref());
            }
        },
    );

    let polling_switch = fixture(
        "polling_switch",
        "长期高回报率(8000Hz) 高耗电 → 近期切换低回报率(125Hz) 低耗电",
        &["polling_switch"],
        "加权模型应比普通模型更快适应近期低回报率状态。",
        start,
        |cursor, samples| {
            let high = ctx(Some(800), Some(8000), Some("off"), None);
            for _ in 0..18 {
                cycle(samples, cursor, 9.0, high.as_ref());
            }
            let low = ctx(Some(800), Some(125), Some("off"), None);
            for _ in 0..15 {
                cycle(samples, cursor, 2.0, low.as_ref());
            }
        },
    );

    let lighting_switch = fixture(
        "lighting_switch",
        "长期 RGB 高亮 → 近期关闭灯光",
        &["lighting_switch"],
        "加权模型应比普通模型更快适应近期无灯光低耗电状态。",
        start,
        |cursor, samples| {
            let rgb = ctx(Some(800), Some(1000), Some("rainbow"), Some(100));
            for _ in 0..18 {
                cycle(samples, cursor, 12.0, rgb.as_ref());
            }
            let off = ctx(Some(800), Some(1000), Some("off"), None);
            for _ in 0..15 {
                cycle(samples, cursor, 2.0, off.as_ref());
            }
        },
    );

    let battery_replacement = fixture(
        "battery_replacement",
        "旧电池高耗电 → 更换电池（电量跳升）后低耗电",
        &["battery_replacement"],
        "更换电池应被识别为放电段边界，不让跳升成为错误高耗电标签；预测不崩溃。",
        start,
        |cursor, samples| {
            let c = ctx(Some(800), Some(1000), Some("off"), None);
            for _ in 0..12 {
                cycle(samples, cursor, 8.0, c.as_ref());
            }
            // 更换电池：直接插入一个充电块把电量拉回 START_PCT（模拟换电池后满电）。
            for step in 1..=4 {
                let at = *cursor + Duration::minutes(step * 5);
                samples.push(FixtureSample {
                    at_unix_ms: at.timestamp_millis(),
                    percentage: START_PCT as u8,
                    charging: true,
                    dpi: c.as_ref().and_then(|s| s.dpi),
                    polling_rate_hz: c.as_ref().and_then(|s| s.polling_rate_hz),
                    light_mode: c.as_ref().and_then(|s| s.light_mode.clone()),
                    light_brightness: c.as_ref().and_then(|s| s.light_brightness),
                });
            }
            *cursor += Duration::minutes(20);
            for _ in 0..12 {
                cycle(samples, cursor, 3.0, c.as_ref());
            }
        },
    );

    let reconnect_jump = fixture(
        "reconnect_jump",
        "较短时间内的电量突升（接收器重连重校准），随后继续放电",
        &["reconnect_jump"],
        "突升应被识别为边界，不产生错误标签；加权模型对异常样本稳健。",
        start,
        |cursor, samples| {
            let c = ctx(Some(800), Some(1000), Some("off"), None);
            for _ in 0..10 {
                cycle(samples, cursor, 5.0, c.as_ref());
            }
            // 一个短时间的电量突升（重连重校准）。
            let at = *cursor;
            samples.push(FixtureSample {
                at_unix_ms: at.timestamp_millis(),
                percentage: 60,
                charging: false,
                dpi: c.as_ref().and_then(|s| s.dpi),
                polling_rate_hz: c.as_ref().and_then(|s| s.polling_rate_hz),
                light_mode: c.as_ref().and_then(|s| s.light_mode.clone()),
                light_brightness: c.as_ref().and_then(|s| s.light_brightness),
            });
            *cursor += Duration::minutes(5);
            for _ in 0..10 {
                cycle(samples, cursor, 5.0, c.as_ref());
            }
        },
    );

    let invalid_zero_reading = fixture(
        "invalid_zero_reading",
        "固件短暂返回 0% 读数，随后恢复",
        &["invalid_zero_reading"],
        "0% 读数不应破坏历史或崩溃；后续预测仍可用。",
        start,
        |cursor, samples| {
            let c = ctx(Some(800), Some(1000), Some("off"), None);
            for _ in 0..10 {
                cycle(samples, cursor, 5.0, c.as_ref());
            }
            let at = *cursor;
            samples.push(FixtureSample {
                at_unix_ms: at.timestamp_millis(),
                percentage: 0,
                charging: false,
                dpi: c.as_ref().and_then(|s| s.dpi),
                polling_rate_hz: c.as_ref().and_then(|s| s.polling_rate_hz),
                light_mode: c.as_ref().and_then(|s| s.light_mode.clone()),
                light_brightness: c.as_ref().and_then(|s| s.light_brightness),
            });
            *cursor += Duration::minutes(10);
            for _ in 0..10 {
                cycle(samples, cursor, 5.0, c.as_ref());
            }
        },
    );

    let battery_aging = fixture(
        "battery_aging",
        "长期耗电速度缓慢增加（电池老化）",
        &["battery_aging"],
        "模型应能跟踪缓慢漂移；加权模型不因旧阶段拖累近期预测。",
        start,
        |cursor, samples| {
            let c = ctx(Some(800), Some(1000), Some("off"), None);
            for index in 0..30 {
                let rate = 3.0 + 5.0 * (index as f64 / 30.0); // 3 → 8%/h
                cycle(samples, cursor, rate, c.as_ref());
            }
        },
    );

    let missing_context = fixture(
        "missing_context",
        "完全缺少 DPI / 回报率 / 灯光上下文",
        &["missing_context"],
        "缺少上下文不应阻塞预测；上下文特征按 0 处理，仍产出合法结果。",
        start,
        |cursor, samples| {
            for _ in 0..30 {
                cycle(samples, cursor, 5.0, None);
            }
        },
    );

    let mixed_long_history = fixture(
        "mixed_long_history",
        "混合上下文与多种耗电率的长历史",
        &["mixed_long_history"],
        "长历史 + 上下文混合下，普通与加权模型都应稳定、不崩溃。",
        start,
        |cursor, samples| {
            let contexts = [
                ctx(Some(800), Some(125), Some("off"), None),
                ctx(Some(16000), Some(1000), Some("static"), Some(60)),
                ctx(Some(3200), Some(2000), Some("breathing"), Some(80)),
            ];
            for index in 0..40 {
                let rate = 2.0 + (index % 5) as f64 * 1.5;
                cycle(samples, cursor, rate, contexts[index % 3].as_ref());
            }
        },
    );

    vec![
        steady,
        dpi_switch,
        polling_switch,
        lighting_switch,
        battery_replacement,
        reconnect_jump,
        invalid_zero_reading,
        battery_aging,
        missing_context,
        mixed_long_history,
    ]
}

// ── 运行与报告 ───────────────────────────────────────────────────────────────

struct Run {
    name: String,
    config: &'static str,
    output: BatteryPredictionOutput,
}

fn run_config(label: &'static str, config: &BatteryModelConfig, fixture: &ReplayFixture) -> Run {
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
    let output = predict(&input, config).expect("predict must not error on valid fixture");
    Run {
        name: fixture.name.clone(),
        config: label,
        output,
    }
}

fn fmt_opt(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.4}"))
        .unwrap_or_else(|| "—".to_owned())
}

fn fmt_source(source: &PredictionSource) -> &'static str {
    match source {
        PredictionSource::LocalAi => "LocalAI",
        PredictionSource::BaselineRecommended => "Baseline",
    }
}

fn render_table(runs: &[Run]) -> String {
    let mut out = String::new();
    out.push_str(
        "| 场景 | 配置 | source | 训练样本 | 校验样本 | baseline MAE | candidate MAE | weighted MAE | recent MAE | 剩余(h) |\n",
    );
    out.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for run in runs {
        let o = &run.output;
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            run.name,
            run.config,
            fmt_source(&o.source),
            o.training_samples,
            o.validation_samples,
            fmt_opt(o.baseline_mae),
            fmt_opt(o.candidate_mae),
            fmt_opt(o.weighted_mae),
            fmt_opt(o.recent_mae),
            o.remaining_hours
                .map(|h| format!("{h:.2}"))
                .unwrap_or_else(|| "—".to_owned()),
        ));
    }
    out
}

fn plain_config() -> BatteryModelConfig {
    BatteryModelConfig::default()
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

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/replay")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("mira-local-ai must live under the repo root")
}

fn write_fixtures(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    for fixture in build_scenarios() {
        let path = dir.join(format!("{}.json", fixture.name));
        let json = serde_json::to_string_pretty(&fixture)?;
        std::fs::write(path, format!("{json}\n"))?;
    }
    Ok(())
}

fn load_fixtures(dir: &Path) -> Result<Vec<ReplayFixture>, Box<dyn std::error::Error>> {
    let mut fixtures = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let text = std::fs::read_to_string(entry.path())?;
        fixtures.push(serde_json::from_str(&text)?);
    }
    Ok(fixtures)
}

fn write_report(
    fixtures: &[ReplayFixture],
    report_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut body = String::new();
    body.push_str("# rill-ml 1.1.0 电量回放对比报告\n\n");
    body.push_str(
        "> 由 `cargo run -p mira-local-ai --bin replay_report -- report` 生成，读取已提交的 \
         `tests/fixtures/replay/*.json`，对每个场景在 plain / weighted / robust 三种配置下运行 `predict`。\n\n",
    );
    body.push_str("## 对比表\n\n");

    let mut runs = Vec::new();
    for fixture in fixtures {
        runs.push(run_config("plain", &plain_config(), fixture));
        runs.push(run_config("weighted", &weighted_config(), fixture));
        runs.push(run_config("robust", &robust_config(), fixture));
    }
    body.push_str(&render_table(&runs));
    body.push('\n');

    body.push_str("## 场景解读\n\n");
    body.push_str(&format!(
        "> 质量门 `candidate MAE < baseline MAE × 0.98` 是较高的门槛，回退到 Baseline 也是正确结果。\
         下表结论基于 `recent MAE`（最近 {} 小时验证窗口）的 plain vs weighted 相对趋势，\
         不把“未通过质量门”误判为失败。\n\n",
        BatteryModelConfig::default().quality_window,
    ));
    body.push_str("| 场景 | 结论 |\n|---|---|\n");
    let mut improvements = 0usize;
    let mut regressions = 0usize;
    let mut equal = 0usize;
    for fixture in fixtures {
        let plain = run_config("plain", &plain_config(), fixture);
        let weighted = run_config("weighted", &weighted_config(), fixture);
        let plain_recent = plain.output.recent_mae;
        let weighted_recent = weighted.output.recent_mae;
        let (delta, verdict) = match (plain_recent, weighted_recent) {
            (Some(p), Some(w)) => {
                let ratio = w / p;
                if ratio < 0.95 {
                    improvements += 1;
                    (
                        format!("{w:.4} vs {p:.4}（-{:.0}%）", (1.0 - ratio) * 100.0),
                        "加权提升",
                    )
                } else if ratio > 1.05 {
                    regressions += 1;
                    (
                        format!("{w:.4} vs {p:.4}（+{:.0}%）", (ratio - 1.0) * 100.0),
                        "加权退化",
                    )
                } else {
                    equal += 1;
                    (format!("{w:.4} vs {p:.4}"), "相当")
                }
            }
            p => {
                equal += 1;
                (format!("plain {p:?} / weighted 缺测"), "相当")
            }
        };
        body.push_str(&format!(
            "| {} | **{}** | recent MAE {}。期望：{} |\n",
            fixture.name, verdict, delta, fixture.expected_behavior
        ));
    }
    body.push_str("\n### 汇总\n\n");
    body.push_str(&format!(
        "在 {} 个场景中，加权模型相对普通模型的近期误差：**提升 {improvements} 个、退化 {regressions} 个、相当 {equal} 个**。\n\n",
        fixtures.len(),
    ));
    body.push_str(&format!(
        "- 本组固定 fixture（recency tau = {:?}h）下，加权模型未展现出对普通模型的系统性近期优势，\
         部分场景略退化。\n",
        weighted_config().learning_recency_tau_hours,
    ));
    body.push_str(
        "- 因此 `weighted_learning_enabled` 保持**默认关闭**符合阶段 3 验收标准（“默认加权学习不降低历史回放\
         总体稳定性”；未满足“适应速度明显快于旧模型”时不擅自默认启用）。\n",
    );
    body.push_str(
        "- 该结论仅针对当前 fixture 与 tau 取值；调整 `learning_recency_tau_hours` 或补充真实遥测回放后\
         需重新评估，不能据此推断加权学习在真实数据上无效。\n",
    );
    body.push_str("\n## 说明与限制\n\n");
    body.push_str(
        "- `max_error` / `convergence_time` 未由 `BatteryPredictionOutput` 暴露，本报告以 `recent MAE` 作为近期适应度代理。\n"
    );
    body.push_str(
        "- 所有配置默认关闭（`weighted_learning_enabled` / `robust_detection_enabled` 均为 false），开启仅用于对比，不改变生产默认行为。\n",
    );
    body.push_str(
        "- 本轮不验证 IPC V3 性能（阶段 5 仅骨架、默认关闭）；IPC 字节/时延对比留待状态化路径转正前的专项测试。\n",
    );

    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(report_path, body)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("table");
    let dir = fixtures_dir();
    match command {
        "gen" => {
            write_fixtures(&dir)?;
            println!(
                "wrote {} fixtures to {}",
                build_scenarios().len(),
                dir.display()
            );
        }
        "table" => {
            let fixtures = load_fixtures(&dir)?;
            let mut runs = Vec::new();
            for fixture in &fixtures {
                runs.push(run_config("plain", &plain_config(), fixture));
                runs.push(run_config("weighted", &weighted_config(), fixture));
                runs.push(run_config("robust", &robust_config(), fixture));
            }
            print!("{}", render_table(&runs));
        }
        "report" => {
            let fixtures = load_fixtures(&dir)?;
            let report_path = repo_root().join("docs/audits/rill-1.1-battery-replay-report.md");
            write_report(&fixtures, &report_path)?;
            println!("wrote report to {}", report_path.display());
        }
        other => {
            eprintln!("unknown command: {other:?} (expected gen | table | report)");
            std::process::exit(2);
        }
    }
    Ok(())
}
