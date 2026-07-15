// SPDX-License-Identifier: AGPL-3.0-or-later
//! 托盘视觉规则层：根据主题和设置决定颜色。
//!
//! 不负责绘图，只输出颜色值。绘图见 `image.rs`。

use crate::tray::state::TraySettings;

/// 菜单栏 / 任务栏主题（深色或浅色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayTheme {
    Light,
    Dark,
}

impl TrayTheme {
    pub fn is_dark(self) -> bool {
        matches!(self, TrayTheme::Dark)
    }

    pub fn from_system_dark(dark: bool) -> Self {
        if dark {
            TrayTheme::Dark
        } else {
            TrayTheme::Light
        }
    }
}

/// 用户设置的图标颜色模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayIconColorMode {
    Auto,
    White,
    Black,
}

impl TrayIconColorMode {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "white" => TrayIconColorMode::White,
            "black" => TrayIconColorMode::Black,
            _ => TrayIconColorMode::Auto,
        }
    }
}

/// RGBA 颜色（0..=255）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RgbaColor {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        RgbaColor { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        RgbaColor { r, g, b, a }
    }
}

/// 托盘视觉样式：颜色规则集合。
///
/// 与 `TrayStatusState` 配合使用，由 `image.rs` 据此绘制图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrayVisualStyle {
    /// 系统自身的浅色/深色主题，不受托盘图标颜色覆盖影响。
    ///
    /// 未连接时显示的彩色应用图标使用这个主题；鼠标、电量和充电图标
    /// 继续使用下面的 `theme`，以便适配菜单栏背景或用户颜色设置。
    pub system_theme: TrayTheme,
    pub theme: TrayTheme,
    pub icon_color_mode: TrayIconColorMode,

    /// 鼠标外轮廓颜色。
    pub outline: RgbaColor,
    /// 鼠标中键 / 充电闪电颜色（比 outline 更实）。
    pub outline_secondary: RgbaColor,

    /// 正常电量填充色（> 50%）。
    pub normal_fill: RgbaColor,
    /// 充电填充色（充电时）。
    pub charging_fill: RgbaColor,
    /// 低电量填充色（≤ 阈值）。
    pub low_fill: RgbaColor,
    /// 中间档填充色（阈值 < 电量 ≤ 50%）。与现有静态图标保持一致的黄档。
    pub mid_fill: RgbaColor,
    /// 未连接 / 未知电量的弱化填充色。
    pub unknown_fill: RgbaColor,
    /// 接收器标记正常颜色。
    pub receiver_fill: RgbaColor,
    /// 接收器低电量颜色。
    pub receiver_low_fill: RgbaColor,

    /// 文字 / tooltip 颜色（当前未在图标内使用，保留给未来扩展）。
    pub text: RgbaColor,
}

impl TrayVisualStyle {
    /// 从设置和主题生成视觉样式。
    ///
    /// 颜色规则与现有静态 PNG 图标（`scripts/generate-tray-mouse-icons.py`）保持一致，
    /// 确保动态图标和静态 fallback 在视觉上无缝切换：
    /// - 外轮廓：白色（深色主题）/ 黑色（浅色主题），alpha 118
    /// - 中键 / 闪电：白色 / 黑色，alpha 210
    /// - 电量填充：绿（>50%）/ 黄（≤50%）/ 红（≤阈值）
    /// - 充电：填充色保持电量分级，闪电叠加指示充电
    pub fn from_settings(settings: &TraySettings<'_>, theme: TrayTheme) -> Self {
        let icon_color_mode = TrayIconColorMode::from_setting(settings.tray_icon_color);

        // tray_icon_color 覆盖系统主题：
        //   white → 强制深色主题轮廓（白色轮廓适配深色菜单栏）
        //   black → 强制浅色主题轮廓（黑色轮廓适配浅色菜单栏）
        Self::from_color_mode(icon_color_mode, theme)
    }

    /// 直接从颜色模式和已解析的菜单栏主题生成视觉样式。
    ///
    /// macOS 原生托盘会在 auto 模式下把 `theme` 替换为 status item 的
    /// effectiveAppearance；其他平台继续传入系统主题。
    pub fn from_color_mode(icon_color_mode: TrayIconColorMode, theme: TrayTheme) -> Self {
        // tray_icon_color 覆盖传入主题：
        //   white → 强制深色主题轮廓（白色轮廓适配深色菜单栏）
        //   black → 强制浅色主题轮廓（黑色轮廓适配浅色菜单栏）
        //   auto  → 使用调用方解析出的菜单栏/系统主题
        let effective_theme = match icon_color_mode {
            TrayIconColorMode::White => TrayTheme::Dark,
            TrayIconColorMode::Black => TrayTheme::Light,
            TrayIconColorMode::Auto => theme,
        };

        let (outline, secondary) = match effective_theme {
            TrayTheme::Dark => (
                RgbaColor::rgba(255, 255, 255, 118),
                RgbaColor::rgba(255, 255, 255, 210),
            ),
            TrayTheme::Light => (RgbaColor::rgba(0, 0, 0, 118), RgbaColor::rgba(0, 0, 0, 210)),
        };

        // 电量填充色与现有静态图标脚本一致：
        //   green #34C759 (>50%) / yellow #FFCC00 (≤50%) / red #FF3B30 (≤阈值)
        let normal_fill = RgbaColor::rgb(52, 199, 89);
        let mid_fill = RgbaColor::rgb(255, 204, 0);
        let low_fill = RgbaColor::rgb(255, 59, 48);
        let charging_fill = RgbaColor::rgb(52, 199, 89);
        let unknown_fill = RgbaColor::rgba(142, 142, 147, 200);
        let receiver_fill = RgbaColor::rgb(10, 132, 255);
        let receiver_low_fill = low_fill;

        let text = match effective_theme {
            TrayTheme::Dark => RgbaColor::rgba(255, 255, 255, 230),
            TrayTheme::Light => RgbaColor::rgba(0, 0, 0, 230),
        };

        TrayVisualStyle {
            system_theme: theme,
            theme: effective_theme,
            icon_color_mode,
            outline,
            outline_secondary: secondary,
            normal_fill,
            charging_fill,
            low_fill,
            mid_fill,
            unknown_fill,
            receiver_fill,
            receiver_low_fill,
            text,
        }
    }

    #[allow(dead_code)]
    pub fn with_auto_theme(self, theme: TrayTheme) -> Self {
        if self.icon_color_mode == TrayIconColorMode::Auto {
            let mut style = Self::from_color_mode(TrayIconColorMode::Auto, theme);
            style.system_theme = self.system_theme;
            style
        } else {
            self
        }
    }

    /// 根据电量百分比选择填充色。
    ///
    /// 优先级与现有静态图标一致：
    /// - 充电 → charging_fill（绿）
    /// - ≤ 阈值 → low_fill（红）
    /// - ≤ 50% → mid_fill（黄）
    /// - > 50% → normal_fill（绿）
    pub fn fill_for_battery(&self, percentage: u8, charging: bool, threshold: u8) -> RgbaColor {
        if charging {
            self.charging_fill
        } else if percentage <= threshold {
            self.low_fill
        } else if percentage <= 50 {
            self.mid_fill
        } else {
            self.normal_fill
        }
    }

    pub fn fill_for_receiver(
        &self,
        percentage: Option<u8>,
        charging: bool,
        threshold: u8,
    ) -> RgbaColor {
        match percentage {
            None => self.unknown_fill,
            Some(_) if charging => self.charging_fill,
            Some(value) if value <= threshold => self.receiver_low_fill,
            Some(_) => self.receiver_fill,
        }
    }
}

/// 解析托盘主题：结合 `tray_icon_color` 设置和系统主题。
///
/// 调用方负责提供 `system_dark`（来自 `tray_theme_is_dark` 或主题监听器缓存）。
#[cfg(test)]
pub fn resolve_tray_theme(settings: &TraySettings<'_>, system_dark: bool) -> TrayTheme {
    let mode = TrayIconColorMode::from_setting(settings.tray_icon_color);
    match mode {
        TrayIconColorMode::White => TrayTheme::Dark,
        TrayIconColorMode::Black => TrayTheme::Light,
        TrayIconColorMode::Auto => TrayTheme::from_system_dark(system_dark),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_color(color: &str) -> TraySettings<'_> {
        TraySettings {
            show_receiver: false,
            show_connection: true,
            show_battery_title: true,
            low_battery_threshold: 20,
            tray_icon_color: color,
            tray_render_mode: "auto",
        }
    }

    #[test]
    fn dark_theme_uses_light_outline() {
        let settings = settings_with_color("auto");
        let style = TrayVisualStyle::from_settings(&settings, TrayTheme::Dark);
        assert_eq!(style.outline, RgbaColor::rgba(255, 255, 255, 118));
        assert_eq!(style.outline_secondary, RgbaColor::rgba(255, 255, 255, 210));
    }

    #[test]
    fn light_theme_uses_dark_outline() {
        let settings = settings_with_color("auto");
        let style = TrayVisualStyle::from_settings(&settings, TrayTheme::Light);
        assert_eq!(style.outline, RgbaColor::rgba(0, 0, 0, 118));
        assert_eq!(style.outline_secondary, RgbaColor::rgba(0, 0, 0, 210));
    }

    #[test]
    fn white_mode_forces_dark_theme_outline() {
        let settings = settings_with_color("white");
        // 即使系统是浅色主题，white 模式强制使用白色轮廓
        let style = TrayVisualStyle::from_settings(&settings, TrayTheme::Light);
        assert_eq!(style.outline, RgbaColor::rgba(255, 255, 255, 118));
    }

    #[test]
    fn black_mode_forces_light_theme_outline() {
        let settings = settings_with_color("black");
        // 即使系统是深色主题，black 模式强制使用黑色轮廓
        let style = TrayVisualStyle::from_settings(&settings, TrayTheme::Dark);
        assert_eq!(style.outline, RgbaColor::rgba(0, 0, 0, 118));
    }

    #[test]
    fn auto_mode_follows_system_theme() {
        let settings = settings_with_color("auto");
        let dark_style = TrayVisualStyle::from_settings(&settings, TrayTheme::Dark);
        assert_eq!(dark_style.outline, RgbaColor::rgba(255, 255, 255, 118));

        let light_style = TrayVisualStyle::from_settings(&settings, TrayTheme::Light);
        assert_eq!(light_style.outline, RgbaColor::rgba(0, 0, 0, 118));
    }

    #[test]
    fn forced_icon_color_preserves_system_theme_for_colored_app_icon() {
        let white_settings = settings_with_color("white");
        let style = TrayVisualStyle::from_settings(&white_settings, TrayTheme::Light);

        assert_eq!(style.system_theme, TrayTheme::Light);
        assert_eq!(style.theme, TrayTheme::Dark);
    }

    #[test]
    fn menu_bar_auto_override_does_not_change_colored_app_icon_theme() {
        let settings = settings_with_color("auto");
        let system_style = TrayVisualStyle::from_settings(&settings, TrayTheme::Light);
        let dark_menu_bar_style = system_style.with_auto_theme(TrayTheme::Dark);

        assert_eq!(dark_menu_bar_style.system_theme, TrayTheme::Light);
        assert_eq!(dark_menu_bar_style.theme, TrayTheme::Dark);
    }

    #[test]
    fn fill_for_battery_uses_threshold_and_mid_tier() {
        let settings = settings_with_color("auto");
        let style = TrayVisualStyle::from_settings(&settings, TrayTheme::Dark);

        // 充电 → 绿色
        assert_eq!(style.fill_for_battery(15, true, 20), style.charging_fill);

        // ≤ 阈值（未充电）→ 红色
        assert_eq!(style.fill_for_battery(20, false, 20), style.low_fill);

        // 阈值 < 电量 ≤ 50% → 黄色
        assert_eq!(style.fill_for_battery(50, false, 20), style.mid_fill);
        assert_eq!(style.fill_for_battery(30, false, 20), style.mid_fill);

        // > 50% → 绿色
        assert_eq!(style.fill_for_battery(51, false, 20), style.normal_fill);
        assert_eq!(style.fill_for_battery(100, false, 20), style.normal_fill);
    }

    #[test]
    fn fill_for_receiver_uses_setting_threshold() {
        let settings = settings_with_color("auto");
        let style = TrayVisualStyle::from_settings(&settings, TrayTheme::Dark);

        assert_eq!(style.fill_for_receiver(None, false, 20), style.unknown_fill);
        assert_eq!(
            style.fill_for_receiver(Some(5), true, 20),
            style.charging_fill
        );
        assert_eq!(
            style.fill_for_receiver(Some(5), false, 20),
            style.receiver_low_fill
        );
        assert_eq!(
            style.fill_for_receiver(Some(80), false, 20),
            style.receiver_fill
        );
    }

    #[test]
    fn resolve_tray_theme_respects_color_mode() {
        let white_settings = settings_with_color("white");
        assert_eq!(resolve_tray_theme(&white_settings, false), TrayTheme::Dark);

        let black_settings = settings_with_color("black");
        assert_eq!(resolve_tray_theme(&black_settings, true), TrayTheme::Light);

        let auto_settings = settings_with_color("auto");
        assert_eq!(resolve_tray_theme(&auto_settings, true), TrayTheme::Dark);
        assert_eq!(resolve_tray_theme(&auto_settings, false), TrayTheme::Light);
    }
}
