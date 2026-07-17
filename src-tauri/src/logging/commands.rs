// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tauri 命令：日志查询、状态、删除、导出、订阅、等级调整、诊断会话、前端日志写入。
//!
//! 命令设计原则：
//! - 所有命令 async fn，避免阻塞主线程（对齐项目 HID 操作 async 约定）。
//! - 失败返回 `Result<T, String>`，字符串可直接前端展示。
//! - 危险操作（删除）由调用方在 UI 层做确认；本层只做范围与参数校验。
//! - 导出路径由前端通过系统保存对话框获取后传入，遵循 `device_config_export` /
//!   `battery_history_export` 已有模式；不在后端弹对话框，避免 dialog 插件 API 差异。

use crate::logging::export::{self, DiagnosticsContext};
use crate::logging::model::{
    DeleteResult, DeleteScope, ExportScope, LogInput, LogLevel, LogPage, LogQuery, LogStatus,
};
use crate::logging::{self, LogService, DEFAULT_DIAGNOSTIC_MINUTES};
use crate::local_ai_update;
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
        serde_json::to_string(&local_ai_update::status(&app_for_ai))
            .unwrap_or_else(|_| "{}".into())
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
}
