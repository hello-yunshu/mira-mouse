// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::engine::ProtocolPackage;
use hidapi::{HidApi, HidDevice};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

/// Feature index 缓存：按设备路径索引，存储 featureId → 完整 parsed output 映射。
/// feature index 在设备连接期间不变，缓存可避免每轮轮询重复查询。
/// 存储 complete Value（而非仅 featureIndex: u8）以保留 deviceIndex、connection 等
/// derived 字段，防止后续 step 引用 `{fromOutput: "device", field: "deviceIndex"}` 时
/// 因缓存命中丢失字段而报 "missing output reference"。
pub type FeatureIndexCache = HashMap<String, HashMap<u16, Value>>;

/// Onboard memory 缓存：按设备路径索引，存储最近一次 onboard read 的 (outputs, bytes)。
/// 写入 mutation 的预读阶段检查缓存，命中则跳过 16 chunk HID 往返。
/// 写入后的验证读更新缓存。设备断开时由调用方清空。
pub type OnboardMemoryCache = HashMap<String, (BTreeMap<String, Value>, Vec<u8>)>;

/// 已打开的 HID 设备句柄缓存，按设备路径索引。
/// `HidDevice` 不可 Clone，采用取用-归还策略：执行前从缓存取出（未命中则 open_path），
/// 执行成功后归还；执行出错时句柄随 session 析构关闭，不归还（设备可能处于异常状态）。
/// `device_io` 锁已序列化 HID 访问，缓存读写仅持有极短时段，无死锁风险。
pub type HidHandleCache = HashMap<String, HidDevice>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HidIoStats {
    pub handle_cache_hits: u64,
    pub handle_cache_misses: u64,
    pub open_path_attempts: u64,
    pub open_path_failures: u64,
    pub handles_returned: u64,
    pub handle_cache_lock_failures: u64,
}

impl HidIoStats {
    pub fn record_cache_hit(&mut self) {
        self.handle_cache_hits += 1;
    }

    pub fn record_cache_miss(&mut self) {
        self.handle_cache_misses += 1;
        self.open_path_attempts += 1;
    }

    pub fn record_open_failure(&mut self) {
        self.open_path_failures += 1;
    }

    pub fn record_returned(&mut self) {
        self.handles_returned += 1;
    }

    pub fn record_lock_failure(&mut self) {
        self.handle_cache_lock_failures += 1;
    }
}

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
    /// 已打开的 HID 设备句柄缓存（按设备路径索引）。命中时复用句柄，跳过 open_path
    /// 系统调用；未命中时 open_path 并在执行成功后归还。设备断开时由调用方清空。
    pub cached_handles: Option<&'a Mutex<HidHandleCache>>,
    /// 可选 HID I/O 计数器，用于 debug/诊断：统计句柄缓存命中、open_path 次数、
    /// 归还次数和锁失败。未提供时不产生额外可见行为。
    pub hid_io_stats: Option<&'a Mutex<HidIoStats>>,
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
    let mut outputs = package.execute_with_cache(
        ctx.api,
        ctx.path,
        &workflow_id,
        ctx.feature_index_cache,
        ctx.cached_handles,
        ctx.hid_io_stats,
    )?;
    let capabilities = package.capabilities().cloned();
    maybe_merge_onboard_lighting(package, ctx, capabilities.as_ref(), &mut outputs)?;
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
    Ok(standard_reading(outputs, capabilities))
}

fn maybe_merge_onboard_lighting(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
    capabilities: Option<&Value>,
    outputs: &mut BTreeMap<String, Value>,
) -> Result<(), String> {
    if normalized_mouse_lighting(outputs, capabilities).is_some() {
        return Ok(());
    }
    let Some(feature_index) = object(outputs, "featureIndexOnboardProfiles")
        .and_then(|feature| feature.get("featureIndex"))
        .and_then(Value::as_u64)
    else {
        return Ok(());
    };
    if feature_index == 0 {
        return Ok(());
    }

    let Some(onboard_workflow_id) = onboard_mouse_lighting_workflow_id(capabilities) else {
        return Ok(());
    };
    if !package.has_workflow(&onboard_workflow_id) {
        return Ok(());
    }

    // 持锁期间直接遍历 cached_outputs 并插入，避免克隆整个 BTreeMap。
    if let Some(cache) = ctx.onboard_memory_cache {
        if let Ok(guard) = cache.lock() {
            if let Some((cached_outputs, _)) = guard.get(ctx.path) {
                for (key, value) in cached_outputs {
                    outputs.entry(key.clone()).or_insert(value.clone());
                }
                return Ok(());
            }
        }
    }
    // Cache miss: execute onboard workflow and merge results.
    let onboard_outputs = package.execute_with_cache(
        ctx.api,
        ctx.path,
        &onboard_workflow_id,
        ctx.feature_index_cache,
        ctx.cached_handles,
        ctx.hid_io_stats,
    )?;
    for (key, value) in onboard_outputs {
        outputs.entry(key).or_insert(value);
    }
    Ok(())
}

pub fn execute_plugin_workflow(
    ctx: &ProtocolContext,
    workflow_id: &str,
) -> Result<BTreeMap<String, Value>, String> {
    ProtocolPackage::from_files(ctx.files)?.execute_with_cache(
        ctx.api,
        ctx.path,
        workflow_id,
        ctx.feature_index_cache,
        ctx.cached_handles,
        ctx.hid_io_stats,
    )
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
        ctx.cached_handles,
        ctx.hid_io_stats,
    )
}

fn standard_reading(
    outputs: BTreeMap<String, Value>,
    capabilities: Option<Value>,
) -> DeviceReading {
    let mut reading = DeviceReading {
        capabilities: outputs,
        ..DeviceReading::default()
    };

    // Prefer device-reported rates from the protocol; fall back to the static
    // plugin manifest so the UI always receives a supported list.
    if let Some(rates) = object(&reading.capabilities, "reportRateList")
        .or_else(|| object(&reading.capabilities, "reportRateListExtended"))
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

    reading.display_name = object(&reading.capabilities, "deviceName")
        .and_then(|device| device.get("name"))
        .and_then(Value::as_str)
        .and_then(mira_core::normalize_device_display_name);
    reading.connection = object(&reading.capabilities, "device")
        .or_else(|| object(&reading.capabilities, "featureIndexDeviceInfo"))
        .and_then(|device| device.get("connection"))
        .and_then(Value::as_str)
        .and_then(|connection| match connection {
            "usb" => Some(ConnectionKind::Usb),
            "wireless" | "wireless-receiver" => Some(ConnectionKind::Wireless),
            "bluetooth" => Some(ConnectionKind::Bluetooth),
            _ => None,
        });

    if let Some(battery) = object(&reading.capabilities, "battery") {
        reading.battery_percent = reported_battery_percentage(battery, "percentage");
        reading.charging = battery_charging(battery, "charging");
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
    let receiver_idle = object(&reading.capabilities, "receiverIdle");
    let receiver_proxy = object(&reading.capabilities, "receiver");
    let receiver = receiver_idle.or(receiver_proxy);
    if let Some(receiver) = receiver {
        if reading.battery_percent.is_none() {
            reading.battery_percent = receiver_mouse_battery_percentage(receiver);
        }
        if reading.batteries.is_empty() {
            if let Some(percentage) = receiver_mouse_battery_percentage(receiver) {
                reading.batteries.push(mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: "mock.mouseLabel".into(),
                    percentage,
                    charging: false,
                });
            }
        }
        if let Some(percentage) = receiver_status_battery_percentage(receiver) {
            reading.batteries.push(mira_core::DeviceBattery {
                id: "receiver".into(),
                label: "mock.receiverLabel".into(),
                percentage,
                charging: protocol_a_receiver_battery_charging(percentage),
            });
        }
    }
    if let Some(receiver_battery) = object(&reading.capabilities, "receiverBattery") {
        if let Some(percentage) = reported_battery_percentage(receiver_battery, "percentage") {
            upsert_battery(
                &mut reading.batteries,
                mira_core::DeviceBattery {
                    id: "receiver".into(),
                    label: "mock.receiverLabel".into(),
                    percentage,
                    charging: battery_charging(receiver_battery, "charging"),
                },
            );
        }
    }

    reading.profile = crate::onboard_profiles::active_profile_index(&reading.capabilities);

    // If the plugin already emitted a structured "profile" capability, keep it.
    // Otherwise, when 0x8101 Profile Management outputs are present, normalize
    // them into a single capability object so the UI does not need to know the
    // exact workflow output names.
    if object(&reading.capabilities, "profile").is_none()
        && (crate::onboard_profiles::profile_count(&reading.capabilities).is_some()
            || crate::onboard_profiles::profile_management_info(&reading.capabilities).is_some())
    {
        let mut profile = serde_json::Map::new();
        if let Some(current) = reading.profile {
            profile.insert("current".into(), json!(current));
        }
        if let Some(count) = crate::onboard_profiles::profile_count(&reading.capabilities) {
            profile.insert("count".into(), json!(count));
        }
        if let Some(info) = crate::onboard_profiles::profile_management_info(&reading.capabilities)
        {
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

    if let Some(dpi) = object(&reading.capabilities, "dpi")
        .or_else(|| object(&reading.capabilities, "dpiExtended"))
    {
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

    if let Some(settings) = object(&reading.capabilities, "settings")
        .or_else(|| object(&reading.capabilities, "settingsExtended"))
    {
        reading.polling_rate_hz =
            number(settings, "pollingRate").and_then(|value| u16::try_from(value).ok());
    }

    // Compute lighting capabilities from outputs before mutating capabilities.
    // Inlined to avoid simultaneous &reading.capabilities and &mut reading.capabilities.
    let mouse_lighting = normalized_mouse_lighting(&reading.capabilities, capabilities.as_ref());
    let receiver_lighting = normalized_receiver_lighting(&reading.capabilities);
    if let Some(mouse_lighting) = mouse_lighting {
        reading
            .capabilities
            .insert("mouseLighting".into(), Value::Object(mouse_lighting));
    }
    if let Some(receiver_lighting) = receiver_lighting {
        reading
            .capabilities
            .insert("receiverLighting".into(), Value::Object(receiver_lighting));
    }

    reading.light_color = object(&reading.capabilities, "mouseLighting")
        .and_then(|lighting| lighting.get("color"))
        .and_then(Value::as_str)
        .map(str::to_string);

    reading
}

fn normalized_mouse_lighting(
    outputs: &BTreeMap<String, Value>,
    plugin_capabilities: Option<&Value>,
) -> Option<serde_json::Map<String, Value>> {
    if let Some(onboard) = onboard_mouse_lighting(outputs, plugin_capabilities) {
        return Some(onboard);
    }
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
    append_supported_lighting_effects(outputs, &mut lighting);
    Some(lighting)
}

fn append_supported_lighting_effects(
    outputs: &BTreeMap<String, Value>,
    lighting: &mut serde_json::Map<String, Value>,
) {
    let mut effects = BTreeSet::from([0_u64]);
    let mut saw_supports = false;
    for info in ["colorLedInfo", "rgbEffectsInfo"]
        .iter()
        .filter_map(|key| object(outputs, key))
    {
        for (field, value) in [
            ("supportsFixed", 1_u64),
            ("supportsCycle", 3),
            ("supportsWave", 4),
            ("supportsStarlight", 5),
            ("supportsBreathing", 10),
            ("supportsRipple", 11),
            ("supportsCustom", 12),
        ] {
            if let Some(supported) = boolean_like(info, field) {
                saw_supports = true;
                if supported {
                    effects.insert(value);
                }
            }
        }
    }
    if saw_supports {
        lighting.insert("supportedEffects".into(), json!(effects));
    }
}

fn onboard_mouse_lighting(
    outputs: &BTreeMap<String, Value>,
    capabilities: Option<&Value>,
) -> Option<serde_json::Map<String, Value>> {
    let profile = onboard_mouse_lighting_normalizer(capabilities)?;
    if !onboard_profile_lighting_active(outputs) {
        return None;
    }
    let description_output = profile
        .get("sectorSize")
        .and_then(|reference| reference.get("output"))
        .and_then(Value::as_str)
        .unwrap_or("onboardDescription");
    let chunk_prefix = profile
        .get("chunkPrefix")
        .and_then(Value::as_str)
        .unwrap_or("onboardProfileChunk");
    let chunk_field = profile
        .get("chunkField")
        .and_then(Value::as_str)
        .unwrap_or("bytes");
    let description = object(outputs, description_output)?;
    let sector_size = profile
        .get("sectorSize")
        .and_then(|reference| reference.get("field"))
        .and_then(Value::as_str)
        .and_then(|field| number(description, field))
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(256);
    let bytes = onboard_profile_bytes(outputs, sector_size, chunk_prefix, chunk_field)?;
    let layout = profile
        .get("layouts")?
        .as_array()?
        .iter()
        .find(|layout| onboard_layout_matches(layout, description))?
        .as_object()?;
    let effect_offset = normalizer_offset(layout, "effectOffset")?;
    let color_offset = normalizer_offset(layout, "colorOffset")?;
    let speed_offset = normalizer_offset(layout, "speedOffset")?;
    let brightness_offset = normalizer_offset(layout, "brightnessOffset")?;
    let extra_color_offset = normalizer_offset(layout, "extraColorOffset")?;
    if bytes.len() <= extra_color_offset + 2
        || bytes.len() <= brightness_offset
        || bytes.len() <= effect_offset
        || bytes.len() <= color_offset + 2
        || bytes.len() <= speed_offset + 1
    {
        return None;
    }

    let effect = bytes[effect_offset];
    let enabled = profile
        .get("enabledOverride")
        .and_then(|reference| {
            let output = reference.get("output")?.as_str()?;
            let field = reference.get("field")?.as_str()?;
            object(outputs, output).and_then(|value| boolean_like(value, field))
        })
        .unwrap_or(effect != 0);
    let mut lighting = serde_json::Map::new();
    lighting.insert("enabled".into(), json!(enabled));
    lighting.insert("effect".into(), json!(effect));
    lighting.insert(
        "color".into(),
        json!(format!(
            "#{:02x}{:02x}{:02x}",
            bytes[color_offset],
            bytes[color_offset + 1],
            bytes[color_offset + 2]
        )),
    );
    lighting.insert(
        "speed".into(),
        json!(u16::from_be_bytes([
            bytes[speed_offset],
            bytes[speed_offset + 1]
        ])),
    );
    lighting.insert("brightness".into(), json!(bytes[brightness_offset]));
    lighting.insert(
        "extraColor".into(),
        json!(format!(
            "#{:02x}{:02x}{:02x}",
            bytes[extra_color_offset],
            bytes[extra_color_offset + 1],
            bytes[extra_color_offset + 2]
        )),
    );
    Some(lighting)
}

fn onboard_profile_lighting_active(outputs: &BTreeMap<String, Value>) -> bool {
    let mode = object(outputs, "onboardMode").or_else(|| object(outputs, "controlMode"));
    mode.and_then(|mode| number(mode, "mode"))
        .is_none_or(|mode| mode == 1)
}

fn onboard_mouse_lighting_normalizer(
    capabilities: Option<&Value>,
) -> Option<&serde_json::Map<String, Value>> {
    capabilities?
        .get("normalizers")?
        .get("mouseLighting")?
        .get("onboardProfile")?
        .as_object()
}

fn onboard_mouse_lighting_workflow_id(capabilities: Option<&Value>) -> Option<String> {
    onboard_mouse_lighting_normalizer(capabilities)?
        .get("sourceWorkflow")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn onboard_layout_matches(layout: &Value, description: &serde_json::Map<String, Value>) -> bool {
    let Some(layout) = layout.as_object() else {
        return false;
    };
    if layout
        .get("default")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    let Some(condition) = layout.get("when").and_then(Value::as_object) else {
        return false;
    };
    let Some(field) = condition.get("field").and_then(Value::as_str) else {
        return false;
    };
    let Some(expected) = condition.get("eq") else {
        return false;
    };
    description.get(field) == Some(expected)
}

fn normalizer_offset(layout: &serde_json::Map<String, Value>, key: &str) -> Option<usize> {
    layout
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn onboard_profile_bytes(
    outputs: &BTreeMap<String, Value>,
    sector_size: usize,
    chunk_prefix: &str,
    chunk_field: &str,
) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for index in 0.. {
        let key = format!("{chunk_prefix}{index:02}");
        let Some(chunk) = object(outputs, &key) else {
            break;
        };
        let chunk_bytes = chunk.get(chunk_field)?.as_array()?;
        for byte in chunk_bytes {
            bytes.push(u8::try_from(byte.as_u64()?).ok()?);
            if bytes.len() >= sector_size {
                return Some(bytes);
            }
        }
    }
    (bytes.len() >= sector_size).then_some(bytes)
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

fn percentage_value(object: &serde_json::Map<String, Value>, key: &str) -> Option<u8> {
    let value = number(object, key)?;
    (value <= 100).then(|| u8::try_from(value).ok()).flatten()
}

fn reported_battery_percentage(object: &serde_json::Map<String, Value>, key: &str) -> Option<u8> {
    // 仅根据 present 判断电池是否存在；valid 标志在不同连接模式下语义
    // 不一致（USB 直连时 offset 2 常为 0），保留它会误伤真实电量数据。
    if boolean_like(object, "present") == Some(false) {
        return None;
    }
    percentage_value(object, key)
}

fn receiver_mouse_battery_percentage(object: &serde_json::Map<String, Value>) -> Option<u8> {
    if boolean_like(object, "mouseOnline") == Some(false) {
        return None;
    }
    percentage_value(object, "mouseBattery")
}

fn receiver_status_battery_percentage(object: &serde_json::Map<String, Value>) -> Option<u8> {
    let percentage = percentage_value(object, "receiverBattery")?;
    // Protocol A receivers can report 0x32 while charging even when the real
    // level is different; treat it as an unavailable placeholder, not 50%.
    if percentage == 0 || percentage == 50 {
        return None;
    }
    Some(percentage)
}

fn protocol_a_receiver_battery_charging(percentage: u8) -> bool {
    (1..100).contains(&percentage)
}

/// 电池充电状态字段约定：原始字节值 1 表示充电中（与官方前端
/// `1 === mouseBatStatus` / `1 === dongleChargingStatus` 一致）。
/// 0 = 未充电，2 = 满电（或其他状态码）均不视为充电中。
/// 兼容旧 parser 输出的 bool 值（true 视为 1）。
fn battery_charging(object: &serde_json::Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_u64().map(|status| status == 1))
        })
        .unwrap_or(false)
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
    fn normalizes_protocol_a_receiver_battery_charging_status() {
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 88}),
        )]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 88);
        assert!(reading.batteries[1].charging);
    }

    #[test]
    fn protocol_a_receiver_prefers_idle_status_battery() {
        let outputs = BTreeMap::from([
            (
                "receiverIdle".into(),
                json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 87}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 50}),
            ),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 87);
        assert!(reading.batteries[1].charging);
    }

    #[test]
    fn protocol_a_receiver_drops_charging_placeholder_50() {
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 50}),
        )]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.batteries.len(), 1);
        assert_eq!(reading.batteries[0].id, "mouse");
    }

    #[test]
    fn drops_invalid_or_unavailable_battery_percentages() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 101, "charging": false, "valid": true}),
            ),
            (
                "receiverBattery".into(),
                json!({"percentage": 88, "charging": 1, "present": 0}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 80, "mouseOnline": false, "receiverBattery": 0}),
            ),
        ]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.battery_percent, None);
        assert!(reading.batteries.is_empty());
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

    /// 官方前端用 `1 === mouseBatStatus` / `1 === dongleChargingStatus` 判断充电中。
    /// status=0（未充电）和 status=2（满电）都不应显示充电图标。
    #[test]
    fn treats_only_status_one_as_charging() {
        for (status, expected_charging) in [(0u8, false), (1, true), (2, false)] {
            let outputs = BTreeMap::from([(
                "battery".into(),
                json!({"percentage": 80, "charging": status, "valid": true}),
            )]);
            let reading = standard_reading(outputs, None);
            assert_eq!(
                reading.charging, expected_charging,
                "status {status} should report charging={expected_charging}"
            );
        }

        // AM35 接收器同理（receiverBattery output 携带 charging 字段）。
        for (status, expected_charging) in [(0u8, false), (1, true), (2, false)] {
            let outputs = BTreeMap::from([(
                "receiverBattery".into(),
                json!({"percentage": 90, "charging": status, "present": 1}),
            )]);
            let reading = standard_reading(outputs, None);
            assert_eq!(reading.batteries.len(), 1);
            assert_eq!(
                reading.batteries[0].charging, expected_charging,
                "receiver status {status} should report charging={expected_charging}"
            );
        }
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
    fn normalizes_plugin_reported_usb_connection() {
        let outputs = BTreeMap::from([(
            "device".into(),
            json!({"deviceIndex": 255, "connection": "usb"}),
        )]);
        let reading = standard_reading(outputs, None);
        assert_eq!(reading.connection, Some(ConnectionKind::Usb));
    }

    #[test]
    fn limits_plugin_reported_device_name_for_host_layout() {
        let outputs = BTreeMap::from([(
            "deviceName".into(),
            json!({"name": "  Logitech Prototype Mouse With A Very Long Engineering Name  "}),
        )]);
        let reading = standard_reading(outputs, None);
        let display_name = reading.display_name.unwrap();
        assert_eq!(
            display_name.chars().count(),
            mira_core::MAX_DEVICE_DISPLAY_NAME_CHARS
        );
        assert!(display_name.ends_with('…'));
        assert!(!display_name.starts_with(' '));
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

    #[test]
    fn normalizes_hidpp_onboard_profile_lighting_from_plugin_metadata() {
        let mut outputs = BTreeMap::from([(
            "onboardDescription".into(),
            json!({"profileFormatId": 5, "sectorSize": 255}),
        )]);
        outputs.insert("rgbControl".into(), json!({"enabled": false}));
        outputs.insert(
            "rgbEffectsInfo".into(),
            json!({"supportsFixed": false, "supportsCycle": false, "supportsWave": true}),
        );
        for index in 0..16 {
            let mut chunk = vec![0; 16];
            if index == 13 {
                chunk[11] = 3;
                chunk[12] = 0xb8;
                chunk[13] = 0x7a;
                chunk[14] = 0xb0;
            }
            if index == 14 {
                chunk[0] = 100;
                chunk[1] = 100;
                chunk[2] = 0x12;
                chunk[3] = 0x34;
                chunk[4] = 0x56;
            }
            outputs.insert(
                format!("onboardProfileChunk{index:02}"),
                json!({"bytes": chunk}),
            );
        }

        assert!(!standard_reading(outputs.clone(), None)
            .capabilities
            .contains_key("mouseLighting"));

        let capabilities = Some(json!({
            "normalizers": {
                "mouseLighting": {
                    "onboardProfile": {
                        "sectorSize": { "output": "onboardDescription", "field": "sectorSize" },
                        "enabledOverride": { "output": "rgbControl", "field": "enabled" },
                        "chunkPrefix": "onboardProfileChunk",
                        "chunkField": "bytes",
                        "layouts": [{
                            "when": { "field": "profileFormatId", "eq": 5 },
                            "effectOffset": 219,
                            "colorOffset": 220,
                            "speedOffset": 223,
                            "brightnessOffset": 225,
                            "extraColorOffset": 226
                        }]
                    }
                }
            }
        }));
        let reading = standard_reading(outputs, capabilities);
        let mouse = reading
            .capabilities
            .get("mouseLighting")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(reading.light_color.as_deref(), Some("#b87ab0"));
        assert_eq!(mouse.get("enabled").and_then(Value::as_bool), Some(false));
        assert_eq!(mouse.get("effect").and_then(Value::as_u64), Some(3));
        assert_eq!(mouse.get("speed").and_then(Value::as_u64), Some(100));
        assert_eq!(mouse.get("brightness").and_then(Value::as_u64), Some(100));
        assert_eq!(
            mouse.get("extraColor").and_then(Value::as_str),
            Some("#123456")
        );
        assert_eq!(mouse.get("supportedEffects"), None);
    }

    #[test]
    fn normalizes_supported_lighting_effects_from_feature_info() {
        let outputs = BTreeMap::from([
            (
                "mouseEffect".into(),
                json!({"effect": 10, "color": "#123456", "enabled": true}),
            ),
            (
                "colorLedInfo".into(),
                json!({
                    "supportsFixed": true,
                    "supportsCycle": false,
                    "supportsWave": true,
                    "supportsStarlight": false,
                    "supportsBreathing": true,
                    "supportsRipple": false,
                    "supportsCustom": false
                }),
            ),
        ]);
        let reading = standard_reading(outputs, None);
        let mouse = reading
            .capabilities
            .get("mouseLighting")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(mouse.get("supportedEffects"), Some(&json!([0, 1, 4, 10])));
    }

    #[test]
    fn skips_onboard_profile_lighting_when_host_mode_is_active() {
        let mut outputs = BTreeMap::from([
            (
                "onboardDescription".into(),
                json!({"profileFormatId": 5, "sectorSize": 255}),
            ),
            ("onboardMode".into(), json!({"mode": 2, "modeName": "host"})),
        ]);
        outputs.insert("rgbControl".into(), json!({"enabled": false}));
        for index in 0..16 {
            outputs.insert(
                format!("onboardProfileChunk{index:02}"),
                json!({"bytes": vec![255; 16]}),
            );
        }
        let capabilities = Some(json!({
            "normalizers": {
                "mouseLighting": {
                    "onboardProfile": {
                        "sectorSize": { "output": "onboardDescription", "field": "sectorSize" },
                        "enabledOverride": { "output": "rgbControl", "field": "enabled" },
                        "chunkPrefix": "onboardProfileChunk",
                        "chunkField": "bytes",
                        "layouts": [{
                            "when": { "field": "profileFormatId", "eq": 5 },
                            "effectOffset": 219,
                            "colorOffset": 220,
                            "speedOffset": 223,
                            "brightnessOffset": 225,
                            "extraColorOffset": 226
                        }]
                    }
                }
            }
        }));

        let reading = standard_reading(outputs, capabilities);
        assert!(!reading.capabilities.contains_key("mouseLighting"));
        assert_eq!(reading.light_color, None);
    }

    #[test]
    fn hid_io_stats_records_handle_cache_events() {
        let mut stats = HidIoStats::default();
        stats.record_cache_miss();
        stats.record_cache_hit();
        stats.record_returned();
        stats.record_open_failure();
        stats.record_lock_failure();

        assert_eq!(stats.handle_cache_misses, 1);
        assert_eq!(stats.open_path_attempts, 1);
        assert_eq!(stats.handle_cache_hits, 1);
        assert_eq!(stats.handles_returned, 1);
        assert_eq!(stats.open_path_failures, 1);
        assert_eq!(stats.handle_cache_lock_failures, 1);
    }
}
