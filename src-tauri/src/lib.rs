// SPDX-License-Identifier: AGPL-3.0-or-later
use ed25519_dalek::VerifyingKey;
use hidapi::HidApi;
use mira_core::{DeviceSnapshot, LowBatteryCrossing, PluginCapability, PluginCapabilityPlacement};
use mira_plugin_runtime::{
    extract_package, hid, inspect_package, mutate_device_with_package, read_device_with_package,
    writable_mutations_with_package, ConnectionKind, DeviceReading, ExportableField,
    FeatureIndexCache, OnboardMemoryCache, PackageInspection, ProtocolContext, ProtocolPackage,
    TrustStore,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use sys_locale::get_locale;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, WebviewWindow,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;

type CachedPlugins = Vec<(
    PackageInspection,
    hid::DevicesFile,
    std::collections::BTreeMap<String, Vec<u8>>,
)>;

const PLUGIN_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/hello-yunshu/mira-mouse-plugins/main/registry/index.json";
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_PLUGIN_BYTES: u64 = 32 * 1024 * 1024;

/// 将连接类型字符串归一化为规范值（"usb"/"receiver"/"bluetooth"）。
/// devices.json 中接收器连接值可能写作 "wireless" 或 "wireless-receiver"，
/// 统一映射为 "receiver" 以匹配插件 capabilities 声明的 connections 列表。
/// 修复 #3：消除文档词汇表（"receiver"）与 runtime 实际值（"wireless"）的不一致。
fn normalize_connection(value: &str) -> &str {
    match value {
        "wireless" | "wireless-receiver" => "receiver",
        "usb" => "usb",
        "bluetooth" => "bluetooth",
        _ => value,
    }
}

fn find_firmware_version(value: &serde_json::Value) -> Option<semver::Version> {
    match value {
        serde_json::Value::String(text) => semver::Version::parse(text).ok(),
        serde_json::Value::Array(items) => items.iter().find_map(find_firmware_version),
        serde_json::Value::Object(object) => {
            for key in ["firmwareVersion", "version", "versionName", "semver"] {
                if let Some(version) = object.get(key).and_then(find_firmware_version) {
                    return Some(version);
                }
            }
            object.values().find_map(find_firmware_version)
        }
        _ => None,
    }
}

fn firmware_available(
    outputs: &BTreeMap<String, serde_json::Value>,
    min_firmware: Option<&str>,
) -> bool {
    let Some(min_firmware) = min_firmware else {
        return true;
    };
    let Ok(required) = semver::Version::parse(min_firmware) else {
        return false;
    };
    outputs
        .values()
        .find_map(find_firmware_version)
        .is_some_and(|current| current >= required)
}

/// 根据插件声明和 workflow 输出构建能力列表。
/// `outputs` 是设备读取的 workflow 输出，用于 probe 判断：
/// 当 capability.probe 引用的字段值为 0 时，available=false。
/// `connection` 是当前设备连接类型（"usb"/"receiver"/"bluetooth"），
/// 用于 #3 连接类型能力分支筛选。
fn plugin_capabilities(
    inspection: &PackageInspection,
    outputs: &BTreeMap<String, serde_json::Value>,
    connection: &str,
) -> Vec<PluginCapability> {
    let normalized_connection = normalize_connection(connection);
    inspection
        .capabilities
        .iter()
        .map(|capability| {
            // 能力动态协商：根据 probe 声明检查 workflow 输出。
            // probe 引用的字段值为 0 → 设备不支持该能力 → available=false。
            // 无 probe 声明 → 默认 available=true（向后兼容）。
            // 修复 #1：支持整数和浮点（0/0.0 均视为不支持），非数字或字段缺失默认可用。
            let probe_available = capability.probe.as_ref().map_or(true, |probe| {
                match outputs.get(&probe.output) {
                    None => {
                        // probe 引用的 output 不存在：产生该 output 的 workflow 未执行
                        //（可能因 skip_if_zero 被跳过，说明设备不支持该能力）。
                        // fail-closed 标记不可用，避免 false-positive。
                        false
                    }
                    Some(value) => value
                        .as_object()
                        .and_then(|object| object.get(&probe.field))
                        .map(|field_value| {
                            field_value
                                .as_u64()
                                .map(|v| v != 0)
                                .or_else(|| field_value.as_f64().map(|v| v != 0.0))
                                .unwrap_or(true)
                        })
                        .unwrap_or(true),
                }
            });
            // #3 连接类型能力分支：归一化后比较，兼容 "wireless"/"wireless-receiver" 别名。
            let connection_available = capability.connections.as_ref().map_or(true, |allowed| {
                allowed
                    .iter()
                    .any(|conn| normalize_connection(conn) == normalized_connection)
            });
            let firmware_available =
                firmware_available(outputs, capability.min_firmware.as_deref());
            PluginCapability {
                id: capability.id.clone(),
                control: capability.control.as_str().into(),
                label_key: capability.label_key.clone(),
                read_only: capability.read_only,
                placements: capability
                    .placements
                    .iter()
                    .map(|placement| PluginCapabilityPlacement {
                        region: placement.region.as_str().into(),
                        group: placement.group.clone(),
                        order: placement.order,
                        span: placement.span,
                        icon: placement.icon.clone(),
                    })
                    .collect(),
                metadata: capability.metadata.clone(),
                available: probe_available && connection_available && firmware_available,
                connections: capability.connections.clone(),
                min_firmware: capability.min_firmware.clone(),
            }
        })
        .collect()
}

/// 托盘菜单签名：捕获影响菜单结构的所有字段。
/// 签名相同时跳过菜单重建，仅更新 title/tooltip。
#[derive(Clone, PartialEq)]
struct TrayMenuSignature {
    /// 是否有设备连接（None = 未连接，Some = 已连接）
    connected: bool,
    /// 电池信息列表：(label, percentage, charging)
    batteries: Vec<(String, u8, bool)>,
    /// 是否显示连接状态菜单项
    show_connection: bool,
    /// 连接标签（如 "USB"、"无线"）
    connection_label: String,
    /// 设备显示名
    display_name: String,
}

#[derive(Default)]
struct SessionState {
    write_in_progress: Mutex<bool>,
    /// 与 `write_in_progress` 配对，让并发写入排队等待而非立即失败。
    write_cond: Condvar,
    device_io: Mutex<()>,
    /// 缓存所有已连接设备的快照，按 HID 路径索引。
    /// 支持多设备并行管理：每轮轮询更新对应设备的快照，
    /// 断开的设备从 map 中移除。`device_snapshot` 命令返回 primary 设备。
    last_snapshot: Mutex<BTreeMap<String, DeviceSnapshot>>,
    plugins: Mutex<Option<CachedPlugins>>,
    tray_icon_level: Mutex<Option<i16>>,
    tray_is_charging: Mutex<Option<bool>>,
    tray_uses_dark: Mutex<Option<bool>>,
    /// 缓存托盘菜单签名，避免每轮轮询都重建菜单（仅图标做了 diff）。
    /// 签名相同时跳过菜单重建，仅更新 title/tooltip（轻量文本操作）。
    tray_menu_signature: Mutex<Option<TrayMenuSignature>>,
    /// 缓存系统主题检测结果，避免每次 update_tray 都 fork 进程。
    /// 由 ThemeChanged 事件和窗口聚焦事件更新；首次读取时 tray_theme_is_dark 会兜底检测。
    system_dark: Mutex<Option<bool>>,
    /// Channel used to wake the background reader thread for an immediate refresh.
    /// Send `()` to trigger a read; the reader drains pending signals before reading
    /// to avoid redundant work when multiple events fire in quick succession.
    refresh_tx: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    applied_software_profiles: Mutex<BTreeSet<String>>,
    /// 低电量跨阈值检测：仅在电量从高跨入低时通知一次，避免反复弹窗。
    low_battery: Mutex<LowBatteryCrossing>,
    /// 状态变化后的快速轮询剩余次数。
    /// 检测到设备插拔、充电状态变化等事件后，进入短暂的 500ms 快速轮询窗口，
    /// 在不持续高频轮询的前提下，尽快捕获状态变化的收尾（例如充电结束）。
    settling_polls: Mutex<u8>,
    /// 缓存应用设置，避免每次轮询都读磁盘。
    /// 由 settings_set 命令和首次加载时更新。
    cached_settings: Mutex<Option<AppSettings>>,
    /// 缓存 HidApi 实例，避免每次设备读取/写入都重新枚举所有 HID 设备。
    /// 调用方负责在持锁期间完成 HID 操作；device_io 锁已序列化大部分访问。
    cached_hidapi: Mutex<Option<HidApi>>,
    /// 缓存 inspect_bundled_plugins 结果。内置插件在构建时打包，运行时不变，
    /// 因此只需计算一次（SHA-256 + 签名验证）。
    cached_bundled_plugins: Mutex<Option<Vec<BundledPluginInfo>>>,
    /// 缓存 load_software_profiles 结果。由 save_software_profiles 写入后失效。
    cached_software_profiles: Mutex<Option<SoftwareProfileStore>>,
    /// 缓存 ProtocolPackage 解析结果，按 plugin_id 索引。
    /// 每次 read_device_once / device_mutate_blocking 都会调用 from_files，
    /// 而 JSON 文件内容在插件加载后不变，缓存可避免重复解析。
    /// 由插件加载（启动 / install_plugin_update）清空。
    cached_packages: Mutex<HashMap<String, Arc<ProtocolPackage>>>,
    /// #9 防抖式 TTL 缓存：记录每个设备最近一次 HID 读取的时间戳（按 HID 路径索引）。
    /// 非 settling 状态下，500ms 内的重复读取复用 last_snapshot，跳过 HID 往返。
    /// 写入后（device_mutate_blocking）和设备断开（clear_snapshots）时清除。
    last_read_at: Mutex<HashMap<String, Instant>>,
    /// 缓存 logitech-hidpp feature index 查询结果，按设备路径索引。
    /// feature index 在设备连接期间不变，缓存命中时跳过 root-get-feature 的 HID 往返。
    /// 设备断开时由 clear_snapshots 清空。
    feature_index_cache: Mutex<FeatureIndexCache>,
    /// 缓存 onboard memory 读取结果，按设备路径索引。
    /// 写入 mutation 预读时命中缓存则跳过 16 chunk HID 往返；验证读后更新缓存。
    /// 设备断开时由 clear_snapshots 清空。
    onboard_memory_cache: Mutex<OnboardMemoryCache>,
    /// debug 构建中记录上一次 matched device 数量，仅在变化时输出日志。
    #[cfg(debug_assertions)]
    last_matched_count: Mutex<usize>,
}

/// Send an immediate-refresh signal to the background reader thread.
/// No-op if the reader has not been spawned yet.
fn request_refresh(state: &SessionState) {
    if let Ok(tx) = state.refresh_tx.lock() {
        if let Some(sender) = tx.as_ref() {
            let _ = sender.send(());
        }
    }
}

/// 解析或复用插件对应的 ProtocolPackage。
///
/// 插件文件在加载后内容不变，重复解析 JSON 会浪费 CPU。缓存按 `plugin_id::model`
/// 索引（型号覆盖加载 #2：不同型号产生不同的合并结果），在插件加载
/// （启动 / install_plugin_update）时整体清空。
fn get_or_parse_package(
    state: &SessionState,
    inspection: &PackageInspection,
    model: Option<&str>,
    files: &BTreeMap<String, Vec<u8>>,
    plugins: &CachedPlugins,
) -> Result<Arc<ProtocolPackage>, String> {
    // 缓存键：plugin_id + "::" + model，确保不同型号的合并结果独立缓存。
    let cache_key = match model {
        Some(m) if !m.is_empty() => format!("{}::{m}", inspection.plugin_id),
        _ => inspection.plugin_id.to_string(),
    };
    let dependencies = dependency_transport_files(inspection, plugins)?;
    if let Ok(mut cache) = state.cached_packages.lock() {
        if let Some(package) = cache.get(&cache_key) {
            return Ok(package.clone());
        }
        let package = Arc::new(ProtocolPackage::from_files_with_model_and_dependencies(
            files,
            model,
            &dependencies,
        )?);
        cache.insert(cache_key, package.clone());
        return Ok(package);
    }
    // 锁失败（中毒）—— 直接解析，不缓存。
    Ok(Arc::new(
        ProtocolPackage::from_files_with_model_and_dependencies(files, model, &dependencies)?,
    ))
}

/// 清空 ProtocolPackage 缓存。在插件集合变化时调用。
fn invalidate_package_cache(state: &SessionState) {
    if let Ok(mut cache) = state.cached_packages.lock() {
        cache.clear();
    }
}

fn dependency_transport_files<'a>(
    inspection: &PackageInspection,
    plugins: &'a CachedPlugins,
) -> Result<Vec<&'a BTreeMap<String, Vec<u8>>>, String> {
    // 所有依赖（无论 reuseTransport 是否为 true）都校验存在性与版本要求；
    // 仅 reuseTransport=true 时才收集被依赖插件的 transports.json。
    // 当前实现不递归解析依赖的依赖（roadmap #12 仅承诺直接依赖的 transport 复用），
    // 因此不会出现循环依赖导致的无限递归；若未来支持传递依赖，需在此增加环检测。
    let mut transport_files = Vec::new();
    for dependency in &inspection.depends_on {
        let (found, _, files) = plugins
            .iter()
            .find(|(candidate, _, _)| candidate.plugin_id == dependency.plugin_id)
            .ok_or_else(|| {
                format!(
                    "plugin {} depends on missing plugin {}",
                    inspection.plugin_id, dependency.plugin_id
                )
            })?;
        if let Some(requirement) = &dependency.version {
            let requirement = semver::VersionReq::parse(requirement)
                .map_err(|error| format!("invalid dependency version requirement: {error}"))?;
            let version = semver::Version::parse(&found.version)
                .map_err(|error| format!("invalid dependency version: {error}"))?;
            if !requirement.matches(&version) {
                return Err(format!(
                    "plugin {} dependency {} version {} does not satisfy {}",
                    inspection.plugin_id, found.plugin_id, found.version, requirement
                ));
            }
        }
        if dependency.reuse_transport {
            transport_files.push(files);
        }
    }
    Ok(transport_files)
}

/// Number of fast polls performed after a state transition is detected.
/// At 500 ms per poll this covers a 3-second settling window.
const SETTLING_POLL_COUNT: u8 = 6;

/// #9 防抖式 TTL：非 settling 状态下，500ms 内的重复读取复用快照。
/// 防止窗口聚焦、托盘点击等短时间多次 RefreshNow 信号触发重复 HID 往返。
const READ_DEBOUNCE_TTL: Duration = Duration::from_millis(500);

/// Mark that the device state just changed, enabling a short burst of fast polls.
/// This is used for plug/unplug, charging state changes, and after device writes
/// so the UI catches the tail end of the transition without continuous polling.
fn note_state_change(state: &SessionState) {
    if let Ok(mut polls) = state.settling_polls.lock() {
        *polls = SETTLING_POLL_COUNT;
    }
}

/// 从多设备快照 map 中选择 primary 设备。
/// 优先返回真正开放写入的设备，否则返回第一个。
fn primary_snapshot(snapshots: &BTreeMap<String, DeviceSnapshot>) -> Option<&DeviceSnapshot> {
    primary_snapshot_entry(snapshots).map(|(_, snapshot)| snapshot)
}

fn primary_snapshot_entry(
    snapshots: &BTreeMap<String, DeviceSnapshot>,
) -> Option<(&String, &DeviceSnapshot)> {
    snapshots
        .iter()
        .find(|(_, snapshot)| snapshot_allows_writes(snapshot))
        .or_else(|| snapshots.iter().next())
}

/// 替换整个快照 map，并在变化时触发 settling burst。
fn store_snapshots(state: &SessionState, snapshots: BTreeMap<String, DeviceSnapshot>) {
    let mut guard = state
        .last_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let changed = *guard != snapshots;
    *guard = snapshots;
    drop(guard);
    if changed {
        note_state_change(state);
    }
}

/// 更新单个设备的快照，避免 clone 整个 map 引发的读-改-写竞态。
/// 修复 #10：device_mutate_blocking 写入后只更新当前设备，不覆盖其他设备的快照。
fn store_snapshot(state: &SessionState, device_path: String, snapshot: DeviceSnapshot) {
    let mut guard = state
        .last_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let changed = guard.get(&device_path) != Some(&snapshot);
    guard.insert(device_path, snapshot);
    drop(guard);
    if changed {
        note_state_change(state);
    }
}

/// 清空所有快照，并在变化时触发 settling burst。
fn clear_snapshots(state: &SessionState) {
    let mut guard = state
        .last_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let changed = !guard.is_empty();
    guard.clear();
    drop(guard);
    // #9 设备断开时清除 TTL 缓存。
    if let Ok(mut cache) = state.last_read_at.lock() {
        cache.clear();
    }
    // P1 设备断开时清除 feature index 缓存，确保重连后重新查询。
    if let Ok(mut cache) = state.feature_index_cache.lock() {
        cache.clear();
    }
    // UX3 设备断开时清除 onboard memory 缓存，避免重连后使用过期数据。
    if let Ok(mut cache) = state.onboard_memory_cache.lock() {
        cache.clear();
    }
    if changed {
        note_state_change(state);
    }
}

// Production plugin signing key for hello-yunshu/mira-mouse-plugins.
// Replace with the real key id and public key after the first production release.
const PRODUCTION_KEY_ID: &str = "mira-plugins-2026-001";
const PRODUCTION_PUBLIC_KEY_HEX: &str =
    "eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3";

// Test packages are trusted only in debug builds for local development.
// Remove this before a production release.
#[cfg(debug_assertions)]
const TEST_KEY_ID: &str = "TEST-ONLY-mira-plugins";
#[cfg(debug_assertions)]
const TEST_PUBLIC_KEY_HEX: &str =
    "00d34dac6e039baada3d3d9aa65390f2887d09d73b396af8434ecb29c233d666";

fn decode_key(hex_str: &str) -> VerifyingKey {
    let bytes = hex::decode(hex_str).expect("invalid hex in pubkey");
    let array: [u8; 32] = bytes.try_into().expect("pubkey must be 32 bytes");
    VerifyingKey::from_bytes(&array).expect("invalid ed25519 pubkey")
}

fn production_trust_store() -> TrustStore {
    let mut trust = TrustStore::default();
    trust.0.insert(
        PRODUCTION_KEY_ID.to_string(),
        decode_key(PRODUCTION_PUBLIC_KEY_HEX),
    );
    // Test packages are trusted only in debug builds for local development.
    // Release builds must not trust the test key, otherwise anyone with the
    // test private key could sign plugins that production users would accept.
    #[cfg(debug_assertions)]
    trust
        .0
        .insert(TEST_KEY_ID.to_string(), decode_key(TEST_PUBLIC_KEY_HEX));
    trust
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AboutInfo {
    name: String,
    version: String,
    identifier: String,
    platform: String,
    architecture: String,
    rust_version: String,
    build_date: String,
    git_commit: String,
    bundled_plugins: Vec<BundledPluginInfo>,
    contact: ContactLinks,
    updater_active: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundledPluginInfo {
    plugin_id: String,
    version: String,
    asset: String,
    sha256: String,
    publisher_key_id: String,
    release_tag: String,
    bundle_by_default: bool,
    signature_verified: bool,
    evidence: String,
    source: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ContactLinks {
    github: Option<String>,
    repository: Option<String>,
    x: Option<String>,
    telegram: Option<String>,
    developer_name: Option<String>,
    copyright: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    language: String,
    theme: String,
    autostart: bool,
    start_hidden: bool,
    tray_show_battery_title: bool,
    tray_include_receiver_battery: bool,
    tray_show_connection: bool,
    /// 托盘鼠标图标颜色： "white"（白色轮廓）、"black"（黑色轮廓）、"auto"（跟随系统主题）。
    /// 默认 "white"，不自动跟随主题。
    tray_icon_color: String,
    low_battery_threshold: u8,
    night_mode_enabled: bool,
    night_mode_start: String,
    night_mode_end: String,
    refresh_interval_seconds: u16,
    telemetry_disabled: bool,
    automatic_update_checks: bool,
    automatic_update_install: bool,
    automatic_plugin_update_checks: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SoftwareProfileStore {
    schema_version: u32,
    devices: BTreeMap<String, SoftwareProfile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SoftwareProfile {
    mutations: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            theme: "system".into(),
            autostart: false,
            start_hidden: false,
            tray_show_battery_title: true,
            tray_include_receiver_battery: false,
            tray_show_connection: true,
            tray_icon_color: "white".into(),
            low_battery_threshold: 20,
            night_mode_enabled: false,
            night_mode_start: "22:00".into(),
            night_mode_end: "07:00".into(),
            refresh_interval_seconds: 5,
            telemetry_disabled: true,
            automatic_update_checks: true,
            automatic_update_install: false,
            automatic_plugin_update_checks: true,
        }
    }
}

impl AppSettings {
    fn normalized(mut self) -> Self {
        let defaults = Self::default();
        if !matches!(self.language.as_str(), "auto" | "zh-CN" | "en") {
            self.language = defaults.language;
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = defaults.theme;
        }
        if !matches!(self.tray_icon_color.as_str(), "white" | "black" | "auto") {
            self.tray_icon_color = defaults.tray_icon_color;
        }
        if !(5..=50).contains(&self.low_battery_threshold) {
            self.low_battery_threshold = defaults.low_battery_threshold;
        }
        if !(1..=60).contains(&self.refresh_interval_seconds) {
            self.refresh_interval_seconds = defaults.refresh_interval_seconds;
        }
        if !is_clock_time(&self.night_mode_start) {
            self.night_mode_start = defaults.night_mode_start;
        }
        if !is_clock_time(&self.night_mode_end) {
            self.night_mode_end = defaults.night_mode_end;
        }
        self.telemetry_disabled = true;
        self
    }
}

fn is_clock_time(value: &str) -> bool {
    let Some((hour, minute)) = value.split_once(':') else {
        return false;
    };
    hour.len() == 2
        && minute.len() == 2
        && hour.parse::<u8>().is_ok_and(|value| value < 24)
        && minute.parse::<u8>().is_ok_and(|value| value < 60)
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn defaults_match_the_frontend_contract() {
        let settings = AppSettings::default();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.low_battery_threshold, 20);
        assert_eq!(settings.refresh_interval_seconds, 5);
        assert!(settings.telemetry_disabled);
        assert!(!settings.start_hidden);
        assert!(settings.automatic_update_checks);
        assert!(!settings.automatic_update_install);
        assert!(settings.automatic_plugin_update_checks);
    }

    #[test]
    fn invalid_saved_values_are_repaired() {
        let settings = AppSettings {
            theme: String::new(),
            tray_icon_color: "blue".into(),
            low_battery_threshold: 0,
            refresh_interval_seconds: 0,
            night_mode_start: "99:99".into(),
            night_mode_end: String::new(),
            telemetry_disabled: false,
            ..AppSettings::default()
        }
        .normalized();
        assert_eq!(settings, AppSettings::default());
    }

    #[test]
    fn seeds_standard_values_for_software_profile_reapply() {
        let reading = DeviceReading {
            dpi: Some(1850),
            dpi_stages: Some(vec![mira_core::DpiStage {
                value: 1850,
                color: "#9a8bd0".into(),
                enabled: true,
                active: true,
            }]),
            polling_rate_hz: Some(1000),
            capabilities: BTreeMap::from([("controlMode".into(), serde_json::json!({"mode": 2}))]),
            ..DeviceReading::default()
        };
        assert_eq!(control_mode(&reading), Some(2));
        let mut profile = SoftwareProfile::default();
        seed_standard_software_mutations(
            &mut profile,
            &reading,
            &["set-dpi-value".into(), "set-polling-rate".into()],
        );
        assert_eq!(profile.mutations["set-dpi-value"]["dpi"], 1850);
        assert_eq!(profile.mutations["set-polling-rate"]["rate"], 1000);
    }

    #[test]
    fn default_tray_icon_is_decodable_and_transparent() {
        let icon = tauri::image::Image::from_bytes(tray_app_icon_bytes()).unwrap();
        assert_eq!((icon.width(), icon.height()), (64, 64));

        let mut alpha = icon.rgba().iter().skip(3).step_by(4);
        assert!(alpha.clone().any(|value| *value == 0));
        assert!(alpha.any(|value| *value > 0));
    }

    #[test]
    fn device_writes_queue_and_release_after_completion() {
        let state = SessionState::default();
        std::thread::scope(|s| {
            let guard = begin_device_write(&state).unwrap();
            let handle = s.spawn(|| {
                // 并发写入排队等待，而非立即失败。
                let _queued = begin_device_write(&state).unwrap();
            });
            // 给排队线程一点时间进入等待。
            std::thread::sleep(Duration::from_millis(50));
            drop(guard);
            // guard 释放后，排队的写入应能获取锁并完成。
            handle.join().unwrap();
        });
    }

    #[test]
    fn tray_title_uses_primary_and_optional_receiver_batteries() {
        let mut snapshot = DeviceSnapshot {
            display_name: "AM INFINITY 8K MOUSE".into(),
            connection: mira_core::Connection::Wireless,
            battery_percent: Some(64),
            charging: false,
            batteries: vec![
                mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: "鼠标".into(),
                    percentage: 64,
                    charging: false,
                },
                mira_core::DeviceBattery {
                    id: "receiver".into(),
                    label: "接收器".into(),
                    percentage: 100,
                    charging: false,
                },
            ],
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
        };
        let mut settings = AppSettings::default();
        assert_eq!(battery_title(&snapshot, &settings).as_deref(), Some("64%"));
        assert!(!mouse_battery_charging(&snapshot));
        snapshot.batteries[0].charging = true;
        assert!(mouse_battery_charging(&snapshot));
        settings.tray_include_receiver_battery = true;
        assert_eq!(
            battery_title(&snapshot, &settings).as_deref(),
            Some("鼠 64% · 接 100%")
        );
        settings.tray_show_battery_title = false;
        assert_eq!(battery_title(&snapshot, &settings), None);
    }

    fn primary_test_snapshot(
        name: &str,
        evidence: &str,
        readonly: bool,
        writable_mutations: Vec<String>,
    ) -> DeviceSnapshot {
        DeviceSnapshot {
            display_name: name.into(),
            connection: mira_core::Connection::Wireless,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: Default::default(),
            plugin_capabilities: Vec::new(),
            writable_mutations,
            evidence: evidence.into(),
            readonly,
        }
    }

    #[test]
    fn primary_snapshot_prefers_truly_writable_device() {
        let snapshots = BTreeMap::from([
            (
                "a".into(),
                primary_test_snapshot(
                    "readonly verified",
                    "hardware-verified",
                    true,
                    vec!["set-dpi".into()],
                ),
            ),
            (
                "b".into(),
                primary_test_snapshot(
                    "writable verified",
                    "hardware-verified",
                    false,
                    vec!["set-dpi".into()],
                ),
            ),
        ]);
        assert_eq!(
            primary_snapshot(&snapshots).map(|snapshot| snapshot.display_name.as_str()),
            Some("writable verified")
        );
    }

    #[test]
    fn primary_snapshot_falls_back_when_no_device_can_write() {
        let snapshots = BTreeMap::from([
            (
                "a".into(),
                primary_test_snapshot("first readonly", "hardware-verified", true, Vec::new()),
            ),
            (
                "b".into(),
                primary_test_snapshot("source only", "source-confirmed", false, Vec::new()),
            ),
        ]);
        assert_eq!(
            primary_snapshot(&snapshots).map(|snapshot| snapshot.display_name.as_str()),
            Some("first readonly")
        );
    }

    #[test]
    fn plugin_updates_only_offer_newer_semver() {
        let current = BTreeMap::from([
            ("mira.amaster".into(), "1.3.3".into()),
            ("mira.logitech-hidpp".into(), "0.6.1".into()),
        ]);
        let registry = PluginRegistry {
            schema_version: 1,
            plugins: vec![
                PluginRegistryEntry {
                    plugin_id: "mira.amaster".into(),
                    version: "1.3.5".into(),
                    release_tag: "plugin/amaster/v1.3.5".into(),
                    url: "https://github.com/hello-yunshu/mira-mouse-plugins/releases/download/test/amaster.mira-plugin".into(),
                    sha256: "0".repeat(64),
                    publisher_key_id: PRODUCTION_KEY_ID.into(),
                    notes: None,
                },
                PluginRegistryEntry {
                    plugin_id: "mira.logitech-hidpp".into(),
                    version: "0.6.1".into(),
                    release_tag: "plugin/logitech-hidpp/v0.6.1".into(),
                    url: "https://github.com/hello-yunshu/mira-mouse-plugins/releases/download/test/logitech.mira-plugin".into(),
                    sha256: "1".repeat(64),
                    publisher_key_id: PRODUCTION_KEY_ID.into(),
                    notes: None,
                },
            ],
        };
        let updates = plugin_updates_for_versions(&current, registry).unwrap();
        assert!(
            updates
                .iter()
                .find(|item| item.plugin_id == "mira.amaster")
                .unwrap()
                .update_available
        );
        assert!(
            !updates
                .iter()
                .find(|item| item.plugin_id == "mira.logitech-hidpp")
                .unwrap()
                .update_available
        );
    }

    #[test]
    fn exportable_value_reads_capability_source() {
        let snapshot = DeviceSnapshot {
            display_name: "Test Mouse".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: Some(1600),
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: vec![PluginCapability {
                id: "dpi".into(),
                control: "Number".into(),
                label_key: "capability.dpi".into(),
                read_only: false,
                placements: Vec::new(),
                metadata: BTreeMap::from([(
                    "source".into(),
                    serde_json::Value::String("dpi".into()),
                )]),
                available: true,
                connections: None,
                min_firmware: None,
            }],
            writable_mutations: Vec::new(),
            evidence: "hardware-verified".into(),
            readonly: false,
        };
        assert_eq!(
            exportable_value(
                &snapshot,
                &ExportableField {
                    id: "dpi".into(),
                    export_key: "dpi".into(),
                    kind: None,
                    mutation: None,
                    param: None,
                    source: None,
                    sources: None,
                }
            ),
            Some(serde_json::json!(1600))
        );
    }

    #[test]
    fn mutation_for_exportable_uses_capability_metadata() {
        let capability = mira_plugin_runtime::Capability {
            id: "dpi".into(),
            control: mira_plugin_runtime::Control::Number,
            label_key: "capability.dpi".into(),
            read_only: false,
            placements: Vec::new(),
            metadata: BTreeMap::from([
                ("mutation".into(), serde_json::json!("set-dpi-value")),
                ("param".into(), serde_json::json!("dpi")),
            ]),
            probe: None,
            connections: None,
            min_firmware: None,
        };
        let inspection = PackageInspection {
            plugin_id: "test.plugin".into(),
            version: "1.0.0".into(),
            evidence: "hardware-verified".into(),
            signature_verified: true,
            writes_enabled: true,
            capabilities: vec![capability],
            exportable_fields: vec![],
            depends_on: vec![],
            file_count: 0,
        };
        assert_eq!(
            mutation_for_exportable(
                &inspection,
                &ExportableField {
                    id: "dpi".into(),
                    export_key: "dpi".into(),
                    kind: None,
                    mutation: None,
                    param: None,
                    source: None,
                    sources: None,
                }
            ),
            ("set-dpi-value".into(), "dpi".into())
        );
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use mira_plugin_runtime::{Capability, CapabilityProbe, Control};

    fn make_capability(
        id: &str,
        probe: Option<CapabilityProbe>,
        connections: Option<Vec<String>>,
    ) -> Capability {
        Capability {
            id: id.into(),
            control: Control::Toggle,
            label_key: format!("{id}.label"),
            read_only: false,
            placements: Vec::new(),
            metadata: BTreeMap::new(),
            probe,
            connections,
            min_firmware: None,
        }
    }

    fn make_inspection(capabilities: Vec<Capability>) -> PackageInspection {
        PackageInspection {
            plugin_id: "test.plugin".into(),
            version: "1.0.0".into(),
            evidence: "hardware-verified".into(),
            signature_verified: true,
            writes_enabled: true,
            capabilities,
            exportable_fields: vec![],
            depends_on: vec![],
            file_count: 0,
        }
    }

    #[test]
    fn normalize_connection_maps_aliases() {
        // 修复 #3：wireless/wireless-receiver 应归一化为 receiver
        assert_eq!(normalize_connection("wireless"), "receiver");
        assert_eq!(normalize_connection("wireless-receiver"), "receiver");
        assert_eq!(normalize_connection("usb"), "usb");
        assert_eq!(normalize_connection("bluetooth"), "bluetooth");
        assert_eq!(normalize_connection("unknown"), "unknown");
    }

    #[test]
    fn firmware_gate_marks_capability_unavailable_below_minimum() {
        let mut cap = make_capability("advanced", None, None);
        cap.min_firmware = Some("2.0.0".into());
        let inspection = make_inspection(vec![cap]);
        let outputs =
            BTreeMap::from([("firmware".into(), serde_json::json!({"version": "1.9.9"}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(!result[0].available);
    }

    #[test]
    fn firmware_gate_accepts_matching_version() {
        let mut cap = make_capability("advanced", None, None);
        cap.min_firmware = Some("2.0.0".into());
        let inspection = make_inspection(vec![cap]);
        let outputs =
            BTreeMap::from([("firmware".into(), serde_json::json!({"version": "2.1.0"}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(result[0].available);
    }

    #[test]
    fn capability_filter_uses_runtime_reported_connection() {
        let cap = make_capability("wired-only", None, Some(vec!["usb".into()]));
        let inspection = make_inspection(vec![cap]);
        let devices = hid::DevicesFile {
            schema_version: 1,
            devices: Vec::new(),
            hardware_verified_models: Vec::new(),
        };
        let device = hid::MatchedDevice {
            plugin_id: "test.plugin".into(),
            family: "mouse".into(),
            evidence: "hardware-verified".into(),
            connection: "wireless".into(),
            path: "test-path".into(),
            vendor_id: 1,
            product_id: 2,
            usage_page: 3,
            usage: 4,
            model: None,
        };
        let reading = DeviceReading {
            connection: Some(ConnectionKind::Usb),
            ..DeviceReading::default()
        };

        let snapshot = build_device_snapshot(
            reading,
            &inspection,
            &devices,
            &device,
            mira_core::Connection::Wireless,
            Vec::new(),
            None,
        );

        assert_eq!(snapshot.connection, mira_core::Connection::Usb);
        assert!(snapshot.plugin_capabilities[0].available);
    }

    #[test]
    fn probe_zero_integer_marks_unavailable() {
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({"value": 0}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(!result[0].available);
    }

    #[test]
    fn probe_zero_float_marks_unavailable() {
        // 修复 #1：浮点 0.0 也应标记为不可用
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({"value": 0.0}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(!result[0].available);
    }

    #[test]
    fn probe_nonzero_marks_available() {
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({"value": 1600}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(result[0].available);
    }

    #[test]
    fn probe_missing_output_marks_unavailable() {
        // probe 引用的 output 整个不存在（workflow 未执行/被 skip_if_zero 跳过）：
        // fail-closed 标记不可用，避免设备不支持却显示为可用。
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(!result[0].available);
    }

    #[test]
    fn probe_missing_field_defaults_available() {
        // 向后兼容：output 存在但字段缺失时默认可用
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(result[0].available);
    }

    #[test]
    fn no_probe_defaults_available() {
        let cap = make_capability("dpi", None, None);
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(result[0].available);
    }

    #[test]
    fn connections_receiver_alias_matches_wireless() {
        // 修复 #3：声明 ["receiver"] 应匹配 "wireless" 连接
        let cap = make_capability("lighting", None, Some(vec!["receiver".into()]));
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "wireless");
        assert!(result[0].available);
    }

    #[test]
    fn connections_receiver_alias_matches_wireless_receiver() {
        let cap = make_capability("lighting", None, Some(vec!["receiver".into()]));
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "wireless-receiver");
        assert!(result[0].available);
    }

    #[test]
    fn connections_mismatch_marks_unavailable() {
        let cap = make_capability("lighting", None, Some(vec!["usb".into()]));
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "wireless");
        assert!(!result[0].available);
    }

    #[test]
    fn connections_multiple_matches() {
        let cap = make_capability(
            "lighting",
            None,
            Some(vec!["receiver".into(), "usb".into()]),
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "usb");
        assert!(result[0].available);
    }

    #[test]
    fn no_connections_defaults_available() {
        let cap = make_capability("lighting", None, None);
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "wireless");
        assert!(result[0].available);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockFile {
    #[allow(dead_code)]
    schema_version: u32,
    release_ready: bool,
    plugins: Vec<LockPlugin>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockPlugin {
    plugin_id: String,
    #[allow(dead_code)]
    repository: String,
    release_tag: String,
    version: String,
    asset: String,
    sha256: String,
    publisher_key_id: String,
    #[allow(dead_code)]
    plugin_api: String,
    bundle_by_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginRegistry {
    schema_version: u32,
    plugins: Vec<PluginRegistryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginRegistryEntry {
    plugin_id: String,
    version: String,
    release_tag: String,
    url: String,
    sha256: String,
    publisher_key_id: String,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginUpdateInfo {
    plugin_id: String,
    current_version: String,
    available_version: Option<String>,
    release_tag: Option<String>,
    notes: Option<String>,
    update_available: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginInstallResult {
    plugin_id: String,
    version: String,
    previous_version: String,
    restarted_runtime: bool,
}

fn fetch_bounded(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let response = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("Mira-Mouse-Updater")
        .build()
        .map_err(|error| format!("build HTTP client: {error}"))?
        .get(url)
        .send()
        .map_err(|error| format!("download {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("download {url}: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(format!("download exceeds {max_bytes} byte limit"));
    }
    let mut bytes = Vec::new();
    response
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read download: {error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("download exceeds {max_bytes} byte limit"));
    }
    Ok(bytes)
}

fn fetch_plugin_registry() -> Result<PluginRegistry, String> {
    let bytes = fetch_bounded(PLUGIN_REGISTRY_URL, MAX_REGISTRY_BYTES)?;
    let registry: PluginRegistry = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse plugin registry: {error}"))?;
    if registry.schema_version != 1 {
        return Err(format!(
            "unsupported plugin registry schema {}",
            registry.schema_version
        ));
    }
    Ok(registry)
}

fn plugin_updates_for_versions(
    current: &BTreeMap<String, String>,
    registry: PluginRegistry,
) -> Result<Vec<PluginUpdateInfo>, String> {
    let remote = registry
        .plugins
        .into_iter()
        .map(|entry| (entry.plugin_id.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    current
        .iter()
        .map(|(plugin_id, current_version)| {
            let current_semver = semver::Version::parse(current_version)
                .map_err(|error| format!("invalid installed version for {plugin_id}: {error}"))?;
            let candidate = remote.get(plugin_id);
            let update_available = candidate
                .map(|entry| {
                    semver::Version::parse(&entry.version)
                        .map(|version| version > current_semver)
                        .map_err(|error| {
                            format!("invalid registry version for {plugin_id}: {error}")
                        })
                })
                .transpose()?
                .unwrap_or(false);
            Ok(PluginUpdateInfo {
                plugin_id: plugin_id.clone(),
                current_version: current_version.clone(),
                available_version: candidate.map(|entry| entry.version.clone()),
                release_tag: candidate.map(|entry| entry.release_tag.clone()),
                notes: candidate.and_then(|entry| entry.notes.clone()),
                update_available,
            })
        })
        .collect()
}

fn installed_plugins_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("plugins"))
        .map_err(|error| format!("resolve plugin data directory: {error}"))
}

fn read_lock_file() -> Option<LockFile> {
    serde_json::from_slice(include_bytes!("../../plugins.lock.json")).ok()
}

fn bundled_plugins_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("resources").join("plugins"))
        .filter(|dir| dir.exists())
        .or_else(|| {
            let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/plugins");
            if local.exists() {
                Some(local)
            } else {
                None
            }
        })
}

fn inspect_bundled_plugins(app: &AppHandle, trust: &TrustStore) -> Vec<BundledPluginInfo> {
    let Some(lock) = read_lock_file() else {
        return Vec::new();
    };
    let Some(plugins_dir) = bundled_plugins_dir(app) else {
        return Vec::new();
    };
    let trust_store = production_trust_store();
    let combined_trust = if trust.0.is_empty() {
        &trust_store
    } else {
        trust
    };
    lock.plugins
        .into_iter()
        .filter(|plugin| plugin.bundle_by_default)
        .filter_map(|plugin| {
            let asset_path = plugins_dir.join(&plugin.asset);
            let bytes = fs::read(&asset_path).ok()?;
            let actual_sha = hex::encode(Sha256::digest(&bytes));
            let signature_verified = if actual_sha == plugin.sha256 {
                inspect_package(std::io::Cursor::new(&bytes), combined_trust, true)
                    .map(|inspection| inspection.signature_verified)
                    .unwrap_or(false)
            } else {
                false
            };
            Some(BundledPluginInfo {
                plugin_id: plugin.plugin_id,
                version: plugin.version,
                asset: plugin.asset,
                sha256: plugin.sha256,
                publisher_key_id: plugin.publisher_key_id,
                release_tag: plugin.release_tag,
                bundle_by_default: plugin.bundle_by_default,
                signature_verified,
                evidence: if signature_verified {
                    "signature-verified".to_string()
                } else {
                    "signature-unverified".to_string()
                },
                source: "bundled".to_string(),
            })
        })
        .collect()
}

/// Return cached bundled plugin info, computing it once on first access.
/// Bundled plugins are fixed at build time, so the SHA-256 and signature
/// verification only need to run once for the app's lifetime.
fn cached_bundled_plugins(app: &AppHandle) -> Vec<BundledPluginInfo> {
    let state = app.state::<SessionState>();
    if let Ok(mut cache) = state.cached_bundled_plugins.lock() {
        if let Some(ref plugins) = *cache {
            return plugins.clone();
        }
        let plugins = inspect_bundled_plugins(app, &production_trust_store());
        *cache = Some(plugins.clone());
        return plugins;
    }
    // Lock failed (poisoned) — compute directly without caching.
    inspect_bundled_plugins(app, &production_trust_store())
}

fn active_plugins_info(app: &AppHandle) -> Vec<BundledPluginInfo> {
    let mut info = cached_bundled_plugins(app);
    let Ok(versions) = active_plugin_versions(app) else {
        return info;
    };
    let trust = production_trust_store();
    let Ok(directory) = installed_plugins_dir(app) else {
        return info;
    };
    for (plugin_id, version) in versions {
        if info
            .iter()
            .any(|plugin| plugin.plugin_id == plugin_id && plugin.version == version)
        {
            continue;
        }
        let Some(path) = find_installed_plugin_path(&directory, &plugin_id, &trust) else {
            continue;
        };
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok((inspection, files)) = extract_package(Cursor::new(&bytes), &trust, true) else {
            continue;
        };
        let publisher_key_id = files
            .get("plugin.json")
            .and_then(|manifest| serde_json::from_slice::<serde_json::Value>(manifest).ok())
            .and_then(|manifest| {
                manifest
                    .get("publisherKeyId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let installed = BundledPluginInfo {
            plugin_id: inspection.plugin_id.clone(),
            version: inspection.version,
            asset: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("installed.mira-plugin")
                .to_string(),
            sha256: hex::encode(Sha256::digest(&bytes)),
            publisher_key_id,
            release_tag: String::new(),
            bundle_by_default: false,
            signature_verified: inspection.signature_verified,
            evidence: inspection.evidence,
            source: "installed".to_string(),
        };
        if let Some(index) = info.iter().position(|plugin| plugin.plugin_id == plugin_id) {
            info[index] = installed;
        } else {
            info.push(installed);
        }
    }
    info
}

/// Load all bundled plugin packages that can be verified and extract their
/// `devices.json` descriptors. Used by the HID discovery path.
fn load_bundled_plugin_devices(
    app: &AppHandle,
) -> Vec<(
    mira_plugin_runtime::PackageInspection,
    hid::DevicesFile,
    std::collections::BTreeMap<String, Vec<u8>>,
)> {
    let Some(lock) = read_lock_file() else {
        return Vec::new();
    };
    let Some(plugins_dir) = bundled_plugins_dir(app) else {
        return Vec::new();
    };
    let trust = production_trust_store();
    let mut plugins: CachedPlugins = lock
        .plugins
        .into_iter()
        .filter(|plugin| plugin.bundle_by_default)
        .filter_map(|plugin| {
            let result = (|| -> Result<_, String> {
                let asset_path = plugins_dir.join(&plugin.asset);
                let bytes = fs::read(&asset_path)
                    .map_err(|error| format!("read {}: {error}", asset_path.display()))?;
                let actual_sha = hex::encode(Sha256::digest(&bytes));
                if actual_sha != plugin.sha256 {
                    return Err(format!(
                        "SHA-256 mismatch for {}: expected {}, got {actual_sha}",
                        plugin.asset, plugin.sha256
                    ));
                }
                let (_, files) = extract_package(Cursor::new(&bytes), &trust, true)
                    .map_err(|error| format!("extract {}: {error}", plugin.asset))?;
                let devices_bytes = files
                    .get("devices.json")
                    .ok_or_else(|| format!("{} has no devices.json", plugin.asset))?;
                let devices = hid::parse_devices_json(devices_bytes)?;
                let inspection = inspect_package(Cursor::new(&bytes), &trust, true)
                    .map_err(|error| format!("inspect {}: {error}", plugin.asset))?;
                if inspection.plugin_id != plugin.plugin_id || inspection.version != plugin.version
                {
                    return Err(format!(
                        "identity mismatch for {}: lock has {} {}, package has {} {}",
                        plugin.asset,
                        plugin.plugin_id,
                        plugin.version,
                        inspection.plugin_id,
                        inspection.version
                    ));
                }
                Ok((inspection, devices, files))
            })();
            match result {
                Ok(plugin) => Some(plugin),
                Err(error) => {
                    eprintln!("[mira] plugin load failed: {error}");
                    None
                }
            }
        })
        .collect();

    if let Ok(installed_dir) = installed_plugins_dir(app) {
        if let Ok(entries) = fs::read_dir(&installed_dir) {
            for backup in entries.flatten().map(|entry| entry.path()).filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.contains(".rollback."))
            }) {
                let recovered = fs::read(&backup)
                    .ok()
                    .and_then(|bytes| inspect_package(Cursor::new(bytes), &trust, true).ok());
                if let Some(inspection) = recovered {
                    if find_installed_plugin_path(&installed_dir, &inspection.plugin_id, &trust)
                        .is_none()
                    {
                        let target = installed_dir.join(format!(
                            "{}-{}.mira-plugin",
                            inspection.plugin_id, inspection.version
                        ));
                        if let Err(error) = fs::rename(&backup, target) {
                            eprintln!("[mira] plugin rollback recovery failed: {error}");
                        }
                    }
                }
            }
        }
        if let Ok(entries) = fs::read_dir(installed_dir) {
            for path in entries.flatten().map(|entry| entry.path()).filter(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("mira-plugin")
                    && !path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.contains(".rollback."))
            }) {
                let loaded = (|| -> Result<_, String> {
                    let bytes = fs::read(&path)
                        .map_err(|error| format!("read {}: {error}", path.display()))?;
                    let (inspection, files) = extract_package(Cursor::new(&bytes), &trust, true)
                        .map_err(|error| format!("verify {}: {error}", path.display()))?;
                    let devices = hid::parse_devices_json(
                        files
                            .get("devices.json")
                            .ok_or("installed plugin has no devices.json")?,
                    )?;
                    Ok((inspection, devices, files))
                })();
                match loaded {
                    Ok(installed) => {
                        let installed_version = semver::Version::parse(&installed.0.version).ok();
                        let replace = plugins
                            .iter()
                            .position(|plugin| plugin.0.plugin_id == installed.0.plugin_id)
                            .filter(|index| {
                                let current =
                                    semver::Version::parse(&plugins[*index].0.version).ok();
                                installed_version > current
                            });
                        if let Some(index) = replace {
                            plugins[index] = installed;
                        } else if !plugins
                            .iter()
                            .any(|plugin| plugin.0.plugin_id == installed.0.plugin_id)
                        {
                            plugins.push(installed);
                        }
                    }
                    Err(error) => eprintln!("[mira] installed plugin ignored: {error}"),
                }
            }
        }
    }
    plugins
}

fn load_contact_links() -> ContactLinks {
    // Project metadata is part of the application build, so embedding it keeps
    // development and packaged builds on the same path.
    let text = include_str!("../../config/project-metadata.toml");
    let mut github = None;
    let mut repository = None;
    let mut x = None;
    let mut telegram = None;
    let mut developer_name = None;
    let mut copyright = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("github_profile_url = ") {
            github = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("main_repository_url = ") {
            repository = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("x_profile_url = ") {
            x = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("telegram_profile_url = ") {
            telegram = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("developer_display_name = ") {
            developer_name = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("copyright_holder = ") {
            copyright = parse_toml_string(value);
        }
    }
    ContactLinks {
        github,
        repository,
        x,
        telegram,
        developer_name,
        copyright,
    }
}

fn parse_toml_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "\"\"" || trimmed == "''" {
        return None;
    }
    trimmed
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .map(|s| s.to_string())
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|v| v.strip_suffix('\''))
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

fn load_settings(app: &AppHandle) -> AppSettings {
    settings_path(app)
        .and_then(|path| fs::read_to_string(&path).ok())
        .and_then(|text| serde_json::from_str::<AppSettings>(&text).ok())
        .map(AppSettings::normalized)
        .unwrap_or_default()
}

/// Return cached settings if available, otherwise load from disk and cache.
/// The cache is populated on first access and updated whenever `settings_set`
/// is called. This avoids reading `settings.json` from disk on every poll.
fn cached_settings(app: &AppHandle) -> AppSettings {
    let state = app.state::<SessionState>();
    if let Ok(mut cache) = state.cached_settings.lock() {
        if let Some(settings) = cache.as_ref() {
            return settings.clone();
        }
        let settings = load_settings(app);
        *cache = Some(settings.clone());
        return settings;
    }
    // Lock failed (poisoned) — fall back to direct disk read.
    load_settings(app)
}

/// Update the cached settings. Called by `settings_set` after a successful save.
fn update_cached_settings(app: &AppHandle, settings: &AppSettings) {
    if let Ok(mut cache) = app.state::<SessionState>().cached_settings.lock() {
        *cache = Some(settings.clone());
    }
}

fn save_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app).ok_or_else(|| "config dir unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &text).map_err(|e| format!("write settings: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("commit settings: {e}"))?;
    Ok(())
}

fn software_profiles_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("device-profiles.json"))
}

fn load_software_profiles(app: &AppHandle) -> SoftwareProfileStore {
    software_profiles_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| SoftwareProfileStore {
            schema_version: 1,
            ..SoftwareProfileStore::default()
        })
}

/// Return cached software profiles, loading from disk on first access.
/// The cache is invalidated by `save_software_profiles` so subsequent reads
/// after a write always reflect the latest state.
fn cached_software_profiles(app: &AppHandle) -> SoftwareProfileStore {
    let state = app.state::<SessionState>();
    if let Ok(mut cache) = state.cached_software_profiles.lock() {
        if let Some(ref profiles) = *cache {
            return profiles.clone();
        }
        let profiles = load_software_profiles(app);
        *cache = Some(profiles.clone());
        return profiles;
    }
    load_software_profiles(app)
}

fn save_software_profiles(app: &AppHandle, profiles: &SoftwareProfileStore) -> Result<(), String> {
    let path = software_profiles_path(app).ok_or_else(|| "config dir unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config dir: {error}"))?;
    }
    let text = serde_json::to_string_pretty(profiles)
        .map_err(|error| format!("serialize device profiles: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|error| format!("write device profiles: {error}"))?;
    fs::rename(&tmp, &path).map_err(|error| format!("commit device profiles: {error}"))?;
    // Update the cache so the next read doesn't hit the disk.
    if let Ok(mut cache) = app.state::<SessionState>().cached_software_profiles.lock() {
        *cache = Some(profiles.clone());
    }
    Ok(())
}

fn device_config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("device-config.json"))
}

fn read_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.').try_fold(value, |current, segment| {
        current.as_object().and_then(|object| object.get(segment))
    })
}

fn exportable_value(snapshot: &DeviceSnapshot, field: &ExportableField) -> Option<serde_json::Value> {
    // 复合字段：从多个快照路径组合为 object
    if let Some(sources) = &field.sources {
        let snapshot_value = serde_json::to_value(snapshot).ok()?;
        let mut object = serde_json::Map::new();
        for (param, path) in sources {
            if let Some(value) = read_json_path(&snapshot_value, path) {
                object.insert(param.clone(), value.clone());
            }
        }
        return Some(serde_json::Value::Object(object));
    }
    // 显式声明的源路径
    if let Some(source) = &field.source {
        let snapshot_value = serde_json::to_value(snapshot).ok()?;
        return read_json_path(&snapshot_value, source).cloned();
    }
    // 回退：直接从 capabilities map 取值
    if let Some(value) = snapshot.capabilities.get(&field.id) {
        return Some(value.clone());
    }
    // 回退：从 capability metadata 的 source 路径取值
    let snapshot_value = serde_json::to_value(snapshot).ok()?;
    snapshot
        .plugin_capabilities
        .iter()
        .find(|capability| capability.id == field.id)
        .and_then(|capability| capability.metadata.get("source"))
        .and_then(serde_json::Value::as_str)
        .and_then(|path| read_json_path(&snapshot_value, path))
        .cloned()
}

fn current_plugin_for_primary_snapshot(
    app: &AppHandle,
) -> Result<(PackageInspection, DeviceSnapshot), String> {
    let state = app.state::<SessionState>();
    let (primary_path, snapshot) = {
        let guard = state
            .last_snapshot
            .lock()
            .map_err(|_| "device snapshot state unavailable".to_string())?;
        primary_snapshot_entry(&guard)
            .map(|(path, snapshot)| (path.clone(), snapshot.clone()))
            .ok_or_else(|| "no device snapshot is available".to_string())?
    };
    let plugins_guard = state.plugins.lock().map_err(|_| "state lock failed")?;
    let plugins = plugins_guard.as_ref().ok_or("plugins not loaded")?;
    let mut hidapi_guard = state
        .cached_hidapi
        .lock()
        .map_err(|_| "HidApi cache unavailable")?;
    if hidapi_guard.is_none() {
        *hidapi_guard = Some(HidApi::new().map_err(|e| e.to_string())?);
    }
    let api = hidapi_guard.as_mut().unwrap();
    let _ = api.refresh_devices();
    let matched = hid::enumerate_matched_devices(api, plugins);
    let device = matched
        .iter()
        .find(|device| device.path == primary_path)
        .ok_or("primary device is no longer connected")?;
    let inspection = plugins
        .iter()
        .find(|(inspection, _, _)| inspection.plugin_id == device.plugin_id)
        .map(|(inspection, _, _)| inspection.clone())
        .ok_or("matched plugin is unavailable")?;
    Ok((inspection, snapshot))
}

#[tauri::command]
fn device_config_export(app: tauri::AppHandle, path: Option<String>) -> Result<serde_json::Value, String> {
    let (inspection, snapshot) = current_plugin_for_primary_snapshot(&app)?;
    if inspection.exportable_fields.is_empty() {
        return Err(format!(
            "plugin {} does not declare exportable fields",
            inspection.plugin_id
        ));
    }
    let mut fields = serde_json::Map::new();
    for field in &inspection.exportable_fields {
        if let Some(value) = exportable_value(&snapshot, field) {
            fields.insert(field.export_key.clone(), value);
        }
    }
    let config = serde_json::json!({
        "schemaVersion": 1,
        "pluginId": inspection.plugin_id,
        "pluginVersion": inspection.version,
        "device": snapshot.display_name,
        "fields": fields,
    });
    // #11 支持用户指定导出路径（文件选择器），未指定时用默认 app config 路径。
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => device_config_path(&app).ok_or_else(|| "config dir unavailable".to_string())?,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config dir: {error}"))?;
    }
    let text =
        serde_json::to_string_pretty(&config).map_err(|error| format!("serialize: {error}"))?;
    fs::write(&path, text).map_err(|error| format!("write device config: {error}"))?;
    Ok(config)
}

fn mutation_for_exportable(inspection: &PackageInspection, field: &ExportableField) -> (String, String) {
    // 优先使用字段声明的 mutation 和 param
    if let Some(mutation) = &field.mutation {
        let param = field.param.clone().unwrap_or_else(|| "value".to_string());
        return (mutation.clone(), param);
    }
    // 回退：从 capability metadata 推导
    let field_id = &field.id;
    for capability in &inspection.capabilities {
        let mutation = capability
            .metadata
            .get("mutation")
            .and_then(serde_json::Value::as_str);
        if &capability.id == field_id {
            if let Some(mutation) = mutation {
                let param = capability
                    .metadata
                    .get("param")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("value");
                return (mutation.to_string(), param.to_string());
            }
        }
        if mutation == Some(field_id.as_str()) {
            let param = capability
                .metadata
                .get("param")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("value");
            return (field_id.clone(), param.to_string());
        }
    }
    (field_id.clone(), "value".to_string())
}

#[tauri::command]
fn device_config_import(app: tauri::AppHandle, path: Option<String>) -> Result<DeviceSnapshot, String> {
    // #11 支持用户指定导入路径（文件选择器），未指定时用默认 app config 路径。
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => device_config_path(&app).ok_or_else(|| "config dir unavailable".to_string())?,
    };
    let config: serde_json::Value = fs::read_to_string(&path)
        .map_err(|error| format!("read device config: {error}"))
        .and_then(|text| {
            serde_json::from_str(&text).map_err(|error| format!("parse device config: {error}"))
        })?;
    if config
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err("unsupported device config schema".into());
    }
    let fields = config
        .get("fields")
        .and_then(serde_json::Value::as_object)
        .ok_or("device config has no fields object")?;
    let (inspection, snapshot) = current_plugin_for_primary_snapshot(&app)?;
    if config.get("pluginId").and_then(serde_json::Value::as_str)
        != Some(inspection.plugin_id.as_str())
    {
        return Err("device config plugin does not match the connected device".into());
    }
    let mut latest = snapshot;
    let mut applied: Vec<String> = Vec::new();
    for field in &inspection.exportable_fields {
        let Some(value) = fields.get(&field.export_key) else {
            continue;
        };
        let (mutation, param) = mutation_for_exportable(&inspection, field);
        // 复合字段（object）：将键值展开为 mutation 参数
        // 标量字段：使用单个 param 包装
        let params = match value {
            serde_json::Value::Object(map) => map.clone(),
            _ => serde_json::Map::from_iter([(param, value.clone())]),
        };
        match device_mutate_blocking(&app, &mutation, &params) {
            Ok(snapshot) => {
                latest = snapshot;
                applied.push(field.export_key.clone());
            }
            Err(error) => {
                // 修复 #11：导入逐字段写入，第 N 字段失败时前 N-1 个已生效。
                // 返回包含已成功字段列表的错误，让用户知道设备已部分变更。
                if applied.is_empty() {
                    return Err(error);
                }
                return Err(format!(
                    "导入在字段 {export_key} 失败：{error}。已成功导入字段：{applied_list}。建议重新导出当前配置或手动校准未导入字段。",
                    export_key = field.export_key,
                    applied_list = applied.join(", ")
                ));
            }
        }
    }
    Ok(latest)
}

fn control_mode(reading: &DeviceReading) -> Option<u8> {
    reading
        .capabilities
        .get("controlMode")
        .and_then(serde_json::Value::as_object)
        .and_then(|mode| mode.get("mode"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|mode| u8::try_from(mode).ok())
}

fn software_profile_key(device: &hid::MatchedDevice, reading: &DeviceReading) -> String {
    let unit_id = reading
        .capabilities
        .get("deviceInfo")
        .and_then(serde_json::Value::as_object)
        .and_then(|info| info.get("unitId"))
        .and_then(serde_json::Value::as_array)
        .map(|bytes| {
            bytes
                .iter()
                .filter_map(serde_json::Value::as_u64)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{:04x}:{:04x}", device.vendor_id, device.product_id));
    format!("{}:{}:{unit_id}", device.plugin_id, device.family)
}

fn seed_standard_software_mutations(
    profile: &mut SoftwareProfile,
    reading: &DeviceReading,
    allowed: &[String],
) {
    if allowed.iter().any(|mutation| mutation == "set-dpi-value") {
        if let Some(dpi) = reading.dpi {
            let stage = reading
                .dpi_stages
                .as_ref()
                .and_then(|stages| stages.iter().position(|stage| stage.active))
                .map(|index| index + 1)
                .unwrap_or(1);
            profile.mutations.insert(
                "set-dpi-value".into(),
                BTreeMap::from([
                    ("dpi".into(), serde_json::json!(dpi)),
                    ("stage".into(), serde_json::json!(stage)),
                ]),
            );
        }
    }
    if allowed
        .iter()
        .any(|mutation| mutation == "set-polling-rate")
    {
        if let Some(rate) = reading.polling_rate_hz {
            profile.mutations.insert(
                "set-polling-rate".into(),
                BTreeMap::from([("rate".into(), serde_json::json!(rate))]),
            );
        }
    }
}

fn parse_connection(value: &str) -> mira_core::Connection {
    match value {
        "usb" => mira_core::Connection::Usb,
        "wireless" | "wireless-receiver" => mira_core::Connection::Wireless,
        "bluetooth" => mira_core::Connection::Bluetooth,
        _ => mira_core::Connection::Usb,
    }
}

/// Parse a connection string and return both the semantic connection type and
/// the runtime `ConnectionKind` used by the protocol engine.
fn connection_kind(value: &str) -> (mira_core::Connection, ConnectionKind) {
    let connection = parse_connection(value);
    let kind = match connection {
        mira_core::Connection::Usb => ConnectionKind::Usb,
        mira_core::Connection::Wireless => ConnectionKind::Wireless,
        mira_core::Connection::Bluetooth => ConnectionKind::Bluetooth,
        mira_core::Connection::Virtual => ConnectionKind::Usb,
    };
    (connection, kind)
}

fn runtime_connection(value: ConnectionKind) -> mira_core::Connection {
    match value {
        ConnectionKind::Usb => mira_core::Connection::Usb,
        ConnectionKind::Wireless => mira_core::Connection::Wireless,
        ConnectionKind::Bluetooth => mira_core::Connection::Bluetooth,
    }
}

fn capability_connection_label(value: mira_core::Connection) -> &'static str {
    match value {
        mira_core::Connection::Usb => "usb",
        mira_core::Connection::Wireless => "receiver",
        mira_core::Connection::Bluetooth => "bluetooth",
        mira_core::Connection::Virtual => "usb",
    }
}

fn device_evidence_allows_writes(evidence: &str) -> bool {
    matches!(evidence, "hardware-verified" | "protocol-verified")
}

fn snapshot_allows_writes(snapshot: &DeviceSnapshot) -> bool {
    !snapshot.readonly
        && device_evidence_allows_writes(&snapshot.evidence)
        && !snapshot.writable_mutations.is_empty()
}

fn display_name(
    plugin_id: &str,
    family: &str,
    verified_models: &[String],
    evidence: &str,
) -> String {
    if evidence == "hardware-verified" {
        if let Some(model) = verified_models.first() {
            return model.clone();
        }
    }
    format!(
        "{} {}",
        plugin_id.split('.').next_back().unwrap_or(plugin_id),
        family
    )
}

/// Build a `DeviceSnapshot` from a `DeviceReading` and the matched plugin/device
/// context. When `mutation_result` is provided, its `color` field is used as a
/// fallback for `confirmed_light_color` (write path); otherwise the reading's
/// `light_color` is used directly (read path).
fn build_device_snapshot(
    reading: DeviceReading,
    inspection: &PackageInspection,
    devices: &hid::DevicesFile,
    device: &hid::MatchedDevice,
    fallback_connection: mira_core::Connection,
    writable_mutations: Vec<String>,
    mutation_result: Option<&serde_json::Value>,
) -> DeviceSnapshot {
    let resolved_name = reading.display_name.unwrap_or_else(|| {
        display_name(
            &device.plugin_id,
            &device.family,
            &devices.hardware_verified_models,
            &device.evidence,
        )
    });
    let resolved_connection = reading
        .connection
        .map(runtime_connection)
        .unwrap_or(fallback_connection);
    let confirmed_light_color = reading.light_color.or_else(|| {
        mutation_result
            .and_then(|result| result.get("color"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });
    let readonly = !(inspection.signature_verified && inspection.writes_enabled);
    // 能力动态协商：用 workflow 输出计算每个能力的 available 标记。
    // 在 reading.capabilities 被移动前借用，传给 plugin_capabilities。
    // #3 连接类型能力分支：优先使用 workflow 实际读到的连接类型，
    // 避免 devices.json fallback 与真实连接不一致时误判能力可见性。
    let plugin_capabilities = plugin_capabilities(
        inspection,
        &reading.capabilities,
        capability_connection_label(resolved_connection),
    );
    DeviceSnapshot {
        display_name: resolved_name,
        connection: resolved_connection,
        battery_percent: reading.battery_percent,
        charging: reading.charging,
        batteries: reading.batteries,
        dpi: reading.dpi,
        dpi_stages: reading.dpi_stages,
        polling_rate_hz: reading.polling_rate_hz,
        supported_polling_rates_hz: reading.supported_polling_rates_hz,
        profile: reading.profile.map(|p| (p + 1).to_string()),
        confirmed_light_color,
        capabilities: reading.capabilities,
        plugin_capabilities,
        writable_mutations,
        evidence: device.evidence.clone(),
        readonly,
    }
}

#[tauri::command]
fn device_snapshot(state: tauri::State<'_, SessionState>) -> Option<DeviceSnapshot> {
    let guard = state.last_snapshot.lock().ok()?;
    primary_snapshot(&guard).cloned()
}

/// 返回所有已连接设备的快照列表，供未来多设备 UI 使用。
#[tauri::command]
fn device_snapshots(state: tauri::State<'_, SessionState>) -> Vec<DeviceSnapshot> {
    state
        .last_snapshot
        .lock()
        .map(|guard| guard.values().cloned().collect())
        .unwrap_or_default()
}

/// Trigger an immediate device read on the background thread.
/// The read result is delivered via the `device-updated` event, so this
/// command returns immediately. Used by the manual "刷新" button and any
/// other UI flow that needs a fresh reading without waiting for the fallback poll.
#[tauri::command]
fn device_refresh(state: tauri::State<'_, SessionState>) -> Result<(), String> {
    request_refresh(&state);
    Ok(())
}

struct SoftwareProfileRuntime<'a> {
    api: &'a HidApi,
    connection: ConnectionKind,
    files: &'a BTreeMap<String, Vec<u8>>,
    package: &'a ProtocolPackage,
}

/// 在读取路径中恢复软件配置（控制模式 + 已保存的 mutation）。
///
/// 注意：此函数在 `read_device_once` 持有 `device_io` 锁期间调用，**不经过
/// `begin_device_write` 写入队列**。这是有意为之——`device_mutate_blocking` 的
/// 锁顺序是 `write_in_progress` → `device_io`，若此处再获取 `write_in_progress`
/// 会形成 `device_io` → `write_in_progress` 的反向锁顺序，导致死锁。
/// 此处的写入由 `device_io` 锁序列化（与 `device_mutate_blocking` 互斥），
/// 且仅对已验证插件的 `allowed` mutation 执行，安全性等价。
fn reapply_software_profile(
    app: &AppHandle,
    state: &SessionState,
    device: &hid::MatchedDevice,
    reading: &DeviceReading,
    allowed: &[String],
    runtime: &SoftwareProfileRuntime<'_>,
) -> Option<DeviceReading> {
    let key = software_profile_key(device, reading);
    let profiles = cached_software_profiles(app);
    let profile = profiles.devices.get(&key)?;
    let already_applied = state.applied_software_profiles.lock().ok()?.contains(&key);
    if already_applied && control_mode(reading) == Some(2) {
        return None;
    }
    let context = ProtocolContext {
        api: runtime.api,
        path: &device.path,
        family: &device.family,
        connection: runtime.connection,
        files: runtime.files,
        outputs: reading.capabilities.clone(),
        feature_index_cache: Some(&state.feature_index_cache),
        onboard_memory_cache: Some(&state.onboard_memory_cache),
    };
    let mut failed = false;
    if allowed
        .iter()
        .any(|mutation| mutation == "set-control-mode")
        && control_mode(reading) != Some(2)
    {
        let params = serde_json::Map::from_iter([("mode".into(), serde_json::json!(2))]);
        if let Err(error) =
            mutate_device_with_package(runtime.package, &context, "set-control-mode", &params)
        {
            failed = true;
            eprintln!("[mira] unable to restore software control mode: {error}");
        }
    }
    if !failed {
        for (mutation, params) in &profile.mutations {
            if mutation == "set-control-mode"
                || !allowed.iter().any(|candidate| candidate == mutation)
            {
                continue;
            }
            let params = serde_json::Map::from_iter(params.clone());
            if let Err(error) =
                mutate_device_with_package(runtime.package, &context, mutation, &params)
            {
                failed = true;
                eprintln!("[mira] unable to restore {mutation}: {error}");
            }
        }
    }
    if let Ok(mut applied) = state.applied_software_profiles.lock() {
        applied.insert(key);
    }
    if failed {
        None
    } else {
        read_device_with_package(
            runtime.package,
            &ProtocolContext {
                api: runtime.api,
                path: &device.path,
                family: &device.family,
                connection: runtime.connection,
                files: runtime.files,
                outputs: BTreeMap::new(),
                feature_index_cache: Some(&state.feature_index_cache),
                onboard_memory_cache: Some(&state.onboard_memory_cache),
            },
        )
        .ok()
    }
}

/// Outcome of a background device read, carried out of the locked section so
/// disk I/O (load_settings) and UI work (update_tray, app.emit) can run without
/// holding the device_io / plugins locks.
enum DeviceReadOutcome {
    /// Nothing to do (no plugins, HidApi failed, etc.) — skip silently.
    Skip,
    /// Device disconnected or read failed — clear the cached state.
    Clear,
    /// 设备仍被枚举到但读取失败（如无线设备休眠导致的间歇性超时）。
    /// 保留上次有效快照，避免清除缓存后形成"识别→丢失→识别"的无限循环。
    /// 注意：不应清除 feature_index_cache 等缓存，否则下次读取更慢，加剧循环。
    PreserveLast,
    /// Read succeeded — publish snapshots for all matched devices.
    /// 修复 #10：携带所有 matched 设备的快照，实现多设备并行读取。
    Ready(Vec<(String, DeviceSnapshot)>),
}

/// Read the device once, update the cached snapshot, and emit `device-updated`.
/// Called by the background reader thread on every loop iteration (whether
/// triggered by a signal or the fallback timeout).
fn read_device_once(app: &AppHandle) {
    let state = app.state::<SessionState>();
    // 系统主题缓存由 ThemeChanged 事件和窗口聚焦事件更新，
    // 轮询期间只读缓存，避免每轮都 fork 进程检测主题。
    // Lock scope: only HID I/O and snapshot construction. Disk reads, tray
    // updates and event emission run after the locks are released so they
    // cannot block discover_devices or other commands.
    let outcome = (|| -> Option<DeviceReadOutcome> {
        let _io_guard = state.device_io.lock().ok()?;
        let plugins_guard = state.plugins.lock().ok()?;
        let plugins = plugins_guard.as_ref()?;
        if plugins.is_empty() {
            return None;
        }
        // Reuse the cached HidApi instance and refresh the device list to
        // detect newly plugged/unplugged devices. This avoids re-enumerating
        // all HID devices from scratch on every poll.
        let mut hidapi_guard = state.cached_hidapi.lock().ok()?;
        if hidapi_guard.is_none() {
            *hidapi_guard = Some(HidApi::new().ok()?);
        }
        let cached_api = hidapi_guard.as_mut().unwrap();
        let _ = cached_api.refresh_devices();
        let api: &HidApi = cached_api;
        let matched = hid::enumerate_matched_devices(api, plugins);
        #[cfg(debug_assertions)]
        {
            let count = matched.len();
            if let Ok(mut last) = state.last_matched_count.lock() {
                if *last != count {
                    eprintln!("[mira] background: matched {} device(s)", count);
                    *last = count;
                }
            }
        }

        // 修复 #10：遍历所有 matched 设备，逐个读取并收集快照。
        if matched.is_empty() {
            return Some(DeviceReadOutcome::Clear);
        }
        let mut entries: Vec<(String, DeviceSnapshot)> = Vec::new();
        for device in &matched {
            let (inspection, devices, plugin_files) = match plugins
                .iter()
                .find(|(inspection, _, _)| inspection.plugin_id == device.plugin_id)
            {
                Some(triple) => triple,
                None => continue,
            };

            let (connection, kind) = connection_kind(&device.connection);

            // 缓存命中时直接复用 Arc<ProtocolPackage>，避免每轮轮询都解析 JSON。
            let package = match get_or_parse_package(
                &state,
                inspection,
                device.model.as_deref(),
                plugin_files,
                plugins,
            ) {
                Ok(pkg) => pkg,
                Err(_) => continue,
            };

            // #9 防抖式 TTL 缓存：非 settling 状态下，500ms 内复用快照跳过 HID 往返。
            // settling 期间需要快速轮询捕捉状态变化，不缓存。
            let settling = state
                .settling_polls
                .lock()
                .map(|s| *s > 0)
                .unwrap_or(false);
            if !settling {
                let cache_hit = state
                    .last_read_at
                    .lock()
                    .ok()
                    .is_some_and(|cache| {
                        cache
                            .get(&device.path)
                            .is_some_and(|t| t.elapsed() < READ_DEBOUNCE_TTL)
                    });
                if cache_hit {
                    if let Some(snapshot) = state
                        .last_snapshot
                        .lock()
                        .ok()
                        .and_then(|map| map.get(&device.path).cloned())
                    {
                        entries.push((device.path.clone(), snapshot));
                        continue;
                    }
                }
            }

            match read_device_with_package(
                &package,
                &ProtocolContext {
                    api,
                    path: &device.path,
                    family: &device.family,
                    connection: kind,
                    files: plugin_files,
                    outputs: BTreeMap::new(),
                    feature_index_cache: Some(&state.feature_index_cache),
                    onboard_memory_cache: Some(&state.onboard_memory_cache),
                },
            ) {
                Ok(mut reading) => {
                    let writable_mutations = if inspection.signature_verified
                        && inspection.writes_enabled
                        && device_evidence_allows_writes(&device.evidence)
                    {
                        writable_mutations_with_package(
                            &package,
                            &ProtocolContext {
                                api,
                                path: &device.path,
                                family: &device.family,
                                connection: kind,
                                files: plugin_files,
                                outputs: reading.capabilities.clone(),
                                feature_index_cache: Some(&state.feature_index_cache),
                                onboard_memory_cache: Some(&state.onboard_memory_cache),
                            },
                        )
                        .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    if let Some(updated) = reapply_software_profile(
                        app,
                        &state,
                        device,
                        &reading,
                        &writable_mutations,
                        &SoftwareProfileRuntime {
                            api,
                            connection: kind,
                            files: plugin_files,
                            package: &package,
                        },
                    ) {
                        reading = updated;
                    }
                    let snapshot = build_device_snapshot(
                        reading,
                        inspection,
                        devices,
                        device,
                        connection,
                        writable_mutations,
                        None,
                    );
                    entries.push((device.path.clone(), snapshot));
                    // #9 记录读取时间戳，供下一轮 TTL 防抖判断。
                    if let Ok(mut cache) = state.last_read_at.lock() {
                        cache.insert(device.path.clone(), Instant::now());
                    }
                }
                Err(error) => {
                    eprintln!(
                        "[mira] background read failed for {}: {error}",
                        device.family
                    );
                }
            }
        }

        if entries.is_empty() {
            // 设备仍被枚举到（matched 非空）但所有设备读取都失败。
            // 这种情况通常由无线设备休眠导致的间歇性超时引起。
            // 保留上次有效快照，避免清除缓存后形成"识别→丢失→识别"的无限循环：
            //   - 清除 last_snapshot 会让前端显示"未找到设备"
            //   - 清除 feature_index_cache 会让下次 HID++ 读取重新查询所有 feature index
            //     （罗技 42 步工作流最坏情况 14 秒），加剧超时
            //   - settling 重置后会以 500ms 快速轮询，进一步放大问题
            // 只有 matched 为空（设备真正断开）时才触发 Clear。
            Some(DeviceReadOutcome::PreserveLast)
        } else {
            Some(DeviceReadOutcome::Ready(entries))
        }
    })()
    .unwrap_or(DeviceReadOutcome::Skip);

    // Post-lock: disk I/O, tray updates and event emission run without holding
    // device_io or plugins locks so concurrent commands are not blocked.
    match outcome {
        DeviceReadOutcome::Skip => {}
        DeviceReadOutcome::PreserveLast => {
            // 设备仍被枚举到但读取失败：保留上次有效快照，不触发前端更新。
            // 这样前端不会看到"设备丢失"的闪烁，settle 也不会被重置，
            // 下次轮询会以正常间隔重试。如果设备真正断开，matched 会变空，
            // 下一轮会走到 Clear 分支。
        }
        DeviceReadOutcome::Clear => {
            if let Ok(mut applied) = state.applied_software_profiles.lock() {
                applied.clear();
            }
            clear_snapshots(&state);
            let _ = app.emit("device-updated", Option::<DeviceSnapshot>::None);
            let _ = update_tray(app, None, &cached_settings(app));
        }
        DeviceReadOutcome::Ready(entries) => {
            // 多设备管理：用当前 matched 设备的快照整体替换 map，清除已断开的设备。
            let new_map: BTreeMap<String, DeviceSnapshot> = entries.iter().cloned().collect();
            store_snapshots(&state, new_map.clone());
            // 选择 primary 设备通知前端（向后兼容单设备 API）。
            if let Some(snapshot) = primary_snapshot(&new_map).cloned() {
                // 通知前端有新数据，前端通过事件监听更新，无需轮询
                let _ = app.emit("device-updated", &snapshot);
                let settings = cached_settings(app);
                let _ = update_tray(app, Some(&snapshot), &settings);
                // 低电量跨阈值检测：充电时不触发，充电结束后若仍低于阈值才再次提醒
                let battery_value = if mouse_battery_charging(&snapshot) {
                    None
                } else {
                    mouse_battery_percentage(&snapshot)
                };
                let threshold = settings.low_battery_threshold;
                let notify = state
                    .low_battery
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .update(battery_value, threshold);
                if notify {
                    if let Some(percent) = battery_value {
                        let lang = effective_language(&settings.language);
                        let _ = app
                            .notification()
                            .builder()
                            .title(tr_low_battery_title(lang))
                            .body(tr_low_battery_body(lang, threshold, percent))
                            .show();
                    }
                }
            }
        }
    }
}

/// Background reader thread: event-driven with an adaptive fallback poll.
///
/// The thread sleeps on `mpsc::recv_timeout` instead of a fixed `sleep`:
/// - A `RefreshNow` signal (window focus, manual refresh, tray click, etc.)
///   wakes it immediately for an on-demand read.
/// - If no signal arrives within the fallback interval, it reads anyway to
///   detect disconnects and battery drift.
/// - The fallback interval adapts to the situation:
///   * 500 ms for a short settling window right after a state change.
///   * 1 s when no device is connected and the window is visible, so plug-in
///     is detected quickly without continuous high-frequency polling.
///   * The user's configured `refresh_interval_seconds` when a device is
///     connected and stable.
///   * 60 s when the window is hidden/minimized to tray.
fn spawn_device_reader(app: AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    *app.state::<SessionState>()
        .refresh_tx
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(tx);

    std::thread::spawn(move || loop {
        read_device_once(&app);

        // Determine the adaptive fallback interval.
        // Lock order: last_snapshot before settling_polls, matching store_snapshot.
        // Settings are read outside of any lock so disk I/O cannot block snapshot updates.
        let state = app.state::<SessionState>();
        let visible = app
            .get_webview_window("main")
            .and_then(|window| window.is_visible().ok())
            .unwrap_or(false);
        let (connected, settling_now) = {
            let connected = !state
                .last_snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty();
            let mut settling = state
                .settling_polls
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let settling_now = if *settling > 0 {
                *settling -= 1;
                true
            } else {
                false
            };
            (connected, settling_now)
        };
        let wait = if settling_now {
            std::time::Duration::from_millis(500)
        } else if !visible {
            std::time::Duration::from_secs(60)
        } else if connected {
            std::time::Duration::from_secs(u64::from(
                cached_settings(&app).refresh_interval_seconds,
            ))
        } else {
            // No device connected: poll faster so plug-in is noticed quickly.
            std::time::Duration::from_secs(1)
        };

        match rx.recv_timeout(wait) {
            Ok(()) => {
                // Drain any additional pending signals so a burst of focus
                // events doesn't trigger a burst of redundant reads.
                while rx.try_recv().is_ok() {}
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredDevice {
    plugin_id: String,
    family: String,
    connection: String,
    evidence: String,
    path: String,
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
}

#[tauri::command]
fn discover_devices(
    state: tauri::State<'_, SessionState>,
) -> Result<Vec<DiscoveredDevice>, String> {
    let plugins_guard = state.plugins.lock().map_err(|_| "state lock failed")?;
    let plugins = plugins_guard.as_ref().ok_or("plugins not loaded")?;
    if plugins.is_empty() {
        return Ok(Vec::new());
    }
    // Reuse the cached HidApi instance and refresh the device list.
    let mut hidapi_guard = state
        .cached_hidapi
        .lock()
        .map_err(|_| "HidApi cache unavailable")?;
    if hidapi_guard.is_none() {
        *hidapi_guard = Some(HidApi::new().map_err(|e| e.to_string())?);
    }
    let cached_api = hidapi_guard.as_mut().unwrap();
    let _ = cached_api.refresh_devices();
    let api: &HidApi = cached_api;
    let matched = hid::enumerate_matched_devices(api, plugins);
    Ok(matched
        .into_iter()
        .map(|d| DiscoveredDevice {
            plugin_id: d.plugin_id,
            family: d.family,
            connection: d.connection,
            evidence: d.evidence,
            // 不暴露原始 HID 路径（macOS 上可能包含序列号），
            // 用脱敏标识符替代，前端仅用作 React key。
            path: format!(
                "device-{:04x}-{:04x}-{:02x}-{:02x}",
                d.vendor_id, d.product_id, d.usage_page, d.usage
            ),
            vendor_id: d.vendor_id,
            product_id: d.product_id,
            usage_page: d.usage_page,
            usage: d.usage,
        })
        .collect())
}

struct WriteFlagGuard<'a>(&'a Mutex<bool>, &'a Condvar);

impl Drop for WriteFlagGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = false;
        }
        self.1.notify_one();
    }
}

fn begin_device_write(state: &SessionState) -> Result<WriteFlagGuard<'_>, String> {
    let mut active = state
        .write_in_progress
        .lock()
        .map_err(|_| "transaction state unavailable")?;
    // 排队等待前一个写入完成。最多等 25 秒，留 5 秒给实际 HID 写入，
    // 配合 device_mutate 的 30 秒总超时。
    let (guard, wait_result) = state
        .write_cond
        .wait_timeout_while(active, Duration::from_secs(25), |a| *a)
        .map_err(|_| "transaction state unavailable")?;
    active = guard;
    if *active || wait_result.timed_out() {
        return Err("写入排队超时：前一个写入仍未完成，请稍后重试".into());
    }
    *active = true;
    drop(active);
    Ok(WriteFlagGuard(&state.write_in_progress, &state.write_cond))
}

fn remember_software_profile(
    app: &AppHandle,
    state: &SessionState,
    device: &hid::MatchedDevice,
    reading: &DeviceReading,
    allowed: &[String],
    mutation: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if control_mode(reading).is_none() {
        return Ok(());
    }
    let key = software_profile_key(device, reading);
    let mut profiles = cached_software_profiles(app);
    if control_mode(reading) == Some(1) {
        profiles.devices.remove(&key);
        if let Ok(mut applied) = state.applied_software_profiles.lock() {
            applied.remove(&key);
        }
        return save_software_profiles(app, &profiles);
    }
    if control_mode(reading) != Some(2) {
        return Ok(());
    }
    let profile = profiles.devices.entry(key.clone()).or_default();
    if mutation == "set-control-mode" {
        seed_standard_software_mutations(profile, reading, allowed);
    } else {
        profile
            .mutations
            .insert(mutation.to_string(), params.clone().into_iter().collect());
    }
    profiles.schema_version = 1;
    save_software_profiles(app, &profiles)?;
    if let Ok(mut applied) = state.applied_software_profiles.lock() {
        applied.insert(key);
    }
    Ok(())
}

#[tauri::command]
async fn device_mutate(
    app: tauri::AppHandle,
    mutation: String,
    params: serde_json::Map<String, serde_json::Value>,
) -> Result<DeviceSnapshot, String> {
    // HID 写入/回读是阻塞式调用，必须放在独立线程中执行，否则主线程会被卡死。
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_app = app.clone();
    std::thread::spawn(move || {
        let result = device_mutate_blocking(&worker_app, &mutation, &params);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("设备写入超时（30 秒）。鼠标可能处于休眠状态，请移动鼠标唤醒后重试。写入可能仍在后台执行，设备状态将在完成后自动刷新。".into())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("设备写入线程异常退出".into()),
    }
}

fn device_mutate_blocking(
    app: &tauri::AppHandle,
    mutation: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<DeviceSnapshot, String> {
    let state = app.state::<SessionState>();
    // Lock scope: only HID I/O and snapshot construction.
    // Disk I/O (profile save, load_settings), tray updates and event emission
    // run after the locks are released so they cannot block discover_devices or reads.
    let (device_path, snapshot, profile_remember) = {
        let _write_guard = begin_device_write(&state)?;
        let _io_guard = state
            .device_io
            .lock()
            .map_err(|_| "device I/O state unavailable")?;
        let plugins_guard = state.plugins.lock().map_err(|_| "state lock failed")?;
        let plugins = plugins_guard.as_ref().ok_or("plugins not loaded")?;
        // Reuse the cached HidApi instance and refresh the device list.
        let mut hidapi_guard = state
            .cached_hidapi
            .lock()
            .map_err(|_| "HidApi cache unavailable")?;
        if hidapi_guard.is_none() {
            *hidapi_guard = Some(HidApi::new().map_err(|e| e.to_string())?);
        }
        let cached_api = hidapi_guard.as_mut().unwrap();
        let _ = cached_api.refresh_devices();
        let api: &HidApi = cached_api;
        let matched = hid::enumerate_matched_devices(api, plugins);
        // 修复 #10：优先写入 primary 设备（与前端展示一致）。
        // 多设备并存时，按枚举顺序的 find(first writable) 会误写非目标设备。
        // 这里从 last_snapshot 取 primary 设备的 HID 路径精确定位；
        // primary 已断开时回退到第一个可写设备，再回退到首个匹配设备。
        let primary_path = state
            .last_snapshot
            .lock()
            .ok()
            .and_then(|guard| primary_snapshot_entry(&guard).map(|(path, _)| path.clone()));
        let device = matched
            .iter()
            .find(|device| Some(&device.path) == primary_path.as_ref())
            .or_else(|| {
                matched
                    .iter()
                    .find(|device| device_evidence_allows_writes(&device.evidence))
            })
            .or_else(|| matched.first())
            .ok_or("supported device is not connected")?;
        let (inspection, devices, files) = plugins
            .iter()
            .find(|(inspection, _, _)| inspection.plugin_id == device.plugin_id)
            .ok_or("matched plugin is unavailable")?;
        if !inspection.signature_verified || !inspection.writes_enabled {
            return Err("the matched plugin is not trusted for device writes".into());
        }
        if !device_evidence_allows_writes(&device.evidence) {
            return Err("device writes require verified protocol evidence".into());
        }

        let (connection, kind) = connection_kind(&device.connection);
        // Parse the protocol package once and reuse it for writable_mutations,
        // read_device, and mutate_device to avoid re-parsing the JSON files 4 times.
        // 缓存命中时复用 Arc<ProtocolPackage>，写入路径同样受益。
        let package =
            get_or_parse_package(&state, inspection, device.model.as_deref(), files, plugins)?;
        let context = ProtocolContext {
            api,
            path: &device.path,
            family: &device.family,
            connection: kind,
            files,
            outputs: BTreeMap::new(),
            feature_index_cache: Some(&state.feature_index_cache),
            onboard_memory_cache: Some(&state.onboard_memory_cache),
        };
        let allowed = writable_mutations_with_package(&package, &context)?;
        if !allowed.iter().any(|candidate| candidate == mutation) {
            return Err(format!("unsupported device mutation {mutation}"));
        }
        let before = read_device_with_package(&package, &context)?;
        let mutate_context = ProtocolContext {
            api,
            path: &device.path,
            family: &device.family,
            connection: kind,
            files,
            outputs: before.capabilities.clone(),
            feature_index_cache: Some(&state.feature_index_cache),
            onboard_memory_cache: Some(&state.onboard_memory_cache),
        };
        let mutation_result =
            mutate_device_with_package(&package, &mutate_context, mutation, params)?;
        let reading = read_device_with_package(&package, &context)?;
        // 修复 P-2：remember_software_profile 涉及磁盘 I/O（save_software_profiles），
        // 原代码在锁作用域内调用，违反 L2720-2722 注释承诺，会阻塞并发
        // discover_devices / read。改为仅在锁内 clone 所需数据，锁外再执行磁盘写入。
        let profile_remember = if control_mode(&reading).is_some() {
            Some((device.clone(), reading.clone(), allowed.clone()))
        } else {
            None
        };
        let snapshot = build_device_snapshot(
            reading,
            inspection,
            devices,
            device,
            connection,
            allowed,
            Some(&mutation_result),
        );
        (device.path.clone(), snapshot, profile_remember)
    };

    // Post-lock: disk I/O, tray updates and event emission run without holding
    // device_io or plugins locks so concurrent reads are not blocked.
    // 修复 P-2：remember_software_profile 在锁外执行磁盘写入，保持与注释承诺一致。
    if let Some((device_clone, reading_clone, allowed_clone)) = profile_remember {
        remember_software_profile(
            app,
            &state,
            &device_clone,
            &reading_clone,
            &allowed_clone,
            mutation,
            params,
        )?;
    }
    // 修复 #10：用单条 insert 替代 clone→insert→store，避免覆盖其他设备的快照。
    store_snapshot(&state, device_path.clone(), snapshot.clone());
    // #9 写入后清除 TTL 缓存，确保下一轮读取强制刷新 HID。
    if let Ok(mut cache) = state.last_read_at.lock() {
        cache.remove(&device_path);
    }
    let _ = app.emit("device-updated", &snapshot);
    let _ = update_tray(app, Some(&snapshot), &cached_settings(app));
    Ok(snapshot)
}

#[tauri::command]
fn autostart_state(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|err| format!("failed to read autostart state: {err}"))
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch
            .enable()
            .map_err(|err| format!("failed to enable autostart: {err}"))
    } else {
        autolaunch
            .disable()
            .map_err(|err| format!("failed to disable autostart: {err}"))
    }
}

fn active_plugin_versions(app: &AppHandle) -> Result<BTreeMap<String, String>, String> {
    let state = app.state::<SessionState>();
    let plugins = state
        .plugins
        .lock()
        .map_err(|_| "plugin state lock failed".to_string())?;
    Ok(plugins
        .as_ref()
        .into_iter()
        .flatten()
        .map(|plugin| (plugin.0.plugin_id.clone(), plugin.0.version.clone()))
        .collect())
}

#[tauri::command]
async fn plugin_updates_check(app: tauri::AppHandle) -> Result<Vec<PluginUpdateInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let current = active_plugin_versions(&app)?;
        plugin_updates_for_versions(&current, fetch_plugin_registry()?)
    })
    .await
    .map_err(|error| format!("plugin update task failed: {error}"))?
}

fn find_installed_plugin_path(
    directory: &std::path::Path,
    plugin_id: &str,
    trust: &TrustStore,
) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("mira-plugin")
                && !path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.contains(".rollback."))
        })
        .find(|path| {
            fs::read(path)
                .ok()
                .and_then(|bytes| inspect_package(Cursor::new(bytes), trust, true).ok())
                .is_some_and(|inspection| inspection.plugin_id == plugin_id)
        })
}

fn install_plugin_update(app: &AppHandle, plugin_id: &str) -> Result<PluginInstallResult, String> {
    let current = active_plugin_versions(app)?;
    let previous_version = current
        .get(plugin_id)
        .cloned()
        .ok_or_else(|| format!("plugin {plugin_id} is not installed"))?;
    let registry = fetch_plugin_registry()?;
    let entry = registry
        .plugins
        .into_iter()
        .find(|entry| entry.plugin_id == plugin_id)
        .ok_or_else(|| format!("plugin {plugin_id} is not in the update registry"))?;
    let current_semver = semver::Version::parse(&previous_version)
        .map_err(|error| format!("invalid installed version: {error}"))?;
    let next_semver = semver::Version::parse(&entry.version)
        .map_err(|error| format!("invalid registry version: {error}"))?;
    if next_semver <= current_semver {
        return Err(format!("plugin {plugin_id} is already up to date"));
    }
    let allowed_prefix = "https://github.com/hello-yunshu/mira-mouse-plugins/releases/download/";
    if !entry.url.starts_with(allowed_prefix) {
        return Err("plugin asset URL is outside the trusted release origin".into());
    }
    let bytes = fetch_bounded(&entry.url, MAX_PLUGIN_BYTES)?;
    let actual_sha = hex::encode(Sha256::digest(&bytes));
    if actual_sha != entry.sha256 {
        return Err(format!(
            "plugin SHA-256 mismatch: expected {}, got {actual_sha}",
            entry.sha256
        ));
    }
    let trust = production_trust_store();
    let (inspection, files) = extract_package(Cursor::new(&bytes), &trust, true)
        .map_err(|error| format!("plugin signature or package validation failed: {error}"))?;
    if inspection.plugin_id != entry.plugin_id || inspection.version != entry.version {
        return Err("plugin registry identity does not match signed package".into());
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        files
            .get("plugin.json")
            .ok_or("signed package has no plugin.json")?,
    )
    .map_err(|error| format!("parse signed plugin manifest: {error}"))?;
    if manifest
        .get("publisherKeyId")
        .and_then(serde_json::Value::as_str)
        != Some(entry.publisher_key_id.as_str())
    {
        return Err("plugin registry publisher does not match signed package".into());
    }
    let devices = hid::parse_devices_json(
        files
            .get("devices.json")
            .ok_or("signed package has no devices.json")?,
    )?;

    let directory = installed_plugins_dir(app)?;
    fs::create_dir_all(&directory).map_err(|error| format!("create plugin directory: {error}"))?;
    let previous_path = find_installed_plugin_path(&directory, plugin_id, &trust);
    let backup_path = directory.join(format!("{plugin_id}.rollback.mira-plugin"));
    if backup_path.exists() {
        fs::remove_file(&backup_path).map_err(|error| format!("remove stale rollback: {error}"))?;
    }
    if let Some(path) = &previous_path {
        fs::rename(path, &backup_path)
            .map_err(|error| format!("prepare plugin rollback: {error}"))?;
    }
    let final_path = directory.join(format!("{plugin_id}-{}.mira-plugin", entry.version));
    let write_result = (|| -> Result<(), String> {
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|error| format!("create plugin temporary file: {error}"))?;
        std::io::Write::write_all(&mut temporary, &bytes)
            .map_err(|error| format!("write plugin temporary file: {error}"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| format!("sync plugin temporary file: {error}"))?;
        temporary
            .persist(&final_path)
            .map_err(|error| format!("install plugin atomically: {}", error.error))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        if backup_path.exists() {
            if let Some(path) = &previous_path {
                let _ = fs::rename(&backup_path, path);
            }
        }
        return Err(error);
    }

    let runtime_result = {
        let state = app.state::<SessionState>();
        let result = state
            .plugins
            .lock()
            .map_err(|_| "plugin state lock failed".to_string())
            .map(|mut plugins| {
                let active = plugins.get_or_insert_with(Vec::new);
                active.retain(|plugin| plugin.0.plugin_id != plugin_id);
                active.push((inspection, devices, files));
            });
        // 插件集合变化后清空 ProtocolPackage 缓存，避免使用旧版本解析结果。
        invalidate_package_cache(&state);
        result
    };
    if let Err(error) = runtime_result {
        let _ = fs::remove_file(&final_path);
        if backup_path.exists() {
            if let Some(path) = &previous_path {
                let _ = fs::rename(&backup_path, path);
            }
        }
        return Err(error);
    }
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }
    Ok(PluginInstallResult {
        plugin_id: plugin_id.to_string(),
        version: entry.version,
        previous_version,
        restarted_runtime: true,
    })
}

#[tauri::command]
async fn plugin_update_install(
    app: tauri::AppHandle,
    plugin_id: String,
) -> Result<PluginInstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || install_plugin_update(&app, &plugin_id))
        .await
        .map_err(|error| format!("plugin install task failed: {error}"))?
}

#[tauri::command]
fn about_info(app: tauri::AppHandle) -> Result<AboutInfo, String> {
    let package = app.package_info();
    let bundled = active_plugins_info(&app);
    Ok(AboutInfo {
        name: package.name.to_string(),
        version: package.version.to_string(),
        identifier: app.config().identifier.to_string(),
        platform: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        build_date: env!("BUILD_DATE").to_string(),
        git_commit: env!("GIT_COMMIT").to_string(),
        bundled_plugins: bundled,
        contact: load_contact_links(),
        updater_active: read_lock_file().is_some_and(|lock| lock.release_ready),
    })
}

#[tauri::command]
fn settings_get(app: tauri::AppHandle) -> Result<AppSettings, String> {
    Ok(cached_settings(&app))
}

#[tauri::command]
fn settings_set(app: tauri::AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    let settings = settings.normalized();
    let tray_icon_color_changed = cached_settings(&app).tray_icon_color != settings.tray_icon_color;
    save_settings(&app, &settings)?;
    update_cached_settings(&app, &settings);
    if tray_icon_color_changed {
        // Force this settings change through to the native tray immediately,
        // even if the cached icon variant drifted from the currently shown icon.
        if let Ok(mut active_dark) = app.state::<SessionState>().tray_uses_dark.lock() {
            *active_dark = None;
        }
    }
    let snapshot = app
        .state::<SessionState>()
        .last_snapshot
        .lock()
        .ok()
        .and_then(|guard| primary_snapshot(&guard).cloned());
    update_tray(&app, snapshot.as_ref(), &settings)
        .map_err(|error| format!("update tray: {error}"))?;
    Ok(settings)
}

#[tauri::command]
fn export_diagnostics(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let package = app.package_info();
    let bundled = active_plugins_info(&app);
    // Diagnostics are intentionally minimal and sanitized: no serial numbers,
    // no HID report payloads, no user device identifiers. Only app metadata,
    // platform, and bundled plugin verification status are included.
    Ok(serde_json::json!({
        "app": {
            "name": package.name,
            "version": package.version.to_string(),
            "identifier": app.config().identifier,
        },
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "rust_version": env!("CARGO_PKG_RUST_VERSION"),
        "autostart_enabled": app.autolaunch().is_enabled().unwrap_or(false),
        "bundled_plugins": bundled,
        "updater_active": read_lock_file().is_some_and(|lock| lock.release_ready),
        "note": "Diagnostics contain no device serial numbers, HID payloads, or user-identifying data.",
    }))
}

fn focus_main(window: Option<WebviewWindow>) {
    if let Some(window) = window {
        let _ = window.show();
        let _ = window.set_focus();
        // macOS: 从托盘恢复窗口时，重新显示 Dock 图标。
        #[cfg(target_os = "macos")]
        {
            use tauri::ActivationPolicy;
            let _ = window
                .app_handle()
                .set_activation_policy(ActivationPolicy::Regular);
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(test, allow(dead_code))]
fn apply_windows_backdrop(window: &WebviewWindow) {
    use window_vibrancy::{apply_acrylic, apply_mica};

    if let Err(mica_error) = apply_mica(window, None) {
        if let Err(acrylic_error) = apply_acrylic(window, Some((216, 176, 183, 110))) {
            eprintln!(
                "[mira] Windows backdrop unavailable: Mica: {mica_error}; Acrylic: {acrylic_error}"
            );
        }
    }
}

const TRAY_ID: &str = "mira-status";

/// Resolve a settings language value ("auto"|"zh-CN"|"en") to a concrete language code.
/// "auto" follows the system locale via `sys_locale`; defaults to Chinese when undetectable.
fn effective_language(settings_language: &str) -> &'static str {
    match settings_language {
        "zh-CN" => "zh-CN",
        "en" => "en",
        _ => {
            let locale = get_locale().unwrap_or_default();
            if locale.starts_with("en") {
                "en"
            } else {
                "zh-CN"
            }
        }
    }
}

fn tr_connection(connection: mira_core::Connection, lang: &str) -> &'static str {
    match (connection, lang) {
        (mira_core::Connection::Usb, _) => "USB",
        (mira_core::Connection::Wireless, "en") => "Wireless",
        (mira_core::Connection::Wireless, _) => "无线",
        (mira_core::Connection::Bluetooth, "en") => "Bluetooth",
        (mira_core::Connection::Bluetooth, _) => "蓝牙",
        (mira_core::Connection::Virtual, "en") => "Virtual",
        (mira_core::Connection::Virtual, _) => "虚拟",
    }
}

fn tr_open(lang: &str) -> &'static str {
    if lang == "en" { "Open Mira" } else { "打开 Mira" }
}

fn tr_quit(lang: &str) -> &'static str {
    if lang == "en" { "Quit Mira" } else { "退出 Mira" }
}

fn tr_disconnected(lang: &str) -> &'static str {
    if lang == "en" { "No supported mouse connected" } else { "未连接受支持的鼠标" }
}

fn tr_charging_suffix(lang: &str) -> &'static str {
    if lang == "en" { " · Charging" } else { " · 充电中" }
}

fn tr_mouse_label(lang: &str) -> &'static str {
    if lang == "en" { "M" } else { "鼠" }
}

fn tr_receiver_label(lang: &str) -> &'static str {
    if lang == "en" { "R" } else { "接" }
}

fn tr_battery_fallback_label(lang: &str) -> &'static str {
    if lang == "en" { "Mouse" } else { "鼠标" }
}

fn tr_low_battery_title(lang: &str) -> &'static str {
    if lang == "en" { "Low battery alert" } else { "低电量提醒" }
}

fn tr_low_battery_body(lang: &str, threshold: u8, percent: u8) -> String {
    if lang == "en" {
        format!("Mouse battery is below {}% (currently {}%)", threshold, percent)
    } else {
        format!("鼠标电量已低于 {}%（当前 {}%）", threshold, percent)
    }
}

fn tr_tooltip_connected(lang: &str, connection: &str, name: &str) -> String {
    if lang == "en" {
        format!("Mira · {} · {}", connection, name)
    } else {
        format!("Mira · {} · {}", connection, name)
    }
}

fn tr_tooltip_disconnected(lang: &str) -> String {
    if lang == "en" {
        "Mira · No supported mouse connected".to_string()
    } else {
        "Mira · 未连接受支持的鼠标".to_string()
    }
}

fn tr_connection_status(lang: &str, connection: &str, name: &str) -> String {
    if lang == "en" {
        format!("Connection: {} · {}", connection, name)
    } else {
        format!("连接：{} · {}", connection, name)
    }
}

fn tr_battery_item(lang: &str, label: &str, percentage: u8, charging: bool) -> String {
    let charging_suffix = if charging { tr_charging_suffix(lang) } else { "" };
    if lang == "en" {
        format!("{} battery: {}%{}", label, percentage, charging_suffix)
    } else {
        format!("{}电量：{}%{}", label, percentage, charging_suffix)
    }
}

fn connection_label(connection: mira_core::Connection, lang: &str) -> &'static str {
    tr_connection(connection, lang)
}

fn battery_title(snapshot: &DeviceSnapshot, settings: &AppSettings) -> Option<String> {
    if !settings.tray_show_battery_title {
        return None;
    }
    let lang = effective_language(&settings.language);
    let mouse_percentage = mouse_battery_percentage(snapshot)?;
    let mut title = format!("{mouse_percentage}%");
    if settings.tray_include_receiver_battery {
        if let Some(receiver) = snapshot
            .batteries
            .iter()
            .find(|battery| battery.id == "receiver")
        {
            title = format!("{} {mouse_percentage}% · {} {}%", tr_mouse_label(lang), tr_receiver_label(lang), receiver.percentage);
        }
    }
    Some(title)
}

fn mouse_battery_percentage(snapshot: &DeviceSnapshot) -> Option<u8> {
    snapshot
        .batteries
        .iter()
        .find(|battery| battery.id == "mouse")
        .or_else(|| snapshot.batteries.first())
        .map(|battery| battery.percentage)
        .or(snapshot.battery_percent)
}

fn mouse_battery_charging(snapshot: &DeviceSnapshot) -> bool {
    snapshot
        .batteries
        .iter()
        .find(|battery| battery.id == "mouse")
        .or_else(|| snapshot.batteries.first())
        .map(|battery| battery.charging)
        .unwrap_or(snapshot.charging)
}

/// Battery level icons for each (dark, charging) combination.
/// Index 0 = 0%, 1 = 10%, ..., 9 = 90%, 10 = 100%.
/// `include_bytes!` requires string literals, so the 44 icons are expanded
/// into four `const` arrays once; the lookup function then indexes by level.
const TRAY_ICONS_LIGHT_CHARGING: [&[u8]; 11] = [
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-0.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-10.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-20.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-30.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-40.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-50.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-60.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-70.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-80.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-90.png"),
    include_bytes!("../icons/tray-mouse-charging-levels/mouse-100.png"),
];
const TRAY_ICONS_DARK_CHARGING: [&[u8]; 11] = [
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-0.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-10.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-20.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-30.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-40.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-50.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-60.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-70.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-80.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-90.png"),
    include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-100.png"),
];
const TRAY_ICONS_LIGHT: [&[u8]; 11] = [
    include_bytes!("../icons/tray-mouse-levels/mouse-0.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-10.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-20.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-30.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-40.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-50.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-60.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-70.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-80.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-90.png"),
    include_bytes!("../icons/tray-mouse-levels/mouse-100.png"),
];
const TRAY_ICONS_DARK: [&[u8]; 11] = [
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-0.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-10.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-20.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-30.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-40.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-50.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-60.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-70.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-80.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-90.png"),
    include_bytes!("../icons/tray-mouse-levels-dark/mouse-100.png"),
];

fn tray_icon_bytes(level: u8, dark: bool, charging: bool) -> &'static [u8] {
    // `level` is pre-rounded to a multiple of 10 by the caller (0..=100).
    // `min(10)` clamps any stray value to the 100% icon.
    let index = (level / 10).min(10) as usize;
    match (dark, charging) {
        (true, true) => TRAY_ICONS_DARK_CHARGING[index],
        (false, true) => TRAY_ICONS_LIGHT_CHARGING[index],
        (true, false) => TRAY_ICONS_DARK[index],
        (false, false) => TRAY_ICONS_LIGHT[index],
    }
}

fn tray_app_icon_bytes() -> &'static [u8] {
    include_bytes!("../icons/tray-app-icon.png")
}

/// 直接查询系统外观，不依赖窗口状态。
/// macOS: `defaults read -g AppleInterfaceStyle`（Light 模式返回非零退出码，Dark 返回 "Dark"）
/// Windows: `reg query ...AppsUseLightTheme`（0x0=Dark, 0x1=Light）
/// Linux: 回退到 window.theme()
fn detect_system_dark(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        {
            return output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .eq_ignore_ascii_case("dark");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("reg")
            .args([
                "query",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output()
        {
            // 0x0 = Dark, 0x1 = Light
            return String::from_utf8_lossy(&output.stdout).contains("0x0");
        }
    }
    // Linux 或命令不可用时，回退到 window.theme()
    app.get_webview_window("main")
        .and_then(|window| window.theme().ok())
        .map(|theme| theme == tauri::Theme::Dark)
        .unwrap_or(false)
}

/// 读取缓存的系统主题。缓存由 `read_device_once`（电量轮询）和
/// `ThemeChanged` 事件更新，避免每次 `update_tray` 都 fork 进程。
fn tray_theme_is_dark(app: &AppHandle) -> bool {
    let state = app.state::<SessionState>();
    if let Ok(cache) = state.system_dark.lock() {
        if let Some(dark) = *cache {
            return dark;
        }
    }
    // 缓存为空（首次调用），直接检测一次并填充缓存
    let dark = detect_system_dark(app);
    if let Ok(mut cache) = state.system_dark.lock() {
        *cache = Some(dark);
    }
    dark
}

fn update_tray(
    app: &AppHandle,
    snapshot: Option<&DeviceSnapshot>,
    settings: &AppSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let desired_dark = match settings.tray_icon_color.as_str() {
        "white" => true,
        "black" => false,
        _ => tray_theme_is_dark(app), // "auto" 跟随系统主题
    };
    let state = app.state::<SessionState>();
    if let Ok(mut active) = state.tray_icon_level.lock() {
        let desired_level = snapshot
            .map(|snapshot| {
                let percentage = mouse_battery_percentage(snapshot).unwrap_or(0);
                (((percentage.saturating_add(5)) / 10).min(10) * 10) as i16
            })
            .unwrap_or(-1);
        let desired_charging = snapshot.map(mouse_battery_charging).unwrap_or(false);
        let mut theme_changed = false;
        if let Ok(mut active_dark) = state.tray_uses_dark.lock() {
            theme_changed = *active_dark != Some(desired_dark);
            *active_dark = Some(desired_dark);
        }
        let mut charging_changed = false;
        if let Ok(mut active_charging) = state.tray_is_charging.lock() {
            charging_changed = *active_charging != Some(desired_charging);
            *active_charging = Some(desired_charging);
        }
        if *active != Some(desired_level) || theme_changed || charging_changed {
            if desired_level >= 0 {
                tray.set_icon(Some(tauri::image::Image::from_bytes(tray_icon_bytes(
                    desired_level as u8,
                    desired_dark,
                    desired_charging,
                ))?))?;
            } else {
                tray.set_icon(Some(
                    tauri::image::Image::from_bytes(tray_app_icon_bytes())?,
                ))?;
            }
            tray.set_icon_as_template(false)?;
            *active = Some(desired_level);
        }
    }
    let menu = Menu::new(app)?;

    let lang = effective_language(&settings.language);

    // 计算当前菜单签名，用于判断是否需要重建菜单
    let current_signature = if let Some(snapshot) = snapshot {
        let mut batteries = snapshot.batteries.clone();
        if batteries.is_empty() {
            if let Some(percentage) = snapshot.battery_percent {
                batteries.push(mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: tr_battery_fallback_label(lang).into(),
                    percentage,
                    charging: snapshot.charging,
                });
            }
        }
        TrayMenuSignature {
            connected: true,
            batteries: batteries
                .iter()
                .map(|b| (b.label.clone(), b.percentage, b.charging))
                .collect(),
            show_connection: settings.tray_show_connection,
            connection_label: connection_label(snapshot.connection, lang).to_string(),
            display_name: snapshot.display_name.clone(),
        }
    } else {
        TrayMenuSignature {
            connected: false,
            batteries: Vec::new(),
            show_connection: false,
            connection_label: String::new(),
            display_name: String::new(),
        }
    };

    // 比较签名：相同则跳过菜单重建，仅更新 title/tooltip（轻量文本操作）
    let menu_changed = state
        .tray_menu_signature
        .lock()
        .map(|cached| *cached != Some(current_signature.clone()))
        .unwrap_or(true);

    if menu_changed {
        if let Some(snapshot) = snapshot {
            let mut batteries = snapshot.batteries.clone();
            if batteries.is_empty() {
                if let Some(percentage) = snapshot.battery_percent {
                    batteries.push(mira_core::DeviceBattery {
                        id: "mouse".into(),
                        label: tr_battery_fallback_label(lang).into(),
                        percentage,
                        charging: snapshot.charging,
                    });
                }
            }
            for (index, battery) in batteries.iter().enumerate() {
                let item = MenuItem::with_id(
                    app,
                    format!("battery-{index}"),
                    tr_battery_item(lang, &battery.label, battery.percentage, battery.charging),
                    true,
                    None::<&str>,
                )?;
                menu.append(&item)?;
            }
            if settings.tray_show_connection {
                let connection = MenuItem::with_id(
                    app,
                    "connection-status",
                    tr_connection_status(
                        lang,
                        connection_label(snapshot.connection, lang),
                        &snapshot.display_name,
                    ),
                    true,
                    None::<&str>,
                )?;
                menu.append(&connection)?;
            }
        } else {
            let disconnected = MenuItem::with_id(
                app,
                "disconnected",
                tr_disconnected(lang),
                true,
                None::<&str>,
            )?;
            menu.append(&disconnected)?;
        }

        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(
            app,
            "open",
            tr_open(lang),
            true,
            None::<&str>,
        )?)?;
        menu.append(& MenuItem::with_id(
            app,
            "quit",
            tr_quit(lang),
            true,
            None::<&str>,
        )?)?;
        tray.set_menu(Some(menu))?;
        if let Ok(mut cached) = state.tray_menu_signature.lock() {
            *cached = Some(current_signature);
        }
    }

    // title/tooltip 是轻量文本操作，每次都更新
    if let Some(snapshot) = snapshot {
        // On macOS, `None` means "leave the existing title unchanged".
        // An empty string is required to actually hide a previously shown percentage.
        tray.set_title(Some(battery_title(snapshot, settings).unwrap_or_default()))?;
        tray.set_tooltip(Some(tr_tooltip_connected(
            lang,
            connection_label(snapshot.connection, lang),
            &snapshot.display_name,
        )))?;
    } else {
        tray.set_title(Some(""))?;
        tray.set_tooltip(Some(tr_tooltip_disconnected(lang)))?;
    }
    Ok(())
}

fn build_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let lang = effective_language("auto");
    let open_i = MenuItem::with_id(app, "open", tr_open(lang), true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", tr_quit(lang), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_i, &quit_i])?;
    let initial_icon = tauri::image::Image::from_bytes(tray_app_icon_bytes())?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(initial_icon)
        .icon_as_template(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            _ => {
                focus_main(app.get_webview_window("main"));
                request_refresh(&app.state::<SessionState>());
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                focus_main(app.get_webview_window("main"));
                request_refresh(&app.state::<SessionState>());
            }
        })
        .tooltip("Mira · 未连接受支持的鼠标")
        .build(app)?;
    let settings = cached_settings(app.handle());
    let snapshot = app
        .state::<SessionState>()
        .last_snapshot
        .lock()
        .ok()
        .and_then(|guard| primary_snapshot(&guard).cloned());
    update_tray(app.handle(), snapshot.as_ref(), &settings)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .manage(SessionState::default())
        .manage(production_trust_store())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            focus_main(app.get_webview_window("main"))
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // macOS native Vibrancy backdrop.
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                use window_vibrancy::{
                    apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                };
                let _ = apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::UnderWindowBackground,
                    Some(NSVisualEffectState::Active),
                    None,
                );
                // Show after applying Vibrancy to avoid a black startup flash.
                let _ = window.show();
            }

            // Windows 11 uses Mica; Windows 10 v1809+ falls back to Acrylic.
            #[cfg(target_os = "windows")]
            if let Some(window) = app.get_webview_window("main") {
                apply_windows_backdrop(&window);
            }

            // Load bundled plugins once and cache them for the app lifetime.
            let plugins = load_bundled_plugin_devices(app.handle());
            #[cfg(debug_assertions)]
            eprintln!("[mira] loaded {} bundled plugin(s)", plugins.len());
            {
                let state = app.state::<SessionState>();
                *state.plugins.lock().unwrap_or_else(|e| e.into_inner()) = Some(plugins);
                // 启动加载后清空缓存，确保首次读取使用最新插件文件。
                invalidate_package_cache(&state);
            }

            // Retry tray setup a few times: on some platforms the tray is not
            // ready immediately at startup and a single attempt fails silently.
            let mut tray_ok = false;
            for attempt in 1..=3 {
                match build_tray(app) {
                    Ok(()) => {
                        tray_ok = true;
                        break;
                    }
                    Err(err) => {
                        eprintln!("[mira] tray setup attempt {attempt}/3 failed: {err}");
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            }
            if !tray_ok {
                eprintln!(
                    "[mira] tray setup failed after 3 attempts; tray icon will be unavailable"
                );
            }

            // Listen for window events: focus triggers an immediate device read,
            // and theme changes refresh the tray icon so the mouse battery outline
            // stays readable on both light and dark menu bars/taskbars.
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            // 点击官方关闭按钮时不退出应用，改为隐藏到托盘。
                            // macOS 同时从 Dock 中隐藏，仅保留在状态栏。
                            api.prevent_close();
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.hide();
                                #[cfg(target_os = "macos")]
                                {
                                    use tauri::ActivationPolicy;
                                    let _ = app_handle
                                        .set_activation_policy(ActivationPolicy::Accessory);
                                }
                            }
                        }
                        tauri::WindowEvent::Focused(true) => {
                            // Window gained focus — refresh device state on demand.
                            // 同时刷新系统主题缓存：窗口隐藏期间系统主题可能已变化，
                            // 但 ThemeChanged 事件可能未触发（窗口不可见时）。
                            let state = app_handle.state::<SessionState>();
                            let dark = detect_system_dark(&app_handle);
                            if let Ok(mut cache) = state.system_dark.lock() {
                                *cache = Some(dark);
                            }
                            request_refresh(&app_handle.state::<SessionState>());
                        }
                        tauri::WindowEvent::ThemeChanged(_) => {
                            let state = app_handle.state::<SessionState>();
                            // 窗口可见时系统主题变化会触发此事件，立即刷新缓存
                            // 而不是等下一次电量轮询（最多 5-60 秒延迟）。
                            let dark = detect_system_dark(&app_handle);
                            if let Ok(mut cache) = state.system_dark.lock() {
                                *cache = Some(dark);
                            }
                            let snapshot = state
                                .last_snapshot
                                .lock()
                                .ok()
                                .and_then(|guard| primary_snapshot(&guard).cloned());
                            let settings = cached_settings(&app_handle);
                            let _ = update_tray(&app_handle, snapshot.as_ref(), &settings);
                        }
                        _ => {}
                    }
                });
            }

            // Spawn background thread that reads the device periodically.
            // This keeps `device_snapshot` instant — the UI never blocks on HID I/O.
            spawn_device_reader(app.handle().clone());

            let start_hidden = cached_settings(app.handle()).start_hidden;
            if let Some(window) = app.get_webview_window("main") {
                if start_hidden {
                    if let Err(err) = window.hide() {
                        eprintln!("[mira] hide main window failed: {err}");
                    }
                    // macOS: 启动即隐藏到托盘时，也从 Dock 中隐藏。
                    #[cfg(target_os = "macos")]
                    {
                        use tauri::ActivationPolicy;
                        app.set_activation_policy(ActivationPolicy::Accessory);
                    }
                } else {
                    if let Err(err) = window.show() {
                        eprintln!("[mira] show main window failed: {err}");
                    }
                    let _ = window.set_focus();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            device_snapshot,
            device_snapshots,
            device_refresh,
            device_mutate,
            discover_devices,
            autostart_state,
            set_autostart,
            plugin_updates_check,
            plugin_update_install,
            device_config_export,
            device_config_import,
            about_info,
            settings_get,
            settings_set,
            export_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("Mira application runtime failed");
}
