// SPDX-License-Identifier: AGPL-3.0-or-later
//! 有界内存环形缓冲。默认保留最近约 4_000 条；超出后丢弃最旧条目。

use crate::logging::model::LogEntry;
use std::collections::VecDeque;

/// 默认容量。可在创建时覆盖。
pub const DEFAULT_CAPACITY: usize = 4_000;

/// 最近 N 条用于状态摘要（错误/警告计数）。
const STATUS_WINDOW: usize = 500;

#[derive(Debug)]
pub struct LogBuffer {
    queue: VecDeque<LogEntry>,
    capacity: usize,
    next_id: u64,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(64);
        Self {
            queue: VecDeque::with_capacity(cap),
            capacity: cap,
            next_id: 1,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 分配下一个 ID（不插入）。
    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// 入队。超出容量时丢弃最旧条目。
    pub fn push(&mut self, entry: LogEntry) {
        if self.queue.len() >= self.capacity {
            self.queue.pop_front();
        }
        self.queue.push_back(entry);
    }

    /// 清空所有条目（保留 next_id）。
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// 删除 id < threshold 的所有条目。返回删除条数。
    pub fn drop_before_id(&mut self, threshold: u64) -> usize {
        let before = self.queue.len();
        self.queue.retain(|e| e.id >= threshold);
        before - self.queue.len()
    }

    /// 删除时间早于 cutoff 的条目。cutoff 是 RFC3339 字符串，比较按字典序即可。
    pub fn drop_older_than(&mut self, cutoff: &str) -> usize {
        let before = self.queue.len();
        self.queue.retain(|e| e.timestamp.as_str() >= cutoff);
        before - self.queue.len()
    }

    /// 删除不属于当前会话的条目。返回删除条数。
    pub fn drop_other_sessions(&mut self, current_session: &str) -> usize {
        let before = self.queue.len();
        self.queue.retain(|e| e.session_id == current_session);
        before - self.queue.len()
    }

    /// 按查询条件分页。从最新（最大 id）向最旧（最小 id）遍历。
    /// 返回 (entries, has_more, oldest_id_in_page)。
    pub fn page<Matcher: FnMut(&LogEntry) -> bool>(
        &self,
        limit: usize,
        before_id: Option<u64>,
        mut matcher: Matcher,
    ) -> (Vec<LogEntry>, bool, Option<u64>) {
        let limit = limit.max(1);
        let mut picked: Vec<LogEntry> = Vec::with_capacity(limit);
        let mut oldest: Option<u64> = None;
        // VecDeque 从后向前遍历：从最新到最旧。
        for entry in self.queue.iter().rev() {
            // 游标：仅取 id < before_id。
            if let Some(boundary) = before_id {
                if entry.id >= boundary {
                    continue;
                }
            }
            if !matcher(entry) {
                continue;
            }
            if oldest.is_none() {
                oldest = Some(entry.id);
            }
            picked.push(entry.clone());
            if picked.len() >= limit {
                break;
            }
        }
        // has_more：在剩余范围内是否还有匹配项。
        let mut has_more = false;
        if picked.len() >= limit {
            let last_id = picked.last().map(|e| e.id).unwrap_or(0);
            for entry in self.queue.iter().rev() {
                if entry.id >= last_id {
                    continue;
                }
                if let Some(boundary) = before_id {
                    if entry.id >= boundary {
                        continue;
                    }
                }
                if matcher(entry) {
                    has_more = true;
                    break;
                }
            }
        }
        (picked, has_more, oldest)
    }

    /// 最近 N 条中按等级计数。
    pub fn recent_counts(&self) -> (usize, usize) {
        let mut errors = 0;
        let mut warns = 0;
        let take = self.queue.len().min(STATUS_WINDOW);
        for entry in self.queue.iter().rev().take(take) {
            match entry.level {
                crate::logging::model::LogLevel::Error => errors += 1,
                crate::logging::model::LogLevel::Warn => warns += 1,
                _ => {}
            }
        }
        (errors, warns)
    }

    /// 返回所有条目（用于导出）。克隆成本可控（条数有界）。
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.queue.iter().cloned().collect()
    }

    /// 返回当前会话的所有条目。
    pub fn snapshot_for_session(&self, session_id: &str) -> Vec<LogEntry> {
        self.queue
            .iter()
            .filter(|e| e.session_id == session_id)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::model::{Fields, LogLevel, LogSource};

    fn make_entry(id: u64, level: LogLevel, target: &str, session: &str) -> LogEntry {
        LogEntry {
            id,
            timestamp: format!("2026-07-17T10:00:{id:02}+08:00"),
            level,
            source: LogSource::App,
            target: target.into(),
            message: format!("message {id}"),
            session_id: session.into(),
            correlation_id: None,
            fields: Fields::new(),
        }
    }

    #[test]
    fn push_drops_oldest_when_full() {
        let mut buf = LogBuffer::new(3);
        buf.push(make_entry(1, LogLevel::Info, "a", "s1"));
        buf.push(make_entry(2, LogLevel::Info, "a", "s1"));
        buf.push(make_entry(3, LogLevel::Info, "a", "s1"));
        buf.push(make_entry(4, LogLevel::Info, "a", "s1"));
        assert_eq!(buf.len(), 3);
        let snap = buf.snapshot();
        assert_eq!(snap[0].id, 2);
        assert_eq!(snap[2].id, 4);
    }

    #[test]
    fn next_id_is_monotonic() {
        let mut buf = LogBuffer::new(10);
        assert_eq!(buf.next_id(), 1);
        assert_eq!(buf.next_id(), 2);
        assert_eq!(buf.next_id(), 3);
    }

    #[test]
    fn page_returns_latest_first() {
        let mut buf = LogBuffer::new(10);
        for i in 1..=5 {
            buf.push(make_entry(i, LogLevel::Info, "a", "s1"));
        }
        let (page, has_more, oldest) = buf.page(3, None, |_| true);
        assert_eq!(page.len(), 3);
        // 最新三条：id 5, 4, 3
        assert_eq!(page[0].id, 5);
        assert_eq!(page[2].id, 3);
        assert!(has_more);
        assert_eq!(oldest, Some(5));
    }

    #[test]
    fn page_respects_cursor() {
        let mut buf = LogBuffer::new(10);
        for i in 1..=5 {
            buf.push(make_entry(i, LogLevel::Info, "a", "s1"));
        }
        let (page, has_more, _) = buf.page(10, Some(3), |_| true);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, 2);
        assert_eq!(page[1].id, 1);
        assert!(!has_more);
    }

    #[test]
    fn page_applies_matcher() {
        let mut buf = LogBuffer::new(10);
        buf.push(make_entry(1, LogLevel::Info, "device", "s1"));
        buf.push(make_entry(2, LogLevel::Error, "device", "s1"));
        buf.push(make_entry(3, LogLevel::Info, "plugin", "s1"));
        buf.push(make_entry(4, LogLevel::Error, "plugin", "s1"));

        let (page, _, _) = buf.page(10, None, |e| e.level == LogLevel::Error);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, 4);
        assert_eq!(page[1].id, 2);
    }

    #[test]
    fn drop_before_id_removes_older_entries() {
        let mut buf = LogBuffer::new(10);
        for i in 1..=5 {
            buf.push(make_entry(i, LogLevel::Info, "a", "s1"));
        }
        let removed = buf.drop_before_id(3);
        assert_eq!(removed, 2);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn drop_older_than_removes_by_timestamp() {
        let mut buf = LogBuffer::new(10);
        for i in 1..=5 {
            buf.push(make_entry(i, LogLevel::Info, "a", "s1"));
        }
        // 删除时间早于 "2026-07-17T10:00:03" 的条目。
        let removed = buf.drop_older_than("2026-07-17T10:00:03+08:00");
        assert_eq!(removed, 2);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn drop_other_sessions_keeps_only_current() {
        let mut buf = LogBuffer::new(10);
        buf.push(make_entry(1, LogLevel::Info, "a", "old"));
        buf.push(make_entry(2, LogLevel::Info, "a", "current"));
        buf.push(make_entry(3, LogLevel::Info, "a", "current"));
        let removed = buf.drop_other_sessions("current");
        assert_eq!(removed, 1);
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn recent_counts_aggregates_errors_and_warns() {
        let mut buf = LogBuffer::new(100);
        buf.push(make_entry(1, LogLevel::Error, "a", "s1"));
        buf.push(make_entry(2, LogLevel::Warn, "a", "s1"));
        buf.push(make_entry(3, LogLevel::Info, "a", "s1"));
        buf.push(make_entry(4, LogLevel::Error, "a", "s1"));
        buf.push(make_entry(5, LogLevel::Warn, "a", "s1"));
        let (errors, warns) = buf.recent_counts();
        assert_eq!(errors, 2);
        assert_eq!(warns, 2);
    }

    #[test]
    fn snapshot_for_session_filters_correctly() {
        let mut buf = LogBuffer::new(10);
        buf.push(make_entry(1, LogLevel::Info, "a", "old"));
        buf.push(make_entry(2, LogLevel::Info, "a", "current"));
        buf.push(make_entry(3, LogLevel::Info, "a", "current"));
        let snap = buf.snapshot_for_session("current");
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].id, 2);
        assert_eq!(snap[1].id, 3);
    }

    #[test]
    fn capacity_minimum_is_enforced() {
        let buf = LogBuffer::new(10);
        assert_eq!(buf.capacity(), 10);
        let buf = LogBuffer::new(0);
        assert_eq!(buf.capacity(), 64);
    }
}
