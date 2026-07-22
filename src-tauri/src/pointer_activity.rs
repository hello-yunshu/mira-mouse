// SPDX-License-Identifier: AGPL-3.0-or-later
//! Permission-free pointer activity hints used by plugin-declared wake recovery.
//!
//! These watchers deliberately do not install event taps, keyboard hooks, or
//! accessibility monitors. They only report that a system pointer moved; the
//! plugin contract decides whether that hint is relevant, and the Host applies
//! bounded retries before touching a sleeping device.

use std::{sync::Arc, time::Duration};

const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub fn spawn(notify: impl Fn() + Send + Sync + 'static) {
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(notify);

    #[cfg(target_os = "macos")]
    spawn_macos(notify);

    #[cfg(target_os = "windows")]
    spawn_windows(notify);

    #[cfg(target_os = "linux")]
    spawn_linux_x11(notify);
}

/// Native Wayland does not expose a permission-free global pointer position.
/// When XWayland is available, the X11 watcher remains preferred; otherwise a
/// window focus event is the least invasive recovery hint available.
pub fn focus_fallback_needed() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none()
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
fn spawn_macos(notify: Arc<dyn Fn() + Send + Sync>) {
    std::thread::spawn(move || {
        use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};

        loop {
            let age = CGEventSource::seconds_since_last_event_type(
                CGEventSourceStateID::HIDSystemState,
                CGEventType::MouseMoved,
            );
            let recent = age.is_finite() && age <= POLL_INTERVAL.as_secs_f64() * 1.75;
            if recent {
                notify();
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

#[cfg(target_os = "windows")]
fn spawn_windows(notify: Arc<dyn Fn() + Send + Sync>) {
    std::thread::spawn(move || {
        use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

        let mut previous: Option<(i32, i32)> = None;
        loop {
            let mut point = POINT { x: 0, y: 0 };
            if unsafe { GetCursorPos(&mut point) } != 0 {
                let current = (point.x, point.y);
                if previous.is_some_and(|value| value != current) {
                    notify();
                }
                previous = Some(current);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

#[cfg(target_os = "linux")]
fn spawn_linux_x11(notify: Arc<dyn Fn() + Send + Sync>) {
    std::thread::spawn(move || {
        use std::{os::raw::c_uint, ptr};
        use x11_dl::xlib::Xlib;

        let Ok(xlib) = Xlib::open() else {
            return;
        };
        let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
        if display.is_null() {
            // Native Wayland intentionally has no permission-free global
            // pointer stream. Window focus remains the recovery fallback.
            return;
        }
        let root = unsafe { (xlib.XDefaultRootWindow)(display) };
        let mut previous: Option<(i32, i32)> = None;
        loop {
            let mut root_return = 0;
            let mut child_return = 0;
            let mut root_x = 0;
            let mut root_y = 0;
            let mut win_x = 0;
            let mut win_y = 0;
            let mut mask: c_uint = 0;
            let ok = unsafe {
                (xlib.XQueryPointer)(
                    display,
                    root,
                    &mut root_return,
                    &mut child_return,
                    &mut root_x,
                    &mut root_y,
                    &mut win_x,
                    &mut win_y,
                    &mut mask,
                )
            };
            if ok != 0 {
                let current = (root_x, root_y);
                if previous.is_some_and(|value| value != current) {
                    notify();
                }
                previous = Some(current);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}
