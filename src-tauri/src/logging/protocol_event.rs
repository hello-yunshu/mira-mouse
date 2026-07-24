// SPDX-License-Identifier: AGPL-3.0-or-later
//! 结构化协议事件构造器。
//!
//! 为插件读取、HID 交换、inventory 等协议层操作提供统一的 `LogInput` 构造器。
//! 所有事件共享 `event` 字段（事件类型）与 `correlationId`（关联一次读取会话），
//! 便于"复制当前设备诊断"按 `correlationId` + `target` 前缀筛选。
//!
//! 设计约束（对齐 spec 十五.1 / 十五.2）：
//! - 默认不包含 HID payload（request hex / response hex）。
//! - payload 仅在"协议诊断模式"启用时由 `hid_feature_exchange_with_payload` 携带，
//!   且必须经过 command-aware masking。
//! - 字段值仅使用 `FieldValue` 标量类型，不扩大为递归 JSON。
//! - 所有事件 target 以 `plugin::` 或 `hid::` 前缀开头，便于 `target_prefix` 过滤。

use crate::logging::model::{FieldValue, Fields, LogInput, LogLevel, LogSource};

/// 协议事件 target 前缀。用于 LogQuery::target_prefix 过滤。
#[allow(dead_code)]
pub const TARGET_PREFIX_PLUGIN: &str = "plugin::";
#[allow(dead_code)]
pub const TARGET_PREFIX_HID: &str = "hid::";

/// 事件类型常量。用于 LogInput.fields["event"] 字段。
pub mod event {
    pub const PLUGIN_READ_STARTED: &str = "plugin-read-started";
    pub const PLUGIN_READ_COMPLETED: &str = "plugin-read-completed";
    pub const PLUGIN_READ_FAILED: &str = "plugin-read-failed";
    pub const PLUGIN_READ_STEP_SUCCEEDED: &str = "plugin-read-step-succeeded";
    pub const PLUGIN_READ_STEP_SKIPPED: &str = "plugin-read-step-skipped";
    pub const PLUGIN_READ_STEP_NOT_SUPPORTED: &str = "plugin-read-step-not-supported";
    pub const PLUGIN_READ_STEP_FAILED: &str = "plugin-read-step-failed";
    pub const PLUGIN_INVENTORY_COMPLETED: &str = "plugin-inventory-completed";
    pub const PLUGIN_INVENTORY_PARTIAL: &str = "plugin-inventory-partial";
    pub const HID_FEATURE_EXCHANGE: &str = "hid-feature-exchange";
    pub const HID_BUSY_RETRY: &str = "hid-busy-retry";
    pub const HID_RESPONSE_MISMATCH: &str = "hid-response-mismatch";
    pub const HID_CHECKSUM_FAILED: &str = "hid-checksum-failed";
}

/// 协议事件公共字段。所有协议事件都应包含这些字段（若已知）。
///
/// 字段名遵循已存在的允许列表（redaction.rs::ALLOWED_NAME_PARTS），
/// 因此不会被脱敏器误判为敏感字段。
pub struct ProtocolEventContext<'a> {
    pub correlation_id: &'a str,
    pub plugin_id: &'a str,
    pub plugin_version: &'a str,
    pub family: &'a str,
    pub model: Option<&'a str>,
    pub device_key: &'a str,
    pub vendor_id: u16,
    pub product_id: u16,
    pub connection: &'a str,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: Option<i32>,
}

impl<'a> ProtocolEventContext<'a> {
    /// 将公共字段写入 Fields 表。
    fn write_common(&self, fields: &mut Fields) {
        fields.insert("pluginId".into(), FieldValue::from(self.plugin_id));
        fields.insert(
            "pluginVersion".into(),
            FieldValue::from(self.plugin_version),
        );
        fields.insert("family".into(), FieldValue::from(self.family));
        if let Some(model) = self.model {
            fields.insert("model".into(), FieldValue::from(model));
        }
        fields.insert("deviceKey".into(), FieldValue::from(self.device_key));
        fields.insert(
            "vendorId".into(),
            FieldValue::from(format!("0x{:04x}", self.vendor_id)),
        );
        fields.insert(
            "productId".into(),
            FieldValue::from(format!("0x{:04x}", self.product_id)),
        );
        fields.insert("connection".into(), FieldValue::from(self.connection));
        fields.insert(
            "usagePage".into(),
            FieldValue::from(format!("0x{:04x}", self.usage_page)),
        );
        fields.insert(
            "usage".into(),
            FieldValue::from(format!("0x{:04x}", self.usage)),
        );
        if let Some(iface) = self.interface_number {
            fields.insert("interfaceNumber".into(), FieldValue::from(iface as i64));
        }
    }
}

/// 创建一个协议事件 LogInput，附带公共字段。
fn new_protocol_event(
    level: LogLevel,
    target: &'static str,
    event: &'static str,
    message: String,
    ctx: &ProtocolEventContext,
    extra: Vec<(&'static str, FieldValue)>,
) -> LogInput {
    let mut fields = Fields::new();
    ctx.write_common(&mut fields);
    fields.insert("event".into(), FieldValue::from(event));
    for (k, v) in extra {
        fields.insert(k.into(), v);
    }
    LogInput {
        level,
        source: LogSource::Plugin,
        target: target.into(),
        message,
        correlation_id: Some(ctx.correlation_id.into()),
        fields,
    }
}

/// plugin-read-started：一次设备读取会话开始。
pub fn plugin_read_started(
    ctx: &ProtocolEventContext,
    workflow: &str,
    read_plan: &str,
) -> LogInput {
    new_protocol_event(
        LogLevel::Info,
        "plugin::read",
        event::PLUGIN_READ_STARTED,
        format!("read started: workflow={workflow} plan={read_plan}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("readPlan", FieldValue::from(read_plan)),
        ],
    )
}

/// plugin-read-completed：一次设备读取会话成功完成。
pub fn plugin_read_completed(
    ctx: &ProtocolEventContext,
    workflow: &str,
    read_plan: &str,
    duration_ms: u64,
    successful_outputs: usize,
    failed_outputs: usize,
    projection_valid: bool,
) -> LogInput {
    new_protocol_event(
        LogLevel::Info,
        "plugin::read",
        event::PLUGIN_READ_COMPLETED,
        format!(
            "read completed: workflow={workflow} plan={read_plan} ({successful_outputs} ok, {failed_outputs} failed)"
        ),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("readPlan", FieldValue::from(read_plan)),
            ("durationMs", FieldValue::from(duration_ms)),
            (
                "successfulOutputs",
                FieldValue::from(successful_outputs as i64),
            ),
            (
                "failedOutputs",
                FieldValue::from(failed_outputs as i64),
            ),
            ("projectionValid", FieldValue::from(projection_valid)),
            ("status", FieldValue::from("ok")),
        ],
    )
}

/// plugin-read-failed：一次设备读取会话失败。
pub fn plugin_read_failed(
    ctx: &ProtocolEventContext,
    workflow: &str,
    read_plan: &str,
    duration_ms: u64,
    error_kind: &str,
    reason: &str,
) -> LogInput {
    // reason 截断至 256 字符，避免过长的错误消息。
    let reason_truncated: String = reason.chars().take(256).collect();
    new_protocol_event(
        LogLevel::Warn,
        "plugin::read",
        event::PLUGIN_READ_FAILED,
        format!("read failed: workflow={workflow} plan={read_plan}: {error_kind}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("readPlan", FieldValue::from(read_plan)),
            ("durationMs", FieldValue::from(duration_ms)),
            ("errorKind", FieldValue::from(error_kind)),
            ("reason", FieldValue::from(reason_truncated)),
            ("status", FieldValue::from("failed")),
        ],
    )
}

/// plugin-read-step-succeeded：一个 workflow step 成功完成。
#[allow(clippy::too_many_arguments)]
pub fn plugin_read_step_succeeded(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    parser: &str,
    output: &str,
    duration_ms: u64,
    response_length: usize,
    cache_hit: bool,
) -> LogInput {
    new_protocol_event(
        LogLevel::Debug,
        "plugin::read::step",
        event::PLUGIN_READ_STEP_SUCCEEDED,
        format!("step ok: {command} → {output}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("command", FieldValue::from(command)),
            ("parser", FieldValue::from(parser)),
            ("output", FieldValue::from(output)),
            ("durationMs", FieldValue::from(duration_ms)),
            ("responseLength", FieldValue::from(response_length as i64)),
            ("cacheHit", FieldValue::from(cache_hit)),
            ("status", FieldValue::from("ok")),
        ],
    )
}

/// plugin-read-step-skipped：step 被 skip_if_zero 跳过。
pub fn plugin_read_step_skipped(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    output: &str,
) -> LogInput {
    new_protocol_event(
        LogLevel::Debug,
        "plugin::read::step",
        event::PLUGIN_READ_STEP_SKIPPED,
        format!("step skipped: {command} → {output}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("command", FieldValue::from(command)),
            ("output", FieldValue::from(output)),
            ("status", FieldValue::from("skipped")),
        ],
    )
}

/// plugin-read-step-not-supported：step 返回 not-supported 状态。
pub fn plugin_read_step_not_supported(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    output: &str,
) -> LogInput {
    new_protocol_event(
        LogLevel::Info,
        "plugin::read::step",
        event::PLUGIN_READ_STEP_NOT_SUPPORTED,
        format!("step not-supported: {command} → {output}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("command", FieldValue::from(command)),
            ("output", FieldValue::from(output)),
            ("status", FieldValue::from("not-supported")),
        ],
    )
}

/// plugin-read-step-failed：一个可选 step 失败但 workflow 继续。
#[allow(clippy::too_many_arguments)]
pub fn plugin_read_step_failed(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    parser: &str,
    output: &str,
    error_kind: &str,
    reason: &str,
    duration_ms: Option<u64>,
) -> LogInput {
    let mut extras = vec![
        ("workflow", FieldValue::from(workflow)),
        ("command", FieldValue::from(command)),
        ("parser", FieldValue::from(parser)),
        ("output", FieldValue::from(output)),
        ("errorKind", FieldValue::from(error_kind)),
        ("status", FieldValue::from("failed")),
    ];
    if let Some(ms) = duration_ms {
        extras.push(("durationMs", FieldValue::from(ms)));
    }
    // reason 截断至 256 字符，避免过长的错误消息。
    let reason_truncated: String = reason.chars().take(256).collect();
    extras.push(("reason", FieldValue::from(reason_truncated)));
    new_protocol_event(
        LogLevel::Warn,
        "plugin::read::step",
        event::PLUGIN_READ_STEP_FAILED,
        format!("step failed: {command} → {output}: {error_kind}"),
        ctx,
        extras,
    )
}

/// plugin-inventory-completed：inventory 工作流完整完成。
pub fn plugin_inventory_completed(
    ctx: &ProtocolEventContext,
    workflow: &str,
    duration_ms: u64,
    successful_outputs: usize,
) -> LogInput {
    new_protocol_event(
        LogLevel::Info,
        "plugin::inventory",
        event::PLUGIN_INVENTORY_COMPLETED,
        format!("inventory completed: {workflow} ({successful_outputs} outputs)"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("durationMs", FieldValue::from(duration_ms)),
            (
                "successfulOutputs",
                FieldValue::from(successful_outputs as i64),
            ),
            ("partial", FieldValue::from(false)),
        ],
    )
}

/// plugin-inventory-partial：inventory 工作流部分完成（有失败但已合并）。
pub fn plugin_inventory_partial(
    ctx: &ProtocolEventContext,
    workflow: &str,
    duration_ms: u64,
    successful_outputs: usize,
    failed_outputs: usize,
) -> LogInput {
    new_protocol_event(
        LogLevel::Warn,
        "plugin::inventory",
        event::PLUGIN_INVENTORY_PARTIAL,
        format!("inventory partial: {workflow} ({successful_outputs} ok, {failed_outputs} failed)"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("durationMs", FieldValue::from(duration_ms)),
            (
                "successfulOutputs",
                FieldValue::from(successful_outputs as i64),
            ),
            ("failedOutputs", FieldValue::from(failed_outputs as i64)),
            ("partial", FieldValue::from(true)),
        ],
    )
}

/// hid-feature-exchange：一次 HID feature report 交换。
///
/// 默认不携带 payload。仅在 `include_payload=true` 且协议诊断模式启用时，
/// 调用方应传入 `request_hex` / `response_hex`（已脱敏）。
#[allow(clippy::too_many_arguments)]
pub fn hid_feature_exchange(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    transaction_id: Option<u16>,
    command_class: Option<u16>,
    command_id: Option<u16>,
    attempt: u32,
    busy_reads: u32,
    duration_ms: u64,
    request_length: usize,
    response_length: usize,
    checksum_valid: Option<bool>,
    correlation_valid: Option<bool>,
    request_hex: Option<&str>,
    response_hex: Option<&str>,
) -> LogInput {
    let mut extras = vec![
        ("workflow", FieldValue::from(workflow)),
        ("command", FieldValue::from(command)),
        ("attempt", FieldValue::from(attempt as i64)),
        ("busyReads", FieldValue::from(busy_reads as i64)),
        ("durationMs", FieldValue::from(duration_ms)),
        ("requestLength", FieldValue::from(request_length as i64)),
        ("responseLength", FieldValue::from(response_length as i64)),
    ];
    if let Some(tid) = transaction_id {
        extras.push(("transactionId", FieldValue::from(format!("0x{:02x}", tid))));
    }
    if let Some(class) = command_class {
        extras.push(("commandClass", FieldValue::from(format!("0x{:02x}", class))));
    }
    if let Some(id) = command_id {
        extras.push(("commandId", FieldValue::from(format!("0x{:02x}", id))));
    }
    if let Some(valid) = checksum_valid {
        extras.push(("checksumValid", FieldValue::from(valid)));
    }
    if let Some(valid) = correlation_valid {
        extras.push(("correlationValid", FieldValue::from(valid)));
    }
    // payload 仅在显式传入时携带（调用方应仅在协议诊断模式启用时传入）。
    // Redactor 会再次扫描敏感模式，但调用方有责任先做 command-aware masking。
    if let Some(hex) = request_hex {
        extras.push(("requestHex", FieldValue::from(hex)));
    }
    if let Some(hex) = response_hex {
        extras.push(("responseHex", FieldValue::from(hex)));
    }
    new_protocol_event(
        LogLevel::Trace,
        "hid::feature-exchange",
        event::HID_FEATURE_EXCHANGE,
        format!("hid exchange: {command} (attempt {attempt})"),
        ctx,
        extras,
    )
}

/// hid-busy-retry：设备返回 BUSY，重读。
pub fn hid_busy_retry(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    attempt: u32,
    max_attempts: u32,
) -> LogInput {
    new_protocol_event(
        LogLevel::Debug,
        "hid::busy-retry",
        event::HID_BUSY_RETRY,
        format!("busy retry: {command} attempt {attempt}/{max_attempts}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("command", FieldValue::from(command)),
            ("attempt", FieldValue::from(attempt as i64)),
            ("maxAttempts", FieldValue::from(max_attempts as i64)),
        ],
    )
}

/// hid-response-mismatch：响应 transaction/class/id 与请求不匹配。
pub fn hid_response_mismatch(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    expected: &str,
    actual: &str,
) -> LogInput {
    new_protocol_event(
        LogLevel::Warn,
        "hid::response-mismatch",
        event::HID_RESPONSE_MISMATCH,
        format!("response mismatch: {command} expected={expected} actual={actual}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("command", FieldValue::from(command)),
            ("expected", FieldValue::from(expected)),
            ("actual", FieldValue::from(actual)),
        ],
    )
}

/// hid-checksum-failed：响应校验和不匹配。
pub fn hid_checksum_failed(
    ctx: &ProtocolEventContext,
    workflow: &str,
    command: &str,
    expected: u8,
    actual: u8,
) -> LogInput {
    new_protocol_event(
        LogLevel::Warn,
        "hid::checksum-failed",
        event::HID_CHECKSUM_FAILED,
        format!("checksum failed: {command} expected=0x{expected:02x} actual=0x{actual:02x}"),
        ctx,
        vec![
            ("workflow", FieldValue::from(workflow)),
            ("command", FieldValue::from(command)),
            ("expectedChecksum", FieldValue::from(expected as i64)),
            ("actualChecksum", FieldValue::from(actual as i64)),
        ],
    )
}

// ===========================================================================
// 协议诊断模式：command-aware HID payload masking
//
// 对齐 spec 13.2：默认诊断不包含 HID payload。只有用户明确启动"协议诊断模式"
// 后才临时记录 request/response hex，且必须经过 command-aware masking：
// - serial / macro / bind / credential 等命令的 payload 被脱敏或拒绝；
// - 每条 payload 长度有上限；
// - 宏 payload 永远不自动进入日志。
// ===========================================================================

/// payload hex 字符串最大长度（字符数，含空格分隔）。
/// 90 字节 report ≈ 269 字符（含空格）；512 足以覆盖大多数 HID report 且防止单条日志过大。
pub const MAX_PAYLOAD_HEX_LEN: usize = 512;

/// `MaskSensitive` 策略保留的协议头字节数。
///
/// 大多数 HID 协议的前几字节是协议头（status/report_id + transaction/device_index +
/// command_class/feature_id + command_id/function_id + reserved/sw_id），
/// 保留这些字节足以诊断协议交换结构，又不泄露设备特有数据（serial、宏内容等）。
/// 雷蛇 90B：前 5 字节 = status(1) + txn(1) + class(1) + id(1) + reserved(1)。
/// Logitech HID++：前 4-5 字节 = report_id + device_index + feature_id + function_id + sw_id。
pub const MASK_KEEP_PREFIX_BYTES: usize = 5;

/// 命令的 HID payload 处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadPolicy {
    /// 允许记录完整 payload（受长度上限限制）。
    Allow,
    /// 记录 payload 但对敏感数据字节脱敏（保留协议头，其余替换为 `[redacted]`）。
    MaskSensitive,
    /// 完全拒绝记录 payload。
    Deny,
}

/// 根据命令标识符分类 payload 处理策略。
///
/// 品牌无关的关键词匹配。调用方传入 workflow step 的 `command` 字符串
/// （通常是命令在 commands.json 中的 id 或 name）。
///
/// - macro / bind / remap / password / credential / secret → `Deny`
/// - serial → `MaskSensitive`
/// - 其他 → `Allow`
pub fn classify_command(command: &str) -> PayloadPolicy {
    let lower = command.to_ascii_lowercase();
    // 宏、按键绑定、凭据：永远不记录 payload
    if lower.contains("macro")
        || lower.contains("bind")
        || lower.contains("remap")
        || lower.contains("password")
        || lower.contains("credential")
        || lower.contains("secret")
    {
        return PayloadPolicy::Deny;
    }
    // 序列号：脱敏数据字节（保留协议头）
    if lower.contains("serial") {
        return PayloadPolicy::MaskSensitive;
    }
    PayloadPolicy::Allow
}

/// 将字节切片格式化为 hex 字符串（空格分隔，大写）。
pub fn format_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 对 payload hex 字符串应用 masking 策略。
///
/// 返回 `None` 表示命令策略为 `Deny`（不记录 payload）。
/// 返回 `Some(masked_hex)` 表示可以记录的（可能已脱敏的）payload hex。
pub fn mask_payload(hex: &str, policy: PayloadPolicy) -> Option<String> {
    match policy {
        PayloadPolicy::Deny => None,
        PayloadPolicy::Allow => Some(truncate_hex(hex, MAX_PAYLOAD_HEX_LEN)),
        PayloadPolicy::MaskSensitive => Some(mask_hex_prefix(hex)),
    }
}

/// 截断 hex 字符串到最大长度（按字符数，不切断 hex 对）。
fn truncate_hex(hex: &str, max_len: usize) -> String {
    if hex.len() <= max_len {
        return hex.to_string();
    }
    let truncated: String = hex.chars().take(max_len).collect();
    format!("{truncated}…")
}

/// 保留 hex 字符串的前 `MASK_KEEP_PREFIX_BYTES` 字节，其余替换为 `[redacted:N bytes]`。
fn mask_hex_prefix(hex: &str) -> String {
    let tokens: Vec<&str> = hex.split_whitespace().collect();
    if tokens.len() <= MASK_KEEP_PREFIX_BYTES {
        // payload 比协议头还短，无法脱敏。太短没有敏感数据风险，直接返回。
        return hex.to_string();
    }
    let prefix = tokens[..MASK_KEEP_PREFIX_BYTES].join(" ");
    let redacted_count = tokens.len() - MASK_KEEP_PREFIX_BYTES;
    format!("{prefix} [redacted:{redacted_count} bytes]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> ProtocolEventContext<'static> {
        ProtocolEventContext {
            correlation_id: "device-abcd1234",
            plugin_id: "mira.razer-chroma",
            plugin_version: "0.1.0",
            family: "razer-modern-1f-usb",
            model: Some("basilisk-v3"),
            device_key: "0001:0002:00",
            vendor_id: 0x1532,
            product_id: 0x0099,
            connection: "usb",
            usage_page: 0xFF00,
            usage: 0x0001,
            interface_number: Some(0),
        }
    }

    #[test]
    fn plugin_read_started_has_common_fields() {
        let ctx = make_ctx();
        let input = plugin_read_started(&ctx, "razer-modern-1f-read", "quick");
        assert_eq!(input.level, LogLevel::Info);
        assert_eq!(input.source, LogSource::Plugin);
        assert_eq!(input.target, "plugin::read");
        assert_eq!(input.correlation_id.as_deref(), Some("device-abcd1234"));
        assert_eq!(
            input.fields.get("event").and_then(|v| match v {
                FieldValue::Text(s) => Some(s.as_str()),
                _ => None,
            }),
            Some("plugin-read-started")
        );
        assert!(input.fields.contains_key("pluginId"));
        assert!(input.fields.contains_key("model"));
        assert!(input.fields.contains_key("workflow"));
        assert!(input.fields.contains_key("readPlan"));
    }

    #[test]
    fn step_failed_truncates_long_reason() {
        let ctx = make_ctx();
        let long_reason = "x".repeat(500);
        let input = plugin_read_step_failed(
            &ctx,
            "wf",
            "cmd",
            "parser",
            "out",
            "timeout",
            &long_reason,
            None,
        );
        match input.fields.get("reason") {
            Some(FieldValue::Text(s)) => assert_eq!(s.len(), 256),
            other => panic!("expected truncated text, got {other:?}"),
        }
    }

    #[test]
    fn hid_feature_exchange_omits_payload_by_default() {
        let ctx = make_ctx();
        let input = hid_feature_exchange(
            &ctx,
            "wf",
            "cmd",
            Some(0x1F),
            Some(0x04),
            Some(0x85),
            1,
            0,
            12,
            90,
            90,
            Some(true),
            Some(true),
            None,
            None,
        );
        assert!(!input.fields.contains_key("requestHex"));
        assert!(!input.fields.contains_key("responseHex"));
        assert!(input.fields.contains_key("transactionId"));
        assert!(input.fields.contains_key("commandClass"));
        assert!(input.fields.contains_key("commandId"));
        assert_eq!(input.level, LogLevel::Trace);
    }

    #[test]
    fn hid_feature_exchange_includes_payload_when_provided() {
        let ctx = make_ctx();
        let input = hid_feature_exchange(
            &ctx,
            "wf",
            "cmd",
            None,
            None,
            None,
            1,
            0,
            5,
            90,
            90,
            None,
            None,
            Some("00 1f 04 85 00"),
            Some("02 1f 04 85 00 ..."),
        );
        assert!(input.fields.contains_key("requestHex"));
        assert!(input.fields.contains_key("responseHex"));
    }

    #[test]
    fn target_prefixes_are_distinct() {
        assert!(TARGET_PREFIX_PLUGIN != TARGET_PREFIX_HID);
        assert!(TARGET_PREFIX_PLUGIN.ends_with("::"));
        assert!(TARGET_PREFIX_HID.ends_with("::"));
    }

    #[test]
    fn inventory_partial_has_partial_flag() {
        let ctx = make_ctx();
        let input = plugin_inventory_partial(&ctx, "wf", 100, 5, 2);
        match input.fields.get("partial") {
            Some(FieldValue::Boolean(b)) => assert!(*b),
            other => panic!("expected partial=true, got {other:?}"),
        }
        match input.fields.get("failedOutputs") {
            Some(FieldValue::Integer(n)) => assert_eq!(*n, 2),
            other => panic!("expected failedOutputs=2, got {other:?}"),
        }
    }

    // ---- payload masking 测试 ----

    #[test]
    fn classify_command_allows_normal_commands() {
        assert_eq!(classify_command("getDpi"), PayloadPolicy::Allow);
        assert_eq!(classify_command("getPollingRate"), PayloadPolicy::Allow);
        assert_eq!(classify_command("getBattery"), PayloadPolicy::Allow);
        assert_eq!(classify_command("getFeatureIndex"), PayloadPolicy::Allow);
    }

    #[test]
    fn classify_command_denies_macro_and_bind() {
        assert_eq!(classify_command("getMacro"), PayloadPolicy::Deny);
        assert_eq!(classify_command("setMacro"), PayloadPolicy::Deny);
        assert_eq!(classify_command("getKeyBind"), PayloadPolicy::Deny);
        assert_eq!(classify_command("remapKey"), PayloadPolicy::Deny);
        assert_eq!(classify_command("getPassword"), PayloadPolicy::Deny);
        assert_eq!(classify_command("setCredential"), PayloadPolicy::Deny);
    }

    #[test]
    fn classify_command_masks_serial() {
        assert_eq!(classify_command("getSerial"), PayloadPolicy::MaskSensitive);
        assert_eq!(
            classify_command("readSerialNumber"),
            PayloadPolicy::MaskSensitive
        );
    }

    #[test]
    fn classify_command_is_case_insensitive() {
        assert_eq!(classify_command("GETMACRO"), PayloadPolicy::Deny);
        assert_eq!(classify_command("GetSerial"), PayloadPolicy::MaskSensitive);
        assert_eq!(classify_command("GETDPI"), PayloadPolicy::Allow);
    }

    #[test]
    fn mask_payload_deny_returns_none() {
        assert_eq!(mask_payload("00 1f 04", PayloadPolicy::Deny), None);
    }

    #[test]
    fn mask_payload_allow_keeps_full_hex() {
        let hex = "00 1F 04 85 00 01 02 03";
        let result = mask_payload(hex, PayloadPolicy::Allow);
        assert_eq!(result.as_deref(), Some(hex));
    }

    #[test]
    fn mask_payload_allow_truncates_long_hex() {
        let long = (0..200)
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let result = mask_payload(&long, PayloadPolicy::Allow).unwrap();
        assert!(result.len() <= MAX_PAYLOAD_HEX_LEN + 4); // +4 for ellipsis
        assert!(result.ends_with('…'));
    }

    #[test]
    fn mask_payload_mask_sensitive_redacts_data_bytes() {
        // 90 字节 payload：前 5 字节保留，其余替换为 [redacted:85 bytes]
        let hex: String = (0..90)
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let result = mask_payload(&hex, PayloadPolicy::MaskSensitive).unwrap();
        assert!(result.contains("00 01 02 03 04"));
        assert!(result.contains("[redacted:85 bytes]"));
        // serial number 字节不应出现
        assert!(!result.contains("05 06 07 08"));
    }

    #[test]
    fn mask_payload_mask_sensitive_short_payload_kept_as_is() {
        // 短于协议头的 payload 直接保留（无敏感数据风险）
        let hex = "00 1F 04";
        let result = mask_payload(hex, PayloadPolicy::MaskSensitive).unwrap();
        assert_eq!(result, hex);
    }

    #[test]
    fn format_hex_produces_uppercase_spaced_output() {
        let bytes = [0x00, 0x1f, 0xab, 0xff];
        assert_eq!(format_hex(&bytes), "00 1F AB FF");
    }

    #[test]
    fn mask_payload_continuous_hex_serial_is_redacted() {
        // 模拟 runtime 传入的连续 hex 格式（hex::encode 输出）
        // 90 字节 serial payload，前 5 字节协议头后是序列号数据
        let continuous_hex: String = (0..90u8).map(|b| format!("{b:02x}")).collect();
        // 先通过 format_hex 转换为空格分隔（host 侧 normalize_hex 逻辑）
        let bytes = hex::decode(&continuous_hex).unwrap();
        let spaced_hex = format_hex(&bytes);
        let result = mask_payload(&spaced_hex, PayloadPolicy::MaskSensitive).unwrap();
        assert!(result.contains("[redacted:"));
        // 序列号区域不应完整出现
        assert!(!result.contains("05 06 07 08 09 0A 0B 0C"));
    }
}
