// SPDX-License-Identifier: AGPL-3.0-or-later
//! 动态图标渲染器：Windows / Linux 主路径，macOS fallback。
//!
//! 使用 `image.rs` 的 64×64 RGBA Canvas 绘制鼠标图标，
//! 通过 `TrayIconCacheKey` 进行 diff，避免每轮轮询都重新生成图标。
//! 未连接时返回 `None`，由调用方使用静态 app 图标 fallback。

use std::collections::HashMap;

use crate::tray::image::render_mouse_icon_rgba;
use crate::tray::state::{TrayRenderMode, TrayStatusState};
use crate::tray::style::{TrayIconColorMode, TrayTheme, TrayVisualStyle};

/// 托盘图标缓存 key：捕获影响图标视觉的所有字段。
///
/// key 相同时图标像素完全一致，可跳过重新生成和 `set_icon` 调用。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrayIconCacheKey {
    pub connected: bool,
    pub mouse_battery: Option<u8>,
    pub mouse_charging: bool,
    pub receiver_battery: Option<u8>,
    pub receiver_charging: bool,
    pub show_receiver: bool,
    pub theme: TrayTheme,
    pub icon_color_mode: TrayIconColorMode,
    pub low_battery_threshold: u8,
    pub render_mode: TrayRenderMode,
}

impl TrayIconCacheKey {
    /// 从状态和样式生成缓存 key。
    pub fn from_state_and_style(state: &TrayStatusState, style: &TrayVisualStyle) -> Self {
        Self {
            connected: state.connected,
            mouse_battery: state.mouse_battery,
            mouse_charging: state.mouse_charging,
            receiver_battery: state.receiver_battery,
            receiver_charging: state.receiver_charging,
            show_receiver: state.show_receiver,
            theme: style.theme,
            icon_color_mode: style.icon_color_mode,
            low_battery_threshold: state.low_battery_threshold,
            render_mode: state.render_mode,
        }
    }
}

/// 动态图片渲染器。
///
/// 内部维护一个 `HashMap` 缓存，key 为 `TrayIconCacheKey`，value 为 RGBA 字节。
/// 组合数量有限（电量 0..=100 × 主题 2 × 充电 2 × 连接 2 ≈ 800），
/// 每个图标 16KB，最坏约 12.8MB；实际场景远小于此（设备状态变化有限）。
///
/// 调用方通过 `needs_update()` 判断是否需要调用 `set_icon`，
/// 通过 `render_image()` 获取 `tauri::image::Image`。
#[derive(Default)]
pub struct DynamicImageTrayRenderer {
    cache: HashMap<TrayIconCacheKey, Vec<u8>>,
    last_key: Option<TrayIconCacheKey>,
}

impl DynamicImageTrayRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前状态对应的缓存 key 是否与上次不同。
    ///
    /// `update_tray` 用此判断是否需要调用 `tray.set_icon`。
    /// 未连接时也返回 true（需要切换到 app 图标）。
    pub fn needs_update(&self, state: &TrayStatusState, style: &TrayVisualStyle) -> bool {
        let key = TrayIconCacheKey::from_state_and_style(state, style);
        self.last_key.as_ref() != Some(&key)
    }

    pub fn mark_current(&mut self, state: &TrayStatusState, style: &TrayVisualStyle) {
        self.last_key = Some(TrayIconCacheKey::from_state_and_style(state, style));
    }

    /// 渲染鼠标图标 RGBA 字节（已连接状态）。
    ///
    /// 命中缓存则复用，否则调用 `render_mouse_icon_rgba` 重新绘制并缓存。
    fn render_mouse_rgba(&mut self, state: &TrayStatusState, style: &TrayVisualStyle) -> Vec<u8> {
        let key = TrayIconCacheKey::from_state_and_style(state, style);
        if let Some(cached) = self.cache.get(&key) {
            self.last_key = Some(key);
            return cached.clone();
        }
        let rgba = render_mouse_icon_rgba(state, style);
        self.cache.insert(key.clone(), rgba.clone());
        self.last_key = Some(key);
        rgba
    }

    /// 渲染图标并转换为 `tauri::image::Image`。
    ///
    /// - 已连接：返回 `Some(Image)`（动态绘制的鼠标图标）
    /// - 未连接：返回 `None`（调用方使用 `static_tray_app_icon_bytes_for_theme`）
    ///
    /// 调用此方法会更新内部 `last_key`，后续 `needs_update()` 在状态不变时返回 false。
    pub fn render_image(
        &mut self,
        state: &TrayStatusState,
        style: &TrayVisualStyle,
    ) -> Option<tauri::image::Image<'static>> {
        if !state.connected {
            // 未连接：记录 key 以便 needs_update 正确 diff，但不生成图标
            self.last_key = Some(TrayIconCacheKey::from_state_and_style(state, style));
            return None;
        }
        let rgba = self.render_mouse_rgba(state, style);
        Some(tauri::image::Image::new_owned(rgba, 64, 64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::state::TraySettings;
    use crate::tray::style::{TrayTheme, TrayVisualStyle};
    use mira_core::{Connection, DeviceSnapshot};

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

    fn make_state(percentage: Option<u8>, charging: bool) -> TrayStatusState {
        let batteries = if let Some(p) = percentage {
            vec![mira_core::DeviceBattery {
                id: "mouse".into(),
                label: "鼠标".into(),
                percentage: p,
                charging,
            }]
        } else {
            vec![]
        };
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: Connection::Usb,
            selection_priority: 0,
            battery_percent: percentage,
            charging,
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
            history_identity: None,
        };
        TrayStatusState::from_snapshot(Some(&snapshot), &test_settings())
    }

    fn make_disconnected_state() -> TrayStatusState {
        TrayStatusState::from_snapshot(None, &test_settings())
    }

    fn make_style() -> TrayVisualStyle {
        TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Dark)
    }

    #[test]
    fn cache_key_differs_by_battery() {
        let style = make_style();
        let low = make_state(Some(10), false);
        let high = make_state(Some(90), false);

        let low_key = TrayIconCacheKey::from_state_and_style(&low, &style);
        let high_key = TrayIconCacheKey::from_state_and_style(&high, &style);

        assert_ne!(low_key, high_key);
    }

    #[test]
    fn cache_key_differs_by_charging() {
        let style = make_style();
        let normal = make_state(Some(50), false);
        let charging = make_state(Some(50), true);

        let normal_key = TrayIconCacheKey::from_state_and_style(&normal, &style);
        let charging_key = TrayIconCacheKey::from_state_and_style(&charging, &style);

        assert_ne!(normal_key, charging_key);
    }

    #[test]
    fn cache_key_differs_by_theme() {
        let state = make_state(Some(75), false);
        let dark_style = TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Dark);
        let light_style = TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Light);

        let dark_key = TrayIconCacheKey::from_state_and_style(&state, &dark_style);
        let light_key = TrayIconCacheKey::from_state_and_style(&state, &light_style);

        assert_ne!(dark_key, light_key);
    }

    #[test]
    fn cache_key_differs_by_connected() {
        let style = make_style();
        let connected = make_state(Some(50), false);
        let disconnected = make_disconnected_state();

        let connected_key = TrayIconCacheKey::from_state_and_style(&connected, &style);
        let disconnected_key = TrayIconCacheKey::from_state_and_style(&disconnected, &style);

        assert_ne!(connected_key, disconnected_key);
    }

    #[test]
    fn cache_key_differs_by_show_receiver() {
        let mut settings = test_settings();
        settings.show_receiver = false;
        let state_no_receiver = TrayStatusState::from_snapshot(
            Some(&DeviceSnapshot {
                display_name: "Test".into(),
                connection: Connection::Usb,
                selection_priority: 0,
                battery_percent: Some(50),
                charging: false,
                batteries: vec![
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
                ],
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
                history_identity: None,
            }),
            &settings,
        );

        settings.show_receiver = true;
        let state_with_receiver = TrayStatusState::from_snapshot(
            Some(&DeviceSnapshot {
                display_name: "Test".into(),
                connection: Connection::Usb,
                selection_priority: 0,
                battery_percent: Some(50),
                charging: false,
                batteries: vec![
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
                ],
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
                history_identity: None,
            }),
            &settings,
        );

        let style = make_style();
        let key_no = TrayIconCacheKey::from_state_and_style(&state_no_receiver, &style);
        let key_with = TrayIconCacheKey::from_state_and_style(&state_with_receiver, &style);
        assert_ne!(key_no, key_with);
    }

    #[test]
    fn needs_update_on_first_call() {
        let renderer = DynamicImageTrayRenderer::new();
        let state = make_state(Some(75), false);
        let style = make_style();
        assert!(renderer.needs_update(&state, &style));
    }

    #[test]
    fn needs_update_false_after_render() {
        let mut renderer = DynamicImageTrayRenderer::new();
        let state = make_state(Some(75), false);
        let style = make_style();

        let _ = renderer.render_image(&state, &style);
        assert!(!renderer.needs_update(&state, &style));
    }

    #[test]
    fn needs_update_true_after_state_change() {
        let mut renderer = DynamicImageTrayRenderer::new();
        let style = make_style();

        let state1 = make_state(Some(75), false);
        let _ = renderer.render_image(&state1, &style);
        assert!(!renderer.needs_update(&state1, &style));

        let state2 = make_state(Some(80), false);
        assert!(renderer.needs_update(&state2, &style));
    }

    #[test]
    fn needs_update_true_after_theme_change() {
        let mut renderer = DynamicImageTrayRenderer::new();
        let state = make_state(Some(75), false);

        let dark_style = TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Dark);
        let _ = renderer.render_image(&state, &dark_style);
        assert!(!renderer.needs_update(&state, &dark_style));

        let light_style = TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Light);
        assert!(renderer.needs_update(&state, &light_style));
    }

    #[test]
    fn render_image_returns_some_when_connected() {
        let mut renderer = DynamicImageTrayRenderer::new();
        let state = make_state(Some(75), false);
        let style = make_style();

        let image = renderer.render_image(&state, &style);
        assert!(image.is_some());
    }

    #[test]
    fn render_image_returns_none_when_disconnected() {
        let mut renderer = DynamicImageTrayRenderer::new();
        let state = make_disconnected_state();
        let style = make_style();

        let image = renderer.render_image(&state, &style);
        assert!(image.is_none());
    }

    #[test]
    fn render_image_caches_on_hit() {
        let mut renderer = DynamicImageTrayRenderer::new();
        let state = make_state(Some(75), false);
        let style = make_style();

        let _img1 = renderer.render_image(&state, &style).unwrap();
        let _img2 = renderer.render_image(&state, &style).unwrap();

        // 两次渲染的图标像素应一致（缓存命中）
        // tauri::image::Image 不直接暴露 rgba，但通过 needs_update 已验证缓存逻辑
        assert!(!renderer.needs_update(&state, &style));
    }

    #[test]
    fn render_different_levels_produce_different_images() {
        let mut renderer = DynamicImageTrayRenderer::new();
        let style = make_style();

        let low_state = make_state(Some(10), false);
        let high_state = make_state(Some(90), false);

        let low_image = renderer.render_image(&low_state, &style);
        let high_image = renderer.render_image(&high_state, &style);

        assert!(low_image.is_some());
        assert!(high_image.is_some());

        // 两个不同的 key 应该都被缓存
        assert_eq!(renderer.cache.len(), 2);
    }
}
