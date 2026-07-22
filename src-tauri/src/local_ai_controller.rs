// SPDX-License-Identifier: AGPL-3.0-or-later
//! 常驻式本地 AI Runtime 控制器。
//!
//! 总开关 `local_ai_analysis_enabled` 翻转到 on 时调用 `start()` 启动当前平台的 rill-runtime
//! 子进程并完成握手;翻转到 off 时调用 `stop()` 优雅退出。`predict()` 复用已建立的
//! stdin/stdout 通道,避免每次预测的进程启动开销。IO/解析错误或子进程意外退出
//! 标记 `Failed` 并在冷却窗口外重启；Wasmtime timeout/trap 会使 component instance
//! 不可复用，因此立即丢弃且不进入 fatal 冷却，下次预测启动干净实例。
//!
//! 失败、超时或未安装时,predict 返回空 map,调用方回退确定性算法。

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
        Mutex, PoisonError,
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Local, Utc};
use mira_protocol::{
    BatteryPredictionInput, BatteryPredictionOutput, BatterySampleInput, PredictionSource,
    BATTERY_USAGE_CAPABILITY,
};
use rill_runtime_protocol::{
    RuntimeRequest, RuntimeResponseV2 as RuntimeResponse, MAX_MESSAGE_BYTES, RUNTIME_API_VERSION,
};
use tauri::AppHandle;

use crate::{
    battery_history::BatterySample,
    local_ai_runtime,
    local_ai_runtime::RuntimeInstallation,
    logging::model::{FieldValue, Fields, LogInput, LogLevel, LogSource},
    logging::LogService,
};

/// 单次请求/响应超时。与原 `predict_batteries` 的 RUNTIME_TIMEOUT 保持一致。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// 失败后冷却窗口:窗口内不重试,直接回退。避免高频预测引起频繁重启。
const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);
/// stop() 优雅退出的等待时间;超时后强制 kill。
const STOP_GRACE: Duration = Duration::from_millis(500);
/// stderr 缓冲上限,超限丢弃旧数据。仅用于错误诊断。
const MAX_STDERR_BYTES: usize = 64 * 1024;
/// stderr 单行截断上限,避免超长行冲击日志缓冲。
const MAX_STDERR_LINE: usize = 4096;
/// stderr 日志限流:每秒最多记录这么多行,超出后折叠为摘要。
const STDERR_RATE_LIMIT: u32 = 50;
/// Keep mature 30-day histories below the 1 MiB IPC envelope while retaining
/// enough recent samples for the quality gate. At the normal five-minute
/// cadence this is roughly two weeks per battery component.
const MAX_PREDICTION_SAMPLES: usize = 4_096;
/// The handler's relative quality gate is necessary but not sufficient when
/// battery percentages are quantized. Reject a model whose absolute drain-rate
/// error is still too large to support a user-facing remaining-time estimate.
const MAX_ACCEPTABLE_CANDIDATE_MAE_PER_HOUR: f64 = 2.0;
const MIN_HOST_TRAINING_SAMPLES: u64 = 12;
const MIN_HOST_VALIDATION_SAMPLES: u64 = 8;
const MAX_CANDIDATE_TO_BASELINE_MAE_RATIO: f64 = 0.95;

/// 控制器向上层 supervisor 投递的崩溃/成功事件。
///
/// supervisor 维护 10 分钟滑动窗口,累积 ≥3 次 Failed 时触发自动回滚;
/// 收到 Success 即清零窗口。所有事件发送都是 best-effort,supervisor 已
/// 退出时静默丢弃,不影响 controller 主路径。
#[derive(Debug, Clone)]
pub enum CrashEvent {
    /// spawn/握手/predict 路径上任意失败。`reason` 预留给后续诊断日志,当前不读取。
    Failed {
        at: Instant,
        #[allow(dead_code)]
        reason: String,
    },
    /// predict() 完整跑完所有 batch 且至少返回 1 个结果。
    Success,
}

/// `parse_prediction` 返回的错误分类。区分三类以便 `predict()` 采取不同处置:
///
/// - [`PredictionError::Fatal`] — runtime 通道/进程级故障(runtime 可能已死),
///   走 `mark_failed`:kill 子进程 + 30s 冷却 + 投 `CrashEvent::Failed`。
/// - [`PredictionError::HandlerInterrupted`] — handler 调用因 fuel/epoch 超时或 trap
///   被 Wasmtime 中断。进程仍在，但 component instance 已不可再次进入；必须丢弃
///   当前 runtime，并让下次预测立即启动干净实例。timeout 不计入 bundle 回滚，
///   trap 计入。
/// - [`PredictionError::HandlerBug`] — handler bug(rill-runtime 返回
///   `retryable=false`,如 `handlerInvalidOutput`)。runtime 仍健康:
///   不 kill、不冷却,但投 `CrashEvent::Failed` 让 supervisor 在 3/10min 阈值时回滚。
#[derive(Debug)]
enum PredictionError {
    Fatal(String),
    HandlerInterrupted {
        reason: String,
        report_failure: bool,
    },
    HandlerBug(String),
}

#[derive(Debug, PartialEq)]
enum PredictionDecision {
    Estimate(f64),
    BaselineRecommended { reason: String },
}

/// 控制器状态。所有字段通过单一 Mutex 串行化访问,避免并发竞争。
struct ControllerInner {
    running: Option<RunningRuntime>,
    state: ControllerState,
    /// 由 lib.rs setup 阶段注入的 crash 事件 sender。None 时静默丢弃事件。
    crash_tx: Option<Sender<CrashEvent>>,
    /// 由 lib.rs setup 阶段注入的统一日志服务。None 时静默丢弃日志。
    log_service: Option<LogService>,
}

struct RunningRuntime {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<Result<Vec<u8>, String>>,
    /// 单调递增的 request_id 序列,避免与历史响应混淆。
    request_seq: AtomicU64,
}

#[derive(Clone, Debug)]
enum ControllerState {
    Stopped,
    Running,
    Failed { last_attempt: Instant },
}

#[derive(Default)]
pub struct LocalAiController {
    inner: Mutex<ControllerInner>,
}

impl Default for ControllerInner {
    fn default() -> Self {
        Self {
            running: None,
            state: ControllerState::Stopped,
            crash_tx: None,
            log_service: None,
        }
    }
}

fn local_ai_log_input(
    level: LogLevel,
    target: impl Into<String>,
    event: &'static str,
    message: impl Into<String>,
    mut fields: Fields,
) -> LogInput {
    fields.insert("event".to_string(), FieldValue::from(event));
    LogInput {
        level,
        source: LogSource::LocalAi,
        target: target.into(),
        message: message.into(),
        correlation_id: None,
        fields,
    }
}

impl LocalAiController {
    /// 在 lib.rs setup 阶段注入 crash 事件 sender,启动 supervisor 监听。
    pub fn set_crash_sender(&self, tx: Sender<CrashEvent>) {
        let mut guard = self.lock();
        guard.crash_tx = Some(tx);
    }

    /// 在 lib.rs setup 阶段注入统一日志服务,让 controller 可写入 local-ai 来源日志。
    pub fn set_log_service(&self, svc: LogService) {
        let mut guard = self.lock();
        guard.log_service = Some(svc);
    }

    /// 通过统一日志服务写入一条 local-ai 来源日志,复用已有 guard(避免重锁)。
    fn log_with_guard(
        guard: &std::sync::MutexGuard<'_, ControllerInner>,
        level: LogLevel,
        target: &str,
        event: &'static str,
        message: impl Into<String>,
        fields: Fields,
    ) {
        if let Some(svc) = guard.log_service.as_ref() {
            svc.write(local_ai_log_input(level, target, event, message, fields));
        }
    }

    /// Best-effort 投递事件:supervisor 已退出或未注入时静默丢弃。
    /// 不阻塞、不返回错误——crash 监控是 predict 路径的旁路,不能影响主流程。
    fn emit_crash_event(&self, event: CrashEvent) {
        let guard = self.lock();
        if let Some(tx) = guard.crash_tx.as_ref() {
            let _ = tx.send(event);
        }
    }

    /// 开关 on 时调用。启动 sidecar 子进程并完成握手;失败时标记 Failed。
    pub fn start(&self, app: &AppHandle) -> Result<(), String> {
        let mut guard = self.lock();
        if matches!(guard.state, ControllerState::Running) && guard.running.is_some() {
            return Ok(());
        }
        Self::log_with_guard(
            &guard,
            LogLevel::Info,
            "local_ai::lifecycle",
            "local-ai-starting",
            "local AI starting",
            BTreeMap::from([
                ("enabled".to_string(), FieldValue::from(true)),
                ("stage".to_string(), FieldValue::from("starting")),
            ]),
        );
        let Some(installation) = local_ai_runtime::resolve_installation(app) else {
            guard.state = ControllerState::Failed {
                last_attempt: Instant::now(),
            };
            Self::log_with_guard(
                &guard,
                LogLevel::Error,
                "local_ai::lifecycle",
                "local-ai-unavailable",
                "runtime or model pack not available",
                BTreeMap::from([
                    ("fallback".to_string(), FieldValue::from(true)),
                    ("stage".to_string(), FieldValue::from("resolve-assets")),
                ]),
            );
            let crash_tx = guard.crash_tx.clone();
            drop(guard);
            send_crash_event(
                crash_tx,
                CrashEvent::Failed {
                    at: Instant::now(),
                    reason: "runtime or model pack not available".into(),
                },
            );
            return Err("local AI runtime or model pack not available".to_string());
        };
        let probe = match spawn_and_handshake(&installation, &mut guard) {
            Ok(probe) => probe,
            Err(error) => {
                guard.state = ControllerState::Failed {
                    last_attempt: Instant::now(),
                };
                Self::log_with_guard(
                    &guard,
                    LogLevel::Error,
                    "local_ai::lifecycle",
                    "local-ai-handshake-failed",
                    format!("handshake failed: {error}"),
                    BTreeMap::from([
                        ("reason".to_string(), FieldValue::from(error.clone())),
                        ("fallback".to_string(), FieldValue::from(true)),
                        ("stage".to_string(), FieldValue::from("handshake")),
                    ]),
                );
                let crash_tx = guard.crash_tx.clone();
                drop(guard);
                send_crash_event(
                    crash_tx,
                    CrashEvent::Failed {
                        at: Instant::now(),
                        reason: error.clone(),
                    },
                );
                return Err(error);
            }
        };
        guard.state = ControllerState::Running;
        Self::log_with_guard(
            &guard,
            LogLevel::Info,
            "local_ai::lifecycle",
            "local-ai-ready",
            "local AI runtime started, handshake ok",
            BTreeMap::from([
                (
                    "runtimeVersion".to_string(),
                    FieldValue::from(probe.runtime_version),
                ),
                (
                    "modelVersion".to_string(),
                    FieldValue::from(probe.model_pack_version),
                ),
                (
                    "handlerVersion".to_string(),
                    FieldValue::from(probe.handler_version),
                ),
                (
                    "handlerApiVersion".to_string(),
                    FieldValue::from(u64::from(probe.handler_api_version)),
                ),
            ]),
        );
        Ok(())
    }

    /// 开关 off 时调用。关闭 stdin 让子进程自然退出,超时后 kill。
    pub fn stop(&self) {
        let mut guard = self.lock();
        let Some(mut runtime) = guard.running.take() else {
            guard.state = ControllerState::Stopped;
            return;
        };
        Self::log_with_guard(
            &guard,
            LogLevel::Info,
            "local_ai::lifecycle",
            "local-ai-stopping",
            "local AI stopping",
            BTreeMap::from([("stage".to_string(), FieldValue::from("stopping"))]),
        );
        // drop stdin 触发 EOF,rill-runtime 的 BufReader::read_until 返回 0 退出主循环。
        drop(runtime.stdin);
        let status = runtime.child.wait_timeout(STOP_GRACE);
        if status.is_err() {
            let _ = runtime.child.kill();
            let _ = runtime.child.wait();
        }
        guard.state = ControllerState::Stopped;
    }

    /// 重启:先 stop 再 start。bundle 更新后调用以加载新版本。
    pub fn restart(&self, app: &AppHandle) -> Result<(), String> {
        self.stop();
        self.start(app)
    }

    /// 批量预测电量。保持与 `local_ai_runtime::predict_batteries` 相同的签名语义:
    /// 返回 key → remaining_hours;任何失败返回空 map(调用方回退确定性算法)。
    pub fn predict(
        &self,
        app: &AppHandle,
        batches: &[(String, Vec<&BatterySample>)],
        now: DateTime<Utc>,
    ) -> BTreeMap<String, f64> {
        if batches.is_empty() {
            return BTreeMap::new();
        }
        let prediction_started_at = Instant::now();
        // 1. 检查状态,必要时触发重启。
        {
            let guard = self.lock();
            match &guard.state {
                ControllerState::Stopped => return BTreeMap::new(),
                ControllerState::Failed { last_attempt, .. } => {
                    if last_attempt.elapsed() < FAILURE_COOLDOWN {
                        return BTreeMap::new();
                    }
                }
                ControllerState::Running => {}
            }
            // 冷却窗口外:清掉旧 runtime,标记 Starting(用 Stopped 占位,下面 start 会更新)。
            if guard.running.is_none() {
                drop(guard);
                if self.start(app).is_err() {
                    return BTreeMap::new();
                }
            }
        }

        // 2. 串行发送所有请求并读取响应。
        let mut results = BTreeMap::new();
        let mut handler_failure_reason: Option<String> = None;
        let mut fallback_count = 0usize;
        let mut guard = self.lock();
        let prediction_log_service = guard.log_service.clone();
        let Some(runtime) = guard.running.as_mut() else {
            return BTreeMap::new();
        };
        let seq_base = runtime.request_seq.fetch_add(1, Ordering::Relaxed);
        let mut requests = Vec::with_capacity(batches.len());
        for (index, (_, samples)) in batches.iter().enumerate() {
            let request_id = format!("mira-battery-predict-{}-{}", seq_base, index);
            // current_context 取该设备最近一次样本携带的上下文。
            // 样本上下文由 `record_samples` 从宿主 `DeviceSnapshot` 缓存投影而来，
            // 不触发额外 HID 读取；缓存更新后新样本自动携带新参数。
            let current_context = samples
                .iter()
                .max_by_key(|sample| sample.at)
                .and_then(|sample| sample.context.clone());
            let recent_start = samples.len().saturating_sub(MAX_PREDICTION_SAMPLES);
            let battery_input = BatteryPredictionInput {
                now_unix_ms: now.timestamp_millis(),
                now_timezone_offset_minutes: timezone_offset_minutes(now),
                samples: samples[recent_start..]
                    .iter()
                    .map(|sample| BatterySampleInput {
                        at_unix_ms: sample.at.timestamp_millis(),
                        timezone_offset_minutes: timezone_offset_minutes(sample.at),
                        percentage: sample.percentage,
                        charging: sample.charging,
                        context: sample.context.clone(),
                    })
                    .collect(),
                current_context,
            };
            let input = serde_json::to_value(&battery_input)
                .expect("battery prediction input is always serializable");
            requests.push((
                request_id.clone(),
                RuntimeRequest::Invoke {
                    request_id,
                    api_version: RUNTIME_API_VERSION,
                    capability: BATTERY_USAGE_CAPABILITY.into(),
                    input,
                },
            ));
        }
        for (_request_id, request) in &requests {
            let line = match serde_json::to_vec(request) {
                Ok(bytes) => bytes,
                Err(_) => {
                    mark_failed(&mut guard, "encode request");
                    return BTreeMap::new();
                }
            };
            if line.len() > MAX_MESSAGE_BYTES {
                // This is an input-size fallback, not a sidecar failure. Leave the
                // healthy process running and let the caller use its deterministic
                // estimate for this response.
                return BTreeMap::new();
            }
            if let Err(error) = runtime.stdin.write_all(&line).and_then(|_| {
                runtime
                    .stdin
                    .write_all(b"\n")
                    .and_then(|_| runtime.stdin.flush())
            }) {
                mark_failed(&mut guard, &format!("write request: {error}"));
                return BTreeMap::new();
            }
        }

        // 3. 逐条读取响应,带超时。
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        for (index, (request_id, _)) in requests.iter().enumerate() {
            if Instant::now() >= deadline {
                mark_failed(&mut guard, "response timeout");
                return BTreeMap::new();
            }
            // 由专用线程执行阻塞 read，主线程通过 recv_timeout 才能真正打断超时。
            let remaining = deadline.saturating_duration_since(Instant::now());
            let buf = match runtime.responses.recv_timeout(remaining) {
                Ok(Ok(buf)) => buf,
                Ok(Err(error)) => {
                    mark_failed(&mut guard, &format!("read response: {error}"));
                    return BTreeMap::new();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    mark_failed(&mut guard, "response timeout");
                    return BTreeMap::new();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    mark_failed(&mut guard, "runtime stdout closed");
                    return BTreeMap::new();
                }
            };
            let response: RuntimeResponse = match serde_json::from_slice(&buf) {
                Ok(value) => value,
                Err(error) => {
                    mark_failed(&mut guard, &format!("decode response: {error}"));
                    return BTreeMap::new();
                }
            };
            match parse_prediction(&response, request_id) {
                Ok(PredictionDecision::Estimate(remaining)) => {
                    if let Some((key, _)) = batches.get(index) {
                        results.insert(key.clone(), remaining);
                    }
                }
                Ok(PredictionDecision::BaselineRecommended { reason }) => {
                    fallback_count += 1;
                    if let Some(svc) = prediction_log_service.as_ref() {
                        svc.write(local_ai_log_input(
                            LogLevel::Debug,
                            "local_ai::predict",
                            "local-ai-baseline-selected",
                            format!("deterministic baseline selected: {reason}"),
                            BTreeMap::from([
                                ("reason".to_string(), FieldValue::from(reason)),
                                ("fallback".to_string(), FieldValue::from(true)),
                            ]),
                        ));
                    }
                }
                Err(PredictionError::HandlerInterrupted {
                    reason,
                    report_failure,
                }) => {
                    // Wasmtime 的 fuel/epoch/trap 中断会让 component instance 无法再次
                    // enter。进程本身即使还活着也不能复用，因此立即丢弃；不进入 30s
                    // 冷却，让下一次预测能启动干净实例。
                    Self::log_with_guard(
                        &guard,
                        LogLevel::Warn,
                        "local_ai::predict",
                        "local-ai-handler-interrupted",
                        format!("handler interrupted, runtime will restart: {reason}"),
                        BTreeMap::from([
                            ("reason".to_string(), FieldValue::from(reason.clone())),
                            ("restart".to_string(), FieldValue::from(true)),
                            ("fallback".to_string(), FieldValue::from(true)),
                        ]),
                    );
                    discard_interrupted_runtime(&mut guard);
                    let crash_tx = report_failure.then(|| guard.crash_tx.clone()).flatten();
                    drop(guard);
                    if let Some(tx) = crash_tx {
                        let _ = tx.send(CrashEvent::Failed {
                            at: Instant::now(),
                            reason,
                        });
                    }
                    return BTreeMap::new();
                }
                Err(PredictionError::HandlerBug(reason)) => {
                    // handler 真实 bug(runtime 仍健康,不需要 kill/冷却),
                    // 记录一次失败并继续排空本轮剩余响应，避免已写入 runtime 的响应
                    // 留在 channel 中污染下一次 request_id；其他设备仍可正常使用 AI 结果。
                    if let Some(svc) = prediction_log_service.as_ref() {
                        svc.write(local_ai_log_input(
                            LogLevel::Warn,
                            "local_ai::predict",
                            "local-ai-handler-failed",
                            format!("handler bug, falling back: {reason}"),
                            BTreeMap::from([
                                ("reason".to_string(), FieldValue::from(reason.clone())),
                                ("fallback".to_string(), FieldValue::from(true)),
                            ]),
                        ));
                    }
                    handler_failure_reason.get_or_insert(reason);
                }
                Err(PredictionError::Fatal(reason)) => {
                    mark_failed(&mut guard, &reason);
                    return BTreeMap::new();
                }
            }
        }
        // A normal baseline recommendation still proves that runtime + handler
        // completed the request. Clear the supervisor failure window even when
        // the safety gate intentionally declines every model estimate.
        // 必须先 drop guard 再 emit,否则 emit_crash_event 重新 lock 会死锁。
        let has_results = !results.is_empty();
        let batch_count = batches.len();
        let result_count = results.len();
        let duration_ms = prediction_started_at.elapsed().as_millis() as u64;
        if handler_failure_reason.is_some() {
            Self::log_with_guard(
                &guard,
                LogLevel::Warn,
                "local_ai::predict",
                "local-ai-prediction-partial",
                format!(
                    "prediction batch completed with handler errors: {result_count}/{batch_count} devices returned estimates"
                ),
                BTreeMap::from([
                    ("status".to_string(), FieldValue::from("partial")),
                    ("batchCount".to_string(), FieldValue::from(batch_count as u64)),
                    ("resultCount".to_string(), FieldValue::from(result_count as u64)),
                    ("fallbackCount".to_string(), FieldValue::from(fallback_count as u64)),
                    ("durationMs".to_string(), FieldValue::from(duration_ms)),
                ]),
            );
        } else if has_results {
            Self::log_with_guard(
                &guard,
                LogLevel::Info,
                "local_ai::predict",
                "local-ai-prediction-completed",
                format!(
                    "prediction batch ok: {result_count}/{batch_count} devices returned estimates; {fallback_count} used deterministic fallback"
                ),
                BTreeMap::from([
                    ("status".to_string(), FieldValue::from("ok")),
                    ("batchCount".to_string(), FieldValue::from(batch_count as u64)),
                    ("resultCount".to_string(), FieldValue::from(result_count as u64)),
                    ("fallbackCount".to_string(), FieldValue::from(fallback_count as u64)),
                    ("durationMs".to_string(), FieldValue::from(duration_ms)),
                ]),
            );
        } else if batch_count > 0 {
            Self::log_with_guard(
                &guard,
                LogLevel::Info,
                "local_ai::predict",
                "local-ai-prediction-completed",
                format!(
                    "prediction batch completed normally: {fallback_count}/{batch_count} devices used deterministic fallback"
                ),
                BTreeMap::from([
                    ("status".to_string(), FieldValue::from("fallback")),
                    ("batchCount".to_string(), FieldValue::from(batch_count as u64)),
                    ("resultCount".to_string(), FieldValue::from(0_u64)),
                    ("fallbackCount".to_string(), FieldValue::from(fallback_count as u64)),
                    ("durationMs".to_string(), FieldValue::from(duration_ms)),
                ]),
            );
        }
        drop(guard);
        if let Some(reason) = handler_failure_reason {
            self.emit_crash_event(CrashEvent::Failed {
                at: Instant::now(),
                reason,
            });
        } else {
            self.emit_crash_event(CrashEvent::Success);
        }
        results
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ControllerInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn send_crash_event(crash_tx: Option<Sender<CrashEvent>>, event: CrashEvent) {
    if let Some(tx) = crash_tx {
        let _ = tx.send(event);
    }
}

fn mark_failed(guard: &mut std::sync::MutexGuard<'_, ControllerInner>, reason: &str) {
    // 先记录失败原因到统一日志,便于诊断。log_service 未注入时静默丢弃。
    LocalAiController::log_with_guard(
        guard,
        LogLevel::Error,
        "local_ai::runtime",
        "local-ai-runtime-failed",
        format!("local AI marked failed: {reason}"),
        BTreeMap::from([
            ("reason".to_string(), FieldValue::from(reason)),
            ("fallback".to_string(), FieldValue::from(true)),
        ]),
    );
    if let Some(mut runtime) = guard.running.take() {
        let _ = runtime.child.kill();
        let _ = runtime.child.wait();
    }
    guard.state = ControllerState::Failed {
        last_attempt: Instant::now(),
    };
    // 投递 crash 事件给 supervisor。sender 缺失时静默丢弃(测试或未注入场景)。
    if let Some(tx) = guard.crash_tx.as_ref() {
        let _ = tx.send(CrashEvent::Failed {
            at: Instant::now(),
            reason: reason.to_string(),
        });
    }
}

/// Drop a Wasmtime instance after an interrupt/trap without entering the fatal
/// 30-second cooldown. Wasmtime component instances cannot be re-entered after
/// such a trap, even though the sidecar process may still answer IPC requests.
fn discard_interrupted_runtime(guard: &mut std::sync::MutexGuard<'_, ControllerInner>) {
    if let Some(mut runtime) = guard.running.take() {
        let _ = runtime.child.kill();
        let _ = runtime.child.wait();
    }
    // Running + no process is the controller's existing "start on next predict"
    // state. Keeping it distinct from Failed avoids the fatal-error cooldown.
    guard.state = ControllerState::Running;
}

fn spawn_and_handshake(
    installation: &RuntimeInstallation,
    guard: &mut std::sync::MutexGuard<'_, ControllerInner>,
) -> Result<local_ai_runtime::RuntimeProbe, String> {
    local_ai_runtime::ensure_safe_runtime_file(&installation.executable)?;
    local_ai_runtime::ensure_safe_runtime_file(&installation.model_pack)?;
    local_ai_runtime::ensure_safe_runtime_file(&installation.handler_pack)?;
    let mut command = Command::new(&installation.executable);
    command
        .arg("serve")
        .arg("--pack")
        .arg(&installation.model_pack)
        .arg("--handler")
        .arg(&installation.handler_pack);
    for key in &installation.model_trust_keys {
        command.arg("--trust-key").arg(key);
    }
    for key in &installation.handler_trust_keys {
        command.arg("--handler-trust-key").arg(key);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("start local AI runtime: {error}"))?;
    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("local AI runtime stdin unavailable".into());
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("local AI runtime stdout unavailable".into());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("local AI runtime stderr unavailable".into());
        }
    };

    // 握手请求:写一行 + 等一行响应(带超时)。
    let handshake = RuntimeRequest::Handshake {
        request_id: "mira-handshake".into(),
        api_version: RUNTIME_API_VERSION,
        client_name: "mira".into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
    };
    let line =
        serde_json::to_vec(&handshake).map_err(|error| format!("encode handshake: {error}"))?;
    let mut stdin = stdin;
    // stdout 由专用线程逐行读取，并通过 channel 同时提供握手和后续预测响应。
    let (response_tx, response_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = stdout;
        loop {
            let mut buf = Vec::new();
            match read_line(&mut reader, &mut buf) {
                Ok(Some(())) => {
                    if response_tx.send(Ok(buf)).is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = response_tx.send(Err("runtime stdout closed".into()));
                    break;
                }
                Err(error) => {
                    let _ = response_tx.send(Err(error));
                    break;
                }
            }
        }
    });
    // 后台读 stderr 线程:逐行转发到统一日志(限流+截断),并保留尾部用于失败诊断。
    let log_service = guard.log_service.clone();
    let _stderr_thread = std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut stderr_tail: Vec<u8> = Vec::new();
        let mut window_start = Instant::now();
        let mut lines_in_window: u32 = 0;
        let mut suppressed_count: u64 = 0;
        let mut buf = String::new();
        loop {
            buf.clear();
            match reader.read_line(&mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    let line = buf.trim_end_matches(['\r', '\n']);
                    // 维护尾部缓冲(用于子进程退出后的诊断快照)。
                    let line_bytes = line.as_bytes();
                    if stderr_tail.len() + line_bytes.len() + 1 > MAX_STDERR_BYTES {
                        let need = stderr_tail.len() + line_bytes.len() + 1 - MAX_STDERR_BYTES;
                        stderr_tail.drain(..need.min(stderr_tail.len()));
                    }
                    stderr_tail.extend_from_slice(line_bytes);
                    stderr_tail.push(b'\n');

                    // 1 秒窗口限流:超出 STDERR_RATE_LIMIT 的行折叠为摘要。
                    let now = Instant::now();
                    if now.duration_since(window_start) >= Duration::from_secs(1) {
                        if suppressed_count > 0 {
                            if let Some(svc) = log_service.as_ref() {
                                svc.write(local_ai_log_input(
                                    LogLevel::Warn,
                                    "local_ai::stderr",
                                    "local-ai-stderr-suppressed",
                                    format!(
                                        "stderr rate limit: {suppressed_count} additional lines suppressed"
                                    ),
                                    BTreeMap::from([(
                                        "suppressedCount".to_string(),
                                        FieldValue::from(suppressed_count),
                                    )]),
                                ));
                            }
                            suppressed_count = 0;
                        }
                        window_start = now;
                        lines_in_window = 0;
                    }
                    if lines_in_window >= STDERR_RATE_LIMIT {
                        suppressed_count += 1;
                        continue;
                    }
                    lines_in_window += 1;

                    // 截断超长行(按字符边界,避免破坏 UTF-8)。
                    let line_char_count = line.chars().count();
                    let was_truncated = line_char_count > MAX_STDERR_LINE;
                    let display_line: String = if was_truncated {
                        let truncated: String = line.chars().take(MAX_STDERR_LINE).collect();
                        format!("{truncated}…(truncated)")
                    } else {
                        line.to_string()
                    };

                    // 启发式等级映射:并非所有 stderr 都是 error。
                    // panic/fatal/error → Error,warn → Warn,其余默认 Info。
                    let lower = display_line.to_ascii_lowercase();
                    let level = if lower.contains("panic")
                        || lower.contains("fatal")
                        || lower.contains("error")
                    {
                        LogLevel::Error
                    } else if lower.contains("warn") {
                        LogLevel::Warn
                    } else {
                        LogLevel::Info
                    };

                    if let Some(svc) = log_service.as_ref() {
                        svc.write(local_ai_log_input(
                            level,
                            "local_ai::stderr",
                            "local-ai-stderr",
                            display_line,
                            BTreeMap::from([
                                ("source".to_string(), FieldValue::from("stderr")),
                                ("truncated".to_string(), FieldValue::from(was_truncated)),
                                (
                                    "characterCount".to_string(),
                                    FieldValue::from(line_char_count as u64),
                                ),
                            ]),
                        ));
                    }
                }
                Err(_) => break,
            }
        }
        // 收尾:刷新最后窗口中被抑制的行计数。
        if suppressed_count > 0 {
            if let Some(svc) = log_service.as_ref() {
                svc.write(local_ai_log_input(
                    LogLevel::Warn,
                    "local_ai::stderr",
                    "local-ai-stderr-suppressed",
                    format!("stderr rate limit: {suppressed_count} additional lines suppressed"),
                    BTreeMap::from([(
                        "suppressedCount".to_string(),
                        FieldValue::from(suppressed_count),
                    )]),
                ));
            }
        }
        stderr_tail
    });

    let write_result = stdin
        .write_all(&line)
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush());
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("write handshake: {error}"));
    }

    let response: RuntimeResponse = match response_rx.recv_timeout(REQUEST_TIMEOUT) {
        Ok(Ok(buf)) => match serde_json::from_slice(&buf) {
            Ok(response) => response,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("decode handshake response: {error}"));
            }
        },
        Ok(Err(error)) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("read handshake: {error}"));
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("local AI handshake timed out".into());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("local AI runtime closed stdout during handshake".into());
        }
    };
    let probe = match local_ai_runtime::validate_handshake_response(&response) {
        Ok(probe) => probe,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    guard.running = Some(RunningRuntime {
        child,
        stdin,
        responses: response_rx,
        request_seq: AtomicU64::new(0),
    });
    Ok(probe)
}

fn read_line(reader: &mut impl Read, buf: &mut Vec<u8>) -> Result<Option<()>, String> {
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) if buf.is_empty() => return Ok(None),
            Ok(0) => return Err("runtime stdout closed before newline".into()),
            Ok(_) => {
                if byte[0] == b'\n' {
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                    return Ok(Some(()));
                }
                buf.push(byte[0]);
                if buf.len() > MAX_MESSAGE_BYTES {
                    return Err("runtime response exceeds message limit".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn parse_prediction(
    response: &RuntimeResponse,
    expected_request_id: &str,
) -> Result<PredictionDecision, PredictionError> {
    match response {
        RuntimeResponse::Result {
            request_id,
            api_version,
            output,
        } if request_id == expected_request_id && *api_version == RUNTIME_API_VERSION => {
            let output: BatteryPredictionOutput =
                serde_json::from_value(output.clone()).map_err(|error| {
                    PredictionError::Fatal(format!("decode battery prediction output: {error}"))
                })?;
            match output.source {
                PredictionSource::LocalAi => {
                    let remaining = output
                        .remaining_hours
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or_else(|| {
                            PredictionError::HandlerBug(
                                "local AI returned an invalid estimate".into(),
                            )
                        })?;
                    if output.training_samples < MIN_HOST_TRAINING_SAMPLES {
                        return Ok(PredictionDecision::BaselineRecommended {
                            reason: format!(
                                "hostInsufficientTrainingData({}/{})",
                                output.training_samples, MIN_HOST_TRAINING_SAMPLES
                            ),
                        });
                    }
                    if output.validation_samples < MIN_HOST_VALIDATION_SAMPLES {
                        return Ok(PredictionDecision::BaselineRecommended {
                            reason: format!(
                                "hostInsufficientValidationData({}/{})",
                                output.validation_samples, MIN_HOST_VALIDATION_SAMPLES
                            ),
                        });
                    }
                    let Some(candidate_mae) = output
                        .candidate_mae
                        .filter(|value| value.is_finite() && *value >= 0.0)
                    else {
                        return Ok(PredictionDecision::BaselineRecommended {
                            reason: "hostQualityMetricsUnavailable".into(),
                        });
                    };
                    if candidate_mae > MAX_ACCEPTABLE_CANDIDATE_MAE_PER_HOUR {
                        return Ok(PredictionDecision::BaselineRecommended {
                            reason: format!(
                                "hostCandidateMaeTooHigh({candidate_mae:.3}>{MAX_ACCEPTABLE_CANDIDATE_MAE_PER_HOUR:.3})"
                            ),
                        });
                    }
                    if let Some(baseline_mae) = output
                        .baseline_mae
                        .filter(|value| value.is_finite() && *value > 0.0)
                    {
                        if candidate_mae > baseline_mae * MAX_CANDIDATE_TO_BASELINE_MAE_RATIO {
                            return Ok(PredictionDecision::BaselineRecommended {
                                reason: format!(
                                    "hostCandidateImprovementTooSmall({candidate_mae:.3}/{baseline_mae:.3})"
                                ),
                            });
                        }
                    }
                    Ok(PredictionDecision::Estimate(remaining))
                }
                PredictionSource::BaselineRecommended => {
                    Ok(PredictionDecision::BaselineRecommended {
                        reason: output.reason,
                    })
                }
            }
        }
        // rill-runtime 能返回结构化响应说明 IPC 仍健康，但 Wasmtime 的 timeout/trap
        // 已使当前 component instance 不可再次进入。两者都必须重建 runtime；timeout
        // 属于资源预算/系统繁忙，不计入 bundle 回滚，真实 trap 则需要上报 supervisor。
        // 其他 handler 错误没有中断实例，可继续复用进程并交由 supervisor 判断回滚。
        RuntimeResponse::Error {
            code,
            message,
            retryable,
            ..
        } => {
            let reason = format!("local AI prediction failed ({code}): {message}");
            if *retryable || code == "handlerTimeout" {
                Err(PredictionError::HandlerInterrupted {
                    reason,
                    report_failure: false,
                })
            } else if code == "handlerTrap" {
                Err(PredictionError::HandlerInterrupted {
                    reason,
                    report_failure: true,
                })
            } else {
                Err(PredictionError::HandlerBug(reason))
            }
        }
        _ => Err(PredictionError::Fatal(
            "local AI prediction contract mismatch".into(),
        )),
    }
}

fn timezone_offset_minutes(at: DateTime<Utc>) -> i32 {
    at.with_timezone(&Local).offset().local_minus_utc() / 60
}

/// 等待子进程退出的辅助方法,带超时。
trait WaitTimeoutExt {
    fn wait_timeout(&mut self, dur: Duration) -> Result<std::process::ExitStatus, ()>;
}

impl WaitTimeoutExt for Child {
    fn wait_timeout(&mut self, dur: Duration) -> Result<std::process::ExitStatus, ()> {
        let start = Instant::now();
        loop {
            match self.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) if start.elapsed() < dur => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => return Err(()),
                Err(_) => return Err(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! 覆盖协议错误分类以及 interrupted runtime 的进程丢弃/即时恢复状态；
    //! 完整 WASM/IPC 路径由 handler 发布工作流的 4,096 样本 smoke gate 验证。

    use super::*;
    use crate::local_ai_runtime::MIRA_HANDLER_ID;
    use mira_protocol::BatteryPredictionOutput;
    use rill_runtime_protocol::HANDLER_API_VERSION;

    const REQUEST_ID: &str = "mira-battery-predict-test";

    #[test]
    fn local_ai_log_input_always_contains_a_structured_event() {
        let input = local_ai_log_input(
            LogLevel::Info,
            "local_ai::predict",
            "local-ai-prediction-completed",
            "prediction complete",
            BTreeMap::from([("resultCount".to_string(), FieldValue::from(2_u64))]),
        );

        assert_eq!(input.source, LogSource::LocalAi);
        assert!(matches!(
            input.fields.get("event"),
            Some(FieldValue::Text(value)) if value == "local-ai-prediction-completed"
        ));
        assert!(matches!(
            input.fields.get("resultCount"),
            Some(FieldValue::Integer(2))
        ));
    }

    fn result_response(output: BatteryPredictionOutput) -> RuntimeResponse {
        RuntimeResponse::Result {
            request_id: REQUEST_ID.into(),
            api_version: RUNTIME_API_VERSION,
            output: serde_json::to_value(&output).unwrap(),
        }
    }

    fn error_response(code: &str, message: &str, retryable: bool) -> RuntimeResponse {
        RuntimeResponse::Error {
            request_id: REQUEST_ID.into(),
            api_version: RUNTIME_API_VERSION,
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_runtime_is_discarded_without_fatal_cooldown() {
        let controller = LocalAiController::default();
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let (_response_tx, responses) = mpsc::channel();

        let mut guard = controller.lock();
        guard.running = Some(RunningRuntime {
            child,
            stdin,
            responses,
            request_seq: AtomicU64::new(0),
        });
        guard.state = ControllerState::Running;

        discard_interrupted_runtime(&mut guard);

        assert!(guard.running.is_none());
        assert!(matches!(&guard.state, ControllerState::Running));
    }

    fn prediction_output(
        source: PredictionSource,
        remaining_hours: Option<f64>,
    ) -> BatteryPredictionOutput {
        BatteryPredictionOutput {
            remaining_hours,
            source,
            reason: "test".into(),
            training_samples: 20,
            validation_samples: 10,
            baseline_mae: Some(2.0),
            candidate_mae: Some(1.0),
        }
    }

    #[test]
    fn parse_prediction_restarts_runtime_for_retryable_timeout() {
        // handlerTimeout 是 rill-runtime 当前唯一标 retryable=true 的错误。
        let response = error_response("handlerTimeout", "handlerTimeout", true);
        let result = parse_prediction(&response, REQUEST_ID);
        assert!(matches!(
            result,
            Err(PredictionError::HandlerInterrupted {
                report_failure: false,
                ..
            })
        ));
    }

    #[test]
    fn parse_prediction_restarts_and_reports_handler_trap() {
        let response = error_response("handlerTrap", "wasm unreachable", false);
        let result = parse_prediction(&response, REQUEST_ID);
        assert!(matches!(
            result,
            Err(PredictionError::HandlerInterrupted {
                report_failure: true,
                ..
            })
        ));
    }

    #[test]
    fn parse_prediction_returns_handler_bug_for_non_retryable_error() {
        // handlerInvalidOutput 等未中断实例的错误仍交由 supervisor 判断 bundle 回滚。
        let response = error_response("handlerInvalidOutput", "invalid JSON", false);
        let result = parse_prediction(&response, REQUEST_ID);
        assert!(matches!(result, Err(PredictionError::HandlerBug(_))));
    }

    #[test]
    fn parse_prediction_returns_fatal_for_contract_mismatch() {
        // 收到非 Result/Error 变体(如 Handshake)表示协议/版本错位,runtime 可能不兼容。
        let response = RuntimeResponse::Handshake {
            request_id: REQUEST_ID.into(),
            api_version: RUNTIME_API_VERSION,
            runtime_version: "0.7.1".into(),
            model_pack_id: "mira.battery.default".into(),
            model_pack_version: "0.5.0".into(),
            capabilities: vec![BATTERY_USAGE_CAPABILITY.into()],
            handler_id: MIRA_HANDLER_ID.into(),
            handler_version: "0.8.2".into(),
            handler_api_version: HANDLER_API_VERSION,
            effective_capabilities: vec![BATTERY_USAGE_CAPABILITY.into()],
        };
        let result = parse_prediction(&response, REQUEST_ID);
        assert!(matches!(result, Err(PredictionError::Fatal(_))));
    }

    #[test]
    fn parse_prediction_returns_fatal_for_decode_failure() {
        // Result 变体但 output 不是合法 BatteryPredictionOutput JSON → 协议不匹配。
        let response = RuntimeResponse::Result {
            request_id: REQUEST_ID.into(),
            api_version: RUNTIME_API_VERSION,
            output: serde_json::Value::String("not-a-battery-output".into()),
        };
        let result = parse_prediction(&response, REQUEST_ID);
        assert!(matches!(result, Err(PredictionError::Fatal(_))));
    }

    #[test]
    fn parse_prediction_returns_baseline_decision_for_handler_fallback() {
        // BaselineRecommended 是 handler 的正常回退路径,不是错误。
        let response = result_response(prediction_output(
            PredictionSource::BaselineRecommended,
            None,
        ));
        let result = parse_prediction(&response, REQUEST_ID);
        assert_eq!(
            result.unwrap(),
            PredictionDecision::BaselineRecommended {
                reason: "test".into()
            }
        );
    }

    #[test]
    fn parse_prediction_returns_some_for_local_ai() {
        // LocalAi 通过质量门的有效预测。
        let response = result_response(prediction_output(PredictionSource::LocalAi, Some(12.5)));
        let result = parse_prediction(&response, REQUEST_ID);
        assert_eq!(result.unwrap(), PredictionDecision::Estimate(12.5));
    }

    #[test]
    fn parse_prediction_rejects_large_absolute_error_without_blaming_runtime() {
        let mut output = prediction_output(PredictionSource::LocalAi, Some(24.4));
        output.baseline_mae = Some(9.45);
        output.candidate_mae = Some(8.09);
        let result = parse_prediction(&result_response(output), REQUEST_ID).unwrap();
        assert!(matches!(
            result,
            PredictionDecision::BaselineRecommended { reason }
                if reason.starts_with("hostCandidateMaeTooHigh")
        ));
    }

    #[test]
    fn parse_prediction_requires_enough_host_training_evidence() {
        let mut output = prediction_output(PredictionSource::LocalAi, Some(24.4));
        output.training_samples = MIN_HOST_TRAINING_SAMPLES - 1;
        let result = parse_prediction(&result_response(output), REQUEST_ID).unwrap();
        assert!(matches!(
            result,
            PredictionDecision::BaselineRecommended { reason }
                if reason.starts_with("hostInsufficientTrainingData")
        ));
    }

    #[test]
    fn parse_prediction_returns_handler_bug_for_invalid_local_ai_estimate() {
        // source=LocalAi 但 remaining_hours 为 None/非有限/负值 → handler 返回了无效值,
        // runtime 本身健康,归类为 HandlerBug(不 kill,但投 supervisor)。
        let response = result_response(prediction_output(PredictionSource::LocalAi, None));
        let result = parse_prediction(&response, REQUEST_ID);
        assert!(matches!(result, Err(PredictionError::HandlerBug(_))));

        let response_nan =
            result_response(prediction_output(PredictionSource::LocalAi, Some(f64::NAN)));
        let result_nan = parse_prediction(&response_nan, REQUEST_ID);
        assert!(matches!(result_nan, Err(PredictionError::HandlerBug(_))));

        let response_neg =
            result_response(prediction_output(PredictionSource::LocalAi, Some(-1.0)));
        let result_neg = parse_prediction(&response_neg, REQUEST_ID);
        assert!(matches!(result_neg, Err(PredictionError::HandlerBug(_))));
    }

    #[test]
    fn parse_prediction_returns_fatal_for_wrong_request_id() {
        // request_id 不匹配 → 协议错位(可能是 runtime 串响应),Fatal。
        let response = result_response(prediction_output(
            PredictionSource::BaselineRecommended,
            None,
        ));
        let result = parse_prediction(&response, "some-other-request-id");
        assert!(matches!(result, Err(PredictionError::Fatal(_))));
    }
}
