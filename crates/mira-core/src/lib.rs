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
    /// ITERATION-004 §2.1：Dashboard priority 0..100（越高越优先）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    /// Dashboard 角色：fixed-core（DPI/回报率/灯光）| candidate（竞争槽位）| system（系统入口）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard_role: Option<String>,
    /// 固定槽位 1..3（仅 DPI=1、回报率=2、灯光=3）。插件不得声明 fixedSlot=4。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_slot: Option<u8>,
    /// 是否有资格竞争第 4 槽位（需同时 priority >= 90）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fourth_slot_eligible: Option<bool>,
    /// 全局去重键（如 "dashboard.dpi"、"system.all-readings"）。相同 dedupeKey 只保留一个。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    /// 未进入首页时的回退区域。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSnapshot {
    pub display_name: String,
    pub connection: Connection,
    /// 插件根据设备描述或实际连接类型声明的默认选择优先级。
    #[serde(default)]
    pub selection_priority: i32,
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
    /// Per-output read statuses, keyed by workflow output name. Stored as
    /// `serde_json::Value` to avoid a circular dependency between mira-core
    /// and mira-plugin-runtime (which owns the typed `ReadStatus` enum). The
    /// runtime serializes `ReadStatus` into these values; the UI interprets
    /// them as `"ok" | "skipped" | "not-supported" | { "failed": string }`.
    #[serde(default)]
    pub read_statuses: BTreeMap<String, Value>,
    /// 接收器场景下鼠标是否就位（基于 receiverIdle.mouseOnline）。
    /// - `None`：非接收器连接（USB 直连 / 蓝牙），鼠标总是视为就位。
    /// - `Some(true)`：接收器已插入且鼠标在线。
    /// - `Some(false)`：接收器已插入但鼠标未就绪，UI 应显示等待提示而非
    ///   残缺的 Dashboard（鼠标名 + 接收器灯光 + 接收器电量）。
    ///
    /// 由协议层（mira-plugin-runtime）填充，宿主与 UI 仅读取。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse_ready: Option<bool>,
    /// 设备所属的协议 family（如 "protocol-a-direct"、"am35-direct"）。
    /// 用于前端 `visibleWhen.path = "family"` 实现协议感知的能力可见性，
    /// 避免在 Protocol A 与 AM35 之间硬编码 pluginId/vendorId 分支。
    /// 由宿主从 `MatchedDevice.family` 填充。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
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
