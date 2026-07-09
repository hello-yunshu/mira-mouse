// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const MAX_DEVICE_DISPLAY_NAME_CHARS: usize = 32;

pub fn normalize_device_display_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= MAX_DEVICE_DISPLAY_NAME_CHARS {
        return Some(trimmed.to_string());
    }
    Some(
        trimmed
            .chars()
            .take(MAX_DEVICE_DISPLAY_NAME_CHARS.saturating_sub(1))
            .collect::<String>()
            + "…",
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DpiStage {
    pub value: u16,
    pub color: String,
    pub enabled: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBattery {
    pub id: String,
    pub label: String,
    pub percentage: u8,
    pub charging: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    pub group: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapability {
    pub id: String,
    pub control: String,
    pub label_key: String,
    pub read_only: bool,
    #[serde(default)]
    pub placements: Vec<PluginCapabilityPlacement>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    /// 设备实际是否支持该能力（运行时探测结果）。
    /// 由 Host 根据 probe 声明和 workflow 输出计算，前端据此过滤渲染。
    /// 默认 true（向后兼容：无 probe 声明的能力始终可用）。
    #[serde(default = "default_available")]
    pub available: bool,
    /// 连接类型能力分支（#3）：声明该能力仅在指定连接类型下可见。
    /// 可选值："usb"、"receiver"、"bluetooth"。未声明时所有连接类型均可见。
    #[serde(default)]
    pub connections: Option<Vec<String>>,
    /// 固件版本门槛（#4）：声明该能力所需的最低固件版本。
    /// 格式为 semver（如 "1.2.3"）。未声明时无版本限制。
    #[serde(default)]
    pub min_firmware: Option<String>,
}

fn default_available() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityPlacement {
    pub region: String,
    pub group: Option<String>,
    pub order: i32,
    pub span: u8,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSnapshot {
    pub display_name: String,
    pub connection: Connection,
    pub battery_percent: Option<u8>,
    pub charging: bool,
    #[serde(default)]
    pub batteries: Vec<DeviceBattery>,
    pub dpi: Option<u16>,
    pub dpi_stages: Option<Vec<DpiStage>>,
    pub polling_rate_hz: Option<u16>,
    #[serde(default, rename = "supportedPollingRatesHz")]
    pub supported_polling_rates_hz: Option<Vec<u16>>,
    pub profile: Option<String>,
    pub confirmed_light_color: Option<String>,
    pub capabilities: BTreeMap<String, Value>,
    #[serde(default)]
    pub plugin_capabilities: Vec<PluginCapability>,
    #[serde(default)]
    pub writable_mutations: Vec<String>,
    pub evidence: String,
    /// 设备是否处于只读模式：插件未签名/签名失效/未启用写入时为 true。
    /// UI 据此明确显示「未信任插件 · 只读模式」，而非静默隐藏写入控件。
    #[serde(default)]
    pub readonly: bool,
    /// 匹配该设备的插件 ID（如 "mira.amaster"），用于前端 i18n namespace 解析。
    #[serde(default)]
    pub plugin_id: Option<String>,
    /// 插件声明的跨连接/跨接口身份，用于历史统计等宿主通用功能做合并。
    #[serde(default)]
    pub history_identity: Option<DeviceIdentity>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Connection {
    Usb,
    Wireless,
    Bluetooth,
    Virtual,
}

#[derive(Debug, Default)]
pub struct LowBatteryCrossing {
    below: bool,
}

impl LowBatteryCrossing {
    pub fn update(&mut self, value: Option<u8>, threshold: u8) -> bool {
        let now = is_low_battery(value, threshold);
        let notify = now && !self.below;
        self.below = now;
        notify
    }

    pub fn sync(&mut self, value: Option<u8>, threshold: u8) {
        self.below = is_low_battery(value, threshold);
    }
}

fn is_low_battery(value: Option<u8>, threshold: u8) -> bool {
    value.is_some_and(|v| v <= threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_battery_only_notifies_on_crossing() {
        let mut crossing = LowBatteryCrossing::default();
        assert!(crossing.update(Some(20), 20));
        assert!(!crossing.update(Some(19), 20));
        assert!(!crossing.update(Some(50), 20));
        assert!(crossing.update(Some(20), 20));
    }

    #[test]
    fn low_battery_threshold_change_syncs_without_notifying() {
        let mut crossing = LowBatteryCrossing::default();
        crossing.sync(Some(25), 30);
        assert!(!crossing.update(Some(25), 30));
        assert!(!crossing.update(Some(24), 30));
        assert!(!crossing.update(Some(31), 30));
        assert!(crossing.update(Some(30), 30));
    }

    #[test]
    fn normalizes_device_display_names_for_host_ui() {
        assert_eq!(
            normalize_device_display_name("  G705 Mouse  ").as_deref(),
            Some("G705 Mouse")
        );
        assert_eq!(normalize_device_display_name("   "), None);
        let long = "ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
        let normalized = normalize_device_display_name(long).unwrap();
        assert_eq!(normalized.chars().count(), MAX_DEVICE_DISPLAY_NAME_CHARS);
        assert!(normalized.ends_with('…'));
    }
}
