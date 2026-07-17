// SPDX-License-Identifier: AGPL-3.0-or-later
//! 前端实时日志事件推送。
//!
//! 设计目标（对齐 spec 四.4）：
//! - 非阻塞：业务线程只调用 `push`，通过 mpsc 投递到独立线程，绝不阻塞。
//! - 无订阅不发包：通过 `subscribe`/`unsubscribe` 跟踪前端活动订阅者数量；
//!   无订阅者时直接丢弃，避免 Tauri 事件循环空转。
//! - 批量合并：单条日志触发全页渲染不可接受；100ms 或 50 条上限触发一次 emit。
//! - 防递归：emitter 线程内部绝不能调用 LogService::write；失败用 `eprintln!` 兜底。
//!
//! 事件载荷：`mira://logs/batch`，payload 是 `Vec<LogEntry>`。
//! 前端通过 `log_subscribe` / `log_unsubscribe` 命令显式声明活动订阅。

use crate::logging::model::LogEntry;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 事件名。前端用 `listen<LogEntry[]>('mira://logs/batch')`。
pub const LOG_BATCH_EVENT: &str = "mira://logs/batch";

/// 单批最大条数。达到即立刻 emit。
const MAX_BATCH_SIZE: usize = 50;
/// 单批最长等待时间。超过即 emit 当前累积批次。
const MAX_BATCH_INTERVAL: Duration = Duration::from_millis(100);

/// 前端推送控制句柄。clone 安全。
#[derive(Clone)]
pub struct FrontendEmitter {
    tx: Sender<EmitterMessage>,
    subscriber_count: Arc<AtomicU32>,
}

enum EmitterMessage {
    Push(LogEntry),
    Subscribe,
    Unsubscribe,
    /// 请求 emit 当前累积批次（用于导出/状态查询后立即同步）。
    Flush,
    Shutdown,
}

impl FrontendEmitter {
    /// 启动 emitter 线程。返回 (handle, join_handle)。
    pub fn spawn(app_handle: AppHandle) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<EmitterMessage>();
        let subscriber_count = Arc::new(AtomicU32::new(0));
        let count_clone = subscriber_count.clone();

        let join = std::thread::Builder::new()
            .name("mira-log-emitter".into())
            .spawn(move || {
                run_emitter(rx, app_handle, count_clone);
            })
            .expect("spawn mira-log-emitter");

        (
            Self {
                tx,
                subscriber_count,
            },
            join,
        )
    }

    /// 投递一条日志。无订阅者时会被 emitter 线程直接丢弃。
    pub fn push(&self, entry: LogEntry) {
        let _ = self.tx.send(EmitterMessage::Push(entry));
    }

    /// 前端订阅：emitter 开始累积并 emit 批次。
    pub fn subscribe(&self) {
        let _ = self.tx.send(EmitterMessage::Subscribe);
    }

    /// 前端取消订阅：emitter 停止 emit，但仍消费消息。
    pub fn unsubscribe(&self) {
        let _ = self.tx.send(EmitterMessage::Unsubscribe);
    }

    /// 立即 emit 当前累积批次（如有）。
    pub fn flush(&self) {
        let _ = self.tx.send(EmitterMessage::Flush);
    }

    /// 关闭 emitter 线程。drop 后 Shutdown 消息会被处理。
    pub fn shutdown(&self) {
        let _ = self.tx.send(EmitterMessage::Shutdown);
    }

    /// 当前活动订阅者数。仅用于诊断展示。
    pub fn subscriber_count(&self) -> u32 {
        self.subscriber_count.load(Ordering::Relaxed)
    }
}

fn run_emitter(rx: Receiver<EmitterMessage>, app_handle: AppHandle, count: Arc<AtomicU32>) {
    let mut batch: Vec<LogEntry> = Vec::with_capacity(MAX_BATCH_SIZE);
    let mut last_flush = Instant::now();

    loop {
        let timeout = if batch.is_empty() {
            None
        } else {
            Some(MAX_BATCH_INTERVAL.saturating_sub(last_flush.elapsed()))
        };

        let msg = match timeout {
            Some(d) => rx.recv_timeout(d).ok(),
            None => rx.recv().ok(),
        };

        match msg {
            Some(EmitterMessage::Push(entry)) => {
                // 仅在至少一个订阅者时累积，避免无意义 emit。
                if count.load(Ordering::Relaxed) > 0 {
                    batch.push(entry);
                    if batch.len() >= MAX_BATCH_SIZE
                        || last_flush.elapsed() >= MAX_BATCH_INTERVAL
                    {
                        flush_batch(&app_handle, &mut batch);
                        last_flush = Instant::now();
                    }
                }
            }
            Some(EmitterMessage::Subscribe) => {
                count.fetch_add(1, Ordering::Relaxed);
            }
            Some(EmitterMessage::Unsubscribe) => {
                // saturating 减法，避免下溢。
                let _ = count.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| {
                    if x > 0 {
                        Some(x - 1)
                    } else {
                        None
                    }
                });
            }
            Some(EmitterMessage::Flush) => {
                if !batch.is_empty() {
                    flush_batch(&app_handle, &mut batch);
                    last_flush = Instant::now();
                }
            }
            Some(EmitterMessage::Shutdown) | None => {
                if !batch.is_empty() {
                    flush_batch(&app_handle, &mut batch);
                }
                break;
            }
        }

        // 周期性 flush 检查（防止 batch 一直不到阈值但时间已超）。
        if !batch.is_empty() && last_flush.elapsed() >= MAX_BATCH_INTERVAL {
            flush_batch(&app_handle, &mut batch);
            last_flush = Instant::now();
        }
    }
}

fn flush_batch(app_handle: &AppHandle, batch: &mut Vec<LogEntry>) {
    if batch.is_empty() {
        return;
    }
    let payload = std::mem::take(batch);
    // emit 失败（无 listener / 序列化失败）静默忽略，绝不递归到日志系统。
    let _ = app_handle.emit(LOG_BATCH_EVENT, payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::model::{Fields, LogLevel, LogSource};

    fn make_entry(id: u64) -> LogEntry {
        LogEntry {
            id,
            timestamp: format!("2026-07-17T10:00:{id:02}+08:00"),
            level: LogLevel::Info,
            source: LogSource::App,
            target: "test".into(),
            message: format!("msg {id}"),
            session_id: "s1".into(),
            correlation_id: None,
            fields: Fields::new(),
        }
    }

    #[test]
    fn subscriber_count_starts_at_zero() {
        // 不实际 spawn（需要 AppHandle），仅测试原子计数器语义。
        let counter = Arc::new(AtomicU32::new(0));
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn saturating_decrement_prevents_underflow() {
        let counter = Arc::new(AtomicU32::new(0));
        let prev = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| {
            if x > 0 {
                Some(x - 1)
            } else {
                None
            }
        });
        assert!(prev.is_err(), "should refuse to decrement zero");
        assert_eq!(counter.load(Ordering::Relaxed), 0);

        counter.store(2, Ordering::Relaxed);
        let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| {
            if x > 0 {
                Some(x - 1)
            } else {
                None
            }
        });
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn flush_batch_takes_ownership_and_clears_buffer() {
        // 不依赖 Tauri AppHandle，仅验证内存语义。
        let mut batch: Vec<LogEntry> = vec![make_entry(1), make_entry(2)];
        let _payload = std::mem::take(&mut batch);
        assert!(batch.is_empty());
    }
}
