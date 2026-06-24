// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::engine::ProtocolPackage;
use hidapi::HidApi;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Usb,
    Wireless,
    Bluetooth,
}

#[derive(Debug, Default, Clone)]
pub struct DeviceReading {
    pub display_name: Option<String>,
    pub connection: Option<ConnectionKind>,
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub batteries: Vec<mira_core::DeviceBattery>,
    pub dpi: Option<u16>,
    pub dpi_stages: Option<Vec<mira_core::DpiStage>>,
    pub polling_rate_hz: Option<u16>,
    pub supported_polling_rates_hz: Option<Vec<u16>>,
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
    pub outputs: BTreeMap<String, Value>,
}

pub fn read_device(ctx: &ProtocolContext) -> Result<DeviceReading, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    read_device_with_package(&package, ctx)
}

/// Like `read_device` but reuses a pre-parsed `ProtocolPackage` to avoid
/// re-parsing the JSON files on every call.
pub fn read_device_with_package(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
) -> Result<DeviceReading, String> {
    let workflow_id = format!("{}-read", ctx.family);
    let outputs = package.execute(ctx.api, ctx.path, &workflow_id)?;
    #[cfg(debug_assertions)]
    eprintln!(
        "[mira] plugin workflow {workflow_id}: {}",
        serde_json::to_string(&outputs).unwrap_or_else(|_| "<serialization failed>".into())
    );
    let capabilities = package.capabilities().cloned();
    Ok(standard_reading(outputs, capabilities))
}

pub fn execute_plugin_workflow(
    ctx: &ProtocolContext,
    workflow_id: &str,
) -> Result<BTreeMap<String, Value>, String> {
    ProtocolPackage::from_files(ctx.files)?.execute(ctx.api, ctx.path, workflow_id)
}

pub fn writable_mutations(ctx: &ProtocolContext) -> Result<Vec<String>, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    Ok(package.mutation_ids(ctx.family, Some(&ctx.outputs)))
}

/// Like `writable_mutations` but reuses a pre-parsed `ProtocolPackage`.
pub fn writable_mutations_with_package(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
) -> Result<Vec<String>, String> {
    Ok(package.mutation_ids(ctx.family, Some(&ctx.outputs)))
}

pub fn mutate_device(
    ctx: &ProtocolContext,
    mutation: &str,
    params: &Map<String, Value>,
) -> Result<Value, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    mutate_device_with_package(&package, ctx, mutation, params)
}

/// Like `mutate_device` but reuses a pre-parsed `ProtocolPackage`.
pub fn mutate_device_with_package(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
    mutation: &str,
    params: &Map<String, Value>,
) -> Result<Value, String> {
    let mutation_id = format!("{}-{mutation}", ctx.family);
    package.mutate(ctx.api, ctx.path, &mutation_id, params, &ctx.outputs)
}

fn standard_reading(
    outputs: BTreeMap<String, Value>,
    capabilities: Option<Value>,
) -> DeviceReading {
    let mut reading = DeviceReading {
        capabilities: outputs.clone(),
        ..DeviceReading::default()
    };

    // Prefer device-reported rates from the protocol; fall back to the static
    // plugin manifest so the UI always receives a supported list.
    if let Some(rates) = object(&outputs, "reportRateList")
        .or_else(|| object(&outputs, "reportRateListExtended"))
        .and_then(|value| value.get("supportedRates"))
        .and_then(Value::as_array)
    {
        let rates: Vec<u16> = rates
            .iter()
            .filter_map(|value| value.as_u64().and_then(|rate| u16::try_from(rate).ok()))
            .collect();
        if !rates.is_empty() {
            reading.supported_polling_rates_hz = Some(rates);
        }
    }

    if reading.supported_polling_rates_hz.is_none() {
        if let Some(caps) = capabilities.as_ref().and_then(Value::as_object) {
            if let Some(rates) = caps.get("pollingRatesHz").and_then(Value::as_array) {
                let rates: Vec<u16> = rates
                    .iter()
                    .filter_map(|value| value.as_u64().and_then(|rate| u16::try_from(rate).ok()))
                    .collect();
                if !rates.is_empty() {
                    reading.supported_polling_rates_hz = Some(rates);
                }
            }
        }
    }

    reading.display_name = object(&outputs, "deviceName")
        .and_then(|device| device.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    reading.connection = object(&outputs, "device")
        .or_else(|| object(&outputs, "featureIndexDeviceInfo"))
        .and_then(|device| device.get("connection"))
        .and_then(Value::as_str)
        .and_then(|connection| match connection {
            "usb" => Some(ConnectionKind::Usb),
            "wireless" | "wireless-receiver" => Some(ConnectionKind::Wireless),
            "bluetooth" => Some(ConnectionKind::Bluetooth),
            _ => None,
        });

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

    reading.profile = crate::onboard_profiles::active_profile_index(&outputs);

    // If the plugin already emitted a structured "profile" capability, keep it.
    // Otherwise, when 0x8101 Profile Management outputs are present, normalize
    // them into a single capability object so the UI does not need to know the
    // exact workflow output names.
    if object(&outputs, "profile").is_none()
        && (crate::onboard_profiles::profile_count(&outputs).is_some()
            || crate::onboard_profiles::profile_management_info(&outputs).is_some())
    {
        let mut profile = serde_json::Map::new();
        if let Some(current) = reading.profile {
            profile.insert("current".into(), json!(current));
        }
        if let Some(count) = crate::onboard_profiles::profile_count(&outputs) {
            profile.insert("count".into(), json!(count));
        }
        if let Some(info) = crate::onboard_profiles::profile_management_info(&outputs) {
            profile.insert(
                "management".to_string(),
                json!({
                    "featureVersion": info.feature_version,
                    "maxProfileCount": info.max_profile_count,
                    "profileNameLength": info.profile_name_length,
                }),
            );
        }
        reading
            .capabilities
            .insert("profile".into(), Value::Object(profile));
    }

    if let Some(dpi) = object(&outputs, "dpi").or_else(|| object(&outputs, "dpiExtended")) {
        let current = number(dpi, "currentStage").and_then(|value| usize::try_from(value).ok());
        let values = array(dpi, "dpiX");
        let colors = array(dpi, "stageColors");
        if let Some(values) = values {
            // Array-based DPI stages (e.g. AMaster protocol A).
            let count = number(dpi, "stageCount")
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(values.len())
                .min(8);
            let stages: Vec<_> = (0..count)
                .filter_map(|index| {
                    let value = values
                        .get(index)?
                        .as_u64()
                        .and_then(|value| u16::try_from(value).ok())?;
                    let color = colors
                        .and_then(|colors| colors.get(index)?.as_str())
                        .unwrap_or("#9a8bd0")
                        .to_string();
                    Some(mira_core::DpiStage {
                        value,
                        color,
                        enabled: true,
                        active: current.map(|c| c == index + 1).unwrap_or(index == 0),
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
        } else if let Some(value) = number(dpi, "dpiValue") {
            // Single-value DPI (e.g. HID++ 2.0 AdjustableDPI). The DSL parser
            // returns one DPI value for the active stage; expose it as a single
            // stage so the UI can render and edit it without a stage list.
            if let Ok(value) = u16::try_from(value) {
                reading.dpi = Some(value);
                reading.dpi_stages = Some(vec![mira_core::DpiStage {
                    value,
                    color: "#9a8bd0".into(),
                    enabled: true,
                    active: true,
                }]);
            }
        }
    }

    if let Some(settings) =
        object(&outputs, "settings").or_else(|| object(&outputs, "settingsExtended"))
    {
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
        let reading = standard_reading(outputs, None);
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
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[0].label, "鼠标");
        assert_eq!(reading.batteries[1].label, "接收器");
        assert_eq!(reading.batteries[1].percentage, 100);
    }

    #[test]
    fn normalizes_plugin_reported_identity() {
        let outputs = BTreeMap::from([
            (
                "device".into(),
                json!({"deviceIndex": 1, "connection": "wireless"}),
            ),
            ("deviceName".into(), json!({"name": "G705 Mouse"})),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.display_name.as_deref(), Some("G705 Mouse"));
        assert_eq!(reading.connection, Some(ConnectionKind::Wireless));
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
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.light_color.as_deref(), Some("#FB223C"));
    }

    #[test]
    fn single_value_dpi_produces_one_active_stage() {
        // HID++ 2.0 AdjustableDPI returns one DPI value for the active stage.
        // The runtime should expose it as a single-stage list so the UI can
        // render and edit it without a full stage array.
        let outputs = BTreeMap::from([("dpi".into(), json!({"dpiValue": 1600, "stageIndex": 0}))]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.dpi, Some(1600));
        let stages = reading.dpi_stages.expect("dpi stages");
        assert_eq!(stages.len(), 1);
        assert!(stages[0].active);
        assert_eq!(stages[0].value, 1600);
    }

    #[test]
    fn dpi_array_falls_back_to_default_color_when_missing() {
        // Plugins that don't expose per-stage colors should still produce
        // usable stages — the UI replaces the placeholder color later.
        let outputs =
            BTreeMap::from([("dpi".into(), json!({"stageCount": 2, "dpiX": [400, 800]}))]);
        let reading = standard_reading(outputs, None);
        let stages = reading.dpi_stages.expect("dpi stages");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].color, "#9a8bd0");
    }

    #[test]
    fn reads_supported_polling_rates_from_report_rate_list() {
        // rateListFlags = 0b00001011 means 1 ms (1000), 2 ms (500), and 8 ms (125) are supported.
        let outputs = BTreeMap::from([
            (
                "reportRateList".into(),
                json!({"rateListFlags": 0x0B, "supportedRates": [1000, 500, 125]}),
            ),
            ("settings".into(), json!({"pollingRate": 500})),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.polling_rate_hz, Some(500));
        assert_eq!(
            reading.supported_polling_rates_hz,
            Some(vec![1000, 500, 125])
        );
    }

    #[test]
    fn reads_extended_hidpp_dpi_and_polling_rate() {
        let outputs = BTreeMap::from([
            (
                "dpiExtended".into(),
                json!({"dpiValue": 2400, "sensorIndex": 0}),
            ),
            ("settingsExtended".into(), json!({"pollingRate": 8000})),
            (
                "reportRateListExtended".into(),
                json!({"rateListFlags": 0x0078, "supportedRates": [1000, 2000, 4000, 8000]}),
            ),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.dpi, Some(2400));
        assert_eq!(reading.polling_rate_hz, Some(8000));
        assert_eq!(
            reading.supported_polling_rates_hz,
            Some(vec![1000, 2000, 4000, 8000])
        );
    }

    #[test]
    fn falls_back_polling_rates_to_capabilities() {
        let outputs = BTreeMap::from([("settings".into(), json!({"pollingRate": 1000}))]);
        let capabilities = Some(json!({"pollingRatesHz": [125, 250, 500, 1000]}));
        let reading = standard_reading(outputs, capabilities);
        assert_eq!(
            reading.supported_polling_rates_hz,
            Some(vec![125, 250, 500, 1000])
        );
    }
}
