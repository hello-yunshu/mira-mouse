// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tauri 命令：日志查询、状态、删除、导出、订阅、等级调整、诊断会话、前端日志写入。
//!
//! 命令设计原则：
//! - 所有命令 async fn，避免阻塞主线程（对齐项目 HID 操作 async 约定）。
//! - 失败返回 `Result<T, String>`，字符串可直接前端展示。
//! - 危险操作（删除）由调用方在 UI 层做确认；本层只做范围与参数校验。
//! - 导出路径由前端通过系统保存对话框获取后传入，遵循 `device_config_export` /
//!   `battery_history_export` 已有模式；不在后端弹对话框，避免 dialog 插件 API 差异。

use crate::local_ai_update;
use crate::logging::export::{self, DiagnosticsContext};
use crate::logging::model::{
    DeleteResult, DeleteScope, ExportScope, LogInput, LogLevel, LogPage, LogQuery, LogStatus,
};
use crate::logging::{self, LogService, DEFAULT_DIAGNOSTIC_MINUTES};
use std::path::PathBuf;
use tauri::State;

/// 查询历史日志。返回最新的一页，前端用 before_id 游标继续翻。
#[tauri::command]
pub async fn log_query(query: LogQuery, state: State<'_, LogService>) -> Result<LogPage, String> {
    Ok(state.query(&query))
}

/// 当前日志服务状态。
#[tauri::command]
pub async fn log_status(state: State<'_, LogService>) -> Result<LogStatus, String> {
    Ok(state.status())
}

/// 删除磁盘历史日志。
#[tauri::command]
pub async fn log_delete(
    scope: DeleteScope,
    state: State<'_, LogService>,
) -> Result<DeleteResult, String> {
    Ok(state.delete(scope))
}

/// 前端订阅实时日志批次事件。
/// 后端开始向前端 emit `mira://logs/batch` 事件。
#[tauri::command]
pub async fn log_subscribe(state: State<'_, LogService>) -> Result<(), String> {
    state.subscribe();
    Ok(())
}

/// 前端取消订阅。
#[tauri::command]
pub async fn log_unsubscribe(state: State<'_, LogService>) -> Result<(), String> {
    state.unsubscribe();
    Ok(())
}

/// 临时设置最低采集等级。
/// `level` 取值：error / warn / info / debug / trace。
#[tauri::command]
pub async fn log_set_level(level: LogLevel, state: State<'_, LogService>) -> Result<(), String> {
    state.set_level(level);
    Ok(())
}

/// 开始临时诊断会话。
/// - `minutes`: 持续分钟数，会被 clamp 到 [1, 30]。
/// - `level`: 临时采集等级，通常 debug 或 trace。
/// - `auto_expire`: true 时启动后台到期线程，到期自动恢复。
#[tauri::command]
pub async fn log_start_diagnostic_session(
    minutes: Option<i64>,
    level: Option<LogLevel>,
    auto_expire: Option<bool>,
    state: State<'_, LogService>,
) -> Result<(), String> {
    let minutes = minutes.unwrap_or(DEFAULT_DIAGNOSTIC_MINUTES);
    let level = level.unwrap_or(logging::DEFAULT_MIN_LEVEL);
    let auto_expire = auto_expire.unwrap_or(true);
    state.start_diagnostic_session(minutes, level, auto_expire);
    Ok(())
}

/// 手动停止临时诊断会话。
#[tauri::command]
pub async fn log_stop_diagnostic_session(state: State<'_, LogService>) -> Result<(), String> {
    state.stop_diagnostic_session();
    Ok(())
}

/// 开始协议诊断会话：授权对指定设备临时记录 HID payload（request/response hex）。
///
/// - `device_key`: 目标设备 key（VID:PID:interface）。只对此设备的 HID 交换记录 payload。
/// - `minutes`: 持续分钟数，clamp 到 [1, 30]。
/// - `auto_expire`: true 时启动后台到期线程，到期自动停止。
///
/// 协议诊断模式不影响日志采集等级；前端应同时调用 `log_start_diagnostic_session`
/// 提升到 Trace 才能使 `hid-feature-exchange` 事件被采集。
/// payload 经过 command-aware masking（serial 脱敏、macro 拒绝等），见
/// `protocol_event::classify_command` / `mask_payload`。
#[tauri::command]
pub async fn log_start_protocol_diagnostic(
    device_key: String,
    minutes: Option<i64>,
    auto_expire: Option<bool>,
    state: State<'_, LogService>,
) -> Result<(), String> {
    let minutes = minutes.unwrap_or(DEFAULT_DIAGNOSTIC_MINUTES);
    let auto_expire = auto_expire.unwrap_or(true);
    state.start_protocol_diagnostic(device_key, minutes, auto_expire);
    Ok(())
}

/// 手动停止协议诊断会话。
#[tauri::command]
pub async fn log_stop_protocol_diagnostic(state: State<'_, LogService>) -> Result<(), String> {
    state.stop_protocol_diagnostic();
    Ok(())
}

/// 前端写入少量经过筛选的日志。
/// 用于记录前端关键事件与异常，受频率与长度限制。
/// 失败静默返回 Ok，避免前端日志命令形成递归。
#[tauri::command]
pub async fn log_write(input: LogInput, state: State<'_, LogService>) -> Result<(), String> {
    state.write(input);
    Ok(())
}

/// 导出日志。
/// `scope` 决定导出范围与文件格式；`path` 由前端通过系统保存对话框获取。
/// - Filtered: 按 query 筛选内存缓冲 → JSONL
/// - CurrentSession: 当前会话内存缓冲 → JSONL
/// - DiagnosticsBundle: 不支持，应调用 log_export_diagnostics_bundle
///
/// 返回写入字节数。
#[tauri::command]
pub async fn log_export(
    scope: ExportScope,
    path: String,
    state: State<'_, LogService>,
) -> Result<ExportOutcomeDto, String> {
    let output_path = PathBuf::from(&path);
    let outcome = match scope {
        ExportScope::Filtered { query } => {
            let entries = state.query_filtered_entries(&query);
            export::export_filtered(entries, &output_path)
        }
        ExportScope::CurrentSession => {
            let entries = state.buffer_snapshot_for_session();
            export::export_session(entries, &output_path)
        }
        ExportScope::DiagnosticsBundle => {
            return Err(
                "DiagnosticsBundle scope requires log_export_diagnostics_bundle command".into(),
            );
        }
    }
    .map_err(|e| format!("export failed: {e}"))?;

    Ok(ExportOutcomeDto {
        path,
        entry_count: outcome.entry_count,
        bytes_written: outcome.bytes_written,
        truncated: outcome.truncated,
    })
}

/// 导出诊断包 ZIP。
///
/// 诊断上下文（应用版本、平台、架构、会话 ID、本地 AI 状态等）由本命令在后端
/// 自行收集，不再依赖前端传入——前端无法可靠获取平台/架构等信息，且原先前端
/// 构造的 `ctx` 各字段均为空，导致诊断包缺少系统信息。
///
/// `path` 由前端通过系统保存对话框获取。
#[tauri::command]
pub async fn log_export_diagnostics_bundle(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, LogService>,
) -> Result<ExportOutcomeDto, String> {
    let output_path = PathBuf::from(&path);
    let log_status = state.status();
    let entries = state.buffer_snapshot_for_session();

    // 本地 AI 状态可能读取文件 / 状态，放到阻塞线程中执行。
    let app_for_ai = app.clone();
    let local_ai_status_json = tauri::async_runtime::spawn_blocking(move || {
        serde_json::to_string(&local_ai_update::status(&app_for_ai)).unwrap_or_else(|_| "{}".into())
    })
    .await
    .map_err(|e| format!("diagnostics bundle failed: {e}"))?;

    let package = app.package_info();
    let ctx = DiagnosticsContext {
        app_name: package.name.to_string(),
        app_version: package.version.to_string(),
        app_identifier: app.config().identifier.to_string(),
        platform: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        session_id: log_status.session_id,
        // app-info / plugin-status 暂无对应的只读收集命令，保留占位；
        // summary.json 已含版本/平台等关键信息，日志本身在 logs.jsonl 中。
        app_info_json: "{}".into(),
        plugin_status_json: "{}".into(),
        local_ai_status_json,
        recent_error_count: log_status.recent_error_count,
        recent_warn_count: log_status.recent_warn_count,
    };

    let outcome = export::export_diagnostics_bundle(entries, &ctx, &output_path)
        .map_err(|e| format!("diagnostics bundle failed: {e}"))?;
    Ok(ExportOutcomeDto {
        path,
        entry_count: outcome.entry_count,
        bytes_written: outcome.bytes_written,
        truncated: outcome.truncated,
    })
}

/// 打开日志目录（跨平台）。使用系统默认文件管理器。
#[tauri::command]
pub async fn log_open_dir(state: State<'_, LogService>) -> Result<(), String> {
    let dir = state.storage_dir();
    open_path_in_file_manager(&dir).map_err(|e| format!("open dir failed: {e}"))
}

/// 设备定向诊断导出输入。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDiagnosticsInput {
    /// 插件 ID（必填，用于日志筛选）。
    pub plugin_id: String,
    /// 设备 key（VID:PID:interface 格式，用于日志筛选）。
    pub device_key: String,
    /// 设备 model（可选，用于日志筛选，缩小到特定型号）。
    pub model: Option<String>,
    /// 会话 ID（可选，用于日志筛选，缩小到当前会话）。
    pub session_id: Option<String>,
    /// 关联 ID（可选，缩小到特定读取会话）。
    pub correlation_id: Option<String>,
    /// 当前"全部读数"的 JSON 表示（前端从快照传入）。
    pub readings_json: String,
    /// 当前 read statuses 的 JSON 表示（前端从快照传入）。
    pub read_statuses_json: String,
    /// 是否包含临时协议诊断（HID payload）。仅在协议诊断模式启用时有效。
    #[serde(default)]
    pub include_protocol_payload: bool,
    /// 输出格式："markdown" 或 "json"。
    #[serde(default = "default_device_diagnostics_format")]
    pub format: String,
}

fn default_device_diagnostics_format() -> String {
    "markdown".into()
}

/// 设备定向诊断导出结果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDiagnosticsOutcome {
    pub path: String,
    pub bytes_written: u64,
    pub log_entry_count: usize,
    /// 报告内容（用于前端复制到剪贴板）。
    pub content: String,
}

/// 导出设备定向诊断报告。
///
/// 对齐 spec 13.3：以 pluginId、deviceKey、correlationId 筛选日志，
/// 包含当前"全部读数"和 read statuses，自动脱敏，输出 Markdown 或 JSON。
/// 不复制其他设备和本地 AI 的无关日志。
///
/// `path` 为空字符串时跳过文件写入，仅返回 `content` 供前端复制到剪贴板。
#[tauri::command]
pub async fn log_export_device_diagnostics(
    input: DeviceDiagnosticsInput,
    path: String,
    state: State<'_, LogService>,
) -> Result<DeviceDiagnosticsOutcome, String> {
    use crate::logging::model::FieldValue;
    use std::collections::BTreeMap;

    // 构建日志查询：按 pluginId + deviceKey 精确筛选，可选 model 和 sessionId 收窄范围。
    let mut fields_exact = BTreeMap::new();
    fields_exact.insert(
        "pluginId".into(),
        FieldValue::from(input.plugin_id.as_str()),
    );
    fields_exact.insert(
        "deviceKey".into(),
        FieldValue::from(input.device_key.as_str()),
    );
    if let Some(ref model) = input.model {
        if !model.is_empty() {
            fields_exact.insert("model".into(), FieldValue::from(model.as_str()));
        }
    }

    let query = LogQuery {
        source: None,
        min_level: Some(LogLevel::Warn),
        keyword: None,
        session_id: input.session_id.clone(),
        from: None,
        to: None,
        before_id: None,
        limit: Some(500),
        correlation_id: input.correlation_id.clone(),
        target_prefix: None,
        fields_exact: Some(fields_exact),
    };

    let entries = state.query_filtered_entries(&query);

    // 如果不包含协议诊断，过滤掉含 requestHex/responseHex 的条目。
    let filtered_entries: Vec<_> = if input.include_protocol_payload {
        entries
    } else {
        entries
            .into_iter()
            .filter(|e| {
                !e.fields.contains_key("requestHex") && !e.fields.contains_key("responseHex")
            })
            .collect()
    };

    let log_count = filtered_entries.len();

    // 组装报告内容。
    let content = match input.format.as_str() {
        "json" => build_device_diagnostics_json(&input, &filtered_entries),
        _ => build_device_diagnostics_markdown(&input, &filtered_entries),
    };

    let bytes_written = content.len() as u64;

    // path 为空时跳过文件写入（前端仅复制到剪贴板）。
    if !path.is_empty() {
        std::fs::write(&path, &content)
            .map_err(|e| format!("write device diagnostics failed: {e}"))?;
    }

    Ok(DeviceDiagnosticsOutcome {
        path,
        bytes_written,
        log_entry_count: log_count,
        content,
    })
}

fn build_device_diagnostics_markdown(
    input: &DeviceDiagnosticsInput,
    entries: &[crate::logging::model::LogEntry],
) -> String {
    let mut md = String::new();
    md.push_str("# Mira 设备诊断报告\n\n");
    md.push_str(&format!("- **插件**: `{}`\n", input.plugin_id));
    md.push_str(&format!("- **设备 Key**: `{}`\n", input.device_key));
    if let Some(ref cid) = input.correlation_id {
        md.push_str(&format!("- **关联 ID**: `{}`\n", cid));
    }
    md.push_str(&format!(
        "- **生成时间**: {}\n",
        chrono::Utc::now().to_rfc3339()
    ));
    md.push_str(&format!(
        "- **包含协议诊断**: {}\n",
        if input.include_protocol_payload {
            "是"
        } else {
            "否"
        }
    ));
    md.push('\n');

    // 全部读数
    md.push_str("## 全部读数\n\n");
    md.push_str("```json\n");
    md.push_str(&input.readings_json);
    md.push_str("\n```\n\n");

    // Read Statuses
    md.push_str("## 读取状态\n\n");
    md.push_str("```json\n");
    md.push_str(&input.read_statuses_json);
    md.push_str("\n```\n\n");

    // 相关日志
    md.push_str("## 相关日志（Warn 及以上）\n\n");
    if entries.is_empty() {
        md.push_str("无相关日志。\n\n");
    } else {
        for entry in entries {
            md.push_str(&format!(
                "### [{}] {} — {}\n\n",
                entry.level, entry.timestamp, entry.target
            ));
            md.push_str(&entry.message);
            md.push('\n');
            if !entry.fields.is_empty() {
                md.push_str("\n| 字段 | 值 |\n| --- | --- |\n");
                for (k, v) in &entry.fields {
                    let value_json = serde_json::to_value(v).unwrap_or(serde_json::Value::Null);
                    md.push_str(&format!("| {} | {} |\n", k, value_json));
                }
            }
            md.push('\n');
        }
    }

    md.push_str("---\n*此报告由 Mira 自动生成，已自动脱敏。*\n");
    md
}

fn build_device_diagnostics_json(
    input: &DeviceDiagnosticsInput,
    entries: &[crate::logging::model::LogEntry],
) -> String {
    use serde_json::json;
    let report = json!({
        "type": "mira-device-diagnostics",
        "generatedAt": chrono::Utc::now().to_rfc3339(),
        "pluginId": input.plugin_id,
        "deviceKey": input.device_key,
        "correlationId": input.correlation_id,
        "includeProtocolPayload": input.include_protocol_payload,
        "readings": serde_json::from_str::<serde_json::Value>(&input.readings_json).unwrap_or(serde_json::Value::Null),
        "readStatuses": serde_json::from_str::<serde_json::Value>(&input.read_statuses_json).unwrap_or(serde_json::Value::Null),
        "logs": entries.iter().map(|e| {
            serde_json::json!({
                "timestamp": e.timestamp,
                "level": e.level,
                "target": e.target,
                "message": e.message,
                "correlationId": e.correlation_id,
                "fields": e.fields,
            })
        }).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
}

/// 导出结果，简化为可序列化 DTO。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcomeDto {
    pub path: String,
    pub entry_count: usize,
    pub bytes_written: u64,
    pub truncated: bool,
}

fn open_path_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "opening log dir is not supported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::model::{Fields, LogEntry, LogSource};

    #[test]
    fn open_path_returns_unsupported_on_unknown_platform() {
        // 仅验证 cfg 门控逻辑存在；实际行为由操作系统决定。
        let _ = std::path::PathBuf::from("/tmp/nonexistent");
    }

    #[test]
    fn export_outcome_dto_serializes_to_camel_case() {
        let dto = ExportOutcomeDto {
            path: "/tmp/out.jsonl".into(),
            entry_count: 42,
            bytes_written: 1024,
            truncated: false,
        };
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["path"], "/tmp/out.jsonl");
        assert_eq!(json["entryCount"], 42);
        assert_eq!(json["bytesWritten"], 1024);
        assert_eq!(json["truncated"], false);
    }

    #[test]
    fn device_diagnostics_input_deserializes_camel_case() {
        let json = r#"{
            "pluginId": "mira.razer-chroma",
            "deviceKey": "0001:0002:00",
            "correlationId": "device-abc123",
            "readingsJson": "{}",
            "readStatusesJson": "{}",
            "includeProtocolPayload": true,
            "format": "json"
        }"#;
        let input: DeviceDiagnosticsInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.plugin_id, "mira.razer-chroma");
        assert_eq!(input.device_key, "0001:0002:00");
        assert_eq!(input.correlation_id.as_deref(), Some("device-abc123"));
        assert!(input.include_protocol_payload);
        assert_eq!(input.format, "json");
    }

    #[test]
    fn device_diagnostics_input_defaults_format_to_markdown() {
        let json = r#"{
            "pluginId": "p",
            "deviceKey": "k",
            "readingsJson": "{}",
            "readStatusesJson": "{}"
        }"#;
        let input: DeviceDiagnosticsInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.format, "markdown");
        assert!(!input.include_protocol_payload);
    }

    #[test]
    fn device_diagnostics_outcome_serializes_camel_case() {
        let outcome = DeviceDiagnosticsOutcome {
            path: "/tmp/diag.md".into(),
            bytes_written: 100,
            log_entry_count: 3,
            content: "# report".into(),
        };
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["path"], "/tmp/diag.md");
        assert_eq!(json["bytesWritten"], 100);
        assert_eq!(json["logEntryCount"], 3);
        assert_eq!(json["content"], "# report");
    }

    fn make_log_entry(level: LogLevel, target: &str, fields: Fields) -> LogEntry {
        LogEntry {
            id: 1,
            timestamp: "2026-07-24T10:00:00+08:00".into(),
            level,
            source: LogSource::Plugin,
            target: target.into(),
            message: "test message".into(),
            session_id: "s1".into(),
            correlation_id: Some("device-abc".into()),
            fields,
        }
    }

    #[test]
    fn markdown_report_includes_readings_and_statuses() {
        let input = DeviceDiagnosticsInput {
            plugin_id: "mira.razer".into(),
            device_key: "0001:0002:00".into(),
            model: None,
            session_id: None,
            correlation_id: Some("device-abc".into()),
            readings_json: r#"{"dpi":{"value":1600}}"#.into(),
            read_statuses_json: r#"{"dpi":"ok"}"#.into(),
            include_protocol_payload: false,
            format: "markdown".into(),
        };
        let entries = vec![];
        let md = build_device_diagnostics_markdown(&input, &entries);
        assert!(md.contains("mira.razer"));
        assert!(md.contains("0001:0002:00"));
        assert!(md.contains("device-abc"));
        assert!(md.contains(r#""dpi":{"value":1600}"#));
        assert!(md.contains(r#""dpi":"ok""#));
        assert!(md.contains("无相关日志"));
    }

    #[test]
    fn markdown_report_lists_warn_and_error_entries() {
        let input = DeviceDiagnosticsInput {
            plugin_id: "p".into(),
            device_key: "k".into(),
            model: None,
            session_id: None,
            correlation_id: None,
            readings_json: "{}".into(),
            read_statuses_json: "{}".into(),
            include_protocol_payload: false,
            format: "markdown".into(),
        };
        let mut fields = Fields::new();
        fields.insert(
            "workflow".into(),
            crate::logging::model::FieldValue::from("wf"),
        );
        let entries = vec![
            make_log_entry(LogLevel::Warn, "plugin::read", fields.clone()),
            make_log_entry(LogLevel::Error, "plugin::read", fields),
        ];
        let md = build_device_diagnostics_markdown(&input, &entries);
        assert!(md.contains("[warn]"));
        assert!(md.contains("[error]"));
        assert!(md.contains("test message"));
        assert!(md.contains("workflow"));
    }

    #[test]
    fn json_report_produces_valid_json() {
        let input = DeviceDiagnosticsInput {
            plugin_id: "p".into(),
            device_key: "k".into(),
            model: None,
            session_id: None,
            correlation_id: None,
            readings_json: r#"{"dpi":1600}"#.into(),
            read_statuses_json: r#"{"dpi":"ok"}"#.into(),
            include_protocol_payload: false,
            format: "json".into(),
        };
        let entries = vec![];
        let json_str = build_device_diagnostics_json(&input, &entries);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "mira-device-diagnostics");
        assert_eq!(parsed["pluginId"], "p");
        assert_eq!(parsed["deviceKey"], "k");
        assert_eq!(parsed["readings"]["dpi"], 1600);
        assert_eq!(parsed["readStatuses"]["dpi"], "ok");
        assert!(parsed["logs"].is_array());
    }

    #[test]
    fn json_report_includes_log_entries() {
        let input = DeviceDiagnosticsInput {
            plugin_id: "p".into(),
            device_key: "k".into(),
            model: None,
            session_id: None,
            correlation_id: None,
            readings_json: "{}".into(),
            read_statuses_json: "{}".into(),
            include_protocol_payload: false,
            format: "json".into(),
        };
        let entries = vec![make_log_entry(
            LogLevel::Warn,
            "plugin::read",
            Fields::new(),
        )];
        let json_str = build_device_diagnostics_json(&input, &entries);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let logs = parsed["logs"].as_array().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["level"], "warn");
        assert_eq!(logs[0]["target"], "plugin::read");
    }
}
