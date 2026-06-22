// SPDX-License-Identifier: AGPL-3.0-or-later
use ed25519_dalek::VerifyingKey;
use hidapi::HidApi;
use mira_core::{DeviceSnapshot, LowBatteryCrossing, PluginCapability, PluginCapabilityPlacement};
use mira_plugin_runtime::{
    extract_package, hid, inspect_package, mutate_device, read_device, writable_mutations,
    ConnectionKind, DeviceReading, PackageInspection, ProtocolContext, TrustStore,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Cursor,
    path::PathBuf,
    sync::Mutex,
};
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

fn plugin_capabilities(inspection: &PackageInspection) -> Vec<PluginCapability> {
    inspection
        .capabilities
        .iter()
        .map(|capability| PluginCapability {
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
        })
        .collect()
}

#[derive(Default)]
struct SessionState {
    write_in_progress: Mutex<bool>,
    device_io: Mutex<()>,
    last_snapshot: Mutex<Option<DeviceSnapshot>>,
    plugins: Mutex<Option<CachedPlugins>>,
    tray_icon_level: Mutex<Option<i16>>,
    tray_is_charging: Mutex<Option<bool>>,
    tray_uses_dark: Mutex<Option<bool>>,
    /// 缓存系统主题检测结果，避免每次 update_tray 都 fork 进程。
    /// 由 read_device_once（电量轮询）和 ThemeChanged 事件更新。
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

/// Number of fast polls performed after a state transition is detected.
/// At 500 ms per poll this covers a 3-second settling window.
const SETTLING_POLL_COUNT: u8 = 6;

/// Mark that the device state just changed, enabling a short burst of fast polls.
/// This is used for plug/unplug, charging state changes, and after device writes
/// so the UI catches the tail end of the transition without continuous polling.
fn note_state_change(state: &SessionState) {
    if let Ok(mut polls) = state.settling_polls.lock() {
        *polls = SETTLING_POLL_COUNT;
    }
}

/// Update the cached snapshot and, if it actually changed, enable the settling burst.
fn store_snapshot(state: &SessionState, snapshot: Option<DeviceSnapshot>) {
    let mut guard = state.last_snapshot.lock().unwrap();
    let changed = guard.as_ref() != snapshot.as_ref();
    *guard = snapshot;
    drop(guard);
    if changed {
        note_state_change(state);
    }
}

// Production plugin signing key for hello-yunshu/mira-mouse-plugins.
// Replace with the real key id and public key after the first production release.
const PRODUCTION_KEY_ID: &str = "mira-plugins-2026-001";
const PRODUCTION_PUBLIC_KEY_HEX: &str =
    "eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3";

// Test packages are trusted in all builds for local development.
// Remove this before a production release.
const TEST_KEY_ID: &str = "TEST-ONLY-mira-plugins";
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

#[derive(Serialize)]
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
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ContactLinks {
    github: Option<String>,
    x: Option<String>,
    telegram: Option<String>,
    developer_name: Option<String>,
    copyright: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
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
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SoftwareProfileStore {
    schema_version: u32,
    devices: BTreeMap<String, SoftwareProfile>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SoftwareProfile {
    mutations: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
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
        }
    }
}

impl AppSettings {
    fn normalized(mut self) -> Self {
        let defaults = Self::default();
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = defaults.theme;
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
    }

    #[test]
    fn invalid_saved_values_are_repaired() {
        let settings = AppSettings {
            theme: String::new(),
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
    fn device_writes_are_exclusive_and_release_after_completion() {
        let state = SessionState::default();
        let guard = begin_device_write(&state).unwrap();
        assert!(begin_device_write(&state).is_err());
        drop(guard);
        assert!(begin_device_write(&state).is_ok());
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockFile {
    #[allow(dead_code)]
    schema_version: u32,
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
            })
        })
        .collect()
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
    lock.plugins
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
        .collect()
}

fn load_contact_links() -> ContactLinks {
    // Contact links are loaded from config/project-metadata.toml when provided.
    // Until the user supplies verified GitHub/X/Telegram URLs, all entries remain None
    // and the About page hides them per spec §10.2.
    let path = PathBuf::from("config/project-metadata.toml");
    if let Ok(text) = fs::read_to_string(&path) {
        let mut github = None;
        let mut x = None;
        let mut telegram = None;
        let mut developer_name = None;
        let mut copyright = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("main_repository_url = ") {
                github = parse_toml_string(value);
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
            x,
            telegram,
            developer_name,
            copyright,
        }
    } else {
        ContactLinks::default()
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

fn save_software_profiles(app: &AppHandle, profiles: &SoftwareProfileStore) -> Result<(), String> {
    let path = software_profiles_path(app).ok_or_else(|| "config dir unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config dir: {error}"))?;
    }
    let text = serde_json::to_string_pretty(profiles)
        .map_err(|error| format!("serialize device profiles: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|error| format!("write device profiles: {error}"))?;
    fs::rename(&tmp, &path).map_err(|error| format!("commit device profiles: {error}"))
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

fn runtime_connection(value: ConnectionKind) -> mira_core::Connection {
    match value {
        ConnectionKind::Usb => mira_core::Connection::Usb,
        ConnectionKind::Wireless => mira_core::Connection::Wireless,
        ConnectionKind::Bluetooth => mira_core::Connection::Bluetooth,
    }
}

fn device_evidence_allows_writes(evidence: &str) -> bool {
    matches!(evidence, "hardware-verified" | "protocol-verified")
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

#[tauri::command]
fn device_snapshot(state: tauri::State<'_, SessionState>) -> Option<DeviceSnapshot> {
    state.last_snapshot.lock().ok()?.as_ref().cloned()
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

fn reapply_software_profile(
    app: &AppHandle,
    state: &SessionState,
    api: &HidApi,
    device: &hid::MatchedDevice,
    connection: ConnectionKind,
    files: &BTreeMap<String, Vec<u8>>,
    reading: &DeviceReading,
    allowed: &[String],
) -> Option<DeviceReading> {
    let key = software_profile_key(device, reading);
    let profiles = load_software_profiles(app);
    let profile = profiles.devices.get(&key)?;
    let already_applied = state.applied_software_profiles.lock().ok()?.contains(&key);
    if already_applied && control_mode(reading) == Some(2) {
        return None;
    }
    let context = ProtocolContext {
        api,
        path: &device.path,
        family: &device.family,
        connection,
        files,
        outputs: reading.capabilities.clone(),
    };
    let mut failed = false;
    if allowed
        .iter()
        .any(|mutation| mutation == "set-control-mode")
        && control_mode(reading) != Some(2)
    {
        let params = serde_json::Map::from_iter([("mode".into(), serde_json::json!(2))]);
        if let Err(error) = mutate_device(&context, "set-control-mode", &params) {
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
            if let Err(error) = mutate_device(&context, mutation, &params) {
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
        read_device(&ProtocolContext {
            api,
            path: &device.path,
            family: &device.family,
            connection,
            files,
            outputs: BTreeMap::new(),
        })
        .ok()
    }
}

/// Read the device once, update the cached snapshot, and emit `device-updated`.
/// Called by the background reader thread on every loop iteration (whether
/// triggered by a signal or the fallback timeout).
fn read_device_once(app: &AppHandle) {
    let state = app.state::<SessionState>();
    // 顺便刷新系统主题缓存：和电量轮询合并，不产生额外的轮询开销。
    // 这样 update_tray 只读缓存，无需每次都 fork 进程检测主题。
    let dark = detect_system_dark(app);
    if let Ok(mut cache) = state.system_dark.lock() {
        *cache = Some(dark);
    }
    let _io_guard = match state.device_io.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let plugins_guard = match state.plugins.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let Some(plugins) = plugins_guard.as_ref() else {
        return;
    };
    if plugins.is_empty() {
        return;
    }
    let Ok(api) = HidApi::new() else {
        return;
    };
    let matched = hid::enumerate_matched_devices(&api, plugins);
    #[cfg(debug_assertions)]
    eprintln!("[mira] background: matched {} device(s)", matched.len());

    let Some(first) = matched
        .iter()
        .find(|device| device_evidence_allows_writes(&device.evidence))
        .or_else(|| matched.first())
    else {
        if let Ok(mut applied) = state.applied_software_profiles.lock() {
            applied.clear();
        }
        store_snapshot(&state, None);
        let _ = app.emit("device-updated", Option::<DeviceSnapshot>::None);
        let _ = update_tray(app, None, &load_settings(app));
        return;
    };

    let Some((inspection, devices, plugin_files)) = plugins
        .iter()
        .find(|(inspection, _, _)| inspection.plugin_id == first.plugin_id)
    else {
        return;
    };

    let connection = parse_connection(&first.connection);
    let kind = match connection {
        mira_core::Connection::Usb => ConnectionKind::Usb,
        mira_core::Connection::Wireless => ConnectionKind::Wireless,
        mira_core::Connection::Bluetooth => ConnectionKind::Bluetooth,
        mira_core::Connection::Virtual => ConnectionKind::Usb,
    };

    match read_device(&ProtocolContext {
        api: &api,
        path: &first.path,
        family: &first.family,
        connection: kind,
        files: plugin_files,
        outputs: BTreeMap::new(),
    }) {
        Ok(mut reading) => {
            let writable_mutations = if inspection.signature_verified
                && inspection.writes_enabled
                && device_evidence_allows_writes(&first.evidence)
            {
                writable_mutations(&ProtocolContext {
                    api: &api,
                    path: &first.path,
                    family: &first.family,
                    connection: kind,
                    files: plugin_files,
                    outputs: reading.capabilities.clone(),
                })
                .unwrap_or_default()
            } else {
                Vec::new()
            };
            if let Some(updated) = reapply_software_profile(
                app,
                &state,
                &api,
                first,
                kind,
                plugin_files,
                &reading,
                &writable_mutations,
            ) {
                reading = updated;
            }
            let resolved_name = reading.display_name.clone().unwrap_or_else(|| {
                display_name(
                    &first.plugin_id,
                    &first.family,
                    &devices.hardware_verified_models,
                    &first.evidence,
                )
            });
            let resolved_connection = reading
                .connection
                .map(runtime_connection)
                .unwrap_or(connection);
            let snapshot = DeviceSnapshot {
                display_name: resolved_name,
                connection: resolved_connection,
                battery_percent: reading.battery_percent,
                charging: reading.charging,
                batteries: reading.batteries,
                dpi: reading.dpi,
                dpi_stages: reading.dpi_stages,
                polling_rate_hz: reading.polling_rate_hz,
                supported_polling_rates_hz: reading.supported_polling_rates_hz,
                profile: reading.profile.map(|p| format!("Profile {}", p + 1)),
                confirmed_light_color: reading.light_color,
                capabilities: reading.capabilities,
                plugin_capabilities: plugin_capabilities(inspection),
                writable_mutations,
                evidence: first.evidence.clone(),
            };
            store_snapshot(&state, Some(snapshot.clone()));
            // 通知前端有新数据，前端通过事件监听更新，无需轮询
            let _ = app.emit("device-updated", &snapshot);
            let settings = load_settings(app);
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
                .unwrap()
                .update(battery_value, threshold);
            if notify {
                if let Some(percent) = battery_value {
                    let _ = app
                        .notification()
                        .builder()
                        .title("低电量提醒")
                        .body(format!(
                            "鼠标电量已低于 {}%（当前 {}%）",
                            threshold, percent
                        ))
                        .show();
                }
            }
        }
        Err(error) => {
            if let Ok(mut applied) = state.applied_software_profiles.lock() {
                applied.clear();
            }
            eprintln!(
                "[mira] background read failed for {}: {error}",
                first.family
            );
            // 读取失败时通知前端清空设备状态
            store_snapshot(&state, None);
            let _ = app.emit("device-updated", Option::<DeviceSnapshot>::None);
            let _ = update_tray(app, None, &load_settings(app));
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
    *app.state::<SessionState>().refresh_tx.lock().unwrap() = Some(tx);

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
            let connected = state.last_snapshot.lock().unwrap().is_some();
            let mut settling = state.settling_polls.lock().unwrap();
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
            std::time::Duration::from_secs(u64::from(load_settings(&app).refresh_interval_seconds))
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
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let matched = hid::enumerate_matched_devices(&api, plugins);
    Ok(matched
        .into_iter()
        .map(|d| DiscoveredDevice {
            plugin_id: d.plugin_id,
            family: d.family,
            connection: d.connection,
            evidence: d.evidence,
            path: d.path,
            vendor_id: d.vendor_id,
            product_id: d.product_id,
            usage_page: d.usage_page,
            usage: d.usage,
        })
        .collect())
}

#[tauri::command]
fn can_install_update(state: tauri::State<'_, SessionState>) -> Result<(), String> {
    if *state
        .write_in_progress
        .lock()
        .map_err(|_| "transaction state unavailable")?
    {
        Err("A device write is still in progress".into())
    } else {
        Ok(())
    }
}

struct WriteFlagGuard<'a>(&'a Mutex<bool>);

impl Drop for WriteFlagGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = false;
        }
    }
}

fn begin_device_write(state: &SessionState) -> Result<WriteFlagGuard<'_>, String> {
    let mut active = state
        .write_in_progress
        .lock()
        .map_err(|_| "transaction state unavailable")?;
    if *active {
        return Err("A device write is still in progress".into());
    }
    *active = true;
    drop(active);
    Ok(WriteFlagGuard(&state.write_in_progress))
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
    let mut profiles = load_software_profiles(app);
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
    let app = app.clone();
    std::thread::spawn(move || {
        let result = device_mutate_blocking(&app, &mutation, &params);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("设备写入超时（30 秒）。鼠标可能处于休眠状态，请移动鼠标唤醒后重试。".into())
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
    let _write_guard = begin_device_write(&state)?;
    let _io_guard = state
        .device_io
        .lock()
        .map_err(|_| "device I/O state unavailable")?;
    let plugins_guard = state.plugins.lock().map_err(|_| "state lock failed")?;
    let plugins = plugins_guard.as_ref().ok_or("plugins not loaded")?;
    let api = HidApi::new().map_err(|error| error.to_string())?;
    let matched = hid::enumerate_matched_devices(&api, plugins);
    let device = matched
        .iter()
        .find(|device| device_evidence_allows_writes(&device.evidence))
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

    let connection = parse_connection(&device.connection);
    let kind = match connection {
        mira_core::Connection::Usb => ConnectionKind::Usb,
        mira_core::Connection::Wireless => ConnectionKind::Wireless,
        mira_core::Connection::Bluetooth => ConnectionKind::Bluetooth,
        mira_core::Connection::Virtual => ConnectionKind::Usb,
    };
    let context = ProtocolContext {
        api: &api,
        path: &device.path,
        family: &device.family,
        connection: kind,
        files,
        outputs: BTreeMap::new(),
    };
    let allowed = writable_mutations(&context)?;
    if !allowed.iter().any(|candidate| candidate == mutation) {
        return Err(format!("unsupported device mutation {mutation}"));
    }
    let before = read_device(&context)?;
    let mutate_context = ProtocolContext {
        api: &api,
        path: &device.path,
        family: &device.family,
        connection: kind,
        files,
        outputs: before.capabilities.clone(),
    };
    let mutation_result = mutate_device(&mutate_context, mutation, params)?;
    let reading = read_device(&context)?;
    remember_software_profile(app, &state, device, &reading, &allowed, mutation, params)?;
    let resolved_name = reading.display_name.clone().unwrap_or_else(|| {
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
        .unwrap_or(connection);
    let snapshot = DeviceSnapshot {
        display_name: resolved_name,
        connection: resolved_connection,
        battery_percent: reading.battery_percent,
        charging: reading.charging,
        batteries: reading.batteries,
        dpi: reading.dpi,
        dpi_stages: reading.dpi_stages,
        polling_rate_hz: reading.polling_rate_hz,
        supported_polling_rates_hz: reading.supported_polling_rates_hz,
        profile: reading
            .profile
            .map(|profile| format!("Profile {}", profile + 1)),
        confirmed_light_color: reading.light_color.or_else(|| {
            mutation_result
                .get("color")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        }),
        capabilities: reading.capabilities,
        plugin_capabilities: plugin_capabilities(inspection),
        writable_mutations: allowed,
        evidence: device.evidence.clone(),
    };
    store_snapshot(&state, Some(snapshot.clone()));
    let _ = app.emit("device-updated", &snapshot);
    let _ = update_tray(app, Some(&snapshot), &load_settings(app));
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

#[tauri::command]
fn app_metadata(app: tauri::AppHandle) -> serde_json::Value {
    let package = app.package_info();
    serde_json::json!({
        "name": package.name,
        "version": package.version.to_string(),
        "identifier": app.config().identifier,
    })
}

#[tauri::command]
fn about_info(app: tauri::AppHandle) -> Result<AboutInfo, String> {
    let package = app.package_info();
    let bundled = inspect_bundled_plugins(&app, &production_trust_store());
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
        updater_active: true,
    })
}

#[tauri::command]
fn settings_get(app: tauri::AppHandle) -> Result<AppSettings, String> {
    Ok(load_settings(&app))
}

#[tauri::command]
fn settings_set(app: tauri::AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    let settings = settings.normalized();
    save_settings(&app, &settings)?;
    let snapshot = app
        .state::<SessionState>()
        .last_snapshot
        .lock()
        .ok()
        .and_then(|snapshot| snapshot.clone());
    update_tray(&app, snapshot.as_ref(), &settings)
        .map_err(|error| format!("update tray: {error}"))?;
    Ok(settings)
}

#[tauri::command]
fn export_diagnostics(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let package = app.package_info();
    let bundled = inspect_bundled_plugins(&app, &production_trust_store());
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
        "updater_active": true,
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

fn connection_label(connection: mira_core::Connection) -> &'static str {
    match connection {
        mira_core::Connection::Usb => "USB",
        mira_core::Connection::Wireless => "无线",
        mira_core::Connection::Bluetooth => "蓝牙",
        mira_core::Connection::Virtual => "虚拟",
    }
}

fn battery_title(snapshot: &DeviceSnapshot, settings: &AppSettings) -> Option<String> {
    if !settings.tray_show_battery_title {
        return None;
    }
    let mouse_percentage = mouse_battery_percentage(snapshot)?;
    let mut title = format!("{mouse_percentage}%");
    if settings.tray_include_receiver_battery {
        if let Some(receiver) = snapshot
            .batteries
            .iter()
            .find(|battery| battery.id == "receiver")
        {
            title = format!("鼠 {mouse_percentage}% · 接 {}%", receiver.percentage);
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

fn tray_icon_bytes(level: u8, dark: bool, charging: bool) -> &'static [u8] {
    // include_bytes! requires a string literal, so each combination is expanded explicitly.
    // Dark mode shows a white outline (tray-mouse-levels-dark/) and light mode a black
    // outline (tray-mouse-levels/) so the battery icon stays readable on both backgrounds.
    match (level, dark, charging) {
        (0, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-0.png"),
        (10, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-10.png"),
        (20, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-20.png"),
        (30, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-30.png"),
        (40, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-40.png"),
        (50, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-50.png"),
        (60, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-60.png"),
        (70, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-70.png"),
        (80, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-80.png"),
        (90, true, true) => include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-90.png"),
        (0..=100, true, true) => {
            include_bytes!("../icons/tray-mouse-charging-levels-dark/mouse-100.png")
        }
        (0, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-0.png"),
        (10, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-10.png"),
        (20, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-20.png"),
        (30, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-30.png"),
        (40, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-40.png"),
        (50, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-50.png"),
        (60, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-60.png"),
        (70, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-70.png"),
        (80, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-80.png"),
        (90, false, true) => include_bytes!("../icons/tray-mouse-charging-levels/mouse-90.png"),
        (0..=100, false, true) => {
            include_bytes!("../icons/tray-mouse-charging-levels/mouse-100.png")
        }
        (0, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-0.png"),
        (10, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-10.png"),
        (20, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-20.png"),
        (30, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-30.png"),
        (40, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-40.png"),
        (50, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-50.png"),
        (60, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-60.png"),
        (70, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-70.png"),
        (80, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-80.png"),
        (90, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-90.png"),
        (0..=100, true, false) => include_bytes!("../icons/tray-mouse-levels-dark/mouse-100.png"),
        (0, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-0.png"),
        (10, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-10.png"),
        (20, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-20.png"),
        (30, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-30.png"),
        (40, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-40.png"),
        (50, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-50.png"),
        (60, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-60.png"),
        (70, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-70.png"),
        (80, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-80.png"),
        (90, false, false) => include_bytes!("../icons/tray-mouse-levels/mouse-90.png"),
        _ => include_bytes!("../icons/tray-mouse-levels/mouse-100.png"),
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

    if let Some(snapshot) = snapshot {
        let mut batteries = snapshot.batteries.clone();
        if batteries.is_empty() {
            if let Some(percentage) = snapshot.battery_percent {
                batteries.push(mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: "鼠标".into(),
                    percentage,
                    charging: snapshot.charging,
                });
            }
        }
        for (index, battery) in batteries.iter().enumerate() {
            let charging = if battery.charging {
                " · 充电中"
            } else {
                ""
            };
            let item = MenuItem::with_id(
                app,
                format!("battery-{index}"),
                format!("{}电量：{}%{charging}", battery.label, battery.percentage),
                true,
                None::<&str>,
            )?;
            menu.append(&item)?;
        }
        if settings.tray_show_connection {
            let connection = MenuItem::with_id(
                app,
                "connection-status",
                format!(
                    "连接：{} · {}",
                    connection_label(snapshot.connection),
                    snapshot.display_name
                ),
                true,
                None::<&str>,
            )?;
            menu.append(&connection)?;
        }
        // On macOS, `None` means "leave the existing title unchanged".
        // An empty string is required to actually hide a previously shown percentage.
        tray.set_title(Some(battery_title(snapshot, settings).unwrap_or_default()))?;
        tray.set_tooltip(Some(format!(
            "Mira · {} · {}",
            connection_label(snapshot.connection),
            snapshot.display_name
        )))?;
    } else {
        let disconnected = MenuItem::with_id(
            app,
            "disconnected",
            "未连接受支持的鼠标",
            true,
            None::<&str>,
        )?;
        menu.append(&disconnected)?;
        tray.set_title(Some(""))?;
        tray.set_tooltip(Some("Mira · 未连接受支持的鼠标"))?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "open",
        "打开 Mira",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "退出 Mira",
        true,
        None::<&str>,
    )?)?;
    tray.set_menu(Some(menu))?;
    Ok(())
}

fn build_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let open_i = MenuItem::with_id(app, "open", "打开 Mira", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出 Mira", true, None::<&str>)?;
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
    let settings = load_settings(app.handle());
    let snapshot = app
        .state::<SessionState>()
        .last_snapshot
        .lock()
        .ok()
        .and_then(|value| value.clone());
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
        .plugin(tauri_plugin_notification::init())
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
            *app.state::<SessionState>().plugins.lock().unwrap() = Some(plugins);

            if let Err(err) = build_tray(app) {
                eprintln!(" tray setup failed: {err}");
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
                                .and_then(|guard| guard.clone());
                            let settings = load_settings(&app_handle);
                            let _ = update_tray(&app_handle, snapshot.as_ref(), &settings);
                        }
                        _ => {}
                    }
                });
            }

            // Spawn background thread that reads the device periodically.
            // This keeps `device_snapshot` instant — the UI never blocks on HID I/O.
            spawn_device_reader(app.handle().clone());

            let start_hidden = load_settings(app.handle()).start_hidden;
            if let Some(window) = app.get_webview_window("main") {
                if start_hidden {
                    if let Err(err) = window.hide() {
                        eprintln!("[mira] hide main window failed: {err}");
                    }
                    // macOS: 启动即隐藏到托盘时，也从 Dock 中隐藏。
                    #[cfg(target_os = "macos")]
                    {
                        use tauri::ActivationPolicy;
                        let _ = app.set_activation_policy(ActivationPolicy::Accessory);
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
            device_refresh,
            device_mutate,
            discover_devices,
            can_install_update,
            autostart_state,
            set_autostart,
            app_metadata,
            about_info,
            settings_get,
            settings_set,
            export_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("Mira application runtime failed");
}
