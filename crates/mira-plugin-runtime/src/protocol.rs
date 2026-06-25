// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::engine::ProtocolPackage;
use hidapi::HidApi;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Feature index 缓存：按设备路径索引，存储 featureId → featureIndex 映射。
/// feature index 在设备连接期间不变，缓存可避免每轮轮询重复查询。
pub type FeatureIndexCache = HashMap<String, HashMap<u16, u8>>;

/// Onboard memory 缓存：按设备路径索引，存储最近一次 onboard read 的 (outputs, bytes)。
/// 写入 mutation 的预读阶段检查缓存，命中则跳过 16 chunk HID 往返。
/// 写入后的验证读更新缓存。设备断开时由调用方清空。
pub type OnboardMemoryCache = HashMap<String, (BTreeMap<String, Value>, Vec<u8>)>;

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
    /// Feature index 缓存（按设备路径索引）。设备连接期间 feature index 不变，
    /// 缓存命中时跳过 root-get-feature 的 HID 往返。设备断开时由调用方清空。
    pub feature_index_cache: Option<&'a Mutex<FeatureIndexCache>>,
    /// Onboard memory 缓存（按设备路径索引）。写入 mutation 预读时命中缓存则跳过
    /// 16 chunk HID 往返；验证读后更新缓存。设备断开时由调用方清空。
    pub onboard_memory_cache: Option<&'a Mutex<OnboardMemoryCache>>,
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
    let outputs =
        package.execute_with_cache(ctx.api, ctx.path, &workflow_id, ctx.feature_index_cache)?;
    #[cfg(debug_assertions)]
    eprintln!(
        "[mira] plugin workflow {workflow_id}: {} outputs: [{}]",
        outputs.len(),
        outputs
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
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
    package.mutate(
        ctx.api,
        ctx.path,
        &mutation_id,
        params,
        &ctx.outputs,
        ctx.onboard_memory_cache,
    )
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
        reading.charging = boolean_like(battery, "charging").unwrap_or(false);
        if let Some(percentage) = reading.battery_percent {
            reading.batteries.push(mira_core::DeviceBattery {
                id: "mouse".into(),
                label: "mock.mouseLabel".into(),
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
                    label: "mock.mouseLabel".into(),
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
                label: "mock.receiverLabel".into(),
                percentage,
                charging: false,
            });
        }
    }
    if let Some(receiver_battery) = object(&outputs, "receiverBattery") {
        if let Some(percentage) =
            number(receiver_battery, "percentage").and_then(|value| u8::try_from(value).ok())
        {
            upsert_battery(
                &mut reading.batteries,
                mira_core::DeviceBattery {
                    id: "receiver".into(),
                    label: "mock.receiverLabel".into(),
                    percentage,
                    charging: boolean_like(receiver_battery, "charging").unwrap_or(false),
                },
            );
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

    normalize_lighting_capabilities(&outputs, &mut reading.capabilities);

    reading.light_color = object(&reading.capabilities, "mouseLighting")
        .and_then(|lighting| lighting.get("color"))
        .and_then(Value::as_str)
        .map(str::to_string);

    reading
}

fn normalize_lighting_capabilities(
    outputs: &BTreeMap<String, Value>,
    capabilities: &mut BTreeMap<String, Value>,
) {
    if let Some(mouse_lighting) = normalized_mouse_lighting(outputs) {
        capabilities.insert("mouseLighting".into(), Value::Object(mouse_lighting));
    }
    if let Some(receiver_lighting) = normalized_receiver_lighting(outputs) {
        capabilities.insert("receiverLighting".into(), Value::Object(receiver_lighting));
    }
}

fn normalized_mouse_lighting(
    outputs: &BTreeMap<String, Value>,
) -> Option<serde_json::Map<String, Value>> {
    let settings = object(outputs, "settings");
    let mode = object(outputs, "mouseLightMode").or_else(|| object(outputs, "mouseEffect"));
    let color = settings
        .and_then(|settings| settings.get("mouseLightStartColor"))
        .or_else(|| object(outputs, "mouseLightColor").and_then(|lighting| lighting.get("color")))
        .or_else(|| mode.and_then(|lighting| lighting.get("color")))
        .and_then(Value::as_str);
    let enabled = settings
        .and_then(|settings| boolean_like(settings, "mouseLightEnabled"))
        .or_else(|| {
            object(outputs, "mouseLightSwitch").and_then(|switch| boolean_like(switch, "enabled"))
        })
        .or_else(|| mode.and_then(|lighting| boolean_like(lighting, "enabled")));

    if color.is_none() && enabled.is_none() && mode.is_none() {
        return None;
    }

    let mut lighting = serde_json::Map::new();
    if let Some(enabled) = enabled {
        lighting.insert("enabled".into(), json!(enabled));
    }
    if let Some(color) = color {
        lighting.insert("color".into(), json!(color));
    }
    if let Some(settings) = settings {
        if let Some(color) = settings.get("mouseLightEndColor").and_then(Value::as_str) {
            lighting.insert("endColor".into(), json!(color));
        }
    }
    if let Some(mode) = mode {
        copy_field(mode, &mut lighting, "effect");
        copy_field(mode, &mut lighting, "effectName");
        copy_field(mode, &mut lighting, "mode");
        copy_field(mode, &mut lighting, "modeName");
        copy_field(mode, &mut lighting, "speed");
        copy_field(mode, &mut lighting, "speedLabel");
        copy_field(mode, &mut lighting, "brightness");
        copy_field(mode, &mut lighting, "brightnessLabel");
    }
    Some(lighting)
}

fn normalized_receiver_lighting(
    outputs: &BTreeMap<String, Value>,
) -> Option<serde_json::Map<String, Value>> {
    if let Some(receiver) = object(outputs, "receiverLighting") {
        return Some(receiver.clone());
    }
    let receiver = object(outputs, "receiverLight")?;
    let mut lighting = serde_json::Map::new();
    if let Some(enabled) = boolean_like(receiver, "enabled") {
        lighting.insert("enabled".into(), json!(enabled));
        if !enabled {
            lighting.insert("effect".into(), json!(0));
        }
    }
    if let Some(effect) = receiver.get("type").and_then(Value::as_u64) {
        lighting.entry("effect").or_insert_with(|| json!(effect));
        lighting.insert("option".into(), json!(effect));
    }
    if let Some(color) = receiver.get("color1").and_then(Value::as_str) {
        lighting.insert("color".into(), json!(color));
    }
    copy_field(receiver, &mut lighting, "speed");
    copy_field(receiver, &mut lighting, "brightness");
    (!lighting.is_empty()).then_some(lighting)
}

fn copy_field(
    source: &serde_json::Map<String, Value>,
    target: &mut serde_json::Map<String, Value>,
    key: &str,
) {
    if let Some(value) = source.get(key) {
        target.insert(key.into(), value.clone());
    }
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

fn boolean_like(object: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    object.get(key).and_then(|value| {
        value
            .as_bool()
            .or_else(|| value.as_u64().map(|number| number != 0))
    })
}

fn upsert_battery(
    batteries: &mut Vec<mira_core::DeviceBattery>,
    battery: mira_core::DeviceBattery,
) {
    if let Some(existing) = batteries
        .iter_mut()
        .find(|existing| existing.id == battery.id)
    {
        *existing = battery;
    } else {
        batteries.push(battery);
    }
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
        assert_eq!(reading.capabilities.len(), 5);
        assert!(reading.capabilities.contains_key("mouseLighting"));
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
        assert_eq!(reading.batteries[0].label, "mock.mouseLabel");
        assert_eq!(reading.batteries[1].label, "mock.receiverLabel");
        assert_eq!(reading.batteries[1].percentage, 100);
    }

    #[test]
    fn normalizes_am35_numeric_charging_and_receiver_battery_output() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 76, "charging": 1, "health": 100, "present": 1}),
            ),
            (
                "receiverBattery".into(),
                json!({"percentage": 95, "charging": 1, "health": 100, "present": 1}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 74, "receiverBattery": 88}),
            ),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.battery_percent, Some(76));
        assert!(reading.charging);
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[0].id, "mouse");
        assert!(reading.batteries[0].charging);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 95);
        assert!(reading.batteries[1].charging);
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
        assert_eq!(
            reading
                .capabilities
                .get("mouseLighting")
                .and_then(|value| value.get("color"))
                .and_then(Value::as_str),
            Some("#FB223C")
        );
    }

    #[test]
    fn never_treats_receiver_lighting_as_mouse_light_color() {
        let outputs = BTreeMap::from([
            ("lighting".into(), json!({"color": "#EEAA00"})),
            ("receiverLighting".into(), json!({"color": "#4BBFB1"})),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.light_color, None);
    }

    #[test]
    fn prefers_explicit_mouse_light_color_over_receiver_lighting() {
        let outputs = BTreeMap::from([
            ("mouseLightColor".into(), json!({"color": "#FB223C"})),
            ("receiverLighting".into(), json!({"color": "#4BBFB1"})),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.light_color.as_deref(), Some("#FB223C"));
    }

    #[test]
    fn normalizes_am35_mouse_and_receiver_lighting_separately() {
        let outputs = BTreeMap::from([
            (
                "mouseLightMode".into(),
                json!({"mode": 2, "modeName": "霓虹", "speed": 1, "brightness": 3}),
            ),
            ("mouseLightColor".into(), json!({"color": "#112233"})),
            (
                "receiverLight".into(),
                json!({"enabled": 1, "type": 7, "color1": "#AABBCC", "speed": 2, "brightness": 4}),
            ),
        ]);
        let reading = standard_reading(outputs, None);
        let mouse = reading
            .capabilities
            .get("mouseLighting")
            .and_then(Value::as_object)
            .unwrap();
        let receiver = reading
            .capabilities
            .get("receiverLighting")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(reading.light_color.as_deref(), Some("#112233"));
        assert_eq!(mouse.get("color").and_then(Value::as_str), Some("#112233"));
        assert_eq!(mouse.get("mode").and_then(Value::as_u64), Some(2));
        assert_eq!(
            receiver.get("color").and_then(Value::as_str),
            Some("#AABBCC")
        );
        assert_eq!(receiver.get("effect").and_then(Value::as_u64), Some(7));
        assert_eq!(receiver.get("option").and_then(Value::as_u64), Some(7));
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
