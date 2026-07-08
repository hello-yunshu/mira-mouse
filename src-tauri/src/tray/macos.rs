// SPDX-License-Identifier: AGPL-3.0-or-later
//! macOS 原生菜单栏控制器：NSStatusItem + NSImage 自绘。
//!
//! ## 架构
//!
//! ```text
//! MacNativeTrayController
//! ├── NSStatusItem (原生菜单栏项)
//! │   ├── NSStatusBarButton (按钮)
//! │   │   ├── NSImage (来自共享 RGBA 渲染器，保证与静态 PNG 视觉一致)
//! │   │   ├── MiraStatusView.drawRect: (自绘层)
//! │   │   ├── tooltip
//! │   │   └── target/action → MiraStatusItemDelegate.openWindow:
//! │   └── NSMenu (右键菜单)
//! │       ├── 电池信息项 (disabled)
//! │       ├── 分隔线
//! │       ├── "打开 Mira" → delegate.openWindow:
//! │       └── "退出 Mira" → delegate.quitApp:
//! ├── MiraStatusItemDelegate (自定义 NSObject，处理菜单动作)
//! ├── TrayIconCacheKey diff (避免重复生成 NSImage)
//! └── TauriTrayController fallback (初始化失败时使用)
//! ```
//!
//! ## 外观一致性
//!
//! 使用 `tray::image::render_mouse_icon_rgba` 生成 RGBA 字节，
//! 然后创建 NSImage。该函数与 `scripts/generate-tray-mouse-icons.py`
//! 参数完全一致，保证与静态 PNG fallback 视觉无缝切换。
//!
//! ## 优先级
//!
//! 可编译 > 不破坏现有功能 > fallback 稳定 > macOS 原生体验 > 视觉细节

use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};

use objc2::rc::Allocated;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{
    define_class, extern_methods, msg_send, AllocAnyThread, ClassType, DefinedClass,
    MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSView,
};
use objc2_foundation::{NSArray, NSData, NSPoint, NSRect, NSSize, NSString};

use crate::tray::dynamic_icon::TrayIconCacheKey;
use crate::tray::image::{encode_rgba_png, render_mouse_icon_rgba};
use crate::tray::renderer::{TauriTrayController, TrayController};
use crate::tray::state::{TrayRenderMode, TrayStatusState};
use crate::tray::style::{TrayIconColorMode, TrayTheme, TrayVisualStyle};

// ─── 全局状态（供 delegate 回调使用） ─────────────────────────────────────

/// 全局 AppHandle，在 `build_tray` 时设置一次。
/// delegate 的 `openWindow:` / `quitApp:` 回调通过此句柄操作主窗口。
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 全局语言代码，每次 `update_tray` 时更新。
static APP_LANG: Mutex<&'static str> = Mutex::new("zh-CN");

/// 存储 AppHandle（由 `lib.rs::build_tray` 调用）。
pub fn set_app_handle(handle: tauri::AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

/// 更新当前语言（由 `lib.rs::update_tray` 调用）。
pub fn set_app_lang(lang: &'static str) {
    if let Ok(mut l) = APP_LANG.lock() {
        *l = lang;
    }
}

fn current_lang() -> &'static str {
    APP_LANG.lock().map(|l| *l).unwrap_or("zh-CN")
}

// ─── 自定义 NSObject 子类：处理菜单动作 ───────────────────────────────────

define_class!(
    #[unsafe(super = NSObject)]
    struct MiraStatusItemDelegate;

    impl MiraStatusItemDelegate {
        /// 菜单 "打开 Mira" / 状态栏按钮点击 → 聚焦主窗口
        #[unsafe(method(openWindow:))]
        fn open_window(&self, _sender: Option<&AnyObject>) {
            if let Some(handle) = APP_HANDLE.get() {
                let _ = handle.run_on_main_thread(|| {
                    crate::focus_main_from_tray(handle);
                });
            }
        }

        /// 菜单电量项 → 聚焦主窗口并打开电量使用情况
        #[unsafe(method(openBatteryUsage:))]
        fn open_battery_usage(&self, _sender: Option<&AnyObject>) {
            if let Some(handle) = APP_HANDLE.get() {
                let _ = handle.run_on_main_thread(|| {
                    crate::open_battery_usage_from_tray(handle);
                });
            }
        }

        /// 菜单 "退出 Mira" → 退出应用
        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: Option<&AnyObject>) {
            if let Some(handle) = APP_HANDLE.get() {
                handle.exit(0);
            }
        }
    }
);

impl MiraStatusItemDelegate {
    extern_methods!(
        /// 创建 delegate 实例（NSObject 的 alloc + init）
        #[unsafe(method(new))]
        fn new() -> Retained<Self>;
    );
}

// ─── 自定义 NSView：只负责 drawRect 自绘，点击事件继续交给 NSStatusBarButton ───────

struct MiraStatusViewIvars {
    image: RefCell<Option<Retained<NSImage>>>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[ivars = MiraStatusViewIvars]
    struct MiraStatusView;

    impl MiraStatusView {
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Option<Retained<Self>> {
            let this = this.set_ivars(MiraStatusViewIvars {
                image: RefCell::new(None),
            });
            unsafe { msg_send![super(this), initWithFrame: frame] }
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let Some(image) = self.ivars().image.borrow().as_ref().cloned() else {
                return;
            };
            let bounds = self.as_super().bounds();
            let side = bounds.size.width.min(bounds.size.height).min(20.0);
            let rect = NSRect::new(
                NSPoint::new(
                    bounds.origin.x + (bounds.size.width - side) / 2.0,
                    bounds.origin.y + (bounds.size.height - side) / 2.0,
                ),
                NSSize::new(side, side),
            );
            image.drawInRect(rect);
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<&NSView> {
            None
        }
    }
);

impl MiraStatusView {
    fn with_frame(frame: NSRect, mtm: MainThreadMarker) -> Option<Retained<Self>> {
        unsafe { msg_send![Self::alloc(mtm), initWithFrame: frame] }
    }

    fn set_image(&self, image: Option<Retained<NSImage>>) {
        *self.ivars().image.borrow_mut() = image;
        self.as_super().setNeedsDisplay(true);
    }
}

// ─── MacNativeTrayController ──────────────────────────────────────────────

/// macOS 原生菜单栏控制器。
///
/// 主路径：NSStatusItem + NSImage（来自共享 RGBA 渲染器）
/// Fallback：TauriTrayController（Tauri TrayIcon + 动态/静态 PNG）
pub struct MacNativeTrayController {
    /// 原生 NSStatusItem。None = 尚未初始化或已失败。
    status_item: Option<Retained<NSStatusItem>>,
    /// 菜单动作委托
    delegate: Option<Retained<MiraStatusItemDelegate>>,
    /// 挂在 NSStatusBarButton 内的自定义绘图 view。
    status_view: Option<Retained<MiraStatusView>>,
    /// 上次渲染的缓存 key，用于 diff
    last_cache_key: Option<TrayIconCacheKey>,
    /// 上次 tooltip 文本，用于 diff
    last_tooltip: Option<String>,
    /// 上次菜单签名，用于 diff
    last_menu_signature: Option<MenuSignature>,
    /// 初始化是否失败。失败后永远使用 fallback。
    failed: bool,
    /// Tauri tray fallback 控制器
    fallback: TauriTrayController,
}

// SAFETY: NSStatusItem 和 MiraStatusItemDelegate 仅在主线程上创建和访问
// （try_init 和 update_native 均通过 MainThreadMarker 守卫）。
// Retained<T> 本身是线程安全的引用计数指针，跨线程传递指针本身不会引发数据竞争，
// 只要使用始终发生在主线程上即可保证安全。
unsafe impl Send for MacNativeTrayController {}

/// 菜单签名：用于判断是否需要重建 NSMenu
#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuSignature {
    connected: bool,
    mouse_battery: Option<u8>,
    mouse_charging: bool,
    receiver_battery: Option<u8>,
    receiver_charging: bool,
    show_receiver: bool,
    device_name: Option<String>,
}

impl MenuSignature {
    fn from_state(state: &TrayStatusState) -> Self {
        MenuSignature {
            connected: state.connected,
            mouse_battery: state.mouse_battery,
            mouse_charging: state.mouse_charging,
            receiver_battery: state.receiver_battery,
            receiver_charging: state.receiver_charging,
            show_receiver: state.show_receiver,
            device_name: state.device_name.clone(),
        }
    }
}

impl Default for MacNativeTrayController {
    fn default() -> Self {
        Self {
            status_item: None,
            delegate: None,
            status_view: None,
            last_cache_key: None,
            last_tooltip: None,
            last_menu_signature: None,
            failed: false,
            fallback: TauriTrayController::new(),
        }
    }
}

impl MacNativeTrayController {
    /// 尝试创建 NSStatusItem。成功后隐藏 Tauri tray。
    fn try_init(&mut self, tray: &tauri::tray::TrayIcon) -> Result<(), Box<dyn std::error::Error>> {
        let mtm = MainThreadMarker::new().ok_or("NSStatusItem must be created on main thread")?;

        // 创建 NSStatusItem
        let status_bar = NSStatusBar::systemStatusBar();
        // NSStatusItemSquareLength = -1.0 → 自动适配菜单栏高度
        let item = status_bar.statusItemWithLength(-1.0);

        // 创建 delegate
        let delegate = MiraStatusItemDelegate::new();
        let view_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(28.0, 22.0));
        let status_view = MiraStatusView::with_frame(view_frame, mtm)
            .ok_or("failed to initialize MiraStatusView")?;

        // 获取 button 并设置 target/action
        if let Some(button) = item.button(mtm) {
            // target/action: 点击按钮 → openWindow:
            // SAFETY: delegate 是有效的 NSObject 子类实例，openWindow: 是有效 selector
            let any_obj: &AnyObject = &delegate;
            unsafe {
                button.setTarget(Some(any_obj));
                button.setAction(Some(objc2::sel!(openWindow:)));
            }
            button
                .as_super()
                .as_super()
                .as_super()
                .addSubview(status_view.as_super());
        }

        item.setLength(28.0);
        self.status_item = Some(item);
        self.delegate = Some(delegate);
        self.status_view = Some(status_view);

        // 隐藏 Tauri tray（避免菜单栏出现两个图标）
        let _ = tray.set_visible(false);

        Ok(())
    }

    /// 从 RGBA 字节创建 NSImage
    fn ns_image_from_rgba(rgba: &[u8], width: f64, height: f64) -> Option<Retained<NSImage>> {
        let png = encode_rgba_png(rgba, 64, 64).ok()?;
        let data = NSData::with_bytes(&png);
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        image.setSize(NSSize::new(width, height));
        Some(image)
    }

    /// 从 PNG 字节创建 NSImage
    fn ns_image_from_png(png: &[u8]) -> Option<Retained<NSImage>> {
        let data = NSData::with_bytes(png);
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        image.setSize(NSSize::new(20.0, 20.0));
        Some(image)
    }

    /// 渲染当前状态的 NSImage
    fn render_image(
        &self,
        state: &TrayStatusState,
        style: &TrayVisualStyle,
    ) -> Option<Retained<NSImage>> {
        if !state.connected {
            // 未连接：使用 app 图标
            let icon_bytes = crate::tray::static_icon::static_tray_app_icon_bytes_for_theme(
                style.theme.is_dark(),
            );
            Self::ns_image_from_png(icon_bytes)
        } else {
            // 已连接：使用共享 RGBA 渲染器（与静态 PNG fallback 视觉一致）
            let rgba = render_mouse_icon_rgba(state, style);
            // 菜单栏高度约 22px，使用 20x20 让图标稍小于菜单栏
            Self::ns_image_from_rgba(&rgba, 20.0, 20.0)
        }
    }

    fn theme_from_appearance(appearance: &NSAppearance) -> Option<TrayTheme> {
        let appearances = NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe {
            NSAppearanceNameDarkAqua
        }]);
        let matched = appearance.bestMatchFromAppearancesWithNames(&appearances)?;
        if matched.isEqualToString(unsafe { NSAppearanceNameDarkAqua }) {
            Some(TrayTheme::Dark)
        } else if matched.isEqualToString(unsafe { NSAppearanceNameAqua }) {
            Some(TrayTheme::Light)
        } else {
            None
        }
    }

    fn effective_status_item_theme(&self, mtm: MainThreadMarker) -> Option<TrayTheme> {
        let item = self.status_item.as_ref()?;
        let button = item.button(mtm)?;
        let appearance = button.effectiveAppearance();
        Self::theme_from_appearance(&appearance)
    }

    /// 构建 NSMenu
    fn build_menu(&self, state: &TrayStatusState, mtm: MainThreadMarker) -> Retained<NSMenu> {
        let lang = current_lang();
        let menu = NSMenu::new(mtm);
        // 禁用自动启用/禁用菜单项（我们手动控制 enabled 状态）
        menu.setAutoenablesItems(false);

        if state.connected {
            // 设备名
            if let Some(name) = &state.device_name {
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str(name));
                item.setEnabled(true);
                menu.addItem(&item);
            }

            // 鼠标电量
            if let Some(pct) = state.mouse_battery {
                let label = if lang == "en" { "Mouse" } else { "鼠标" };
                let text = crate::tr_battery_item(lang, label, pct, state.mouse_charging);
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str(&text));
                if let Some(delegate) = &self.delegate {
                    let any_obj: &AnyObject = delegate;
                    unsafe {
                        item.setTarget(Some(any_obj));
                        item.setAction(Some(objc2::sel!(openBatteryUsage:)));
                    }
                }
                item.setEnabled(true);
                menu.addItem(&item);
            }

            // 接收器电量
            if state.show_receiver {
                if let Some(pct) = state.receiver_battery {
                    let label = if lang == "en" {
                        "Receiver"
                    } else {
                        "接收器"
                    };
                    let text = crate::tr_battery_item(lang, label, pct, state.receiver_charging);
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&NSString::from_str(&text));
                    if let Some(delegate) = &self.delegate {
                        let any_obj: &AnyObject = delegate;
                        unsafe {
                            item.setTarget(Some(any_obj));
                            item.setAction(Some(objc2::sel!(openBatteryUsage:)));
                        }
                    }
                    item.setEnabled(true);
                    menu.addItem(&item);
                }
            }
        } else {
            // 未连接
            let item = NSMenuItem::new(mtm);
            item.setTitle(&NSString::from_str(crate::tr_disconnected(lang)));
            item.setEnabled(true);
            menu.addItem(&item);
        }

        // 分隔线
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        // 打开 Mira
        let open_item = NSMenuItem::new(mtm);
        open_item.setTitle(&NSString::from_str(crate::tr_open(lang)));
        if let Some(delegate) = &self.delegate {
            // SAFETY: delegate 是有效的 NSObject 子类实例，openWindow: 是有效 selector
            let any_obj: &AnyObject = delegate;
            unsafe {
                open_item.setTarget(Some(any_obj));
                open_item.setAction(Some(objc2::sel!(openWindow:)));
            }
        }
        open_item.setEnabled(true);
        menu.addItem(&open_item);

        // 退出 Mira
        let quit_item = NSMenuItem::new(mtm);
        quit_item.setTitle(&NSString::from_str(crate::tr_quit(lang)));
        if let Some(delegate) = &self.delegate {
            // SAFETY: delegate 是有效的 NSObject 子类实例，quitApp: 是有效 selector
            let any_obj: &AnyObject = delegate;
            unsafe {
                quit_item.setTarget(Some(any_obj));
                quit_item.setAction(Some(objc2::sel!(quitApp:)));
            }
        }
        quit_item.setEnabled(true);
        menu.addItem(&quit_item);

        menu
    }

    /// 构建 tooltip 文本
    fn build_tooltip(&self, state: &TrayStatusState) -> String {
        let lang = current_lang();
        if state.connected {
            let name = state.device_name.as_deref().unwrap_or("");
            let conn = state
                .connection
                .map(|c| crate::connection_label(c, lang))
                .unwrap_or("");
            crate::tr_tooltip_connected(lang, conn, name)
        } else {
            crate::tr_tooltip_disconnected(lang)
        }
    }

    /// 更新 NSStatusItem 的图标、菜单和 tooltip
    fn update_native(
        &mut self,
        state: &TrayStatusState,
        style: &TrayVisualStyle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mtm = MainThreadMarker::new().ok_or("update_native must be called on main thread")?;
        let Some(item) = &self.status_item else {
            return Err("NSStatusItem not initialized".into());
        };

        let resolved_style = if style.icon_color_mode == TrayIconColorMode::Auto {
            self.effective_status_item_theme(mtm)
                .map(|theme| style.with_auto_theme(theme))
                .unwrap_or(*style)
        } else {
            *style
        };

        // 1. 图标 diff + 更新
        let cache_key = TrayIconCacheKey::from_state_and_style(state, &resolved_style);
        if self.last_cache_key.as_ref() != Some(&cache_key) {
            let image = self
                .render_image(state, &resolved_style)
                .ok_or("render native tray image failed")?;
            if let Some(view) = &self.status_view {
                view.set_image(Some(image.clone()));
            }
            // Keep the standard NSStatusBarButton image path alive as the
            // visible-system fallback. The custom view still draws via
            // drawRect, but the button image prevents a blank menu bar item if
            // AppKit decides not to composite the subview.
            if let Some(button) = item.button(mtm) {
                button.setImage(Some(&*image));
            }
            self.last_cache_key = Some(cache_key);
        }

        // 2. 菜单 diff + 重建
        let menu_sig = MenuSignature::from_state(state);
        if self.last_menu_signature.as_ref() != Some(&menu_sig) {
            let menu = self.build_menu(state, mtm);
            item.setMenu(Some(&*menu));
            self.last_menu_signature = Some(menu_sig);
        }

        // 3. tooltip diff + 更新
        let tooltip = self.build_tooltip(state);
        if self.last_tooltip.as_deref() != Some(tooltip.as_str()) {
            if let Some(button) = item.button(mtm) {
                let tooltip_ns = NSString::from_str(&tooltip);
                button.setToolTip(Some(&*tooltip_ns));
            }
            self.last_tooltip = Some(tooltip);
        }

        Ok(())
    }
}

impl TrayController for MacNativeTrayController {
    fn update_icon(
        &mut self,
        tray: &tauri::tray::TrayIcon,
        state: &TrayStatusState,
        style: &TrayVisualStyle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if matches!(
            state.render_mode,
            TrayRenderMode::DynamicImage | TrayRenderMode::Static
        ) && self.status_item.is_none()
        {
            return self.fallback.update_icon(tray, state, style);
        }

        // 失败后永远使用 fallback
        if self.failed {
            return self.fallback.update_icon(tray, state, style);
        }

        // 首次调用：尝试创建 NSStatusItem
        if self.status_item.is_none() {
            if let Err(err) = self.try_init(tray) {
                eprintln!("[mira] NSStatusItem init failed, falling back to TauriTray: {err}");
                self.failed = true;
                return self.fallback.update_icon(tray, state, style);
            }
        }

        // 更新原生 NSStatusItem
        match self.update_native(state, style) {
            Ok(()) => Ok(()),
            Err(err) => {
                eprintln!("[mira] NSStatusItem update failed, falling back: {err}");
                self.failed = true;
                // 显示 Tauri tray 作为 fallback
                let _ = tray.set_visible(true);
                self.fallback.update_icon(tray, state, style)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::state::TraySettings;
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

    #[test]
    fn macos_controller_creates_successfully() {
        let _controller = MacNativeTrayController::default();
    }

    #[test]
    fn macos_controller_starts_with_no_status_item() {
        let controller = MacNativeTrayController::default();
        assert!(controller.status_item.is_none());
        assert!(controller.status_view.is_none());
        assert!(!controller.failed);
    }

    #[test]
    fn menu_signature_differs_by_battery() {
        let state1 = make_state(Some(50), false);
        let state2 = make_state(Some(80), false);
        let sig1 = MenuSignature::from_state(&state1);
        let sig2 = MenuSignature::from_state(&state2);
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn menu_signature_differs_by_connected() {
        let connected = make_state(Some(50), false);
        let disconnected = make_disconnected_state();
        let sig1 = MenuSignature::from_state(&connected);
        let sig2 = MenuSignature::from_state(&disconnected);
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn menu_signature_same_for_same_state() {
        let state1 = make_state(Some(50), false);
        let state2 = make_state(Some(50), false);
        let sig1 = MenuSignature::from_state(&state1);
        let sig2 = MenuSignature::from_state(&state2);
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn tooltip_disconnected_correct() {
        let controller = MacNativeTrayController::default();
        let state = make_disconnected_state();
        let tooltip = controller.build_tooltip(&state);
        assert!(tooltip.contains("Mira"));
    }

    #[test]
    fn tooltip_connected_contains_device_name() {
        let controller = MacNativeTrayController::default();
        let state = make_state(Some(75), false);
        let tooltip = controller.build_tooltip(&state);
        assert!(tooltip.contains("Test"));
    }

    #[test]
    fn set_app_lang_updates_global() {
        set_app_lang("en");
        assert_eq!(current_lang(), "en");
        set_app_lang("zh-CN");
        assert_eq!(current_lang(), "zh-CN");
    }
}
