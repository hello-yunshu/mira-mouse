// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceSnapshot {
    pub display_name: String,
    pub connection: Connection,
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub dpi: Option<u16>,
    pub polling_rate_hz: Option<u16>,
    pub profile: Option<String>,
    pub confirmed_light_color: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Connection {
    Usb,
    Wireless,
    Bluetooth,
    Virtual,
}

#[derive(Debug, Default)]
pub struct LowBatteryCrossing {
    below: bool,
}

impl LowBatteryCrossing {
    pub fn update(&mut self, value: Option<u8>, threshold: u8) -> bool {
        let now = value.is_some_and(|v| v <= threshold);
        let notify = now && !self.below;
        self.below = now;
        notify
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_battery_only_notifies_on_crossing() {
        let mut crossing = LowBatteryCrossing::default();
        assert!(crossing.update(Some(20), 20));
        assert!(!crossing.update(Some(19), 20));
        assert!(!crossing.update(Some(50), 20));
        assert!(crossing.update(Some(20), 20));
    }
}
