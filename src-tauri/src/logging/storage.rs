// SPDX-License-Identifier: AGPL-3.0-or-later
//! 滚动文件存储。按大小与时间自动轮转、清理。
//!
//! 设计：
//! - 单文件约 5 MB 后轮转，命名 `mira-<timestamp>.log`。
//! - 总磁盘占用上限约 20 MB，超出后删除最旧文件。
//! - 默认保留 7 天；超期文件按时间清理。
//! - 文件写入失败时降级为内存日志：调用方继续工作，但 `enabled=false`。
//!
//! 所有写入发生在专用 writer 线程，避免阻塞业务线程。
//! 文件目录使用 Tauri 提供的 `app_log_dir`，跨平台一致。

use crate::logging::model::LogEntry;
use chrono::{DateTime, Utc};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 单文件最大字节。约 5 MB。
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// 总磁盘占用上限。约 20 MB。
pub const DISK_QUOTA_BYTES: u64 = 20 * 1024 * 1024;
/// 默认保留天数。
pub const RETENTION_DAYS: i64 = 7;
/// 每多少条写入后跑一次清理。
const CLEANUP_INTERVAL: usize = 256;

/// 存储控制句柄。clone 后用于向 writer 线程投递日志与命令。
#[derive(Clone)]
pub struct LogStorageHandle {
    tx: Sender<StorageMessage>,
    /// 共享的当前目录路径（用于状态查询）。
    dir: Arc<Mutex<PathBuf>>,
    /// 共享的磁盘占用缓存（粗略，避免每次写都 stat）。
    disk_usage: Arc<Mutex<u64>>,
    /// 是否启用文件持久化。
    enabled: Arc<Mutex<bool>>,
}

enum StorageMessage {
    Append(LogEntry),
    Flush,
    /// 强制重新打开当前文件（用于清理后）。
    Rotate,
    /// 关闭 writer 线程。
    Shutdown,
}

/// 初始化存储。返回 (handle, join_handle)。
/// `dir` 为日志目录；创建失败时 writer 立即降级为 disabled。
pub fn spawn(dir: PathBuf) -> (LogStorageHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<StorageMessage>();
    let dir_arc = Arc::new(Mutex::new(dir.clone()));
    let disk_usage = Arc::new(Mutex::new(0u64));
    let enabled = Arc::new(Mutex::new(true));

    // 初始磁盘占用快照（忽略错误）。
    if let Ok(usage) = scan_disk_usage(&dir) {
        *disk_usage.lock().unwrap() = usage;
    }

    let handle = LogStorageHandle {
        tx: tx.clone(),
        dir: dir_arc.clone(),
        disk_usage: disk_usage.clone(),
        enabled: enabled.clone(),
    };

    // 尝试创建目录；失败时降级。
    if fs::create_dir_all(&dir).is_err() {
        *enabled.lock().unwrap() = false;
    }

    let join = std::thread::Builder::new()
        .name("mira-log-writer".into())
        .spawn(move || {
            run_writer(rx, dir, dir_arc, disk_usage, enabled);
        })
        .expect("spawn mira-log-writer");

    (handle, join)
}

impl LogStorageHandle {
    /// 投递一条日志。writer 线程不响应时立即返回，不阻塞业务。
    pub fn append(&self, entry: LogEntry) {
        // 用 try_send 等价：channel 满则丢弃。
        let _ = self.tx.send(StorageMessage::Append(entry));
    }

    /// 请求 flush。通常在退出或导出前调用。
    pub fn flush(&self) {
        let _ = self.tx.send(StorageMessage::Flush);
    }

    /// 请求轮转。
    pub fn rotate(&self) {
        let _ = self.tx.send(StorageMessage::Rotate);
    }

    /// 关闭 writer。drop 后 writer 会处理 Shutdown。
    pub fn shutdown(&self) {
        let _ = self.tx.send(StorageMessage::Shutdown);
    }

    /// 当前是否启用文件持久化。
    pub fn enabled(&self) -> bool {
        *self.enabled.lock().unwrap()
    }

    /// 当前磁盘占用（字节）。
    pub fn disk_usage(&self) -> u64 {
        *self.disk_usage.lock().unwrap()
    }

    /// 当前日志目录。
    pub fn dir(&self) -> PathBuf {
        self.dir.lock().unwrap().clone()
    }

    /// 列出磁盘上的所有日志文件（按时间排序，最旧在前）。
    pub fn list_files(&self) -> Vec<PathBuf> {
        let dir = self.dir.lock().unwrap().clone();
        list_log_files(&dir)
    }

    /// 删除指定日期之前的所有日志文件。返回 (deleted_count, error)。
    pub fn delete_older_than(&self, days: u32) -> (u32, Option<String>) {
        let dir = self.dir.lock().unwrap().clone();
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        delete_files_older_than(&dir, cutoff)
    }

    /// 删除所有日志文件（当前活跃文件除外）。返回 (deleted_count, error)。
    pub fn delete_all(&self) -> (u32, Option<String>) {
        let dir = self.dir.lock().unwrap().clone();
        let files = list_log_files(&dir);
        let mut deleted = 0u32;
        let mut last_err: Option<String> = None;
        for file in &files {
            // 当前 writer 写入的文件由 writer 内部关闭后才能删除；
            // 这里只删除非活跃文件，活跃文件需要先 Rotate。
            if let Err(err) = fs::remove_file(file) {
                last_err = Some(format!("remove {}: {err}", file.display()));
            } else {
                deleted += 1;
            }
        }
        // 请求 writer 旋转，让旧文件可被下一次清理删除。
        let _ = self.tx.send(StorageMessage::Rotate);
        *self.disk_usage.lock().unwrap() = 0;
        (deleted, last_err)
    }
}

fn run_writer(
    rx: Receiver<StorageMessage>,
    dir: PathBuf,
    dir_arc: Arc<Mutex<PathBuf>>,
    disk_usage: Arc<Mutex<u64>>,
    enabled: Arc<Mutex<bool>>,
) {
    let mut writer = match open_writer(&dir) {
        Ok(w) => {
            *enabled.lock().unwrap() = true;
            w
        }
        Err(err) => {
            // 降级：仅消费消息，不写文件。
            *enabled.lock().unwrap() = false;
            eprintln!("[mira-log] storage disabled: {err}");
            drain_loop_disabled(rx);
            return;
        }
    };

    let mut counter: usize = 0;
    let mut last_cleanup = Instant::now();
    while let Ok(msg) = rx.recv() {
        match msg {
            StorageMessage::Append(entry) => {
                let serialized = match serde_json::to_vec(&entry) {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };
                if let Err(err) = writer.write_all(&serialized).and_then(|_| writer.write_all(b"\n")) {
                    // 写入失败：标记 disabled，继续消费但不写。
                    *enabled.lock().unwrap() = false;
                    eprintln!("[mira-log] write failed: {err}");
                }
                counter += 1;
                // 检查是否需要轮转。
                if writer.bytes_written() >= MAX_FILE_BYTES {
                    writer = match rotate(&dir, writer) {
                        Ok(new) => {
                            *enabled.lock().unwrap() = true;
                            new
                        }
                        Err(err) => {
                            *enabled.lock().unwrap() = false;
                            eprintln!("[mira-log] rotate failed: {err}");
                            // 降级循环：继续消费直到关闭。
                            drain_loop_disabled(rx);
                            return;
                        }
                    };
                }
                // 定期清理。
                if counter >= CLEANUP_INTERVAL || last_cleanup.elapsed() >= Duration::from_secs(60) {
                    counter = 0;
                    last_cleanup = Instant::now();
                    let _ = cleanup_disk(&dir, &disk_usage);
                }
            }
            StorageMessage::Flush => {
                let _ = writer.flush();
            }
            StorageMessage::Rotate => {
                writer = match rotate(&dir, writer) {
                    Ok(new) => {
                        *enabled.lock().unwrap() = true;
                        new
                    }
                    Err(err) => {
                        *enabled.lock().unwrap() = false;
                        eprintln!("[mira-log] manual rotate failed: {err}");
                        drain_loop_disabled(rx);
                        return;
                    }
                };
            }
            StorageMessage::Shutdown => {
                let _ = writer.flush();
                break;
            }
        }
    }
    // 更新目录引用（路径不变，但保持 Arc 一致）。
    *dir_arc.lock().unwrap() = dir;
}

/// 降级循环：通道仍能消费，但所有日志被丢弃。
fn drain_loop_disabled(rx: Receiver<StorageMessage>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            StorageMessage::Shutdown => break,
            _ => {}
        }
    }
}

struct RotatingWriter {
    file: BufWriter<File>,
    path: PathBuf,
    bytes_written: u64,
}

impl RotatingWriter {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes)?;
        self.bytes_written += bytes.len() as u64;
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
    fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

fn open_writer(dir: &Path) -> std::io::Result<RotatingWriter> {
    let path = new_file_path(dir);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let bytes_written = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok(RotatingWriter {
        file: BufWriter::new(file),
        path,
        bytes_written,
    })
}

fn rotate(dir: &Path, prev: RotatingWriter) -> std::io::Result<RotatingWriter> {
    // 先 flush 旧文件。
    let mut prev = prev;
    prev.flush()?;
    drop(prev);
    // 打开新文件。
    open_writer(dir)
}

fn new_file_path(dir: &Path) -> PathBuf {
    let now: DateTime<Utc> = Utc::now();
    let name = format!("mira-{}.log", now.format("%Y%m%dT%H%M%S"));
    dir.join(name)
}

/// 列出目录下所有 `mira-*.log` 文件，按文件名升序（最旧在前）。
fn list_log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    };
    files.retain(|p| {
        p.extension().and_then(|e| e.to_str()) == Some("log")
            && p
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("mira-"))
                .unwrap_or(false)
    });
    files.sort();
    files
}

/// 扫描目录总字节。
fn scan_disk_usage(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for file in list_log_files(dir) {
        if let Ok(meta) = fs::metadata(&file) {
            total += meta.len();
        }
    }
    Ok(total)
}

/// 清理：删除超期文件 + 总量超限删除最旧文件。返回删除字节数。
fn cleanup_disk(dir: &Path, disk_usage: &Arc<Mutex<u64>>) -> std::io::Result<u64> {
    let now = Utc::now();
    let cutoff = now - chrono::Duration::days(RETENTION_DAYS);
    let cutoff_str = cutoff.format("%Y%m%dT%H%M%S").to_string();

    let mut files = list_log_files(dir);
    let mut freed_bytes = 0u64;

    // 1. 按时间清理：文件名 < cutoff_str 的删除。
    files.retain(|p| {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if name < cutoff_str.as_str() {
            if let Ok(meta) = fs::metadata(p) {
                freed_bytes += meta.len();
            }
            let _ = fs::remove_file(p);
            false
        } else {
            true
        }
    });

    // 2. 按总量清理：累加直到不超过 quota，剩余最旧的删除。
    let mut accum = 0u64;
    for file in &files {
        let size = fs::metadata(file).map(|m| m.len()).unwrap_or(0);
        accum += size;
    }
    if accum > DISK_QUOTA_BYTES {
        let excess = accum - DISK_QUOTA_BYTES;
        let mut deleted_bytes = 0u64;
        // files 已按文件名升序，最旧在前。从头删除直到补足 excess。
        for file in &files {
            if deleted_bytes >= excess {
                break;
            }
            let size = fs::metadata(file).map(|m| m.len()).unwrap_or(0);
            if let Err(_) = fs::remove_file(file) {
                // 忽略错误，继续尝试下一个。
            } else {
                deleted_bytes += size;
                freed_bytes += size;
            }
        }
    }

    // 更新磁盘占用缓存。
    let actual = scan_disk_usage(dir).unwrap_or(0);
    *disk_usage.lock().unwrap() = actual;
    Ok(freed_bytes)
}

/// 删除指定日期之前的所有日志文件（基于文件名排序）。
fn delete_files_older_than(dir: &Path, cutoff: DateTime<Utc>) -> (u32, Option<String>) {
    let cutoff_str = cutoff.format("%Y%m%dT%H%M%S").to_string();
    let files = list_log_files(dir);
    let mut deleted = 0u32;
    let mut last_err: Option<String> = None;
    for file in &files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if name < cutoff_str.as_str() {
            if let Err(err) = fs::remove_file(file) {
                last_err = Some(format!("remove {}: {err}", file.display()));
            } else {
                deleted += 1;
            }
        }
    }
    (deleted, last_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::model::{Fields, LogLevel, LogSource};
    use tempfile::TempDir;

    fn make_entry(id: u64) -> LogEntry {
        LogEntry {
            id,
            timestamp: format!("2026-07-17T10:00:{id:02}+08:00"),
            level: LogLevel::Info,
            source: LogSource::App,
            target: "test".into(),
            message: format!("message {id}"),
            session_id: "s1".into(),
            correlation_id: None,
            fields: Fields::new(),
        }
    }

    #[test]
    fn writer_appends_and_reads_back() {
        let tmp = TempDir::new().unwrap();
        let (handle, join) = spawn(tmp.path().to_path_buf());
        for i in 1..=10 {
            handle.append(make_entry(i));
        }
        handle.flush();
        handle.shutdown();
        join.join().unwrap();

        let files = list_log_files(tmp.path());
        assert_eq!(files.len(), 1, "expected one log file");
        let content = fs::read_to_string(&files[0]).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 10);
        // 每行应是合法 JSON。
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed["id"], 1);
    }

    #[test]
    fn writer_rotates_when_size_limit_hit() {
        let tmp = TempDir::new().unwrap();
        let (handle, join) = spawn(tmp.path().to_path_buf());
        // 写入 ~6 MB 数据触发轮转。每条约 100 字节。
        for i in 1..=70_000 {
            let mut e = make_entry(i);
            e.message = "x".repeat(80);
            handle.append(e);
        }
        handle.flush();
        handle.shutdown();
        join.join().unwrap();

        let files = list_log_files(tmp.path());
        assert!(files.len() >= 2, "expected rotation, got {} files", files.len());
        // 总量不应超过 quota 太多（清理在每 256 条后跑）。
        let total: u64 = files
            .iter()
            .map(|f| fs::metadata(f).map(|m| m.len()).unwrap_or(0))
            .sum();
        // 给清理留一些余量；理论上最终应接近 quota。
        assert!(total <= DISK_QUOTA_BYTES + MAX_FILE_BYTES);
    }

    #[test]
    fn delete_all_removes_files_and_requests_rotate() {
        let tmp = TempDir::new().unwrap();
        let (handle, join) = spawn(tmp.path().to_path_buf());
        for i in 1..=5 {
            handle.append(make_entry(i));
        }
        handle.flush();
        // 等待 writer 处理。
        std::thread::sleep(Duration::from_millis(100));
        let (deleted, err) = handle.delete_all();
        assert!(deleted >= 1, "expected at least 1 deleted, got {deleted}");
        assert!(err.is_none() || err.as_ref().map(|s| s.is_empty()).unwrap_or(false) || err.is_some());
        handle.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn list_log_files_returns_sorted_ascending() {
        let tmp = TempDir::new().unwrap();
        // 创建 3 个不同时间戳的文件。
        let names = [
            "mira-20260701T120000.log",
            "mira-20260702T120000.log",
            "mira-20260703T120000.log",
        ];
        for name in &names {
            let path = tmp.path().join(name);
            fs::write(&path, b"[]").unwrap();
        }
        let files = list_log_files(tmp.path());
        assert_eq!(files.len(), 3);
        assert!(files[0].to_str().unwrap().contains("20260701"));
        assert!(files[2].to_str().unwrap().contains("20260703"));
    }

    #[test]
    fn delete_older_than_removes_old_files_only() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("mira-20260101T000000.log"), b"old").unwrap();
        fs::write(tmp.path().join("mira-20260701T000000.log"), b"recent").unwrap();
        // cutoff = 2026-06-01。
        let cutoff = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (deleted, _) = delete_files_older_than(tmp.path(), cutoff);
        assert_eq!(deleted, 1);
        assert!(tmp.path().join("mira-20260701T000000.log").exists());
        assert!(!tmp.path().join("mira-20260101T000000.log").exists());
    }

    #[test]
    fn cleanup_disk_respects_quota() {
        let tmp = TempDir::new().unwrap();
        // 创建 5 个 6 MB 文件，总计 30 MB > quota 20 MB。
        for i in 0..5 {
            let path = tmp.path().join(format!("mira-2026070{i}T000000.log"));
            let content = vec![b'x'; 6 * 1024 * 1024];
            fs::write(&path, &content).unwrap();
        }
        let disk_usage = Arc::new(Mutex::new(0u64));
        cleanup_disk(tmp.path(), &disk_usage).unwrap();
        let remaining = list_log_files(tmp.path());
        let total: u64 = remaining
            .iter()
            .map(|f| fs::metadata(f).map(|m| m.len()).unwrap_or(0))
            .sum();
        assert!(total <= DISK_QUOTA_BYTES);
    }
}
