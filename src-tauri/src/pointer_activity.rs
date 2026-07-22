// SPDX-License-Identifier: AGPL-3.0-or-later
//! Permission-free pointer activity hints used by plugin-declared wake recovery.
//!
//! These watchers deliberately do not install event taps, keyboard hooks, or
//! accessibility monitors. They emit activity edges rather than every movement
//! sample, so continuous pointer use can start at most one recovery batch.

use std::{sync::Arc, time::Duration};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const IDLE_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerActivity {
    Active,
    Idle,
}

#[derive(Default)]
struct ActivityEdges {
    active: bool,
    last_activity_at: Option<std::time::Instant>,
}

impl ActivityEdges {
    fn observe(&mut self, activity: bool, now: std::time::Instant) -> Option<PointerActivity> {
        if activity {
            self.last_activity_at = Some(now);
            if !self.active {
                self.active = true;
                return Some(PointerActivity::Active);
            }
            return None;
        }
        if self.active
            && self
                .last_activity_at
                .is_some_and(|last| now.saturating_duration_since(last) >= IDLE_DELAY)
        {
            self.active = false;
            return Some(PointerActivity::Idle);
        }
        None
    }
}

pub fn spawn(notify: impl Fn(PointerActivity) + Send + Sync + 'static) {
    let notify: Arc<dyn Fn(PointerActivity) + Send + Sync> = Arc::new(notify);

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
fn spawn_macos(notify: Arc<dyn Fn(PointerActivity) + Send + Sync>) {
    std::thread::spawn(move || {
        use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};

        let mut edges = ActivityEdges::default();
        loop {
            let age = CGEventSource::seconds_since_last_event_type(
                CGEventSourceStateID::HIDSystemState,
                CGEventType::MouseMoved,
            );
            let recent = age.is_finite() && age <= POLL_INTERVAL.as_secs_f64() * 1.75;
            if let Some(activity) = edges.observe(recent, std::time::Instant::now()) {
                notify(activity);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

#[cfg(target_os = "windows")]
fn spawn_windows(notify: Arc<dyn Fn(PointerActivity) + Send + Sync>) {
    std::thread::spawn(move || {
        use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

        let mut previous: Option<(i32, i32)> = None;
        let mut edges = ActivityEdges::default();
        loop {
            let mut point = POINT { x: 0, y: 0 };
            let mut moved = false;
            if unsafe { GetCursorPos(&mut point) } != 0 {
                let current = (point.x, point.y);
                moved = previous.is_some_and(|value| value != current);
                previous = Some(current);
            }
            if let Some(activity) = edges.observe(moved, std::time::Instant::now()) {
                notify(activity);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

#[cfg(target_os = "linux")]
fn spawn_linux_x11(notify: Arc<dyn Fn(PointerActivity) + Send + Sync>) {
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
        let mut edges = ActivityEdges::default();
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
            let mut moved = false;
            if ok != 0 {
                let current = (root_x, root_y);
                moved = previous.is_some_and(|value| value != current);
                previous = Some(current);
            }
            if let Some(activity) = edges.observe(moved, std::time::Instant::now()) {
                notify(activity);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_motion_emits_one_active_edge_until_idle() {
        let start = std::time::Instant::now();
        let mut edges = ActivityEdges::default();

        assert_eq!(edges.observe(true, start), Some(PointerActivity::Active));
        assert_eq!(
            edges.observe(true, start + Duration::from_millis(200)),
            None
        );
        assert_eq!(
            edges.observe(false, start + Duration::from_millis(900)),
            None
        );
        assert_eq!(
            edges.observe(false, start + Duration::from_millis(1200)),
            Some(PointerActivity::Idle)
        );
        assert_eq!(
            edges.observe(true, start + Duration::from_millis(1400)),
            Some(PointerActivity::Active)
        );
    }
}
