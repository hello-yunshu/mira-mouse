// SPDX-License-Identifier: AGPL-3.0-or-later
//! 托盘状态层：从 `DeviceSnapshot` + `AppSettings` 提取托盘需要的最小状态。
//!
//! 状态层只描述"现在应该显示什么状态"，不负责颜色、不负责绘图。
//! 视觉规则见 `style.rs`，绘图见 `image.rs`。

use mira_core::{Connection, DeviceSnapshot};

/// 传递给渲染层的 AppSettings 子集（避免在 tray 模块内直接依赖完整 `AppSettings`）。
///
/// 由 `lib.rs` 在调用处从 `AppSettings` 构造，保持模块解耦。
#[derive(Debug, Clone, Copy)]
pub struct TraySettings<'a> {
    pub show_receiver: bool,
    pub show_connection: bool,
    pub show_battery_title: bool,
    pub low_battery_threshold: u8,
    pub tray_icon_color: &'a str,
    pub tray_render_mode: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayRenderMode {
    Auto,
    NativeMacos,
    DynamicImage,
    Static,
}

impl TrayRenderMode {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "native-macos" => Self::NativeMacos,
            "dynamic-image" => Self::DynamicImage,
            "static" => Self::Static,
            _ => Self::Auto,
        }
    }
}

/// 托盘统一状态。所有平台共享。
///
/// 注意：不派生 `Hash`，因为 `Connection` 未实现 `Hash`。
/// 缓存 diff 由 `TrayIconCacheKey`（`dynamic_icon.rs`）独立处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayStatusState {
    pub connected: bool,
    /// 设备显示名（已规范化），未连接时为 None。
    pub device_name: Option<String>,
    /// 设备连接类型，未连接时为 None。
    pub connection: Option<Connection>,
    /// 鼠标电量百分比（已 clamp 到 0..=100）。None 表示电量未知。
    pub mouse_battery: Option<u8>,
    pub mouse_charging: bool,
    /// 接收器电量百分比（已 clamp 到 0..=100）。None 表示无接收器或电量未知。
    pub receiver_battery: Option<u8>,
    pub receiver_charging: bool,
    pub show_receiver: bool,
    pub show_connection: bool,
    pub show_battery_title: bool,
    pub low_battery_threshold: u8,
    pub render_mode: TrayRenderMode,
}

impl TrayStatusState {
    /// 从设备快照和设置生成托盘状态。
    ///
    /// `snapshot = None` 时返回未连接状态（不显示电量，不误显示满电）。
    pub fn from_snapshot(snapshot: Option<&DeviceSnapshot>, settings: &TraySettings<'_>) -> Self {
        let Some(snapshot) = snapshot else {
            return TrayStatusState {
                connected: false,
                device_name: None,
                connection: None,
                mouse_battery: None,
                mouse_charging: false,
                receiver_battery: None,
                receiver_charging: false,
                show_receiver: settings.show_receiver,
                show_connection: settings.show_connection,
                show_battery_title: settings.show_battery_title,
                low_battery_threshold: settings.low_battery_threshold,
                render_mode: TrayRenderMode::from_setting(settings.tray_render_mode),
            };
        };

        let mouse_battery = mouse_battery_percentage(snapshot).map(clamp_percentage);
        let mouse_charging = mouse_battery_charging(snapshot);
        let receiver_battery = if settings.show_receiver {
            receiver_battery_percentage(snapshot).map(clamp_percentage)
        } else {
            None
        };
        let receiver_charging = if settings.show_receiver {
            receiver_battery_charging(snapshot)
        } else {
            false
        };

        TrayStatusState {
            connected: true,
            device_name: Some(snapshot.display_name.clone()),
            connection: Some(snapshot.connection),
            mouse_battery,
            mouse_charging,
            receiver_battery,
            receiver_charging,
            show_receiver: settings.show_receiver,
            show_connection: settings.show_connection,
            show_battery_title: settings.show_battery_title,
            low_battery_threshold: settings.low_battery_threshold,
            render_mode: TrayRenderMode::from_setting(settings.tray_render_mode),
        }
    }

    /// 鼠标是否处于低电量状态（未充电且电量 ≤ 阈值）。
    #[allow(dead_code)]
    pub fn mouse_is_low(&self) -> bool {
        !self.mouse_charging
            && self
                .mouse_battery
                .is_some_and(|p| p <= self.low_battery_threshold)
    }
}

/// 鼠标电量百分比：优先从 `batteries` 中查找 `id == "mouse"`，
/// 回退到第一个电池，最后回退 `battery_percent`。
pub fn mouse_battery_percentage(snapshot: &DeviceSnapshot) -> Option<u8> {
    snapshot
        .batteries
        .iter()
        .find(|battery| battery.id == "mouse")
        .or_else(|| snapshot.batteries.first())
        .map(|battery| battery.percentage)
        .or(snapshot.battery_percent)
}

/// 鼠标是否正在充电。
pub fn mouse_battery_charging(snapshot: &DeviceSnapshot) -> bool {
    snapshot
        .batteries
        .iter()
        .find(|battery| battery.id == "mouse")
        .or_else(|| snapshot.batteries.first())
        .map(|battery| battery.charging)
        .unwrap_or(snapshot.charging)
}

/// 接收器电量百分比：从 `batteries` 中查找 `id == "receiver"`。
pub fn receiver_battery_percentage(snapshot: &DeviceSnapshot) -> Option<u8> {
    snapshot
        .batteries
        .iter()
        .find(|battery| battery.id == "receiver")
        .map(|battery| battery.percentage)
}

/// 接收器是否正在充电。
pub fn receiver_battery_charging(snapshot: &DeviceSnapshot) -> bool {
    snapshot
        .batteries
        .iter()
        .find(|battery| battery.id == "receiver")
        .map(|battery| battery.charging)
        .unwrap_or(false)
}

/// 低电量通知用的电量值：充电时返回 None（不触发低电量通知）。
pub fn low_battery_notification_value(snapshot: &DeviceSnapshot) -> Option<u8> {
    if mouse_battery_charging(snapshot) {
        None
    } else {
        mouse_battery_percentage(snapshot)
    }
}

fn clamp_percentage(value: u8) -> u8 {
    value.min(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_settings() -> TraySettings<'static> {
        TraySettings {
            show_receiver: false,
            show_connection: true,
            show_battery_title: true,
            low_battery_threshold: 20,
            tray_icon_color: "auto",
            tray_render_mode: "auto",
        }
    }

    fn make_snapshot(batteries: Vec<mira_core::DeviceBattery>) -> DeviceSnapshot {
        DeviceSnapshot {
            display_name: "Test Mouse".into(),
            connection: Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries,
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: Default::default(),
            plugin_capabilities: Vec::new(),
            writable_mutations: Vec::new(),
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
        }
    }

    #[test]
    fn disconnected_state_has_no_battery() {
        let settings = test_settings();
        let state = TrayStatusState::from_snapshot(None, &settings);
        assert!(!state.connected);
        assert_eq!(state.mouse_battery, None);
        assert!(!state.mouse_charging);
    }

    #[test]
    fn mouse_battery_extracts_from_batteries_id() {
        let snapshot = make_snapshot(vec![
            mira_core::DeviceBattery {
                id: "mouse".into(),
                label: "鼠标".into(),
                percentage: 67,
                charging: false,
            },
            mira_core::DeviceBattery {
                id: "receiver".into(),
                label: "接收器".into(),
                percentage: 100,
                charging: false,
            },
        ]);
        let settings = test_settings();
        let state = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        assert!(state.connected);
        assert_eq!(state.mouse_battery, Some(67));
        assert!(!state.mouse_charging);
    }

    #[test]
    fn mouse_battery_falls_back_to_first_battery() {
        let snapshot = make_snapshot(vec![mira_core::DeviceBattery {
            id: "unknown".into(),
            label: "Mouse".into(),
            percentage: 42,
            charging: true,
        }]);
        assert_eq!(mouse_battery_percentage(&snapshot), Some(42));
        assert!(mouse_battery_charging(&snapshot));
    }

    #[test]
    fn mouse_battery_falls_back_to_battery_percent() {
        let mut snapshot = make_snapshot(vec![]);
        snapshot.battery_percent = Some(88);
        assert_eq!(mouse_battery_percentage(&snapshot), Some(88));
    }

    #[test]
    fn receiver_battery_only_extracted_when_enabled() {
        let snapshot = make_snapshot(vec![
            mira_core::DeviceBattery {
                id: "mouse".into(),
                label: "鼠标".into(),
                percentage: 50,
                charging: false,
            },
            mira_core::DeviceBattery {
                id: "receiver".into(),
                label: "接收器".into(),
                percentage: 30,
                charging: false,
            },
        ]);

        let mut settings = test_settings();
        settings.show_receiver = false;
        let state = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        assert_eq!(state.receiver_battery, None);

        settings.show_receiver = true;
        let state = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        assert_eq!(state.receiver_battery, Some(30));
    }

    #[test]
    fn low_battery_threshold_flows_into_state() {
        let snapshot = make_snapshot(vec![mira_core::DeviceBattery {
            id: "mouse".into(),
            label: "鼠标".into(),
            percentage: 25,
            charging: false,
        }]);
        let mut settings = test_settings();
        settings.low_battery_threshold = 30;
        let state = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        assert_eq!(state.low_battery_threshold, 30);
        assert!(state.mouse_is_low());

        settings.low_battery_threshold = 20;
        let state = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        assert!(!state.mouse_is_low());
    }

    #[test]
    fn charging_mouse_is_not_low_even_below_threshold() {
        let snapshot = make_snapshot(vec![mira_core::DeviceBattery {
            id: "mouse".into(),
            label: "鼠标".into(),
            percentage: 5,
            charging: true,
        }]);
        let settings = test_settings();
        let state = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        assert!(state.mouse_charging);
        assert!(!state.mouse_is_low());
    }

    #[test]
    fn low_battery_notification_value_skips_charging() {
        let snapshot = make_snapshot(vec![mira_core::DeviceBattery {
            id: "mouse".into(),
            label: "鼠标".into(),
            percentage: 5,
            charging: true,
        }]);
        assert_eq!(low_battery_notification_value(&snapshot), None);

        let snapshot = make_snapshot(vec![mira_core::DeviceBattery {
            id: "mouse".into(),
            label: "鼠标".into(),
            percentage: 5,
            charging: false,
        }]);
        assert_eq!(low_battery_notification_value(&snapshot), Some(5));
    }

    #[test]
    fn tray_render_mode_flows_into_state() {
        let mut settings = test_settings();
        settings.tray_render_mode = "static";
        let state = TrayStatusState::from_snapshot(None, &settings);
        assert_eq!(state.render_mode, TrayRenderMode::Static);

        settings.tray_render_mode = "native-macos";
        let state = TrayStatusState::from_snapshot(None, &settings);
        assert_eq!(state.render_mode, TrayRenderMode::NativeMacos);
    }
}
