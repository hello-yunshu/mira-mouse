// SPDX-License-Identifier: AGPL-3.0-or-later

use std::time::{Duration, Instant};

use super::{
    state::TrayStatusState,
    style::{BatterySeverity, TrayVisualStyle},
};

pub const HIGHLIGHT_DURATION: Duration = Duration::from_millis(420);

#[derive(Clone)]
pub struct TrayIconFrame {
    pub state: TrayStatusState,
    pub style: TrayVisualStyle,
    #[cfg(target_os = "windows")]
    pub show_battery_icon: bool,
}

#[derive(Default)]
pub struct BatteryAlert {
    device: String,
    threshold: u8,
    latched: Option<BatterySeverity>,
    highlight: Option<(Instant, BatterySeverity, TrayIconFrame)>,
}

impl BatteryAlert {
    /// Keep the highest active level until the reading clears its boundary by 2 points.
    /// A first reading, device switch, or settings change establishes a baseline.
    pub fn observe(
        &mut self,
        device: &str,
        frame: &mut TrayIconFrame,
        allow_animation: bool,
    ) -> Option<Instant> {
        frame.style.alert_highlight = false;
        let state = &frame.state;
        let threshold = state.low_battery_threshold;
        if !state.connected || self.device != device || self.threshold != threshold {
            self.device = device.to_owned();
            self.threshold = threshold;
            self.latched = None;
            self.highlight = None;
        }

        let Some(percentage) = state.mouse_battery else {
            self.highlight = None;
            return None;
        };
        let severity = BatterySeverity::for_percentage(percentage, threshold);
        let previous = self.latched.replace(severity);
        if state.mouse_charging {
            self.highlight = None;
            return None;
        }

        if let Some(previous) = previous {
            let recovery = BatterySeverity::for_percentage(percentage.saturating_sub(1), threshold);
            self.latched = Some(severity.max(previous.min(recovery)));
        }
        if !allow_animation {
            self.highlight = None;
            return None;
        }

        let start = previous
            .filter(|previous| severity > *previous)
            .map(|_| Instant::now());
        let active = start.or_else(|| {
            self.highlight
                .as_ref()
                .filter(|(_, active_severity, _)| *active_severity == severity)
                .map(|(started, _, _)| *started)
        });
        self.highlight = active.map(|started| (started, severity, frame.clone()));
        frame.style.alert_highlight = active.is_some();
        start
    }

    /// A completed timer may only restore the current highlight, never a newer one.
    pub fn finish(&mut self, started: Instant) -> Option<TrayIconFrame> {
        if self.highlight.as_ref()?.0 != started {
            return None;
        }
        self.highlight.take().map(|(_, _, frame)| frame)
    }
}

pub fn animations_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        !objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
        };
        let mut enabled = 0i32;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to an initialized, writable pointer.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                std::ptr::from_mut(&mut enabled).cast(),
                0,
            )
        };
        ok != 0 && enabled != 0
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::GtkSettingsExt;
        gtk::is_initialized_main_thread()
            && gtk::Settings::default().is_some_and(|settings| settings.is_gtk_enable_animations())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::{
        state::TraySettings,
        style::{TrayTheme, TrayVisualStyle},
    };

    fn frame(percentage: u8, threshold: u8) -> TrayIconFrame {
        let settings = TraySettings {
            show_receiver: false,
            show_connection: false,
            show_battery_title: false,
            low_battery_threshold: threshold,
            tray_icon_color: "auto",
            tray_render_mode: "auto",
        };
        let mut state = TrayStatusState::from_snapshot(None, &settings);
        state.connected = true;
        state.mouse_battery = Some(percentage);
        TrayIconFrame {
            state,
            style: TrayVisualStyle::from_settings(&settings, TrayTheme::Dark),
            #[cfg(target_os = "windows")]
            show_battery_icon: true,
        }
    }

    #[test]
    fn highlights_only_when_entering_a_more_urgent_level() {
        let mut alert = BatteryAlert::default();
        assert!(alert.observe("mouse", &mut frame(24, 20), true).is_none());
        let mut warning = frame(20, 20);
        let warning_start = alert.observe("mouse", &mut warning, true).unwrap();
        assert!(warning.style.alert_highlight);

        let mut latest_warning = frame(18, 20);
        assert!(alert.observe("mouse", &mut latest_warning, true).is_none());
        assert!(latest_warning.style.alert_highlight);
        let restored = alert.finish(warning_start).unwrap();
        assert_eq!(restored.state.mouse_battery, Some(18));
        assert!(!restored.style.alert_highlight);
        #[cfg(target_os = "windows")]
        assert!(restored.show_battery_icon);

        assert!(alert.observe("mouse", &mut frame(21, 20), true).is_none());
        assert!(alert.observe("mouse", &mut frame(20, 20), true).is_none());
        let mut critical = frame(8, 20);
        assert!(alert.observe("mouse", &mut critical, true).is_some());
        assert!(critical.style.alert_highlight);
        assert!(alert.finish(warning_start).is_none());
    }

    #[test]
    fn charging_settings_changes_and_reduced_motion_leave_a_steady_icon() {
        let mut alert = BatteryAlert::default();
        assert!(alert.observe("mouse", &mut frame(24, 20), true).is_none());
        assert!(alert.observe("mouse", &mut frame(24, 30), true).is_none());
        let mut critical = frame(8, 30);
        assert!(alert.observe("mouse", &mut critical, false).is_none());
        assert!(!critical.style.alert_highlight);
        assert!(alert.observe("other", &mut frame(8, 30), true).is_none());
        assert!(alert.observe("other", &mut frame(33, 30), true).is_none());
        let started = alert.observe("other", &mut frame(29, 30), true).unwrap();
        let mut charging = frame(29, 30);
        charging.state.mouse_charging = true;
        assert!(alert.observe("other", &mut charging, true).is_none());
        assert!(!charging.style.alert_highlight);
        assert!(alert.finish(started).is_none());
    }
}
