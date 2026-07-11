// SPDX-License-Identifier: AGPL-3.0-or-later
//! 共享绘图模块：RGBA Canvas + 鼠标图标绘制。
//!
//! 绘图参数与 `scripts/generate-tray-mouse-icons.py` 保持一致，
//! 确保动态图标和静态 PNG fallback 在视觉上无缝切换。
//! 此模块纯 Rust 无平台依赖，被以下渲染器共享：
//! - `dynamic_icon.rs`：Windows / Linux 动态 PNG → Tauri Image
//! - macOS 原生渲染器：通过 NSImage 使用相同的 RGBA 输出

use crate::tray::state::TrayStatusState;
use crate::tray::style::{RgbaColor, TrayVisualStyle};

/// RGBA 像素缓冲。原点 (0,0) 在左上角。
pub struct IconCanvas {
    pub width: u32,
    pub height: u32,
    pixels: Vec<u8>, // RGBA, row-major, top-to-bottom
}

impl IconCanvas {
    pub fn new(width: u32, height: u32) -> Self {
        IconCanvas {
            width,
            height,
            pixels: vec![0u8; (width * height * 4) as usize],
        }
    }

    pub fn into_rgba_bytes(self) -> Vec<u8> {
        self.pixels
    }

    pub fn pixel_index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some(((y as u32 * self.width + x as u32) * 4) as usize)
    }

    /// Alpha 合成一个像素（source over）。
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: RgbaColor) {
        let Some(idx) = self.pixel_index(x, y) else {
            return;
        };
        let sa = color.a as u32;
        let da = self.pixels[idx + 3] as u32;
        // out_a = sa + da * (255 - sa) / 255
        let out_a = sa + da * (255 - sa) / 255;
        if out_a == 0 {
            self.pixels[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        // out_rgb = (src_rgb * sa + dst_rgb * da * (255 - sa) / 255) / out_a
        for c in 0..3 {
            let src = color.channels()[c] as u32 * sa;
            let dst = self.pixels[idx + c] as u32 * da * (255 - sa) / 255;
            self.pixels[idx + c] = ((src + dst) / out_a) as u8;
        }
        self.pixels[idx + 3] = out_a as u8;
    }

    /// 将一个像素清除为完全透明。
    pub fn clear_pixel(&mut self, x: i32, y: i32) {
        let Some(idx) = self.pixel_index(x, y) else {
            return;
        };
        self.pixels[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0]);
    }

    /// 填充实心圆角矩形（alpha 合成）。
    pub fn fill_rounded_rect(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        radius: i32,
        color: RgbaColor,
    ) {
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                if is_inside_rounded_rect(x, y, x0, y0, x1, y1, radius) {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }

    /// 绘制圆角矩形外轮廓（描边，alpha 合成）。
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rounded_rect(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        radius: i32,
        width: i32,
        color: RgbaColor,
    ) {
        if x1 <= x0 || y1 <= y0 || width <= 0 {
            return;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                if is_inside_rounded_rect(x, y, x0, y0, x1, y1, radius)
                    && !is_inside_rounded_rect(
                        x,
                        y,
                        x0 + width,
                        y0 + width,
                        x1 - width,
                        y1 - width,
                        (radius - width).max(0),
                    )
                {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }

    /// 清除圆角矩形区域为透明（用于电量填充中绕开中键区域）。
    pub fn clear_rounded_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, radius: i32) {
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                if is_inside_rounded_rect(x, y, x0, y0, x1, y1, radius) {
                    self.clear_pixel(x, y);
                }
            }
        }
    }

    /// 填充多边形（扫描线算法，even-odd rule）。
    pub fn fill_polygon(&mut self, points: &[(i32, i32)], color: RgbaColor) {
        if points.len() < 3 {
            return;
        }
        let min_y = points.iter().map(|p| p.1).min().unwrap();
        let max_y = points.iter().map(|p| p.1).max().unwrap();
        for y in min_y..=max_y {
            let mut intersections = Vec::new();
            let n = points.len();
            for i in 0..n {
                let (x0, y0) = points[i];
                let (x1, y1) = points[(i + 1) % n];
                if (y0 <= y && y < y1) || (y1 <= y && y < y0) {
                    let dy = (y1 - y0) as f64;
                    if dy.abs() < f64::EPSILON {
                        continue;
                    }
                    let t = (y - y0) as f64 / dy;
                    let x = x0 as f64 + t * (x1 - x0) as f64;
                    intersections.push(x);
                }
            }
            intersections.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < intersections.len() {
                let x_start = intersections[i].ceil() as i32;
                let x_end = intersections[i + 1].floor() as i32;
                for x in x_start..=x_end {
                    self.blend_pixel(x, y, color);
                }
                i += 2;
            }
        }
    }

    /// 清除闭合多边形描边周围的透明安全区。
    ///
    /// 充电闪电会在 64px 图标中缩小到 macOS 约 20px 的菜单栏尺寸；
    /// 用均匀的描边安全区比手写一枚更大的多边形更稳定。
    pub fn clear_polygon_halo(&mut self, points: &[(i32, i32)], width: i32) {
        if points.len() < 2 || width <= 0 {
            return;
        }
        let radius = width as f64 / 2.0;
        let min_x = points.iter().map(|point| point.0).min().unwrap() - width;
        let max_x = points.iter().map(|point| point.0).max().unwrap() + width;
        let min_y = points.iter().map(|point| point.1).min().unwrap() - width;
        let max_y = points.iter().map(|point| point.1).max().unwrap() + width;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let within_halo = (0..points.len()).any(|index| {
                    let start = points[index];
                    let end = points[(index + 1) % points.len()];
                    squared_distance_to_segment(x as f64, y as f64, start, end) <= radius * radius
                });
                if within_halo {
                    self.clear_pixel(x, y);
                }
            }
        }
    }

    /// 填充圆形标记（alpha 合成）。
    pub fn fill_circle(&mut self, cx: i32, cy: i32, radius: i32, color: RgbaColor) {
        if radius <= 0 {
            return;
        }
        let r2 = radius * radius;
        for y in (cy - radius)..=(cy + radius) {
            for x in (cx - radius)..=(cx + radius) {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= r2 {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }
}

fn squared_distance_to_segment(px: f64, py: f64, start: (i32, i32), end: (i32, i32)) -> f64 {
    let (x0, y0) = (start.0 as f64, start.1 as f64);
    let (x1, y1) = (end.0 as f64, end.1 as f64);
    let dx = x1 - x0;
    let dy = y1 - y0;
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return (px - x0).powi(2) + (py - y0).powi(2);
    }
    let position = (((px - x0) * dx + (py - y0) * dy) / length_squared).clamp(0.0, 1.0);
    let nearest_x = x0 + position * dx;
    let nearest_y = y0 + position * dy;
    (px - nearest_x).powi(2) + (py - nearest_y).powi(2)
}

impl RgbaColor {
    pub fn channels(&self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// 判断点 (px, py) 是否在圆角矩形内部。
fn is_inside_rounded_rect(
    px: i32,
    py: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radius: i32,
) -> bool {
    if px < x0 || px >= x1 || py < y0 || py >= y1 {
        return false;
    }
    let radius = radius.min((x1 - x0) / 2).min((y1 - y0) / 2).max(0);
    if radius == 0 {
        return true;
    }
    // 四个圆角的中心
    let corners = [
        (x0 + radius, y0 + radius),
        (x1 - radius - 1, y0 + radius),
        (x0 + radius, y1 - radius - 1),
        (x1 - radius - 1, y1 - radius - 1),
    ];
    // 内矩形（不含圆角区域）
    let inner_x0 = x0 + radius;
    let inner_x1 = x1 - radius;
    let inner_y0 = y0 + radius;
    let inner_y1 = y1 - radius;
    if px >= inner_x0 && px < inner_x1 || py >= inner_y0 && py < inner_y1 {
        return true;
    }
    // 检查是否在某个圆角圆内
    for (cx, cy) in corners {
        let dx = px - cx;
        let dy = py - cy;
        if dx * dx + dy * dy <= radius * radius {
            return true;
        }
    }
    false
}

// ─── 鼠标图标绘制 ───────────────────────────────────────────────────────────
//
// 绘图参数与 `scripts/generate-tray-mouse-icons.py` 一致：
//   SIZE = 64, OUTLINE_WIDTH = 4, GAP = 2, FILL_INSET = 1, OUTLINE_ALPHA = 160
//   WHEEL_WIDTH = 7, WHEEL_LENGTH = 14, WHEEL_GAP = 2
//   鼠标外形: 46×60 圆角矩形, radius=16, 居中
//   中键: 7×14 圆角矩形, 顶部居中
//   电量填充: 从底部向上的圆角矩形, 绕开中键区域
//   充电闪电: 7 点多边形, 叠加在中心，周围保留透明 halo

const ICON_SIZE: u32 = 64;
const OUTLINE_WIDTH: i32 = 4;
const OUTLINE_GAP: i32 = 2;
const FILL_INSET: i32 = 1;
const WHEEL_WIDTH: i32 = 7;
const WHEEL_LENGTH: i32 = 14;
const WHEEL_GAP: i32 = 2;
const SHAPE_RADIUS: i32 = 16;
const FILL_RADIUS: i32 = SHAPE_RADIUS - OUTLINE_WIDTH - OUTLINE_GAP - FILL_INSET;

/// 充电闪电多边形顶点。轮廓与静态 PNG 生成器保持一致；在 20px
/// 菜单栏尺寸仍保留清晰的折角，而不会显得又细又长。
const CHARGING_BOLT: [(i32, i32); 6] = [(38, 13), (32, 28), (46, 28), (29, 47), (35, 32), (23, 32)];
const CHARGING_BOLT_HALO_WIDTH: i32 = 9;

/// 鼠标外形边界：宽 46, 高 60, 居中。
fn mouse_shape_bounds(size: u32) -> (i32, i32, i32, i32) {
    let width = 46i32;
    let height = 60i32;
    let left = (size as i32 - width) / 2;
    let top = (size as i32 - height) / 2;
    (left, top, left + width, top + height)
}

/// 绘制鼠标图标到 canvas。
///
/// 状态优先级：未连接 > 未知电量 > 充电 > 低电量 > 正常。
/// - 未连接：不调用此函数（调用方显示 app 图标）
/// - 未知电量：仅轮廓 + 中键，底部灰色短线
/// - 已知电量：轮廓 + 中键 + 电量填充（分级颜色）+ 充电闪电
pub fn draw_mouse_icon(canvas: &mut IconCanvas, state: &TrayStatusState, style: &TrayVisualStyle) {
    let size = ICON_SIZE;
    let outer = mouse_shape_bounds(size);

    // 1. 外轮廓（圆角矩形描边）
    canvas.stroke_rounded_rect(
        outer.0,
        outer.1,
        outer.2,
        outer.3,
        SHAPE_RADIUS,
        OUTLINE_WIDTH,
        style.outline,
    );

    // 内部区域（轮廓 + 间隙之后）
    let inset = OUTLINE_WIDTH + OUTLINE_GAP;
    let inner = (
        outer.0 + inset,
        outer.1 + inset,
        outer.2 - inset,
        outer.3 - inset,
    );
    let fill_area = (
        inner.0 + FILL_INSET,
        inner.1 + FILL_INSET,
        inner.2 - FILL_INSET,
        inner.3 - FILL_INSET,
    );

    let center_x = (size as i32) / 2;
    let wheel_top = inner.1 + 4;
    let wheel_bottom = wheel_top + WHEEL_LENGTH;
    let wheel_left = center_x - WHEEL_WIDTH / 2;
    let wheel_right = wheel_left + WHEEL_WIDTH;

    // 2. 电量填充（从底部向上）
    if let Some(percentage) = state.mouse_battery {
        let fill_color = style.fill_for_battery(
            percentage,
            state.mouse_charging,
            state.low_battery_threshold,
        );
        let inner_height = fill_area.3 - fill_area.1;
        let fill_height = if percentage >= 100 {
            // 视觉补偿：满电时圆角与轮廓同心，径向间隙处处 3px，
            // 但圆角对角方向的间隙在视觉上比水平/垂直方向显宽，
            // 导致顶部直线段间隙反衬显窄。少填 1px 使顶部间隙为 4px，
            // 补偿视差，与侧面 3px 视觉一致（同 99% 整数截断效果）。
            inner_height - 1
        } else {
            inner_height * percentage as i32 / 100
        };
        if fill_height > 0 {
            let fill_y0 = fill_area.3 - fill_height;
            // 填充圆角矩形
            canvas.fill_rounded_rect(
                fill_area.0,
                fill_y0,
                fill_area.2,
                fill_area.3,
                FILL_RADIUS,
                fill_color,
            );
            // 电量填充绕开中键区域（四周留 WHEEL_GAP 透明边缘）
            if wheel_bottom + WHEEL_GAP >= fill_y0 {
                let clear_top = fill_y0.max(wheel_top - WHEEL_GAP);
                canvas.clear_rounded_rect(
                    wheel_left - WHEEL_GAP,
                    clear_top,
                    wheel_right + WHEEL_GAP,
                    wheel_bottom + WHEEL_GAP,
                    WHEEL_GAP,
                );
            }
        }
    } else {
        // 电量未知：底部绘制灰色弱化短线（不误显示满电）
        let line_height = 3i32;
        canvas.fill_rounded_rect(
            inner.0 + 4,
            inner.3 - line_height - 2,
            inner.2 - 4,
            inner.3 - 2,
            1,
            style.unknown_fill,
        );
    }

    // 3. 中键（圆角矩形，亮色）
    canvas.fill_rounded_rect(
        wheel_left,
        wheel_top,
        wheel_right + 1,
        wheel_bottom,
        WHEEL_WIDTH / 2,
        style.outline_secondary,
    );

    // 4. 充电闪电（多边形，叠加在中心）
    if state.mouse_charging {
        canvas.clear_polygon_halo(&CHARGING_BOLT, CHARGING_BOLT_HALO_WIDTH);
        canvas.fill_polygon(
            &CHARGING_BOLT,
            RgbaColor::rgb(
                style.outline_secondary.r,
                style.outline_secondary.g,
                style.outline_secondary.b,
            ),
        );
    }

    // 5. 接收器状态：右下角小标记，按现有 trayIncludeReceiverBattery 设置控制。
    if state.show_receiver && state.connected {
        let marker_color = style.fill_for_receiver(
            state.receiver_battery,
            state.receiver_charging,
            state.low_battery_threshold,
        );
        canvas.fill_circle(48, 50, 7, style.outline);
        canvas.fill_circle(48, 50, 5, marker_color);
    }
}

/// 生成 64×64 RGBA 鼠标图标。
///
/// 调用方负责将返回的 RGBA 字节转换为平台图标（Tauri Image / NSImage）。
pub fn render_mouse_icon_rgba(state: &TrayStatusState, style: &TrayVisualStyle) -> Vec<u8> {
    let mut canvas = IconCanvas::new(ICON_SIZE, ICON_SIZE);
    draw_mouse_icon(&mut canvas, state, style);
    canvas.into_rgba_bytes()
}

#[allow(dead_code)]
pub fn encode_rgba_png(
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("image dimensions overflow")?;
    if rgba.len() != expected_len {
        return Err(format!(
            "rgba buffer has {} bytes, expected {expected_len}",
            rgba.len()
        )
        .into());
    }

    let mut png_bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    drop(writer);
    Ok(png_bytes)
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

    fn make_style() -> TrayVisualStyle {
        TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Dark)
    }

    #[test]
    fn canvas_starts_transparent() {
        let canvas = IconCanvas::new(64, 64);
        let bytes = canvas.into_rgba_bytes();
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn canvas_blend_pixel_writes_opaque() {
        let mut canvas = IconCanvas::new(64, 64);
        canvas.blend_pixel(10, 20, RgbaColor::rgb(255, 0, 0));
        let bytes = canvas.into_rgba_bytes();
        let idx = ((20 * 64 + 10) * 4) as usize;
        assert_eq!(&bytes[idx..idx + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn canvas_blend_pixel_alpha_composites() {
        let mut canvas = IconCanvas::new(64, 64);
        canvas.blend_pixel(0, 0, RgbaColor::rgba(255, 0, 0, 128));
        canvas.blend_pixel(0, 0, RgbaColor::rgba(0, 255, 0, 128));
        let bytes = canvas.into_rgba_bytes();
        // 第二次混合后：绿色应占主导（source over 半透明绿）
        let idx = 0;
        let g = bytes[idx + 1];
        let r = bytes[idx];
        assert!(g > r, "green should dominate after over compositing");
    }

    #[test]
    fn render_produces_64x64_rgba() {
        let state = make_state(Some(75), false);
        let style = make_style();
        let bytes = render_mouse_icon_rgba(&state, &style);
        assert_eq!(bytes.len(), 64 * 64 * 4);
    }

    #[test]
    fn render_has_transparent_background() {
        let state = make_state(Some(75), false);
        let style = make_style();
        let bytes = render_mouse_icon_rgba(&state, &style);
        // 左上角应在鼠标轮廓外，保持透明
        let corner_idx = 0;
        assert_eq!(bytes[corner_idx + 3], 0);
        // 右下角也应在轮廓外
        let corner_idx = ((63 * 64 + 63) * 4) as usize;
        assert_eq!(bytes[corner_idx + 3], 0);
    }

    #[test]
    fn render_different_levels_produce_different_icons() {
        let style = make_style();

        let low_state = make_state(Some(10), false);
        let low_bytes = render_mouse_icon_rgba(&low_state, &style);

        let high_state = make_state(Some(90), false);
        let high_bytes = render_mouse_icon_rgba(&high_state, &style);

        assert_ne!(low_bytes, high_bytes);
    }

    #[test]
    fn render_charging_differs_from_non_charging() {
        let style = make_style();

        let normal = make_state(Some(50), false);
        let normal_bytes = render_mouse_icon_rgba(&normal, &style);

        let charging = make_state(Some(50), true);
        let charging_bytes = render_mouse_icon_rgba(&charging, &style);

        assert_ne!(normal_bytes, charging_bytes);
    }

    #[test]
    fn render_dark_theme_differs_from_light() {
        let state = make_state(Some(75), false);

        let dark_style = TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Dark);
        let light_style = TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Light);

        let dark_bytes = render_mouse_icon_rgba(&state, &dark_style);
        let light_bytes = render_mouse_icon_rgba(&state, &light_style);

        assert_ne!(dark_bytes, light_bytes);
    }

    #[test]
    fn render_unknown_battery_does_not_show_full() {
        let style = make_style();
        let unknown_state = make_state(None, false);
        let unknown_bytes = render_mouse_icon_rgba(&unknown_state, &style);

        let full_state = make_state(Some(100), false);
        let full_bytes = render_mouse_icon_rgba(&full_state, &style);

        assert_ne!(unknown_bytes, full_bytes);
    }

    #[test]
    fn full_battery_fill_keeps_even_inner_padding() {
        let style = make_style();
        let state = make_state(Some(100), false);
        let bytes = render_mouse_icon_rgba(&state, &style);
        let outer = mouse_shape_bounds(ICON_SIZE);
        let inset = OUTLINE_WIDTH + OUTLINE_GAP;
        let inner = (
            outer.0 + inset,
            outer.1 + inset,
            outer.2 - inset,
            outer.3 - inset,
        );
        let fill_area = (
            inner.0 + FILL_INSET,
            inner.1 + FILL_INSET,
            inner.2 - FILL_INSET,
            inner.3 - FILL_INSET,
        );
        let center_x = (fill_area.0 + fill_area.2) / 2;
        let center_y = (fill_area.1 + fill_area.3) / 2;

        // 满电时顶部视觉补偿少填 1px，所以顶部采样点在 fill_area.1 + 1。
        // x 偏离中心以避开中键透明清除区（wheel clearing x≈27..38）
        let sample_points = [
            (fill_area.0, center_y),
            (fill_area.2 - 1, center_y),
            (center_x - 7, fill_area.1 + 1),
            (center_x, fill_area.3 - 1),
        ];
        for (x, y) in sample_points {
            let idx = ((y * ICON_SIZE as i32 + x) * 4) as usize;
            assert!(
                bytes[idx + 3] > 0,
                "full charge fill should reach padded fill area at {x},{y}"
            );
        }

        let gap_points = [
            (fill_area.0 - 1, center_y),
            (fill_area.2, center_y),
            (center_x, fill_area.1),
            (center_x, fill_area.3),
        ];
        for (x, y) in gap_points {
            let idx = ((y * ICON_SIZE as i32 + x) * 4) as usize;
            assert_eq!(
                bytes[idx + 3],
                0,
                "full charge fill should leave transparent padding at {x},{y}"
            );
        }
    }

    #[test]
    fn charging_bolt_uses_solid_icon_color_and_clear_gap() {
        let style = make_style();
        let state = make_state(Some(100), true);
        let bytes = render_mouse_icon_rgba(&state, &style);

        let bolt_idx = ((28 * ICON_SIZE as i32 + 34) * 4) as usize;
        assert_eq!(&bytes[bolt_idx..bolt_idx + 4], &[255, 255, 255, 255]);

        let gap_idx = ((29 * ICON_SIZE as i32 + 47) * 4) as usize;
        assert_eq!(
            bytes[gap_idx + 3],
            0,
            "charging bolt should leave transparent spacing around the solid shape"
        );

        for (x, y) in [(40, 12), (22, 31), (47, 29), (28, 48)] {
            let idx = ((y * ICON_SIZE as i32 + x) * 4) as usize;
            assert_eq!(
                bytes[idx + 3],
                0,
                "charging bolt halo should stay transparent near the white shape at {x},{y}"
            );
        }

        let light_style = TrayVisualStyle::from_settings(&test_settings(), TrayTheme::Light);
        let light_bytes = render_mouse_icon_rgba(&state, &light_style);
        assert_eq!(&light_bytes[bolt_idx..bolt_idx + 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn render_receiver_marker_changes_icon() {
        let mut settings = test_settings();
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: Connection::Usb,
            battery_percent: Some(80),
            charging: false,
            batteries: vec![
                mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: "鼠标".into(),
                    percentage: 80,
                    charging: false,
                },
                mira_core::DeviceBattery {
                    id: "receiver".into(),
                    label: "接收器".into(),
                    percentage: 10,
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
        };

        settings.show_receiver = false;
        let no_receiver = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        settings.show_receiver = true;
        let with_receiver = TrayStatusState::from_snapshot(Some(&snapshot), &settings);
        let style = make_style();

        assert_ne!(
            render_mouse_icon_rgba(&no_receiver, &style),
            render_mouse_icon_rgba(&with_receiver, &style)
        );
    }

    #[test]
    fn encode_rgba_png_produces_decodable_png() {
        let state = make_state(Some(75), false);
        let style = make_style();
        let bytes = render_mouse_icon_rgba(&state, &style);
        let png = encode_rgba_png(&bytes, 64, 64).unwrap();
        let image = tauri::image::Image::from_bytes(&png).unwrap();
        assert_eq!((image.width(), image.height()), (64, 64));
    }

    #[test]
    fn render_low_battery_uses_red_fill() {
        let style = make_style();
        let state = make_state(Some(10), false); // 10% ≤ 阈值 20 → 红色
        let bytes = render_mouse_icon_rgba(&state, &style);

        // 在电量填充区域（底部中心）应能找到红色像素
        let outer = mouse_shape_bounds(ICON_SIZE);
        let inset = OUTLINE_WIDTH + OUTLINE_GAP;
        let inner_bottom = outer.3 - inset;
        let fill_bottom = inner_bottom - FILL_INSET;
        let fill_x = (outer.0 + outer.2) / 2;
        let fill_y = fill_bottom - 2;
        let idx = ((fill_y * 64 + fill_x) * 4) as usize;
        let r = bytes[idx];
        let g = bytes[idx + 1];
        let b = bytes[idx + 2];
        let a = bytes[idx + 3];
        // 红色 #FF3B30 = (255, 59, 48)
        assert!(a > 0, "pixel should be non-transparent");
        assert!(r > g && r > b, "red should dominate at low battery");
    }

    #[test]
    fn render_normal_battery_uses_green_fill() {
        let style = make_style();
        let state = make_state(Some(80), false); // 80% > 50% → 绿色
        let bytes = render_mouse_icon_rgba(&state, &style);

        // 在电量填充主体区域应能找到绿色像素。选中心偏下位置避开中键透明区域。
        let fill_y = 40;
        let fill_x = 32;
        let idx = ((fill_y * 64 + fill_x) * 4) as usize;
        let r = bytes[idx];
        let g = bytes[idx + 1];
        let b = bytes[idx + 2];
        let a = bytes[idx + 3];
        assert!(a > 0, "pixel should be non-transparent");
        assert!(g > r && g > b, "green should dominate at normal battery");
    }

    #[test]
    fn rounded_rect_inside_check_works() {
        // 10×10 圆角矩形，radius=3
        assert!(is_inside_rounded_rect(5, 5, 0, 0, 10, 10, 3));
        assert!(!is_inside_rounded_rect(-1, 5, 0, 0, 10, 10, 3));
        assert!(!is_inside_rounded_rect(10, 5, 0, 0, 10, 10, 3));
        // 圆角外角应不在内部
        assert!(!is_inside_rounded_rect(0, 0, 0, 0, 10, 10, 3));
        assert!(is_inside_rounded_rect(3, 3, 0, 0, 10, 10, 3));
    }

    #[test]
    fn fill_polygon_draws_triangle() {
        let mut canvas = IconCanvas::new(32, 32);
        canvas.fill_polygon(&[(5, 5), (25, 5), (15, 25)], RgbaColor::rgb(255, 0, 0));
        let bytes = canvas.into_rgba_bytes();
        // 中心点应在三角形内
        let idx = ((12 * 32 + 15) * 4) as usize;
        assert_eq!(bytes[idx + 3], 255);
    }
}
