// SPDX-License-Identifier: AGPL-3.0-or-later
//! 常驻式本地 AI Runtime 控制器。
//!
//! 总开关 `local_ai_analysis_enabled` 翻转到 on 时调用 `start()` 启动 mira-runtime
//! 子进程并完成握手;翻转到 off 时调用 `stop()` 优雅退出。`predict()` 复用已建立的
//! stdin/stdout 通道,避免每次预测的进程启动开销。任何 IO/解析错误或子进程意外退出
//! 都标记 `Failed`,下次 `predict()` 在冷却窗口外自动重启。
//!
//! 失败、超时或未安装时,predict 返回空 map,调用方回退确定性算法。

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
        Mutex, PoisonError,
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use mira_protocol::{
    BatteryPredictionInput, BatteryPredictionOutput, BatterySampleInput, PredictionSource,
    BATTERY_USAGE_CAPABILITY,
};
use rill_runtime_protocol::{
    RuntimeRequest, RuntimeResponse, MAX_MESSAGE_BYTES, RUNTIME_API_VERSION,
};
use tauri::AppHandle;

use crate::{
    battery_history::BatterySample, local_ai_runtime, local_ai_runtime::RuntimeInstallation,
};

/// 单次请求/响应超时。与原 `predict_batteries` 的 RUNTIME_TIMEOUT 保持一致。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// 失败后冷却窗口:窗口内不重试,直接回退。避免高频预测引起频繁重启。
const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);
/// stop() 优雅退出的等待时间;超时后强制 kill。
const STOP_GRACE: Duration = Duration::from_millis(500);
/// stderr 缓冲上限,超限丢弃旧数据。仅用于错误诊断。
const MAX_STDERR_BYTES: usize = 64 * 1024;
/// Keep mature 30-day histories below the 1 MiB IPC envelope while retaining
/// enough recent samples for the quality gate. At the normal five-minute
/// cadence this is roughly two weeks per battery component.
const MAX_PREDICTION_SAMPLES: usize = 4_096;

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

/// 控制器状态。所有字段通过单一 Mutex 串行化访问,避免并发竞争。
struct ControllerInner {
    running: Option<RunningRuntime>,
    state: ControllerState,
    /// 由 lib.rs setup 阶段注入的 crash 事件 sender。None 时静默丢弃事件。
    crash_tx: Option<Sender<CrashEvent>>,
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
        }
    }
}

impl LocalAiController {
    /// 在 lib.rs setup 阶段注入 crash 事件 sender,启动 supervisor 监听。
    pub fn set_crash_sender(&self, tx: Sender<CrashEvent>) {
        let mut guard = self.lock();
        guard.crash_tx = Some(tx);
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
        let Some(installation) = local_ai_runtime::resolve_installation(app) else {
            guard.state = ControllerState::Failed {
                last_attempt: Instant::now(),
            };
            self.emit_crash_event(CrashEvent::Failed {
                at: Instant::now(),
                reason: "runtime or model pack not available".into(),
            });
            return Err("local AI runtime or model pack not available".to_string());
        };
        if let Err(error) = spawn_and_handshake(&installation, &mut guard) {
            guard.state = ControllerState::Failed {
                last_attempt: Instant::now(),
            };
            self.emit_crash_event(CrashEvent::Failed {
                at: Instant::now(),
                reason: error.clone(),
            });
            return Err(error);
        }
        guard.state = ControllerState::Running;
        Ok(())
    }

    /// 开关 off 时调用。关闭 stdin 让子进程自然退出,超时后 kill。
    pub fn stop(&self) {
        let mut guard = self.lock();
        let Some(mut runtime) = guard.running.take() else {
            guard.state = ControllerState::Stopped;
            return;
        };
        // drop stdin 触发 EOF,mira-runtime 的 BufReader::read_until 返回 0 退出主循环。
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
        let mut guard = self.lock();
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
                samples: samples[recent_start..]
                    .iter()
                    .map(|sample| BatterySampleInput {
                        at_unix_ms: sample.at.timestamp_millis(),
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
                Ok(Some(remaining)) => {
                    if let Some((key, _)) = batches.get(index) {
                        results.insert(key.clone(), remaining);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    mark_failed(&mut guard, &error);
                    return BTreeMap::new();
                }
            }
        }
        // 完整跑完所有 batch 且至少返回 1 个结果 → 通知 supervisor 清零失败窗口。
        // 必须先 drop guard 再 emit,否则 emit_crash_event 重新 lock 会死锁。
        let has_results = !results.is_empty();
        drop(guard);
        if has_results {
            self.emit_crash_event(CrashEvent::Success);
        }
        results
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ControllerInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn mark_failed(guard: &mut std::sync::MutexGuard<'_, ControllerInner>, reason: &str) {
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

fn spawn_and_handshake(
    installation: &RuntimeInstallation,
    guard: &mut std::sync::MutexGuard<'_, ControllerInner>,
) -> Result<(), String> {
    local_ai_runtime::ensure_safe_runtime_file(&installation.executable)?;
    local_ai_runtime::ensure_safe_runtime_file(&installation.model_pack)?;
    let mut command = Command::new(&installation.executable);
    command
        .arg("serve")
        .arg("--pack")
        .arg(&installation.model_pack);
    for key in &installation.trust_keys {
        command.arg("--trust-key").arg(key);
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
    // 后台读 stderr 线程,仅用于诊断:子进程退出后拿尾部 MAX_STDERR_BYTES 用于错误信息。
    let _stderr_thread = std::thread::spawn(move || {
        let mut reader = stderr;
        let mut buf = vec![0u8; 1024];
        let mut stderr_tail: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stderr_tail.len() + n > MAX_STDERR_BYTES {
                        let overflow = stderr_tail.len() + n - MAX_STDERR_BYTES;
                        stderr_tail.drain(..overflow.min(stderr_tail.len()));
                    }
                    stderr_tail.extend_from_slice(&buf[..n]);
                }
                Err(_) => break,
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
    if let Err(error) = local_ai_runtime::validate_handshake_response(&response) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    guard.running = Some(RunningRuntime {
        child,
        stdin,
        responses: response_rx,
        request_seq: AtomicU64::new(0),
    });
    Ok(())
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
) -> Result<Option<f64>, String> {
    match response {
        RuntimeResponse::Result {
            request_id,
            api_version,
            output,
        } if request_id == expected_request_id && *api_version == RUNTIME_API_VERSION => {
            let output: BatteryPredictionOutput = serde_json::from_value(output.clone())
                .map_err(|error| format!("decode battery prediction output: {error}"))?;
            match output.source {
                PredictionSource::LocalAi => {
                    let remaining = output
                        .remaining_hours
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or_else(|| "local AI returned an invalid estimate".to_string())?;
                    Ok(Some(remaining))
                }
                PredictionSource::BaselineRecommended => Ok(None),
            }
        }
        RuntimeResponse::Error { code, message, .. } => {
            Err(format!("local AI prediction failed ({code}): {message}"))
        }
        _ => Err("local AI prediction contract mismatch".into()),
    }
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
