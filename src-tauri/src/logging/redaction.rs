// SPDX-License-Identifier: AGPL-3.0-or-later
//! 统一脱敏层。所有进入持久化、前端显示或导出的日志都先经过这里。
//!
//! 策略：字段名允许列表 + 拒绝列表结合；值类型按敏感模式匹配。
//! 脱敏逻辑必须有单元测试，且不会失败（任何错误降级为占位符）。

use crate::logging::model::{FieldValue, LogEntry};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// 单条字符串最大长度（消息与字段值共用）。
pub const MAX_STRING_LEN: usize = 2_048;
/// 单条字段值的最大长度（独立于消息）。
pub const MAX_FIELD_LEN: usize = 512;

/// 敏感字段名关键字（小写匹配）。命中任一即视为敏感，整值替换为占位符。
/// 注意：键名匹配采用包含子串策略，避免遗漏变体。
const SENSITIVE_NAME_PARTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api-key",
    "authorization",
    "auth",
    "cookie",
    "session",
    "credential",
    "privatekey",
    "private-key",
    "signingkey",
    "signing-key",
    "pubkey",
    "public-key",
    // 设备标识类
    "serial",
    "serialnumber",
    "serial-number",
    "imei",
    "uuid",
    "deviceid",
    "device-id",
    "macaddress",
    "mac-address",
    "bdaddr",
    "btaddr",
];

/// 允许保留的字段名前缀/关键字。命中即跳过敏感匹配。
/// 用于 pluginId / workflowId / mutationId 等明确技术字段。
const ALLOWED_NAME_PARTS: &[&str] = &[
    "pluginid",
    "plugin-id",
    "pluginversion",
    "plugin-version",
    "workflowid",
    "workflow-id",
    "mutationid",
    "mutation-id",
    "capability",
    "operation",
    "result",
    "errorcode",
    "error-code",
    "durationms",
    "duration-ms",
    "count",
    "bytes",
    "stage",
    "level",
    "source",
    "target",
    "module",
    "version",
    "platform",
    "arch",
    "elapsed",
    "retry",
    "attempt",
];

/// 占位符。
pub const REDACTED: &str = "[redacted]";
pub const HOME_PLACEHOLDER: &str = "${HOME}";
pub const USER_PLACEHOLDER: &str = "${USER}";

/// 脱敏器。生命周期等同于应用；持有当前用户主目录缓存用于路径替换。
#[derive(Debug, Clone)]
pub struct Redactor {
    home_dir: Option<PathBuf>,
    user_name: Option<String>,
    /// 已知的设备 serial / id 集合。运行时可注入以便精确替换；空集合时退化为模式匹配。
    known_device_ids: BTreeSet<String>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self {
            home_dir: std::env::var_os("HOME").map(PathBuf::from),
            user_name: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
            known_device_ids: BTreeSet::new(),
        }
    }
}

impl Redactor {
    /// 测试用构造器：指定 home_dir 与 user_name，避免依赖环境变量。
    #[cfg(test)]
    pub fn new(home_dir: Option<PathBuf>, user_name: Option<String>) -> Self {
        Self {
            home_dir,
            user_name,
            known_device_ids: BTreeSet::new(),
        }
    }

    /// 测试用：注入已知设备 ID，验证精确替换逻辑。
    #[cfg(test)]
    pub fn register_device_id(&mut self, id: String) {
        self.known_device_ids.insert(id);
    }

    /// 返回主目录路径（用于路径替换展示）。
    pub fn home_dir(&self) -> Option<&std::path::Path> {
        self.home_dir.as_deref()
    }

    /// 返回用户名（用于路径替换展示）。
    pub fn user_name(&self) -> Option<&str> {
        self.user_name.as_deref()
    }

    /// 应用到单条日志。原地修改并返回。
    pub fn apply(&self, entry: &mut LogEntry) {
        entry.message = self.redact_text(&entry.message, MAX_STRING_LEN);
        // target 是模块路径，不做路径替换，但截断控制字符。
        entry.target = strip_control_chars(&entry.target);
        if let Some(correlation) = entry.correlation_id.as_mut() {
            *correlation = self.redact_text(correlation, 128);
        }
        // 字段表：键名敏感则整值替换；否则按值类型处理。
        // 收集到 Vec 避免在迭代中修改 BTreeMap。
        let keys: Vec<String> = entry.fields.keys().cloned().collect();
        for key in keys {
            let sensitive = is_sensitive_name(&key);
            let value = entry.fields.get(&key).cloned();
            if let Some(mut value) = value {
                if sensitive {
                    value = FieldValue::Text(REDACTED.into());
                } else {
                    self.redact_value(&mut value);
                }
                entry.fields.insert(key, value);
            }
        }
    }

    /// 脱敏文本。先替换路径/用户名/已知 ID，再截断与清理控制字符。
    pub fn redact_text(&self, text: &str, max_len: usize) -> String {
        if text.is_empty() {
            return String::new();
        }
        let mut out = text.to_string();
        // 1. 主目录路径替换为 ${HOME}（跨平台：Unix 风格与 OS 字符串均处理）。
        if let Some(home) = self.home_dir.as_ref() {
            let home_str = home.to_string_lossy();
            if !home_str.is_empty() {
                // 处理 /Users/foo 与 C:\Users\foo 两种形式。
                if out.contains(home_str.as_ref()) {
                    out = out.replace(home_str.as_ref(), HOME_PLACEHOLDER);
                }
                // 同时替换未带分隔符的用户名片段。
                if let Some(file_name) = home.file_name() {
                    let name = file_name.to_string_lossy();
                    if !name.is_empty() && name.len() >= 2 {
                        // 仅在路径上下文中替换：以 /name/ 或 \name\ 或 :Users/name 出现。
                        // 保守起见，替换 /name/ 风格（Unix）。
                        let unix_pattern = format!("/{name}/");
                        if out.contains(&unix_pattern) {
                            out = out.replace(&unix_pattern, &format!("/{USER_PLACEHOLDER}/"));
                        }
                    }
                }
            }
        }
        // 2. USERNAME / USER 环境变量替换。
        if let Some(user) = self.user_name.as_ref() {
            if !user.is_empty() && user.len() >= 2 {
                // 仅在路径上下文（/Users/<user> 或 \Users\<user>）中替换，避免误伤普通英文词。
                let mac_pattern = format!("/Users/{user}");
                if out.contains(&mac_pattern) {
                    out = out.replace(&mac_pattern, &format!("/Users/{USER_PLACEHOLDER}"));
                }
                let win_pattern = format!(r"\Users\{user}");
                if out.contains(&win_pattern) {
                    out = out.replace(&win_pattern, &format!(r"\Users\{USER_PLACEHOLDER}"));
                }
            }
        }
        // 3. 已知设备标识替换为 ${DEVICE_ID}。
        for id in &self.known_device_ids {
            if !id.is_empty() && out.contains(id.as_str()) {
                out = out.replace(id.as_str(), "${DEVICE_ID}");
            }
        }
        // 4. URL 凭据与查询参数脱敏。
        out = redact_url_credentials(&out);
        // 5. Bearer / token 模式脱敏。
        out = redact_inline_tokens(&out);
        // 6. 长字符串截断。
        if out.len() > max_len {
            // 按字符边界截断，避免切断 UTF-8。
            let truncated: String = out.chars().take(max_len).collect();
            out = format!("{truncated}…");
        }
        // 7. 控制字符清理。
        out = strip_control_chars(&out);
        out
    }

    /// 脱敏字段值。
    fn redact_value(&self, value: &mut FieldValue) {
        match value {
            FieldValue::Text(text) => {
                *text = self.redact_text(text, MAX_FIELD_LEN);
            }
            FieldValue::Number(_)
            | FieldValue::Integer(_)
            | FieldValue::Boolean(_)
            | FieldValue::Null => {
                // 数值与布尔不脱敏。
            }
        }
    }
}

/// 判断字段名是否敏感。允许列表优先于拒绝列表。
pub fn is_sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // 允许列表优先：明确技术字段不算敏感。
    for allowed in ALLOWED_NAME_PARTS {
        if lower.contains(allowed) {
            return false;
        }
    }
    for part in SENSITIVE_NAME_PARTS {
        if lower.contains(part) {
            return true;
        }
    }
    false
}

/// 替换 URL 中的凭据（user:pass@）与查询参数。
pub fn redact_url_credentials(text: &str) -> String {
    // 形如 scheme://user:pass@host 的凭据。
    // 使用简单字符串扫描，避免引入 regex 依赖。
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 检测 scheme://
        if let Some(end) = find_scheme_end(&bytes[i..]) {
            // 复制 scheme://
            out.push_str(&text[i..i + end]);
            i += end;
            // 找到下一个 '@' 之前是否有 ':'（凭据）。
            let rest = &text[i..];
            if let Some(at_pos) = rest.find('@') {
                if let Some(_colon_pos) = rest[..at_pos].find(':') {
                    // user:pass@ → ${REDACTED}@
                    out.push_str(REDACTED);
                    out.push('@');
                    i += at_pos + 1;
                    continue;
                }
            }
            continue;
        }
        // 复制一个字节并推进。
        let ch = text[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    // 查询参数脱敏：?key=value&... 中的 key=... 整值替换（保守策略：所有参数值替换）。
    // 仅当参数名匹配敏感关键字时替换，避免破坏普通 URL。
    out = redact_query_params(&out);
    out
}

fn find_scheme_end(bytes: &[u8]) -> Option<usize> {
    // 查找 "://" 的位置。
    if bytes.len() < 4 {
        return None;
    }
    for i in 0..bytes.len() - 2 {
        if bytes[i] == b':' && bytes[i + 1] == b'/' && bytes[i + 2] == b'/' {
            // scheme 必须以字母开头。
            if bytes[0].is_ascii_alphabetic() {
                return Some(i + 3);
            }
            return None;
        }
    }
    None
}

fn redact_query_params(text: &str) -> String {
    // 在 ?.. 与 &.. 段中按参数名是否敏感决定是否替换值。
    let q_pos = match text.find('?') {
        Some(pos) => pos,
        None => return text.to_string(),
    };
    let (prefix, query) = text.split_at(q_pos + 1);
    let mut out = String::from(prefix);
    let mut first = true;
    for pair in query.split('&') {
        if !first {
            out.push('&');
        }
        first = false;
        if pair.is_empty() {
            continue;
        }
        if let Some(eq_pos) = pair.find('=') {
            let name = &pair[..eq_pos];
            let value = &pair[eq_pos + 1..];
            if is_sensitive_name(name) {
                out.push_str(name);
                out.push('=');
                out.push_str(REDACTED);
            } else {
                out.push_str(name);
                out.push('=');
                out.push_str(value);
            }
        } else {
            out.push_str(pair);
        }
    }
    out
}

/// 替换 inline bearer / token 模式。
pub fn redact_inline_tokens(text: &str) -> String {
    // 形如 `Bearer eyJ...`、`token: abc...`、`api_key=...` 的模式。
    // 不引入 regex，使用简单的子串扫描。
    let mut out = text.to_string();
    let patterns = [
        ("Bearer ", "bearer "),
        ("Token ", "token "),
        ("api_key=", "api_key="),
        ("apikey=", "apikey="),
    ];
    for (uppercase, lowercase) in patterns {
        for marker in [uppercase, lowercase] {
            let mut search_from = 0;
            while let Some(rel) = out[search_from..].find(marker) {
                let start = search_from + rel;
                let value_start = start + marker.len();
                // 找到下一个空白或行尾。
                let value_end = out[value_start..]
                    .find(|c: char| {
                        c.is_whitespace() || c == ',' || c == ';' || c == '"' || c == '\''
                    })
                    .map(|p| value_start + p)
                    .unwrap_or(out.len());
                if value_end > value_start {
                    out.replace_range(value_start..value_end, REDACTED);
                    // 跳过已替换区域，避免重新匹配同一标记导致死循环。
                    search_from = value_start + REDACTED.len();
                } else {
                    break;
                }
            }
        }
    }
    out
}

/// 清理控制字符与不可显示字符。保留 \t、\n、\r（前端可决定如何处理）。
pub fn strip_control_chars(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\t' || ch == '\n' || ch == '\r' {
            out.push(ch);
        } else if ch.is_control() {
            // 用空格替换控制字符，避免日志注入。
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::model::{Fields, LogLevel, LogSource};

    fn make_entry(message: &str, fields: Vec<(&str, FieldValue)>) -> LogEntry {
        let mut f = Fields::new();
        for (k, v) in fields {
            f.insert(k.into(), v);
        }
        LogEntry {
            id: 1,
            timestamp: "2026-07-17T10:00:00+08:00".into(),
            level: LogLevel::Info,
            source: LogSource::App,
            target: "test".into(),
            message: message.into(),
            session_id: "s1".into(),
            correlation_id: None,
            fields: f,
        }
    }

    fn make_redactor() -> Redactor {
        Redactor::new(
            Some(PathBuf::from("/Users/testuser")),
            Some("testuser".into()),
        )
    }

    #[test]
    fn home_dir_is_replaced_with_placeholder() {
        let r = make_redactor();
        let mut e = make_entry(
            "loaded file from /Users/testuser/Library/Logs/mira.log",
            vec![],
        );
        r.apply(&mut e);
        assert!(e.message.contains("${HOME}"));
        assert!(!e.message.contains("testuser"));
    }

    #[test]
    fn windows_user_path_is_replaced() {
        let r = Redactor::new(
            Some(PathBuf::from(r"C:\Users\testuser")),
            Some("testuser".into()),
        );
        let mut e = make_entry(r"file at C:\Users\testuser\AppData\Local\mira\logs", vec![]);
        r.apply(&mut e);
        assert!(e.message.contains("${HOME}"));
        assert!(!e.message.contains("testuser"));
    }

    #[test]
    fn sensitive_field_is_redacted() {
        let r = make_redactor();
        let mut e = make_entry(
            "request failed",
            vec![(
                "authorization",
                FieldValue::Text("Bearer abc.def.ghi".into()),
            )],
        );
        r.apply(&mut e);
        match e.fields.get("authorization") {
            Some(FieldValue::Text(t)) => assert_eq!(t, REDACTED),
            other => panic!("expected redacted text, got {other:?}"),
        }
    }

    #[test]
    fn allowed_field_keeps_value() {
        let r = make_redactor();
        let mut e = make_entry(
            "workflow done",
            vec![("pluginId", FieldValue::Text("mira.amaster".into()))],
        );
        r.apply(&mut e);
        match e.fields.get("pluginId") {
            Some(FieldValue::Text(t)) => assert_eq!(t, "mira.amaster"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn long_message_is_truncated() {
        let r = Redactor::default();
        let long = "a".repeat(5_000);
        let mut e = make_entry(&long, vec![]);
        r.apply(&mut e);
        // 截断后追加省略号。
        assert!(e.message.len() < long.len() + 4);
        assert!(e.message.ends_with('…'));
    }

    #[test]
    fn url_credentials_are_redacted() {
        let r = Redactor::default();
        let mut e = make_entry(
            "connecting to https://admin:s3cret@api.example.com/path",
            vec![],
        );
        r.apply(&mut e);
        assert!(e.message.contains(REDACTED));
        assert!(!e.message.contains("s3cret"));
        assert!(e.message.contains("api.example.com"));
    }

    #[test]
    fn query_param_with_sensitive_name_is_redacted() {
        let r = Redactor::default();
        let mut e = make_entry(
            "fetched https://example.com/path?token=abc123&page=1",
            vec![],
        );
        r.apply(&mut e);
        assert!(e.message.contains("token="));
        assert!(e.message.contains(REDACTED));
        assert!(e.message.contains("page=1"));
        assert!(!e.message.contains("abc123"));
    }

    #[test]
    fn inline_bearer_token_is_redacted() {
        let r = Redactor::default();
        let mut e = make_entry("got Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig here", vec![]);
        r.apply(&mut e);
        assert!(e.message.contains("Bearer "));
        assert!(e.message.contains(REDACTED));
        assert!(!e.message.contains("eyJhbGciOiJIUzI1NiJ9"));
    }

    #[test]
    fn control_chars_are_stripped() {
        let r = Redactor::default();
        let mut e = make_entry("line1\x00\x07\x1binjected", vec![]);
        r.apply(&mut e);
        // 控制字符被替换为空格。
        assert!(!e.message.contains('\x00'));
        assert!(!e.message.contains('\x07'));
        assert!(!e.message.contains('\x1b'));
    }

    #[test]
    fn known_device_id_is_replaced() {
        let mut r = Redactor::default();
        r.register_device_id("SN-1234567890-ABC".into());
        let mut e = make_entry("device SN-1234567890-ABC connected", vec![]);
        r.apply(&mut e);
        assert!(e.message.contains("${DEVICE_ID}"));
        assert!(!e.message.contains("SN-1234567890-ABC"));
    }

    #[test]
    fn empty_message_stays_empty() {
        let r = Redactor::default();
        let mut e = make_entry("", vec![]);
        r.apply(&mut e);
        assert_eq!(e.message, "");
    }

    #[test]
    fn numeric_field_keeps_value() {
        let r = make_redactor();
        let mut e = make_entry(
            "predict done",
            vec![("durationMs", FieldValue::Integer(42))],
        );
        r.apply(&mut e);
        match e.fields.get("durationMs") {
            Some(FieldValue::Integer(n)) => assert_eq!(*n, 42),
            other => panic!("expected integer, got {other:?}"),
        }
    }
}
