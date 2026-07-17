// SPDX-License-Identifier: AGPL-3.0-or-later
//! 日志导出与诊断包生成。
//!
//! 三种范围：
//! - `Filtered`: 按 LogQuery 从内存缓冲查询，导出 JSONL。
//! - `CurrentSession`: 导出当前会话内存缓冲 + 磁盘上同会话条目，JSONL。
//! - `DiagnosticsBundle`: 生成 ZIP，包含 summary.json / logs.jsonl /
//!   app-info.json / plugin-status.json / local-ai-status.json / privacy-report.txt。
//!
//! 所有导出物已在前端展示和持久化阶段被 Redactor 统一脱敏；此处不再二次处理，
//! 但仍会校验文件大小上限，避免异常情况把超大文件写入用户磁盘。

use crate::logging::model::LogEntry;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// 单次导出 JSONL 大小上限（约 16 MB）。超出截断并附加尾注。
pub const MAX_EXPORT_BYTES: u64 = 16 * 1024 * 1024;

/// 诊断包所需的额外上下文。由调用方（lib.rs）填充。
/// 所有字段已是脱敏后的展示形式；不允许包含原始序列号或路径。
///
/// 同时作为 `log_export_diagnostics_bundle` Tauri 命令的入参，因此需要 `Deserialize`。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsContext {
    pub app_name: String,
    pub app_version: String,
    pub app_identifier: String,
    pub platform: String,
    pub architecture: String,
    pub rust_version: String,
    pub session_id: String,
    /// 前端为字符串形式，由调用方自行 JSON 序列化。
    pub app_info_json: String,
    pub plugin_status_json: String,
    pub local_ai_status_json: String,
    pub recent_error_count: usize,
    pub recent_warn_count: usize,
}

/// 导出结果。`bytes_written` 仅计算主日志文件，ZIP 中的元数据不计入。
#[derive(Debug, Clone)]
pub struct ExportOutcome {
    pub entry_count: usize,
    pub bytes_written: u64,
    pub truncated: bool,
}

/// 将筛选结果导出为 JSONL。
/// 数据来源：内存缓冲（已脱敏）。
pub fn export_filtered(
    entries: Vec<LogEntry>,
    output_path: &Path,
) -> std::io::Result<ExportOutcome> {
    write_jsonl(entries, output_path)
}

/// 将当前会话日志导出为 JSONL。
/// 数据来源：内存缓冲（已脱敏）。
pub fn export_session(
    entries: Vec<LogEntry>,
    output_path: &Path,
) -> std::io::Result<ExportOutcome> {
    write_jsonl(entries, output_path)
}

/// 生成诊断包 ZIP。
/// 文件结构：
/// ```text
/// mira-diagnostics-<date>.zip
/// ├── summary.json
/// ├── logs.jsonl
/// ├── app-info.json
/// ├── plugin-status.json
/// ├── local-ai-status.json
/// └── privacy-report.txt
/// ```
pub fn export_diagnostics_bundle(
    entries: Vec<LogEntry>,
    ctx: &DiagnosticsContext,
    output_path: &Path,
) -> std::io::Result<ExportOutcome> {
    let file = File::create(output_path)?;
    let writer = BufWriter::new(file);
    let mut zip = zip::ZipWriter::new(writer);

    let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // summary.json
    let summary = serde_json::json!({
        "app": {
            "name": ctx.app_name,
            "version": ctx.app_version,
            "identifier": ctx.app_identifier,
        },
        "platform": ctx.platform,
        "architecture": ctx.architecture,
        "rustVersion": ctx.rust_version,
        "sessionId": ctx.session_id,
        "recentErrorCount": ctx.recent_error_count,
        "recentWarnCount": ctx.recent_warn_count,
        "generatedAt": chrono::Utc::now().to_rfc3339(),
        "logEntryCount": entries.len(),
    });
    zip.start_file("summary.json", opts).map_err(io_err)?;
    let _ = serde_json::to_writer(&mut zip, &summary).map_err(io_err)?;
    // zip crate 2.x: starting a new file implicitly ends the previous one.

    // logs.jsonl
    zip.start_file("logs.jsonl", opts).map_err(io_err)?;
    let mut bytes_written: u64 = 0;
    let mut truncated = false;
    for entry in &entries {
        let mut line = match serde_json::to_vec(entry) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        line.push(b'\n');
        if bytes_written + line.len() as u64 > MAX_EXPORT_BYTES {
            let note = b"... truncated due to size limit ...\n";
            let _ = zip.write_all(note);
            truncated = true;
            break;
        }
        let _ = zip.write_all(&line);
        bytes_written += line.len() as u64;
    }

    // app-info.json（已是字符串，直接写入；若为空写 `{}`）
    zip.start_file("app-info.json", opts).map_err(io_err)?;
    let app_info_bytes = if ctx.app_info_json.trim().is_empty() {
        b"{}"
    } else {
        ctx.app_info_json.as_bytes()
    };
    let _ = zip.write_all(app_info_bytes);

    // plugin-status.json
    zip.start_file("plugin-status.json", opts).map_err(io_err)?;
    let plugin_bytes = if ctx.plugin_status_json.trim().is_empty() {
        b"{}"
    } else {
        ctx.plugin_status_json.as_bytes()
    };
    let _ = zip.write_all(plugin_bytes);

    // local-ai-status.json
    zip.start_file("local-ai-status.json", opts)
        .map_err(io_err)?;
    let ai_bytes = if ctx.local_ai_status_json.trim().is_empty() {
        b"{}"
    } else {
        ctx.local_ai_status_json.as_bytes()
    };
    let _ = zip.write_all(ai_bytes);

    // privacy-report.txt
    zip.start_file("privacy-report.txt", opts).map_err(io_err)?;
    let privacy_report = build_privacy_report();
    let _ = zip.write_all(privacy_report.as_bytes());

    zip.finish()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    Ok(ExportOutcome {
        entry_count: entries.len(),
        bytes_written,
        truncated,
    })
}

fn write_jsonl(entries: Vec<LogEntry>, output_path: &Path) -> std::io::Result<ExportOutcome> {
    let file = File::create(output_path)?;
    let mut writer = BufWriter::new(file);
    let mut bytes_written: u64 = 0;
    let mut count: usize = 0;
    let mut truncated = false;

    for entry in &entries {
        let mut line = match serde_json::to_vec(entry) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        line.push(b'\n');
        if bytes_written + line.len() as u64 > MAX_EXPORT_BYTES {
            let note = b"... truncated due to size limit ...\n";
            let _ = writer.write_all(note);
            truncated = true;
            break;
        }
        let _ = writer.write_all(&line);
        bytes_written += line.len() as u64;
        count += 1;
    }
    writer.flush()?;

    Ok(ExportOutcome {
        entry_count: count,
        bytes_written,
        truncated,
    })
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
}

fn build_privacy_report() -> String {
    let mut out = String::new();
    out.push_str("Mira Diagnostics Bundle — Privacy Report\n");
    out.push_str("========================================\n\n");
    out.push_str("本诊断包仅保存于用户本机，应用不会自动上传任何内容。\n\n");
    out.push_str("包含：\n");
    out.push_str("- summary.json: 应用版本、平台、会话 ID、错误/警告计数\n");
    out.push_str("- logs.jsonl: 统一日志条目（已脱敏）\n");
    out.push_str("- app-info.json: 应用配置摘要（不含敏感字段）\n");
    out.push_str("- plugin-status.json: 插件加载与签名校验结果\n");
    out.push_str("- local-ai-status.json: 本地 AI runtime 状态\n\n");
    out.push_str("明确不包含：\n");
    out.push_str("- 用户主目录完整路径（已替换为 ${HOME}）\n");
    out.push_str("- 系统账户名（已替换为 ${USER}）\n");
    out.push_str("- 设备序列号、稳定设备 ID、MAC 地址、蓝牙地址\n");
    out.push_str("- 原始 HID 数据包、Feature Report、完整二进制内容\n");
    out.push_str("- 完整电量历史或样本数据\n");
    out.push_str("- 本地 AI 模型权重、完整输入历史\n");
    out.push_str("- 密码、Token、API Key、Bearer、Cookie、Authorization\n\n");
    out.push_str("脱敏策略：\n");
    out.push_str("- 字段名采用允许列表 + 拒绝列表结合\n");
    out.push_str("- 长字符串截断至 2048 字节，超长字段截断至 512 字节\n");
    out.push_str("- URL 凭据与查询参数中的敏感字段被替换为 [redacted]\n");
    out.push_str("- 已知设备 ID 替换为 ${DEVICE_ID}\n\n");
    out.push_str("如对隐私有顾虑，可在设置中关闭本地 AI 分析或停止临时诊断会话。\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::model::{Fields, LogLevel, LogSource};
    use std::fs;
    use tempfile::TempDir;

    fn make_entry(id: u64, level: LogLevel, source: LogSource, target: &str) -> LogEntry {
        LogEntry {
            id,
            timestamp: format!("2026-07-17T10:00:{id:02}+08:00"),
            level,
            source,
            target: target.into(),
            message: format!("message {id}"),
            session_id: "session-test".into(),
            correlation_id: None,
            fields: Fields::new(),
        }
    }

    #[test]
    fn export_filtered_writes_jsonl() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("filtered.jsonl");
        let entries = vec![
            make_entry(1, LogLevel::Info, LogSource::App, "test"),
            make_entry(2, LogLevel::Error, LogSource::Plugin, "plugin::verify"),
        ];
        let outcome = export_filtered(entries, &path).unwrap();
        assert_eq!(outcome.entry_count, 2);
        assert!(outcome.bytes_written > 0);
        assert!(!outcome.truncated);

        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["id"], 1);
    }

    #[test]
    fn export_session_writes_jsonl() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let entries = vec![
            make_entry(10, LogLevel::Info, LogSource::App, "app"),
            make_entry(11, LogLevel::Warn, LogSource::LocalAi, "local_ai::predict"),
        ];
        let outcome = export_session(entries, &path).unwrap();
        assert_eq!(outcome.entry_count, 2);
    }

    #[test]
    fn export_diagnostics_bundle_creates_zip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("diag.zip");
        let entries = vec![
            make_entry(1, LogLevel::Info, LogSource::App, "test"),
            make_entry(2, LogLevel::Error, LogSource::Plugin, "plugin::verify"),
        ];
        let ctx = DiagnosticsContext {
            app_name: "Mira".into(),
            app_version: "0.8.6".into(),
            app_identifier: "com.mira.app".into(),
            platform: "macos".into(),
            architecture: "aarch64".into(),
            rust_version: "1.85.0".into(),
            session_id: "session-test".into(),
            app_info_json: r#"{"autostart":false}"#.into(),
            plugin_status_json: r#"{"bundled":2}"#.into(),
            local_ai_status_json: r#"{"enabled":true}"#.into(),
            recent_error_count: 1,
            recent_warn_count: 0,
        };
        let outcome = export_diagnostics_bundle(entries, &ctx, &path).unwrap();
        assert_eq!(outcome.entry_count, 2);
        assert!(path.exists());

        // 验证 ZIP 内容可读取。
        let file = File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
            .collect();
        assert!(names.contains(&"summary.json".to_string()));
        assert!(names.contains(&"logs.jsonl".to_string()));
        assert!(names.contains(&"app-info.json".to_string()));
        assert!(names.contains(&"plugin-status.json".to_string()));
        assert!(names.contains(&"local-ai-status.json".to_string()));
        assert!(names.contains(&"privacy-report.txt".to_string()));
    }

    #[test]
    fn export_filtered_truncates_when_over_size_limit() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("large.jsonl");
        let mut entries = Vec::new();
        // 单条约 100 字节，制造远超 16 MB 的数据。
        for i in 0..300_000 {
            let mut e = make_entry(i, LogLevel::Info, LogSource::App, "test");
            e.message = "x".repeat(200);
            entries.push(e);
        }
        let outcome = export_filtered(entries, &path).unwrap();
        assert!(outcome.truncated);
        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.len() <= MAX_EXPORT_BYTES + 1024); // 1024B 余量
    }

    #[test]
    fn privacy_report_contains_key_sections() {
        let report = build_privacy_report();
        assert!(report.contains("包含"));
        assert!(report.contains("明确不包含"));
        assert!(report.contains("脱敏策略"));
        // 隐私报告为中文，序列号对应 "序列号"；HOME 占位符为 "${HOME}"。
        assert!(report.contains("序列号"));
        assert!(report.contains("HOME"));
    }

    #[test]
    fn diagnostics_bundle_handles_empty_status_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty.zip");
        let ctx = DiagnosticsContext {
            app_name: "Mira".into(),
            app_version: "0.8.6".into(),
            app_identifier: "com.mira.app".into(),
            platform: "macos".into(),
            architecture: "aarch64".into(),
            rust_version: "1.85.0".into(),
            session_id: "session-test".into(),
            app_info_json: "".into(),
            plugin_status_json: "".into(),
            local_ai_status_json: "".into(),
            recent_error_count: 0,
            recent_warn_count: 0,
        };
        let outcome = export_diagnostics_bundle(vec![], &ctx, &path).unwrap();
        assert_eq!(outcome.entry_count, 0);
        assert!(path.exists());
    }
}
