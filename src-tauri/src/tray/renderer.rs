// SPDX-License-Identifier: AGPL-3.0-or-later
//! 平台渲染抽象层：定义 `TrayController` trait 和 `TauriTrayController`。
//!
//! - `TrayController`：平台无关的图标更新接口
//! - `TauriTrayController`：基于 Tauri `TrayIcon` 的控制器，使用 `DynamicImageTrayRenderer`
//!   生成动态图标，失败时 fallback 到静态 PNG
//! - macOS 原生 `NSStatusItem` 控制器见 `macos` 模块（step 5）

use crate::tray::dynamic_icon::DynamicImageTrayRenderer;
use crate::tray::state::{TrayRenderMode, TrayStatusState};
use crate::tray::static_icon::static_tray_app_icon_bytes_for_theme;
use crate::tray::style::TrayVisualStyle;

/// 托盘图标控制器：负责把状态和样式渲染为系统托盘图标。
///
/// 菜单、tooltip、title 的更新不在控制器职责内，由 `update_tray` 负责。
pub trait TrayController: Send {
    /// 更新托盘图标。
    ///
    /// `tray` 由调用方从 `app.tray_by_id(TRAY_ID)` 获取。
    /// 内部应做 diff：状态和样式未变化时跳过 `set_icon`。
    fn update_icon(
        &mut self,
        tray: &tauri::tray::TrayIcon,
        state: &TrayStatusState,
        style: &TrayVisualStyle,
    ) -> Result<(), Box<dyn std::error::Error>>;
}

/// 基于 Tauri `TrayIcon` 的控制器。
///
/// 主路径：`DynamicImageTrayRenderer` 生成 64×64 RGBA 动态图标。
/// Fallback 路径：
/// - 未连接：`static_tray_app_icon_bytes_for_theme`（应用图标）
/// - 动态生成失败：`static_tray_icon_bytes`（静态 PNG 分档图标）
/// - `tray_render_mode == "static"`：直接使用静态 PNG
pub struct TauriTrayController {
    renderer: DynamicImageTrayRenderer,
}

impl Default for TauriTrayController {
    fn default() -> Self {
        Self {
            renderer: DynamicImageTrayRenderer::new(),
        }
    }
}

impl TauriTrayController {
    pub fn new() -> Self {
        Self::default()
    }

    fn set_static_icon(
        tray: &tauri::tray::TrayIcon,
        state: &TrayStatusState,
        style: &TrayVisualStyle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let icon_bytes = if state.connected {
            let percentage = state.mouse_battery.unwrap_or(0);
            let level = ((percentage.saturating_add(5)) / 10).min(10) * 10;
            crate::tray::static_icon::static_tray_icon_bytes(
                level,
                style.theme.is_dark(),
                state.mouse_charging,
            )
        } else {
            static_tray_app_icon_bytes_for_theme(style.theme.is_dark())
        };
        let image = tauri::image::Image::from_bytes(icon_bytes)?;
        tray.set_icon(Some(image))?;
        tray.set_icon_as_template(false)?;
        Ok(())
    }
}

impl TrayController for TauriTrayController {
    fn update_icon(
        &mut self,
        tray: &tauri::tray::TrayIcon,
        state: &TrayStatusState,
        style: &TrayVisualStyle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if state.render_mode == TrayRenderMode::Static {
            Self::set_static_icon(tray, state, style)?;
            self.renderer.mark_current(state, style);
            return Ok(());
        }

        // diff：状态和样式未变化时跳过 set_icon
        if !self.renderer.needs_update(state, style) {
            return Ok(());
        }

        // 未连接：使用 app 图标
        if !state.connected {
            let icon_bytes = static_tray_app_icon_bytes_for_theme(style.theme.is_dark());
            let image = tauri::image::Image::from_bytes(icon_bytes)?;
            tray.set_icon(Some(image))?;
            tray.set_icon_as_template(false)?;
            return Ok(());
        }

        // 已连接：尝试动态图标
        match self.renderer.render_image(state, style) {
            Some(image) => {
                tray.set_icon(Some(image))?;
                tray.set_icon_as_template(false)?;
            }
            None => {
                // 动态生成失败：fallback 到静态 PNG
                Self::set_static_icon(tray, state, style)?;
            }
        }
        Ok(())
    }
}

// 平台选择：macOS 使用原生控制器（step 5），其他平台使用 TauriTrayController。
// macOS 原生控制器内部包含 TauriTrayController 作为 fallback。
#[cfg(target_os = "macos")]
pub type PlatformTrayController = crate::tray::macos::MacNativeTrayController;

#[cfg(not(target_os = "macos"))]
pub type PlatformTrayController = TauriTrayController;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::state::TraySettings;
    use crate::tray::style::TrayTheme;
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
    fn tauri_controller_creates_successfully() {
        let _controller = TauriTrayController::new();
    }

    #[test]
    fn tauri_controller_needs_update_initially() {
        let controller = TauriTrayController::new();
        let state = make_state(Some(75), false);
        let style = make_style();
        assert!(controller.renderer.needs_update(&state, &style));
    }

    #[test]
    fn disconnected_state_uses_app_icon_path() {
        // 验证未连接状态不生成动态图标
        let mut renderer = DynamicImageTrayRenderer::new();
        let state = make_disconnected_state();
        let style = make_style();
        let image = renderer.render_image(&state, &style);
        assert!(image.is_none());
    }
}
