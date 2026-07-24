// SPDX-License-Identifier: AGPL-3.0-or-later
//! 屏幕解锁事件监听器：用户回到桌面时主动唤醒鼠标。
//!
//! 与电源唤醒监听器（`spawn_power_watcher`）互补：
//! - 电源唤醒：系统从睡眠恢复，设备可能尚未就绪，只发 PresenceOnly 枚举。
//! - 屏幕解锁：用户明确回到桌面，设备通常已 ready，触发 Quick 主动读取。
//!
//! watcher 常驻进程，`handle_system_unlock_state` 内部检查 `wake_on_unlock`，
//! 设置关闭时直接返回，便于用户实时切换而无需重启应用。
//!
//! ## 平台实现
//!
//! - **macOS**：CoreFoundation `CFNotificationCenterGetDistributedCenter`
//!   监听 `com.apple.screenIsUnlocked`（由 `loginwindow` 进程发出，无需特殊权限）。
//!   在专用线程上运行 CFRunLoop 接收回调。
//! - **Windows**：创建隐藏顶级窗口（非 message-only，因后者不接收 WTS 通知），
//!   调用 `WTSRegisterSessionNotification` 注册接收 `WM_WTSSESSION_CHANGE`，
//!   处理 `WTS_SESSION_UNLOCK`。
//! - **Linux**：`gdbus monitor --system --dest org.freedesktop.login1` 监听
//!   `UnlockSession` 信号（与现有 PrepareForSleep 监听风格一致）。

use tauri::AppHandle;

use crate::handle_system_unlock_state;

/// 启动屏幕解锁事件监听器。
///
/// 在 setup 闭包中调用，与 `spawn_power_watcher` 平行。事件触发后调用
/// `handle_system_unlock_state`，由后者统一处理缓存清空与 Quick 读取。
pub fn spawn(app: AppHandle) {
    #[cfg(target_os = "macos")]
    spawn_macos(app);

    #[cfg(target_os = "windows")]
    spawn_windows(app);

    #[cfg(target_os = "linux")]
    spawn_linux(app);
}

// ─── macOS: CFNotificationCenter (Distributed) ─────────────────────────────
//
// `com.apple.screenIsUnlocked` 由 loginwindow 在用户成功输入密码回到桌面时
// 通过 NSDistributedNotificationCenter 广播。CoreFoundation 的
// `CFNotificationCenterGetDistributedCenter` 是其 C 接口，无需 Objective-C 运行时。
//
// 实现模式与 `install_macos_theme_watcher` 一致：
// - 在 setup 闭包（主线程）注册 observer，复用 Tauri/NSApplication 主 runloop
// - AppHandle 通过 Box::leak 泄漏到静态内存（observer 生命周期与进程相同）
// - FFI 签名与 `install_macos_theme_watcher` 完全一致，避免重复声明冲突
#[cfg(target_os = "macos")]
fn spawn_macos(app: AppHandle) {
    use std::ffi::c_void;
    use std::ptr;

    type CFNotificationCenterRef = *mut c_void;
    type CFStringRef = *const c_void;
    type CFNotificationCallback = extern "C" fn(
        center: CFNotificationCenterRef,
        observer: *mut c_void,
        name: CFStringRef,
        object: *const c_void,
        user_info: *const c_void,
    );

    extern "C" {
        fn CFNotificationCenterGetDistributedCenter() -> CFNotificationCenterRef;
        fn CFNotificationCenterAddObserver(
            center: CFNotificationCenterRef,
            observer: *const c_void,
            call_back: CFNotificationCallback,
            name: CFStringRef,
            object: *const c_void,
            suspension_behavior: u32,
        );
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const u8,
            encoding: u32,
        ) -> CFStringRef;
    }

    // kCFStringEncodingUTF8 = 0x08000100
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    // CFNotificationSuspensionBehaviorDeliverImmediately = 2
    // 解锁事件不延迟投递，确保第一时间触发唤醒。
    const DELIVER_IMMEDIATELY: u32 = 2;

    extern "C" fn unlock_callback(
        _center: CFNotificationCenterRef,
        observer: *mut c_void,
        _name: CFStringRef,
        _object: *const c_void,
        _user_info: *const c_void,
    ) {
        // observer 指向 Box::leak 的 AppHandle（生命周期与进程相同）
        let app: &AppHandle = unsafe { &*(observer as *const AppHandle) };
        handle_system_unlock_state(app);
    }

    // AppHandle 泄漏到静态内存：observer 生命周期需与进程一样长。
    // 应用退出时进程终止，内存自动回收。
    let app_box: &'static AppHandle = Box::leak(Box::new(app));
    let observer = app_box as *const AppHandle as *const c_void;

    let name = unsafe {
        CFStringCreateWithCString(
            ptr::null(),
            c"com.apple.screenIsUnlocked".as_ptr() as *const u8,
            K_CF_STRING_ENCODING_UTF8,
        )
    };

    let center = unsafe { CFNotificationCenterGetDistributedCenter() };
    unsafe {
        CFNotificationCenterAddObserver(
            center,
            observer,
            unlock_callback,
            name,
            ptr::null(),
            DELIVER_IMMEDIATELY,
        );
    }
}

// ─── Windows: WTSRegisterSessionNotification ───────────────────────────────
//
// `WTSRegisterSessionNotification` 注册窗口接收 `WM_WTSSESSION_CHANGE`：
// - `WTS_SESSION_LOCK`（0x7）：会话已锁定
// - `WTS_SESSION_UNLOCK`（0x8）：会话已解锁 ← 我们关心的
//
// 实现模式与 `spawn_windows_power_watcher` 一致：创建隐藏的顶级窗口
// （WS_EX_TOOLWINDOW | WS_POPUP），在 wndproc 中处理消息。
// message-only 窗口（HWND_MESSAGE 父窗口）不接收 WTS 通知，必须用顶级窗口。
#[cfg(target_os = "windows")]
fn spawn_windows(app: AppHandle) {
    use std::ffi::c_void;
    use std::ptr;

    type HWND = *mut c_void;
    type HINSTANCE = *mut c_void;
    type HMENU = *mut c_void;
    type LPVOID = *mut c_void;
    type WPARAM = usize;
    type LPARAM = isize;
    type LRESULT = isize;
    type HBRUSH = *mut c_void;
    type HCURSOR = *mut c_void;
    type HICON = *mut c_void;
    type ATOM = u16;
    type LPCWSTR = *const u16;

    #[repr(C)]
    struct WndClassW {
        style: u32,
        lpfn_wnd_proc: Option<extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: HINSTANCE,
        h_icon: HICON,
        h_cursor: HCURSOR,
        h_brush_background: HBRUSH,
        lpsz_menu_name: LPCWSTR,
        lpsz_class_name: LPCWSTR,
    }

    #[repr(C)]
    struct Msg {
        hwnd: HWND,
        message: u32,
        w_param: WPARAM,
        l_param: LPARAM,
        time: u32,
        pt_x: i32,
        pt_y: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetModuleHandleW(lp_module_name: LPCWSTR) -> HINSTANCE;
        fn RegisterClassW(lp_wnd_class: *const WndClassW) -> ATOM;
        fn CreateWindowExW(
            dw_ex_style: u32,
            lp_class_name: LPCWSTR,
            lp_window_name: LPCWSTR,
            dw_style: u32,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: HWND,
            h_menu: HMENU,
            h_instance: HINSTANCE,
            lp_param: LPVOID,
        ) -> HWND;
        fn DefWindowProcW(hwnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
        fn GetMessageW(
            lp_msg: *mut Msg,
            hwnd: HWND,
            w_msg_filter_min: u32,
            w_msg_filter_max: u32,
        ) -> i32;
        fn TranslateMessage(lp_msg: *const Msg) -> i32;
        fn DispatchMessageW(lp_msg: *const Msg) -> LRESULT;
    }

    #[link(name = "wtsapi32")]
    extern "system" {
        fn WTSRegisterSessionNotification(hwnd: HWND, flags: u32) -> i32;
        fn WTSUnRegisterSessionNotification(hwnd: HWND) -> i32;
    }

    const WM_WTSSESSION_CHANGE: u32 = 0x02A1;
    const WTS_SESSION_UNLOCK: usize = 0x8;
    // NOTIFY_FOR_THIS_SESSION = 0：只接收当前会话的事件。
    const NOTIFY_FOR_THIS_SESSION: u32 = 0;
    // WS_EX_TOOLWINDOW：不在任务栏显示，不出现在 Alt+Tab。
    // WS_POPUP：无装饰的弹出式窗口；不可见时无任何视觉表现。
    const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    const WS_POPUP: u32 = 0x8000_0000;

    // 全局 AppHandle，窗口过程通过此句柄访问 SessionState。
    static UNLOCK_APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

    extern "system" fn unlock_wnd_proc(
        hwnd: HWND,
        msg: u32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if msg == WM_WTSSESSION_CHANGE && w_param == WTS_SESSION_UNLOCK {
            if let Some(app) = UNLOCK_APP_HANDLE.get() {
                handle_system_unlock_state(app);
            }
        }
        unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) }
    }

    std::thread::spawn(move || {
        let _ = UNLOCK_APP_HANDLE.set(app);
        let class_name: Vec<u16> = "MiraUnlockWatcher\0".encode_utf16().collect();
        let h_instance = unsafe { GetModuleHandleW(ptr::null()) };
        let wnd_class = WndClassW {
            style: 0,
            lpfn_wnd_proc: Some(unlock_wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance,
            h_icon: ptr::null_mut(),
            h_cursor: ptr::null_mut(),
            h_brush_background: ptr::null_mut(),
            lpsz_menu_name: ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
        };
        if unsafe { RegisterClassW(&wnd_class) } == 0 {
            return;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                ptr::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                h_instance,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return;
        }
        // 注册接收会话变化通知。失败时窗口仍运行但收不到事件。
        if unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } == 0 {
            // 注册失败：销毁窗口退出。睡眠恢复路径仍兜底。
            return;
        }
        let mut msg = Msg {
            hwnd: ptr::null_mut(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt_x: 0,
            pt_y: 0,
        };
        loop {
            // GetMessageW 阻塞直到有消息；返回 0 表示 WM_QUIT，负值表示错误。
            let ret = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
            if ret <= 0 {
                break;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        unsafe { WTSUnRegisterSessionNotification(hwnd) };
    });
}

// ─── Linux: gdbus monitor logind UnlockSession ─────────────────────────────
//
// `org.freedesktop.login1.Manager` 的 `UnlockSession` 信号在会话解锁时发出。
// 与现有 `spawn_linux_power_watcher` 监听 `PrepareForSleep` 的实现风格一致：
// 用 `gdbus monitor` 子进程持续输出信号，按行匹配。
#[cfg(target_os = "linux")]
fn spawn_linux(app: AppHandle) {
    use std::io::BufRead;
    use std::process::{Command, Stdio};

    std::thread::spawn(move || loop {
        // `gdbus monitor` 持续输出 logind 信号，每行一次。
        // 与现有 PrepareForSleep 监听共用同一 bus，无需额外 D-Bus 依赖。
        let result = Command::new("gdbus")
            .args([
                "monitor",
                "--system",
                "--dest",
                "org.freedesktop.login1",
                "--object-path",
                "/org/freedesktop/login1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();

        let Ok(mut child) = result else {
            // gdbus 不可用（非 GLib 桌面），等待后重试。
            // 睡眠恢复路径仍作为 fallback 兜底。
            std::thread::sleep(std::time::Duration::from_secs(30));
            continue;
        };

        if let Some(stdout) = child.stdout.take() {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(text) = line else {
                    break;
                };
                // 信号名匹配：gdbus monitor 输出形如
                // "The name :1.42 is owned by org.freedesktop.login1"
                // "/org/freedesktop/login1: signal ... UnlockSession ..."
                if text.contains("UnlockSession") {
                    handle_system_unlock_state(&app);
                }
            }
        }

        let _ = child.wait();
        // gdbus 进程意外退出，等待后重启。
        std::thread::sleep(std::time::Duration::from_secs(10));
    });
}
