// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::engine::ProtocolPackage;
use hidapi::HidApi;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Usb,
    Wireless,
    Bluetooth,
}

#[derive(Debug, Default, Clone)]
pub struct DeviceReading {
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub batteries: Vec<mira_core::DeviceBattery>,
    pub dpi: Option<u16>,
    pub dpi_stages: Option<Vec<mira_core::DpiStage>>,
    pub polling_rate_hz: Option<u16>,
    pub profile: Option<u8>,
    pub light_color: Option<String>,
    pub capabilities: BTreeMap<String, Value>,
}

pub struct ProtocolContext<'a> {
    pub api: &'a HidApi,
    pub path: &'a str,
    pub family: &'a str,
    pub connection: ConnectionKind,
    pub files: &'a BTreeMap<String, Vec<u8>>,
}

pub fn read_device(ctx: &ProtocolContext) -> Result<DeviceReading, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    let workflow_id = format!("{}-read", ctx.family);
    let outputs = package.execute(ctx.api, ctx.path, &workflow_id)?;
    #[cfg(debug_assertions)]
    eprintln!(
        "[mira] plugin workflow {workflow_id}: {}",
        serde_json::to_string(&outputs).unwrap_or_else(|_| "<serialization failed>".into())
    );
    Ok(standard_reading(outputs))
}

fn standard_reading(outputs: BTreeMap<String, Value>) -> DeviceReading {
    let mut reading = DeviceReading {
        capabilities: outputs.clone(),
        ..DeviceReading::default()
    };

    if let Some(battery) = object(&outputs, "battery") {
        reading.battery_percent =
            number(battery, "percentage").and_then(|value| u8::try_from(value).ok());
        reading.charging = boolean(battery, "charging").unwrap_or(false);
        if let Some(percentage) = reading.battery_percent {
            reading.batteries.push(mira_core::DeviceBattery {
                id: "mouse".into(),
                label: "鼠标".into(),
                percentage,
                charging: reading.charging,
            });
        }
    }

    // Receiver transports expose their status object alongside ordinary workflow
    // outputs. Keeping this normalization in the runtime lets every UI consume the
    // same multi-device battery contract without knowing a brand protocol.
    if let Some(receiver) = object(&outputs, "receiver") {
        if reading.battery_percent.is_none() {
            reading.battery_percent =
                number(receiver, "mouseBattery").and_then(|value| u8::try_from(value).ok());
        }
        if reading.batteries.is_empty() {
            if let Some(percentage) =
                number(receiver, "mouseBattery").and_then(|value| u8::try_from(value).ok())
            {
                reading.batteries.push(mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: "鼠标".into(),
                    percentage,
                    charging: false,
                });
            }
        }
        if let Some(percentage) =
            number(receiver, "receiverBattery").and_then(|value| u8::try_from(value).ok())
        {
            reading.batteries.push(mira_core::DeviceBattery {
                id: "receiver".into(),
                label: "接收器".into(),
                percentage,
                charging: false,
            });
        }
    }

    if let Some(dpi) = object(&outputs, "dpi") {
        reading.profile = number(dpi, "profile").and_then(|value| u8::try_from(value).ok());
        let current = number(dpi, "currentStage").and_then(|value| usize::try_from(value).ok());
        let count = number(dpi, "stageCount")
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0)
            .min(8);
        let values = array(dpi, "dpiX");
        let colors = array(dpi, "stageColors");
        let stages: Vec<_> = (0..count)
            .filter_map(|index| {
                let value = values?
                    .get(index)?
                    .as_u64()
                    .and_then(|value| u16::try_from(value).ok())?;
                let color = colors?.get(index)?.as_str()?.to_string();
                Some(mira_core::DpiStage {
                    value,
                    color,
                    enabled: true,
                    active: current == Some(index + 1),
                })
            })
            .collect();
        reading.dpi = stages
            .iter()
            .find(|stage| stage.active)
            .map(|stage| stage.value);
        if !stages.is_empty() {
            reading.dpi_stages = Some(stages);
        }
    }

    if let Some(settings) = object(&outputs, "settings") {
        reading.profile = number(settings, "profile")
            .and_then(|value| u8::try_from(value).ok())
            .or(reading.profile);
        reading.polling_rate_hz =
            number(settings, "pollingRate").and_then(|value| u16::try_from(value).ok());
    }

    reading.light_color = object(&outputs, "settings")
        .and_then(|settings| settings.get("mouseLightStartColor"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            object(&outputs, "mouseEffect")
                .or_else(|| object(&outputs, "mouseLighting"))
                .or_else(|| object(&outputs, "lighting"))
                .and_then(|lighting| lighting.get("color"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    reading
}

fn object<'a>(
    outputs: &'a BTreeMap<String, Value>,
    key: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    outputs.get(key)?.as_object()
}

fn number(object: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key)?.as_u64()
}

fn boolean(object: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    object.get(key)?.as_bool()
}

fn array<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Vec<Value>> {
    object.get(key)?.as_array()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_standard_capabilities_without_protocol_offsets() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 83, "charging": false}),
            ),
            (
                "dpi".into(),
                json!({
                    "profile": 0,
                    "currentStage": 2,
                    "stageCount": 2,
                    "dpiX": [400, 800],
                    "stageColors": ["#112233", "#445566"]
                }),
            ),
            (
                "settings".into(),
                json!({"profile": 0, "pollingRate": 1000}),
            ),
            ("mouseEffect".into(), json!({"color": "#AABBCC"})),
        ]);
        let reading = standard_reading(outputs);
        assert_eq!(reading.battery_percent, Some(83));
        assert_eq!(reading.batteries.len(), 1);
        assert_eq!(reading.dpi, Some(800));
        assert_eq!(reading.polling_rate_hz, Some(1000));
        assert_eq!(reading.light_color.as_deref(), Some("#AABBCC"));
        assert_eq!(reading.capabilities.len(), 4);
    }

    #[test]
    fn normalizes_receiver_and_mouse_batteries() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 76, "charging": false}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 75, "receiverBattery": 100}),
            ),
        ]);
        let reading = standard_reading(outputs);
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[0].label, "鼠标");
        assert_eq!(reading.batteries[1].label, "接收器");
        assert_eq!(reading.batteries[1].percentage, 100);
    }

    #[test]
    fn prefers_mouse_settings_color_over_receiver_lighting() {
        let outputs = BTreeMap::from([
            (
                "settings".into(),
                json!({"mouseLightStartColor": "#FB223C"}),
            ),
            ("receiverLighting".into(), json!({"color": "#4BBFB1"})),
        ]);
        let reading = standard_reading(outputs);
        assert_eq!(reading.light_color.as_deref(), Some("#FB223C"));
    }
}
