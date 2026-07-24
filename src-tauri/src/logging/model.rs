// SPDX-License-Identifier: AGPL-3.0-or-later
//! 日志统一数据模型。前后端共享语义，前端通过 `log-types.ts` 镜像定义。

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// 日志等级。语义统一：
/// - `Error`：操作失败、不可恢复异常、关键组件启动失败
/// - `Warn`：已回退、已重试、状态异常但应用仍能继续
/// - `Info`：低频生命周期、用户触发的关键操作结果、版本与状态变化
/// - `Debug`：诊断所需的流程、耗时、分支选择和结构化上下文
/// - `Trace`：极低层且高频的细节，仅短时诊断使用
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// 数字权重，便于比较。`Error=0` 最高，`Trace=4` 最低。
    pub fn weight(self) -> u8 {
        match self {
            LogLevel::Error => 0,
            LogLevel::Warn => 1,
            LogLevel::Info => 2,
            LogLevel::Debug => 3,
            LogLevel::Trace => 4,
        }
    }

    /// 是否 ≥ 指定最低等级。
    pub fn at_least(self, min: LogLevel) -> bool {
        self.weight() <= min.weight()
    }

    /// 小写字符串形式，与 serde 序列化结果一致。
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialOrd for LogLevel {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LogLevel {
    fn cmp(&self, other: &Self) -> Ordering {
        // 权重小 = 等级高。这里按权重升序，便于排序与范围查询。
        self.weight().cmp(&other.weight())
    }
}

/// 日志来源。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogSource {
    #[default]
    App,
    Frontend,
    Plugin,
    LocalAi,
}

/// 字段值类型：可序列化、有界、无递归。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldValue {
    Text(String),
    Number(f64),
    Integer(i64),
    Boolean(bool),
    Null,
}

/// 字段表。键名为字符串，值受 `FieldValue` 限制。
pub type Fields = std::collections::BTreeMap<String, FieldValue>;

/// 单条日志条目。结构稳定，可序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// 单调递增的 ID（同会话内）。便于前端去重与排序。
    pub id: u64,
    /// RFC3339 时间戳（UTC + 偏移），前端可本地化显示。
    pub timestamp: String,
    pub level: LogLevel,
    pub source: LogSource,
    /// 模块或子系统，如 `device::discover`、`plugin::verify`、`local_ai::predict`。
    pub target: String,
    pub message: String,
    /// 应用启动时生成的会话 ID，跨重启隔离。
    pub session_id: String,
    /// 跨模块操作关联 ID，可选。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// 结构化附加字段。
    #[serde(skip_serializing_if = "Fields::is_empty", default)]
    pub fields: Fields,
}

/// 写入日志时的输入。由调用方提供必要信息，由服务端填充 id/timestamp/sessionId。
///
/// 同时作为 `log_write` Tauri 命令的入参，因此需要 `Deserialize`。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogInput {
    pub level: LogLevel,
    pub source: LogSource,
    pub target: String,
    pub message: String,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub fields: Fields,
}

impl LogInput {
    /// 创建一条最简日志输入。
    pub fn new(
        level: LogLevel,
        source: LogSource,
        target: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            level,
            source,
            target: target.into(),
            message: message.into(),
            correlation_id: None,
            fields: Fields::new(),
        }
    }
}

impl From<&str> for FieldValue {
    fn from(value: &str) -> Self {
        FieldValue::Text(value.to_string())
    }
}

impl From<String> for FieldValue {
    fn from(value: String) -> Self {
        FieldValue::Text(value)
    }
}

impl From<i64> for FieldValue {
    fn from(value: i64) -> Self {
        FieldValue::Integer(value)
    }
}

impl From<u64> for FieldValue {
    fn from(value: u64) -> Self {
        // u64 可能超出 i64 范围；保底转 f64。
        if value <= i64::MAX as u64 {
            FieldValue::Integer(value as i64)
        } else {
            FieldValue::Number(value as f64)
        }
    }
}

impl From<f64> for FieldValue {
    fn from(value: f64) -> Self {
        FieldValue::Number(value)
    }
}

impl From<bool> for FieldValue {
    fn from(value: bool) -> Self {
        FieldValue::Boolean(value)
    }
}

/// 判断两个 FieldValue 是否相等（数值宽松匹配：Integer 与 Number 数值相等时视为相等）。
/// 用于 LogQuery::fields_exact 过滤。
fn field_values_equal(a: &FieldValue, b: &FieldValue) -> bool {
    match (a, b) {
        (FieldValue::Text(x), FieldValue::Text(y)) => x == y,
        (FieldValue::Boolean(x), FieldValue::Boolean(y)) => x == y,
        (FieldValue::Integer(x), FieldValue::Integer(y)) => x == y,
        (FieldValue::Number(x), FieldValue::Number(y)) => x == y,
        // 数值宽松匹配：Integer 与 Number 在数值相等时视为相等。
        (FieldValue::Integer(x), FieldValue::Number(y)) => (*x as f64) == *y,
        (FieldValue::Number(x), FieldValue::Integer(y)) => *x == (*y as f64),
        (FieldValue::Null, FieldValue::Null) => true,
        _ => false,
    }
}

/// 历史日志查询条件。所有字段可选，组合使用 AND 语义。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQuery {
    /// 来源筛选；None 表示任意来源。
    #[serde(default)]
    pub source: Option<LogSource>,
    /// 最低日志等级（包含该等级及更高）。
    #[serde(default)]
    pub min_level: Option<LogLevel>,
    /// 关键字（消息与 target 中匹配，不区分大小写）。
    #[serde(default)]
    pub keyword: Option<String>,
    /// 会话 ID 筛选。
    #[serde(default)]
    pub session_id: Option<String>,
    /// 起始时间（RFC3339，含）。
    #[serde(default)]
    pub from: Option<String>,
    /// 结束时间（RFC3339，含）。
    #[serde(default)]
    pub to: Option<String>,
    /// 游标：仅返回 id < before_id 的记录（用于向前翻页）。
    #[serde(default)]
    pub before_id: Option<u64>,
    /// 单页最大条数，默认 200，上限 1000。
    #[serde(default)]
    pub limit: Option<usize>,
    /// 关联 ID 精确筛选。用于"复制当前设备诊断"按 correlation_id 过滤。
    #[serde(default)]
    pub correlation_id: Option<String>,
    /// target 前缀筛选（区分大小写）。例如 `"plugin::"` 匹配所有插件协议事件。
    #[serde(default)]
    pub target_prefix: Option<String>,
    /// 结构化字段精确匹配。键为字段名，值为期望的标量值。
    /// 多个键之间 AND 语义。用于按 pluginId / deviceKey 等过滤。
    #[serde(default)]
    pub fields_exact: Option<BTreeMap<String, FieldValue>>,
}

impl LogQuery {
    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(200).clamp(1, 1000)
    }

    /// 判断一条日志是否匹配筛选条件。`entry` 必须已通过 ID 范围检查。
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if let Some(source) = self.source {
            if entry.source != source {
                return false;
            }
        }
        if let Some(min) = self.min_level {
            if !entry.level.at_least(min) {
                return false;
            }
        }
        if let Some(keyword) = self.keyword.as_deref() {
            if !keyword.trim().is_empty() {
                let needle = keyword.to_ascii_lowercase();
                if !entry.message.to_ascii_lowercase().contains(&needle)
                    && !entry.target.to_ascii_lowercase().contains(&needle)
                {
                    return false;
                }
            }
        }
        if let Some(session) = self.session_id.as_deref() {
            if entry.session_id != session {
                return false;
            }
        }
        if let Some(from) = self.from.as_deref() {
            if entry.timestamp.as_str() < from {
                return false;
            }
        }
        if let Some(to) = self.to.as_deref() {
            if entry.timestamp.as_str() > to {
                return false;
            }
        }
        // correlation_id 精确匹配。None 与 Some 不匹配。
        if let Some(cid) = self.correlation_id.as_deref() {
            if entry.correlation_id.as_deref() != Some(cid) {
                return false;
            }
        }
        // target 前缀匹配（区分大小写）。空字符串前缀视为不筛选。
        if let Some(prefix) = self.target_prefix.as_deref() {
            if !prefix.is_empty() && !entry.target.starts_with(prefix) {
                return false;
            }
        }
        // 结构化字段精确匹配。所有键必须存在且值相等。
        if let Some(exact) = self.fields_exact.as_ref() {
            for (key, expected) in exact {
                match entry.fields.get(key) {
                    Some(actual) => {
                        if !field_values_equal(actual, expected) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }
        true
    }
}

/// 查询返回的分页结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub entries: Vec<LogEntry>,
    /// 是否还有更早的记录（id 更小的）。
    pub has_more: bool,
    /// 当前返回的最早一条记录的 id；用作下一页游标。
    pub oldest_id: Option<u64>,
    /// 当前会话内匹配的总条数（用于状态展示，非精确计数）。
    pub total_in_session: usize,
}

/// 删除范围。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "scope")]
pub enum DeleteScope {
    /// 删除 7 天前日志。
    OlderThanDays { days: u32 },
    /// 删除当前会话之前的所有日志。
    BeforeCurrentSession,
    /// 删除全部本地日志（含当前会话）。
    All,
}

/// 删除结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    pub deleted_files: u32,
    pub deleted_buffer_entries: u64,
    /// 部分成功时为 true（如某些文件被占用）。
    pub partial: bool,
    pub error: Option<String>,
}

/// 导出范围。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "scope")]
pub enum ExportScope {
    /// 导出当前筛选结果。
    Filtered { query: LogQuery },
    /// 导出本次运行会话日志。
    CurrentSession,
    /// 生成完整脱敏诊断包（ZIP）。
    DiagnosticsBundle,
}

/// 日志服务状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub session_id: String,
    /// 当前最低采集等级。
    pub min_level: LogLevel,
    /// 内存缓冲区当前条数。
    pub buffer_count: usize,
    /// 内存缓冲区上限。
    pub buffer_capacity: usize,
    /// 磁盘日志目录（脱敏路径，用 ${HOME} 等占位符）。
    pub storage_dir_display: String,
    /// 磁盘日志占用字节数（近似值）。
    pub disk_usage_bytes: u64,
    /// 磁盘日志占用上限字节数。
    pub disk_quota_bytes: u64,
    /// 当前错误数（最近 N 条中 Error 等级）。
    pub recent_error_count: usize,
    /// 当前警告数（最近 N 条中 Warn 等级）。
    pub recent_warn_count: usize,
    /// 文件写入是否启用（目录不可写时为 false）。
    pub file_persistence_enabled: bool,
    /// 临时诊断会话信息；None 表示未启用。
    pub diagnostic_session: Option<DiagnosticSessionStatus>,
    /// 协议诊断会话信息；None 表示未启用。
    pub protocol_diagnostic: Option<ProtocolDiagnosticStatus>,
}

/// 临时诊断会话状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSessionStatus {
    pub started_at: String,
    pub ends_at: String,
    pub original_level: LogLevel,
    pub current_level: LogLevel,
    pub auto_expire: bool,
}

/// 协议诊断会话状态。
///
/// 协议诊断模式叠加在通用诊断会话之上：通用会话提升日志采集等级，
/// 协议诊断模式额外授权对指定设备的 HID payload（request/response hex）进行临时记录。
/// 两者独立启停，但实践中协议诊断模式通常配合 Debug/Trace 等级使用。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDiagnosticStatus {
    /// 目标设备 key（VID:PID:interface 格式），只对此设备的 HID 交换记录 payload。
    pub device_key: String,
    pub started_at: String,
    pub ends_at: String,
    pub auto_expire: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering_is_correct() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn level_at_least_filters_correctly() {
        assert!(LogLevel::Error.at_least(LogLevel::Error));
        assert!(LogLevel::Error.at_least(LogLevel::Info));
        assert!(!LogLevel::Info.at_least(LogLevel::Error));
        assert!(LogLevel::Info.at_least(LogLevel::Info));
        assert!(LogLevel::Info.at_least(LogLevel::Debug));
        assert!(!LogLevel::Debug.at_least(LogLevel::Info));
    }

    #[test]
    fn level_as_str_matches_serde_lowercase() {
        for level in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            let serde_str = serde_json::to_string(&level).unwrap();
            // serde_json 给字符串带引号，as_str 不带。
            let expected = serde_str.trim_matches('"');
            assert_eq!(level.as_str(), expected);
            assert_eq!(level.to_string(), expected);
        }
    }

    #[test]
    fn query_matches_combines_filters_with_and() {
        let entry = LogEntry {
            id: 1,
            timestamp: "2026-07-17T10:00:00+08:00".into(),
            level: LogLevel::Warn,
            source: LogSource::Plugin,
            target: "plugin::verify".into(),
            message: "signature mismatch".into(),
            session_id: "s1".into(),
            correlation_id: None,
            fields: Fields::new(),
        };
        let q = LogQuery {
            source: Some(LogSource::Plugin),
            min_level: Some(LogLevel::Warn),
            keyword: Some("signature".into()),
            session_id: None,
            from: None,
            to: None,
            before_id: None,
            limit: None,
            correlation_id: None,
            target_prefix: None,
            fields_exact: None,
        };
        assert!(q.matches(&entry));

        let q2 = LogQuery {
            source: Some(LogSource::App),
            ..q.clone()
        };
        assert!(!q2.matches(&entry));

        let q3 = LogQuery {
            min_level: Some(LogLevel::Error),
            ..q.clone()
        };
        assert!(!q3.matches(&entry));

        let q4 = LogQuery {
            keyword: Some("network".into()),
            ..q.clone()
        };
        assert!(!q4.matches(&entry));
    }

    #[test]
    fn query_filters_by_correlation_id() {
        let mut entry = LogEntry {
            id: 1,
            timestamp: "2026-07-17T10:00:00+08:00".into(),
            level: LogLevel::Info,
            source: LogSource::Plugin,
            target: "plugin::read".into(),
            message: "step ok".into(),
            session_id: "s1".into(),
            correlation_id: Some("device-abc123".into()),
            fields: Fields::new(),
        };
        // 匹配的 correlation_id
        let q = LogQuery {
            correlation_id: Some("device-abc123".into()),
            ..LogQuery::default()
        };
        assert!(q.matches(&entry));
        // 不匹配的 correlation_id
        let q = LogQuery {
            correlation_id: Some("device-other".into()),
            ..LogQuery::default()
        };
        assert!(!q.matches(&entry));
        // entry 无 correlation_id 但 query 有：不匹配
        entry.correlation_id = None;
        let q = LogQuery {
            correlation_id: Some("device-abc123".into()),
            ..LogQuery::default()
        };
        assert!(!q.matches(&entry));
    }

    #[test]
    fn query_filters_by_target_prefix() {
        let entry = LogEntry {
            id: 1,
            timestamp: "2026-07-17T10:00:00+08:00".into(),
            level: LogLevel::Info,
            source: LogSource::Plugin,
            target: "plugin::read::step".into(),
            message: "step ok".into(),
            session_id: "s1".into(),
            correlation_id: None,
            fields: Fields::new(),
        };
        // 匹配
        assert!(LogQuery {
            target_prefix: Some("plugin::".into()),
            ..LogQuery::default()
        }
        .matches(&entry));
        // 不匹配
        assert!(!LogQuery {
            target_prefix: Some("hid::".into()),
            ..LogQuery::default()
        }
        .matches(&entry));
        // 空前缀视为不筛选
        assert!(LogQuery {
            target_prefix: Some(String::new()),
            ..LogQuery::default()
        }
        .matches(&entry));
    }

    #[test]
    fn query_filters_by_fields_exact() {
        let mut fields = Fields::new();
        fields.insert("pluginId".into(), FieldValue::from("mira.razer-chroma"));
        fields.insert("durationMs".into(), FieldValue::from(42_i64));
        let entry = LogEntry {
            id: 1,
            timestamp: "2026-07-17T10:00:00+08:00".into(),
            level: LogLevel::Info,
            source: LogSource::Plugin,
            target: "plugin::read".into(),
            message: "ok".into(),
            session_id: "s1".into(),
            correlation_id: None,
            fields,
        };
        // 单字段匹配
        let q = LogQuery {
            fields_exact: Some(BTreeMap::from([(
                "pluginId".into(),
                FieldValue::from("mira.razer-chroma"),
            )])),
            ..LogQuery::default()
        };
        assert!(q.matches(&entry));
        // 数值宽松匹配：Integer 42 == Number 42.0
        let q = LogQuery {
            fields_exact: Some(BTreeMap::from([(
                "durationMs".into(),
                FieldValue::Number(42.0),
            )])),
            ..LogQuery::default()
        };
        assert!(q.matches(&entry));
        // 多字段 AND 匹配
        let q = LogQuery {
            fields_exact: Some(BTreeMap::from([
                ("pluginId".into(), FieldValue::from("mira.razer-chroma")),
                ("durationMs".into(), FieldValue::from(42_i64)),
            ])),
            ..LogQuery::default()
        };
        assert!(q.matches(&entry));
        // 字段值不匹配
        let q = LogQuery {
            fields_exact: Some(BTreeMap::from([(
                "pluginId".into(),
                FieldValue::from("mira.amaster"),
            )])),
            ..LogQuery::default()
        };
        assert!(!q.matches(&entry));
        // 字段不存在
        let q = LogQuery {
            fields_exact: Some(BTreeMap::from([(
                "missingField".into(),
                FieldValue::from("x"),
            )])),
            ..LogQuery::default()
        };
        assert!(!q.matches(&entry));
    }

    #[test]
    fn query_effective_limit_is_bounded() {
        let q = LogQuery {
            limit: Some(0),
            ..LogQuery::default()
        };
        assert_eq!(q.effective_limit(), 1);
        let q = LogQuery {
            limit: Some(10_000),
            ..LogQuery::default()
        };
        assert_eq!(q.effective_limit(), 1000);
        let q = LogQuery {
            limit: Some(500),
            ..LogQuery::default()
        };
        assert_eq!(q.effective_limit(), 500);
    }
}
