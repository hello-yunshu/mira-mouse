// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mira 统一日志服务门面。
//!
//! 整合 buffer / storage / redaction / frontend emitter，对调用方提供
//! `write` / `query` / `status` / `delete` / `export` / `set_level` /
//! `start_diagnostic_session` / `stop_diagnostic_session` 等语义。
//!
//! 关键约束（对齐 spec 四.4 / 五 / 八 / 十一）：
//! - 单一日志入口：业务代码只调用 `LogService::write(LogInput)`。
//! - 防递归：LogService 内部任何错误只能走 `eprintln!`，绝不能再调用 write。
//! - 锁中不阻塞：buffer 锁仅持极短时间；storage / frontend 通过 mpsc 投递。
//! - 默认 `info` 及以上才采集；临时诊断模式可提升到 `debug` 或 `trace`。
//! - 临时诊断会话到期后自动恢复原等级，即使日志页未打开。

pub mod buffer;
pub mod commands;
pub mod export;
pub mod frontend;
pub mod model;
pub mod protocol_event;
pub mod redaction;
pub mod storage;

use chrono::Utc;
use model::{
    DiagnosticSessionStatus, LogEntry, LogInput, LogLevel, LogPage, LogQuery, LogStatus,
    ProtocolDiagnosticStatus,
};
use redaction::Redactor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;

pub use model::{DeleteResult, DeleteScope};

/// 默认采集等级。
pub const DEFAULT_MIN_LEVEL: LogLevel = LogLevel::Info;
/// 默认临时诊断会话持续分钟数。
pub const DEFAULT_DIAGNOSTIC_MINUTES: i64 = 10;
/// 最大临时诊断会话分钟数。
pub const MAX_DIAGNOSTIC_MINUTES: i64 = 30;
/// 最小临时诊断会话分钟数。
pub const MIN_DIAGNOSTIC_MINUTES: i64 = 1;

/// 临时诊断会话运行时状态。
struct DiagnosticSession {
    started_at: chrono::DateTime<Utc>,
    ends_at: chrono::DateTime<Utc>,
    original_level: LogLevel,
    current_level: LogLevel,
    auto_expire: bool,
    /// 取消标志：手动停止 / 会话 drop 时设为 true，自动到期线程退出前检查。
    cancel: Arc<AtomicBool>,
}

impl DiagnosticSession {
    fn status(&self) -> DiagnosticSessionStatus {
        DiagnosticSessionStatus {
            started_at: self.started_at.to_rfc3339(),
            ends_at: self.ends_at.to_rfc3339(),
            original_level: self.original_level,
            current_level: self.current_level,
            auto_expire: self.auto_expire,
        }
    }
}

impl Drop for DiagnosticSession {
    fn drop(&mut self) {
        // 通知自动到期线程退出。线程在 ≤500ms 内响应 cancel 并退出。
        // 刻意不 join：到期线程的自然过期路径只恢复 min_level，不再 drop 会话本身，
        // 因此不会出现「到期线程 drop 含自身 JoinHandle 的会话」的自join死锁。
        // 会话的实际 drop 由 status() 惰性过期或 stop/start 覆盖时触发，
        // 此时 cancel 已置位、线程已退出或即将退出，detach 即可。
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 协议诊断会话：针对单个设备的临时 HID payload 记录授权。
///
/// 与 `DiagnosticSession`（通用日志等级提升）独立：
/// - 通用诊断会话控制日志采集等级（Debug/Trace）。
/// - 协议诊断会话控制是否在 `hid-feature-exchange` 事件中携带 request/response hex。
///
/// 实践中两者通常同时启用：先启动通用诊断会话提升到 Trace，再启动协议诊断会话
/// 授权 payload 记录。两者独立到期，任一到期都会停止 payload 记录。
struct ProtocolDiagnosticSession {
    device_key: String,
    started_at: chrono::DateTime<Utc>,
    ends_at: chrono::DateTime<Utc>,
    auto_expire: bool,
    /// 取消标志：手动停止 / 会话 drop 时设为 true。
    cancel: Arc<AtomicBool>,
}

impl ProtocolDiagnosticSession {
    fn status(&self) -> ProtocolDiagnosticStatus {
        ProtocolDiagnosticStatus {
            device_key: self.device_key.clone(),
            started_at: self.started_at.to_rfc3339(),
            ends_at: self.ends_at.to_rfc3339(),
            auto_expire: self.auto_expire,
        }
    }
}

impl Drop for ProtocolDiagnosticSession {
    fn drop(&mut self) {
        // 与 DiagnosticSession 相同的 cancel 策略：detach 到期线程。
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 统一日志服务。共享状态，clone 安全。
#[derive(Clone)]
pub struct LogService {
    inner: Arc<LogServiceInner>,
}

struct LogServiceInner {
    buffer: Mutex<buffer::LogBuffer>,
    storage: storage::LogStorageHandle,
    redactor: Mutex<Redactor>,
    frontend: frontend::FrontendEmitter,
    session_id: String,
    min_level: Mutex<LogLevel>,
    diagnostic_session: Mutex<Option<DiagnosticSession>>,
    protocol_diagnostic: Mutex<Option<ProtocolDiagnosticSession>>,
}

impl LogService {
    /// 创建并启动服务。
    /// - `session_id`: 应用本次启动会话 ID
    /// - `storage_dir`: 日志目录（来自 app_log_dir）
    /// - `redactor`: 已初始化的脱敏器
    /// - `app_handle`: Tauri AppHandle，用于向前端 emit
    pub fn new(
        session_id: String,
        storage_dir: PathBuf,
        redactor: Redactor,
        app_handle: AppHandle,
    ) -> Self {
        let buffer = Mutex::new(buffer::LogBuffer::new(buffer::DEFAULT_CAPACITY));
        let (storage, _) = storage::spawn(storage_dir);
        let (frontend, _) = frontend::FrontendEmitter::spawn(app_handle);
        let min_level = Mutex::new(DEFAULT_MIN_LEVEL);

        let inner = Arc::new(LogServiceInner {
            buffer,
            storage,
            redactor: Mutex::new(redactor),
            frontend,
            session_id: session_id.clone(),
            min_level,
            diagnostic_session: Mutex::new(None),
            protocol_diagnostic: Mutex::new(None),
        });

        Self { inner }
    }

    /// 写入一条日志。所有持久化、UI 推送、脱敏都在这里完成。
    pub fn write(&self, input: LogInput) {
        let level = input.level;
        let min = *self.inner.min_level.lock().unwrap();
        // 等级低于最低采集：直接丢弃。等级数值越大代表越低（Trace=4）。
        if !level.at_least(min) {
            return;
        }

        // 分配 ID 并入队（短锁）。
        let entry = {
            let mut buf = self.inner.buffer.lock().unwrap();
            let id = buf.next_id();
            LogEntry {
                id,
                timestamp: Utc::now().to_rfc3339(),
                level: input.level,
                source: input.source,
                target: input.target,
                message: input.message,
                session_id: self.inner.session_id.clone(),
                correlation_id: input.correlation_id,
                fields: input.fields,
            }
        };

        // 脱敏。Redactor 不会失败，最坏降级为占位符。
        let mut redacted = entry;
        self.inner.redactor.lock().unwrap().apply(&mut redacted);

        // 三路投递：buffer / storage / frontend。每路都 clone 一次。
        self.inner.buffer.lock().unwrap().push(redacted.clone());
        self.inner.storage.append(redacted.clone());
        self.inner.frontend.push(redacted);
    }

    /// 便捷方法：写入一条 warn 级别的应用日志。
    pub fn warn(&self, target: &'static str, message: impl Into<String>) {
        self.write(LogInput::new(
            LogLevel::Warn,
            model::LogSource::App,
            target,
            message,
        ));
    }

    /// 便捷方法：写入一条 error 级别的应用日志。
    pub fn error(&self, target: &'static str, message: impl Into<String>) {
        self.write(LogInput::new(
            LogLevel::Error,
            model::LogSource::App,
            target,
            message,
        ));
    }

    /// 查询历史日志。
    pub fn query(&self, q: &LogQuery) -> LogPage {
        let limit = q.effective_limit();
        let buffer = self.inner.buffer.lock().unwrap();
        let matcher = |e: &LogEntry| q.matches(e);
        let (entries, has_more, oldest_id) = buffer.page(limit, q.before_id, matcher);

        // total_in_session 是近似值；遍历内存缓冲做一次线性计数。
        let total_in_session = buffer
            .snapshot_for_session(&self.inner.session_id)
            .into_iter()
            .filter(|e| q.matches(e))
            .count();

        LogPage {
            entries,
            has_more,
            oldest_id,
            total_in_session,
        }
    }

    /// 当前日志服务状态。
    pub fn status(&self) -> LogStatus {
        let buffer = self.inner.buffer.lock().unwrap();
        let (recent_error_count, recent_warn_count) = buffer.recent_counts();
        let min_level = *self.inner.min_level.lock().unwrap();
        let storage_dir_display = {
            let redactor = self.inner.redactor.lock().unwrap();
            let dir = self.inner.storage.dir();
            let mut display = dir.to_string_lossy().to_string();
            if let Some(home) = redactor.home_dir() {
                if !home.as_os_str().is_empty() {
                    let home_str = home.to_string_lossy();
                    display = display.replace(home_str.as_ref(), redaction::HOME_PLACEHOLDER);
                }
            }
            if let Some(user) = redactor.user_name() {
                if !user.is_empty() {
                    display = display.replace(user, redaction::USER_PLACEHOLDER);
                }
            }
            display
        };

        // 检查诊断会话是否需要惰性过期（防御性，正常由后台线程处理）。
        let diagnostic_session_status = {
            let mut session = self.inner.diagnostic_session.lock().unwrap();
            if let Some(s) = session.as_ref() {
                if s.auto_expire && Utc::now() >= s.ends_at {
                    // 后台线程应当已经处理；这里仅作展示同步。
                    let original_level = s.original_level;
                    *self.inner.min_level.lock().unwrap() = original_level;
                    *session = None;
                    None
                } else {
                    Some(s.status())
                }
            } else {
                None
            }
        };

        // 检查协议诊断会话是否需要惰性过期。
        let protocol_diagnostic_status = {
            let mut session = self.inner.protocol_diagnostic.lock().unwrap();
            if let Some(s) = session.as_ref() {
                if s.auto_expire && Utc::now() >= s.ends_at {
                    *session = None;
                    None
                } else {
                    Some(s.status())
                }
            } else {
                None
            }
        };

        LogStatus {
            session_id: self.inner.session_id.clone(),
            min_level,
            buffer_count: buffer.len(),
            buffer_capacity: buffer.capacity(),
            storage_dir_display,
            disk_usage_bytes: self.inner.storage.disk_usage(),
            disk_quota_bytes: storage::DISK_QUOTA_BYTES,
            recent_error_count,
            recent_warn_count,
            file_persistence_enabled: self.inner.storage.enabled(),
            diagnostic_session: diagnostic_session_status,
            protocol_diagnostic: protocol_diagnostic_status,
        }
    }

    /// 删除磁盘历史日志。仅作用于磁盘文件，不影响内存缓冲（缓冲中的历史仍在）。
    pub fn delete(&self, scope: DeleteScope) -> DeleteResult {
        let (deleted_files, deleted_buffer_entries, partial, error) = match scope {
            DeleteScope::OlderThanDays { days } => {
                let (count, err) = self.inner.storage.delete_older_than(days);
                let cutoff_str = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
                let buffer_dropped = self
                    .inner
                    .buffer
                    .lock()
                    .unwrap()
                    .drop_older_than(&cutoff_str);
                (count, buffer_dropped as u64, err.is_some(), err)
            }
            DeleteScope::BeforeCurrentSession => {
                let (count, err) = self.inner.storage.delete_all();
                let buffer_dropped = self
                    .inner
                    .buffer
                    .lock()
                    .unwrap()
                    .drop_other_sessions(&self.inner.session_id);
                (count, buffer_dropped as u64, err.is_some(), err)
            }
            DeleteScope::All => {
                let (count, err) = self.inner.storage.delete_all();
                let buffer_dropped = {
                    let mut buf = self.inner.buffer.lock().unwrap();
                    let before = buf.len();
                    buf.clear();
                    before as u64
                };
                (count, buffer_dropped, err.is_some(), err)
            }
        };

        DeleteResult {
            deleted_files,
            deleted_buffer_entries,
            partial,
            error,
        }
    }

    /// 触发 storage flush，确保待写入日志落盘。
    pub fn flush(&self) {
        self.inner.storage.flush();
        self.inner.frontend.flush();
    }

    /// 前端订阅：开始向前端 emit 日志批次。
    pub fn subscribe(&self) {
        self.inner.frontend.subscribe();
    }

    /// 前端取消订阅。
    pub fn unsubscribe(&self) {
        self.inner.frontend.unsubscribe();
    }

    /// 临时设置最低采集等级。无诊断会话时直接覆盖；有诊断会话时更新当前等级。
    pub fn set_level(&self, level: LogLevel) {
        *self.inner.min_level.lock().unwrap() = level;
        // 同步更新诊断会话中的 current_level（如有）。
        if let Some(s) = self.inner.diagnostic_session.lock().unwrap().as_mut() {
            s.current_level = level;
        }
    }

    /// 开始临时诊断会话。
    /// - `minutes`: 持续分钟数，会被限制在 [MIN, MAX] 区间。
    /// - `level`: 临时等级，通常为 Debug 或 Trace。
    /// - `auto_expire`: true 时启动后台到期线程。
    pub fn start_diagnostic_session(&self, minutes: i64, level: LogLevel, auto_expire: bool) {
        let minutes = minutes.clamp(MIN_DIAGNOSTIC_MINUTES, MAX_DIAGNOSTIC_MINUTES);
        let now = Utc::now();
        let ends_at = now + chrono::Duration::minutes(minutes);
        let original_level = *self.inner.min_level.lock().unwrap();

        // 如果已有会话，先取消旧的。
        if self.inner.diagnostic_session.lock().unwrap().is_some() {
            self.stop_diagnostic_session();
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let inner_clone = self.inner.clone();
        let original_level_for_thread = original_level;

        if auto_expire {
            let sleep_duration = Duration::from_secs((minutes as u64) * 60);
            // 到期线程 detach：JoinHandle 不存储，线程靠 cancel 标志退出。
            // 刻意不在 Drop 中 join，避免到期线程 drop 含自身 handle 的会话造成自join死锁。
            std::thread::Builder::new()
                .name("mira-log-diag-expire".into())
                .spawn(move || {
                    // 分段 sleep 以便响应 cancel。
                    let chunk = Duration::from_millis(500);
                    let mut elapsed = Duration::ZERO;
                    while elapsed < sleep_duration {
                        if cancel_clone.load(Ordering::Relaxed) {
                            return;
                        }
                        let step = chunk.min(sleep_duration - elapsed);
                        std::thread::sleep(step);
                        elapsed += step;
                    }
                    if !cancel_clone.load(Ordering::Relaxed) {
                        // 自然到期：只恢复采集等级。不主动 drop 会话（那会触发 Drop→join 自身
                        // 的死锁）。会话的惰性清理由 status() 在检测到 ends_at 已过时执行，
                        // 或由下一次 start/stop 覆盖时触发。
                        *inner_clone.min_level.lock().unwrap() = original_level_for_thread;
                    }
                })
                .expect("spawn mira-log-diag-expire");
        }

        let session = DiagnosticSession {
            started_at: now,
            ends_at,
            original_level,
            current_level: level,
            auto_expire,
            cancel,
        };

        *self.inner.min_level.lock().unwrap() = level;
        *self.inner.diagnostic_session.lock().unwrap() = Some(session);
    }

    /// 手动停止临时诊断会话。恢复原采集等级。
    pub fn stop_diagnostic_session(&self) {
        // 先把会话从 Option 中取出并立即释放 diagnostic_session 锁，
        // 再恢复等级、再 drop 会话。drop 只置 cancel 标志（不 join），
        // 但释放锁后再 drop 仍是更安全的顺序：避免任何 drop 副作用持锁等待。
        let session = { self.inner.diagnostic_session.lock().unwrap().take() };
        if let Some(session) = session {
            session.cancel.store(true, Ordering::Relaxed);
            *self.inner.min_level.lock().unwrap() = session.original_level;
            drop(session);
        }
    }

    /// 开始协议诊断会话：授权对指定设备临时记录 HID payload。
    ///
    /// - `device_key`: 目标设备 key（VID:PID:interface）。只对此设备的 HID 交换记录 payload。
    /// - `minutes`: 持续分钟数，clamp 到 [MIN, MAX]。
    /// - `auto_expire`: true 时启动后台到期线程，到期自动停止。
    ///
    /// 协议诊断模式不影响日志采集等级；调用方应同时启动通用诊断会话
    /// 提升到 Trace 才能使 `hid-feature-exchange` 事件（Trace 级别）被采集。
    pub fn start_protocol_diagnostic(&self, device_key: String, minutes: i64, auto_expire: bool) {
        let minutes = minutes.clamp(MIN_DIAGNOSTIC_MINUTES, MAX_DIAGNOSTIC_MINUTES);
        let now = Utc::now();
        let ends_at = now + chrono::Duration::minutes(minutes);

        // 如果已有会话，先取消旧的。
        if self.inner.protocol_diagnostic.lock().unwrap().is_some() {
            self.stop_protocol_diagnostic();
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let inner_clone = self.inner.clone();

        if auto_expire {
            let sleep_duration = Duration::from_secs((minutes as u64) * 60);
            std::thread::Builder::new()
                .name("mira-log-proto-diag-expire".into())
                .spawn(move || {
                    let chunk = Duration::from_millis(500);
                    let mut elapsed = Duration::ZERO;
                    while elapsed < sleep_duration {
                        if cancel_clone.load(Ordering::Relaxed) {
                            return;
                        }
                        let step = chunk.min(sleep_duration - elapsed);
                        std::thread::sleep(step);
                        elapsed += step;
                    }
                    if !cancel_clone.load(Ordering::Relaxed) {
                        // 自然到期：惰性清理 protocol_diagnostic。不主动 drop 会话
                        // （与 DiagnosticSession 相同的自join死锁规避策略）。
                        *inner_clone.protocol_diagnostic.lock().unwrap() = None;
                    }
                })
                .expect("spawn mira-log-proto-diag-expire");
        }

        let session = ProtocolDiagnosticSession {
            device_key,
            started_at: now,
            ends_at,
            auto_expire,
            cancel,
        };

        *self.inner.protocol_diagnostic.lock().unwrap() = Some(session);
    }

    /// 手动停止协议诊断会话。
    pub fn stop_protocol_diagnostic(&self) {
        let session = { self.inner.protocol_diagnostic.lock().unwrap().take() };
        if let Some(session) = session {
            session.cancel.store(true, Ordering::Relaxed);
            drop(session);
        }
    }

    /// 返回当前协议诊断会话的目标设备 key（已检查过期）。
    ///
    /// 调用方（Host 在记录 HID payload 时）用此方法判断是否应对指定设备
    /// 携带 request/response hex。返回 None 表示不应记录 payload。
    pub fn protocol_diagnostic_device_key(&self) -> Option<String> {
        let mut session = self.inner.protocol_diagnostic.lock().unwrap();
        if let Some(s) = session.as_ref() {
            if s.auto_expire && Utc::now() >= s.ends_at {
                *session = None;
                None
            } else {
                Some(s.device_key.clone())
            }
        } else {
            None
        }
    }

    /// 当前会话的 buffer 快照。
    pub(crate) fn buffer_snapshot_for_session(&self) -> Vec<LogEntry> {
        self.inner
            .buffer
            .lock()
            .unwrap()
            .snapshot_for_session(&self.inner.session_id)
    }

    /// 按查询条件筛选的条目快照（用于导出）。
    pub(crate) fn query_filtered_entries(&self, q: &LogQuery) -> Vec<LogEntry> {
        let buffer = self.inner.buffer.lock().unwrap();
        // 取所有匹配条目（不分页）。
        let snapshot = buffer.snapshot();
        snapshot.into_iter().filter(|e| q.matches(e)).collect()
    }

    /// 日志存储目录（用于"打开日志目录"命令）。
    pub fn storage_dir(&self) -> PathBuf {
        self.inner.storage.dir()
    }
}

/// 生成新的会话 ID（启动时调用）。
/// 格式：YYYYMMDDTHHMMSS-NNNN，NNNN 为 4 位随机后缀。
pub fn new_session_id() -> String {
    let now = Utc::now();
    let ts = now.format("%Y%m%dT%H%M%S").to_string();
    // 简单随机：纳秒位取模。
    let suffix = (now.timestamp_nanos_opt().unwrap_or(0) as u32) % 10_000;
    format!("{ts}-{suffix:04}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在无 Tauri AppHandle 的测试环境下，使用一个简单的内存 stub 替换 LogService
    /// 的 buffer/storage/redaction 链路，仅验证等级过滤与诊断会话语义。
    /// 由于 LogService::new 需要 AppHandle，这里直接测试可独立的辅助逻辑。
    #[test]
    fn new_session_id_has_expected_format() {
        let id = new_session_id();
        assert!(id.len() >= 16);
        assert!(id.contains('T'));
        assert!(id.contains('-'));
    }

    #[test]
    fn diagnostic_minutes_clamped() {
        assert_eq!(
            0i64.clamp(MIN_DIAGNOSTIC_MINUTES, MAX_DIAGNOSTIC_MINUTES),
            MIN_DIAGNOSTIC_MINUTES
        );
        assert_eq!(
            120i64.clamp(MIN_DIAGNOSTIC_MINUTES, MAX_DIAGNOSTIC_MINUTES),
            MAX_DIAGNOSTIC_MINUTES
        );
        assert_eq!(
            10i64.clamp(MIN_DIAGNOSTIC_MINUTES, MAX_DIAGNOSTIC_MINUTES),
            10
        );
    }

    #[test]
    fn level_filter_drops_below_minimum() {
        // 验证 LogLevel::at_least 的语义：LogService::write 依赖它。
        let min = LogLevel::Info;
        assert!(LogLevel::Error.at_least(min));
        assert!(LogLevel::Warn.at_least(min));
        assert!(LogLevel::Info.at_least(min));
        assert!(!LogLevel::Debug.at_least(min));
        assert!(!LogLevel::Trace.at_least(min));
    }

    /// 模拟 LogService::write 的等级过滤逻辑，无需 AppHandle。
    fn level_passes_filter(input_level: LogLevel, min: LogLevel) -> bool {
        input_level.at_least(min)
    }

    #[test]
    fn level_filter_default_info_drops_debug_and_trace() {
        let min = DEFAULT_MIN_LEVEL;
        assert!(level_passes_filter(LogLevel::Error, min));
        assert!(level_passes_filter(LogLevel::Warn, min));
        assert!(level_passes_filter(LogLevel::Info, min));
        assert!(!level_passes_filter(LogLevel::Debug, min));
        assert!(!level_passes_filter(LogLevel::Trace, min));
    }

    #[test]
    fn level_filter_debug_session_accepts_all_above_debug() {
        let min = LogLevel::Debug;
        assert!(level_passes_filter(LogLevel::Error, min));
        assert!(level_passes_filter(LogLevel::Warn, min));
        assert!(level_passes_filter(LogLevel::Info, min));
        assert!(level_passes_filter(LogLevel::Debug, min));
        assert!(!level_passes_filter(LogLevel::Trace, min));
    }

    /// 在没有 AppHandle 的情况下测试 LogService 的诊断会话切换。
    /// 由于无法构造完整 LogService，这里模拟最小状态机。
    struct FakeSessionState {
        min_level: LogLevel,
        original_level: LogLevel,
        in_session: bool,
    }

    impl FakeSessionState {
        fn start(&mut self, new_level: LogLevel) {
            self.original_level = self.min_level;
            self.min_level = new_level;
            self.in_session = true;
        }
        fn stop(&mut self) {
            self.min_level = self.original_level;
            self.in_session = false;
        }
    }

    #[test]
    fn diagnostic_session_restores_original_level_on_stop() {
        let mut state = FakeSessionState {
            min_level: LogLevel::Info,
            original_level: LogLevel::Info,
            in_session: false,
        };
        state.start(LogLevel::Debug);
        assert_eq!(state.min_level, LogLevel::Debug);
        assert!(state.in_session);
        state.stop();
        assert_eq!(state.min_level, LogLevel::Info);
        assert!(!state.in_session);
    }
}
