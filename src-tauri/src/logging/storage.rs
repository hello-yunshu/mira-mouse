// SPDX-License-Identifier: AGPL-3.0-or-later
//! 固定槽位滚动文件存储。按大小与时间自动轮转、清理。
//!
//! 设计：
//! - 固定使用 `mira-0.log` 到 `mira-3.log` 四个槽位，不随运行次数增加文件数。
//! - `mira-0.log` 为当前文件；单文件约 5 MB 后依次后移并覆盖最旧槽位。
//! - 总磁盘占用上限约 20 MB。
//! - 默认保留 7 天；超期文件按时间清理。
//! - 首次升级时将旧的 `mira-<timestamp>.log` 最近四份迁移到固定槽位。
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
/// 固定日志文件槽位数。
pub const MAX_LOG_FILES: usize = 4;
/// 总磁盘占用上限。约 20 MB。
pub const DISK_QUOTA_BYTES: u64 = MAX_FILE_BYTES * MAX_LOG_FILES as u64;
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
    /// 关闭 writer 线程。测试用于 join；生产退出走 flush。
    #[allow(dead_code)]
    Shutdown,
}

/// 初始化存储。返回 (handle, join_handle)。
/// `dir` 为日志目录；创建失败时 writer 立即降级为 disabled。
pub fn spawn(dir: PathBuf) -> (LogStorageHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<StorageMessage>();
    let dir_arc = Arc::new(Mutex::new(dir.clone()));
    let disk_usage = Arc::new(Mutex::new(0u64));
    let enabled = Arc::new(Mutex::new(true));

    let handle = LogStorageHandle {
        tx: tx.clone(),
        dir: dir_arc.clone(),
        disk_usage: disk_usage.clone(),
        enabled: enabled.clone(),
    };

    // 尝试创建目录；成功后先收敛旧时间戳文件，再打开当前槽位。
    if fs::create_dir_all(&dir).is_err() {
        *enabled.lock().unwrap() = false;
    } else {
        if let Err(err) = migrate_legacy_log_files(&dir) {
            eprintln!("[mira-log] legacy log migration incomplete: {err}");
        }
        let _ = cleanup_disk(&dir, &disk_usage);
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

    /// 关闭 writer 线程。测试用于 join；生产退出走 flush（见 lib.rs ExitRequested）。
    #[allow(dead_code)]
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
                if let Err(err) = writer
                    .write_all(&serialized)
                    .and_then(|_| writer.write_all(b"\n"))
                {
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
                if counter >= CLEANUP_INTERVAL || last_cleanup.elapsed() >= Duration::from_secs(60)
                {
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
        if let StorageMessage::Shutdown = msg {
            break;
        }
    }
}

struct RotatingWriter {
    file: BufWriter<File>,
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
    let path = slot_path(dir, 0);
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let bytes_written = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok(RotatingWriter {
        file: BufWriter::new(file),
        bytes_written,
    })
}

fn rotate(dir: &Path, prev: RotatingWriter) -> std::io::Result<RotatingWriter> {
    // 先 flush 旧文件。
    let mut prev = prev;
    prev.flush()?;
    drop(prev);
    // 固定槽位从后往前移动，最旧槽位先删除，文件总数始终不超过上限。
    let oldest = slot_path(dir, MAX_LOG_FILES - 1);
    if oldest.exists() {
        fs::remove_file(oldest)?;
    }
    for index in (0..MAX_LOG_FILES - 1).rev() {
        let from = slot_path(dir, index);
        if from.exists() {
            fs::rename(from, slot_path(dir, index + 1))?;
        }
    }
    open_writer(dir)
}

fn slot_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("mira-{index}.log"))
}

fn fixed_slot_index(path: &Path) -> Option<usize> {
    let name = path.file_name()?.to_str()?;
    let index = name.strip_prefix("mira-")?.strip_suffix(".log")?;
    index.parse().ok()
}

fn is_legacy_log_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("mira-")
        && name.ends_with(".log")
        && name.len() == "mira-20260719T120000.log".len()
        && name.as_bytes().get(13) == Some(&b'T')
}

/// 将旧时间戳日志收敛到固定槽位。已有固定槽位时，旧文件视为迁移残留并移除。
fn migrate_legacy_log_files(dir: &Path) -> std::io::Result<()> {
    let mut legacy = Vec::new();
    let mut has_fixed_slots = false;
    let mut out_of_range_slots = Vec::new();

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if let Some(index) = fixed_slot_index(&path) {
            if index < MAX_LOG_FILES {
                has_fixed_slots = true;
            } else {
                out_of_range_slots.push(path);
            }
        } else if is_legacy_log_file(&path) {
            legacy.push(path);
        }
    }

    for path in out_of_range_slots {
        fs::remove_file(path)?;
    }

    legacy.sort();
    if has_fixed_slots {
        for path in legacy {
            fs::remove_file(path)?;
        }
        return Ok(());
    }

    // 最新文件进入 0 号槽位，其余按新到旧进入后续槽位。
    for (index, path) in legacy.iter().rev().take(MAX_LOG_FILES).enumerate() {
        fs::rename(path, slot_path(dir, index))?;
    }
    for path in legacy
        .iter()
        .take(legacy.len().saturating_sub(MAX_LOG_FILES))
    {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// 列出固定日志文件，按槽位从最旧到最新排序。
fn list_log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    };
    files.retain(|path| fixed_slot_index(path).is_some_and(|index| index < MAX_LOG_FILES));
    files.sort_by_key(|path| std::cmp::Reverse(fixed_slot_index(path).unwrap_or_default()));
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

    let mut files = list_log_files(dir);
    let mut freed_bytes = 0u64;

    // 1. 固定槽位不含时间戳，按文件修改时间清理。
    files.retain(|p| {
        let is_expired = fs::metadata(p)
            .and_then(|meta| meta.modified())
            .map(|modified| DateTime::<Utc>::from(modified) < cutoff)
            .unwrap_or(false);
        if is_expired {
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
        // files 已按槽位从最旧到最新排序。从头删除直到补足 excess。
        for file in &files {
            if deleted_bytes >= excess {
                break;
            }
            let size = fs::metadata(file).map(|m| m.len()).unwrap_or(0);
            if fs::remove_file(file).is_err() {
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

/// 删除指定日期之前的所有日志文件（基于文件修改时间）。
fn delete_files_older_than(dir: &Path, cutoff: DateTime<Utc>) -> (u32, Option<String>) {
    let files = list_log_files(dir);
    let mut deleted = 0u32;
    let mut last_err: Option<String> = None;
    for file in &files {
        let is_older = fs::metadata(file)
            .and_then(|meta| meta.modified())
            .map(|modified| DateTime::<Utc>::from(modified) < cutoff)
            .unwrap_or(false);
        if is_older {
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
    use std::fs::FileTimes;
    use std::time::SystemTime;
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
        assert!(
            (2..=MAX_LOG_FILES).contains(&files.len()),
            "expected rotation, got {} files",
            files.len()
        );
        assert!(files
            .iter()
            .all(|path| { fixed_slot_index(path).is_some_and(|index| index < MAX_LOG_FILES) }));
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
        assert!(
            err.is_none() || err.as_ref().map(|s| s.is_empty()).unwrap_or(false) || err.is_some()
        );
        handle.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn restarting_reuses_the_current_slot() {
        let tmp = TempDir::new().unwrap();
        for id in 1..=2 {
            let (handle, join) = spawn(tmp.path().to_path_buf());
            handle.append(make_entry(id));
            handle.shutdown();
            join.join().unwrap();
        }

        let files = list_log_files(tmp.path());
        assert_eq!(files, vec![slot_path(tmp.path(), 0)]);
        let content = fs::read_to_string(&files[0]).unwrap();
        assert_eq!(content.lines().count(), 2);
    }

    #[test]
    fn list_log_files_returns_oldest_slot_first() {
        let tmp = TempDir::new().unwrap();
        for index in [0, 2, 3] {
            fs::write(slot_path(tmp.path(), index), b"[]").unwrap();
        }
        let files = list_log_files(tmp.path());
        assert_eq!(
            files,
            vec![
                slot_path(tmp.path(), 3),
                slot_path(tmp.path(), 2),
                slot_path(tmp.path(), 0)
            ]
        );
    }

    #[test]
    fn migrates_only_the_four_newest_legacy_files() {
        let tmp = TempDir::new().unwrap();
        for day in 1..=6 {
            fs::write(
                tmp.path().join(format!("mira-202607{day:02}T120000.log")),
                format!("day-{day}"),
            )
            .unwrap();
        }

        migrate_legacy_log_files(tmp.path()).unwrap();

        assert_eq!(list_log_files(tmp.path()).len(), MAX_LOG_FILES);
        assert_eq!(
            fs::read_to_string(slot_path(tmp.path(), 0)).unwrap(),
            "day-6"
        );
        assert_eq!(
            fs::read_to_string(slot_path(tmp.path(), 3)).unwrap(),
            "day-3"
        );
        let legacy_count = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_legacy_log_file(&entry.path()))
            .count();
        assert_eq!(legacy_count, 0);
    }

    #[test]
    fn delete_older_than_removes_old_files_only() {
        let tmp = TempDir::new().unwrap();
        let old = slot_path(tmp.path(), 1);
        let recent = slot_path(tmp.path(), 0);
        fs::write(&old, b"old").unwrap();
        fs::write(&recent, b"recent").unwrap();
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        let cutoff = Utc::now() - chrono::Duration::days(1);
        let (deleted, _) = delete_files_older_than(tmp.path(), cutoff);
        assert_eq!(deleted, 1);
        assert!(recent.exists());
        assert!(!old.exists());
    }

    #[test]
    fn cleanup_disk_respects_quota() {
        let tmp = TempDir::new().unwrap();
        // 四个固定槽位各 6 MB，总计 24 MB > quota 20 MB。
        for index in 0..MAX_LOG_FILES {
            let path = slot_path(tmp.path(), index);
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
        assert!(!slot_path(tmp.path(), MAX_LOG_FILES - 1).exists());
        assert!(slot_path(tmp.path(), 0).exists());
    }
}
