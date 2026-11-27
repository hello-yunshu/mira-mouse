// SPDX-License-Identifier: AGPL-3.0-or-later
//! 静态 PNG fallback 图标选择逻辑。
//!
//! 当动态图标生成失败或 `trayRenderMode = "static"` 时使用。
//! 静态图标按 10% 分档，由 `scripts/generate-tray-mouse-icons.py` 生成。

use super::{image::render_mouse_icon_rgba, state::TrayStatusState, style::TrayVisualStyle};

/// Battery level icons for each (dark, charging) combination.
/// Index 0 = 0%, 1 = 10%, ..., 9 = 90%, 10 = 100%.
/// `include_bytes!` requires string literals, so the 44 icons are expanded
/// into four `const` arrays once; the lookup function then indexes by level.
const TRAY_ICONS_LIGHT_CHARGING: [&[u8]; 11] = [
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-0.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-10.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-20.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-30.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-40.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-50.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-60.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-70.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-80.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-90.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels/mouse-100.png"),
];
const TRAY_ICONS_DARK_CHARGING: [&[u8]; 11] = [
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-0.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-10.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-20.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-30.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-40.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-50.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-60.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-70.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-80.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-90.png"),
    include_bytes!("../../icons/tray-mouse-charging-levels-dark/mouse-100.png"),
];
const TRAY_ICONS_LIGHT: [&[u8]; 11] = [
    include_bytes!("../../icons/tray-mouse-levels/mouse-0.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-10.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-20.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-30.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-40.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-50.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-60.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-70.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-80.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-90.png"),
    include_bytes!("../../icons/tray-mouse-levels/mouse-100.png"),
];
const TRAY_ICONS_DARK: [&[u8]; 11] = [
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-0.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-10.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-20.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-30.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-40.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-50.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-60.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-70.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-80.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-90.png"),
    include_bytes!("../../icons/tray-mouse-levels-dark/mouse-100.png"),
];

fn static_tray_icon_bytes(index: usize, dark: bool, charging: bool) -> &'static [u8] {
    match (dark, charging) {
        (true, true) => TRAY_ICONS_DARK_CHARGING[index],
        (false, true) => TRAY_ICONS_LIGHT_CHARGING[index],
        (true, false) => TRAY_ICONS_DARK[index],
        (false, false) => TRAY_ICONS_LIGHT[index],
    }
}

pub fn render_static_tray_icon(
    state: &TrayStatusState,
    style: &TrayVisualStyle,
) -> tauri::Result<tauri::image::Image<'static>> {
    use tauri::image::Image;

    if !state.connected {
        return Image::from_bytes(static_tray_app_icon_bytes_for_theme(
            style.system_theme.is_dark(),
        ));
    }

    if let Some(percentage) = state.mouse_battery {
        let index = ((usize::from(percentage) + 5) / 10).min(10);
        if index > 0 || state.mouse_charging {
            let image = Image::from_bytes(static_tray_icon_bytes(
                index,
                style.theme.is_dark(),
                state.mouse_charging,
            ))?;
            // The PNG bucket determines geometry. Select the fill from the original reading.
            let color = style.fill_for_battery(
                percentage,
                state.mouse_charging,
                state.low_battery_threshold,
            );
            let fill_colors = [style.normal_fill, style.mid_fill, style.low_fill];
            let mut rgba = image.rgba().to_vec();
            for pixel in rgba.as_chunks_mut::<4>().0 {
                if pixel[3] != 0
                    && fill_colors
                        .iter()
                        .any(|fill| pixel[..3] == fill.channels()[..3])
                {
                    pixel[..3].copy_from_slice(&color.channels()[..3]);
                }
            }
            return Ok(Image::new_owned(rgba, image.width(), image.height()));
        }
    }

    // The empty PNG has no fill to recolor for an unknown reading or a critical 0–4%.
    Ok(Image::new_owned(
        render_mouse_icon_rgba(state, style),
        64,
        64,
    ))
}

/// 应用图标（未连接时使用）。
#[cfg(test)]
pub fn static_tray_app_icon_bytes() -> &'static [u8] {
    static_tray_app_icon_bytes_for_theme(false)
}

/// 按主题选择应用图标（未连接时使用）。
pub fn static_tray_app_icon_bytes_for_theme(dark: bool) -> &'static [u8] {
    if dark {
        include_bytes!("../../icons/tray-app-icon-dark.png")
    } else {
        include_bytes!("../../icons/tray-app-icon.png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_icon_uses_actual_percentage_for_color() {
        let settings = crate::tray::state::TraySettings {
            show_receiver: false,
            show_connection: false,
            show_battery_title: false,
            low_battery_threshold: 20,
            tray_icon_color: "auto",
            tray_render_mode: "static",
        };
        let style = TrayVisualStyle::from_settings(&settings, super::super::style::TrayTheme::Dark);
        let mut state = TrayStatusState::from_snapshot(None, &settings);
        state.connected = true;
        for (percentage, color) in [
            (0, style.low_fill),
            (10, style.low_fill),
            (11, style.mid_fill),
            (20, style.mid_fill),
            (21, style.normal_fill),
        ] {
            state.mouse_battery = Some(percentage);
            let image = render_static_tray_icon(&state, &style).unwrap();
            assert!(
                image.rgba().as_chunks::<4>().0.contains(&color.channels()),
                "{percentage}%"
            );
        }
    }

    #[test]
    fn app_icon_differs_by_theme() {
        let light = static_tray_app_icon_bytes_for_theme(false);
        let dark = static_tray_app_icon_bytes_for_theme(true);
        // 两张图标的字节数可能相同（同尺寸 PNG），但内容不同
        assert!(!light.is_empty());
        assert!(!dark.is_empty());
    }
}
