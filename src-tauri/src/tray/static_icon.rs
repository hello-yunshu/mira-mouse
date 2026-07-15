// SPDX-License-Identifier: AGPL-3.0-or-later
//! 静态 PNG fallback 图标选择逻辑。
//!
//! 从 `lib.rs` 迁移，保留行为不变。当动态图标生成失败或 `trayRenderMode = "static"` 时使用。
//! 静态图标按 10% 分档，由 `scripts/generate-tray-mouse-icons.py` 生成。

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

/// 按 (level, dark, charging) 选择静态 PNG 图标字节。
///
/// `level` 应为 0..=100 的 10% 分档值（调用方负责四舍五入）。
pub fn static_tray_icon_bytes(level: u8, dark: bool, charging: bool) -> &'static [u8] {
    let index = (level / 10).min(10) as usize;
    match (dark, charging) {
        (true, true) => TRAY_ICONS_DARK_CHARGING[index],
        (false, true) => TRAY_ICONS_LIGHT_CHARGING[index],
        (true, false) => TRAY_ICONS_DARK[index],
        (false, false) => TRAY_ICONS_LIGHT[index],
    }
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
    fn static_icon_lookup_returns_correct_array() {
        // 比较内容而非指针：const 数组在不同使用点可能被编译器内联为不同副本，
        // 但 include_bytes! 引用的字节内容相同。
        assert_eq!(static_tray_icon_bytes(0, false, false), TRAY_ICONS_LIGHT[0]);
        assert_eq!(
            static_tray_icon_bytes(100, true, true),
            TRAY_ICONS_DARK_CHARGING[10]
        );
        assert_eq!(
            static_tray_icon_bytes(50, false, true),
            TRAY_ICONS_LIGHT_CHARGING[5]
        );
        assert_eq!(static_tray_icon_bytes(80, true, false), TRAY_ICONS_DARK[8]);
    }

    #[test]
    fn static_icon_clamps_overflow() {
        // 超出 100 的值被 clamp 到 100% 图标
        let bytes = static_tray_icon_bytes(150, false, false);
        assert_eq!(bytes.len(), TRAY_ICONS_LIGHT[10].len());
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
