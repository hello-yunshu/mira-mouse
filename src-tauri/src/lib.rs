// SPDX-License-Identifier: AGPL-3.0-or-later
use ed25519_dalek::VerifyingKey;
use hidapi::HidApi;
use mira_core::DeviceSnapshot;
use mira_plugin_runtime::{
    extract_package, hid, inspect_package, read_device, ConnectionKind, PackageInspection,
    ProtocolContext, TrustStore,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor, path::PathBuf, sync::Mutex};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, WebviewWindow,
};
use tauri_plugin_autostart::ManagerExt;

type CachedPlugins = Vec<(
    PackageInspection,
    hid::DevicesFile,
    std::collections::BTreeMap<String, Vec<u8>>,
)>;

#[derive(Default)]
struct SessionState {
    write_in_progress: Mutex<bool>,
    last_snapshot: Mutex<Option<DeviceSnapshot>>,
    plugins: Mutex<Option<CachedPlugins>>,
    tray_icon_level: Mutex<Option<i16>>,
}

// Production plugin signing key for hello-yunshu/mira-mouse-plugins.
// Replace with the real key id and public key after the first production release.
const PRODUCTION_KEY_ID: &str = "mira-plugins-2026-001";
const PRODUCTION_PUBLIC_KEY_HEX: &str =
    "eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3";

// 测试用签名密钥（仅本地测试，生产构建前移除）
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
    low_battery_threshold: u8,
    night_mode_enabled: bool,
    night_mode_start: String,
    night_mode_end: String,
    refresh_interval_seconds: u16,
    telemetry_disabled: bool,
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
    fn tray_title_uses_primary_and_optional_receiver_batteries() {
        let snapshot = DeviceSnapshot {
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
            profile: None,
            confirmed_light_color: None,
            capabilities: Default::default(),
            evidence: "hardware-verified".into(),
        };
        let mut settings = AppSettings::default();
        assert_eq!(battery_title(&snapshot, &settings).as_deref(), Some("64%"));
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

fn parse_connection(value: &str) -> mira_core::Connection {
    match value {
        "usb" => mira_core::Connection::Usb,
        "wireless" | "wireless-receiver" => mira_core::Connection::Wireless,
        "bluetooth" => mira_core::Connection::Bluetooth,
        _ => mira_core::Connection::Usb,
    }
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

/// Background thread that periodically reads the device and updates the cached snapshot.
/// This keeps the UI responsive — `device_snapshot` returns instantly from the cache.
fn spawn_device_reader(app: AppHandle) {
    std::thread::spawn(move || loop {
        {
            let state = app.state::<SessionState>();
            let plugins_guard = state.plugins.lock().unwrap();

            if let Some(plugins) = plugins_guard.as_ref() {
                if !plugins.is_empty() {
                    if let Ok(api) = HidApi::new() {
                        let matched = hid::enumerate_matched_devices(&api, plugins);
                        #[cfg(debug_assertions)]
                        eprintln!("[mira] background: matched {} device(s)", matched.len());

                        if let Some(first) = matched.first() {
                            if let Some((_, devices, plugin_files)) = plugins
                                .iter()
                                .find(|(inspection, _, _)| inspection.plugin_id == first.plugin_id)
                            {
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
                                }) {
                                    Ok(reading) => {
                                        let snapshot = DeviceSnapshot {
                                            display_name: display_name(
                                                &first.plugin_id,
                                                &first.family,
                                                &devices.hardware_verified_models,
                                                &first.evidence,
                                            ),
                                            connection,
                                            battery_percent: reading.battery_percent,
                                            charging: reading.charging,
                                            batteries: reading.batteries,
                                            dpi: reading.dpi,
                                            dpi_stages: reading.dpi_stages,
                                            polling_rate_hz: reading.polling_rate_hz,
                                            profile: reading
                                                .profile
                                                .map(|p| format!("Profile {}", p + 1)),
                                            confirmed_light_color: reading.light_color,
                                            capabilities: reading.capabilities,
                                            evidence: first.evidence.clone(),
                                        };
                                        *state.last_snapshot.lock().unwrap() =
                                            Some(snapshot.clone());
                                        // 通知前端有新数据，前端通过事件监听更新，无需轮询
                                        let _ = app.emit("device-updated", &snapshot);
                                        let _ = update_tray(
                                            &app,
                                            Some(&snapshot),
                                            &load_settings(&app),
                                        );
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "[mira] background read failed for {}: {error}",
                                            first.family
                                        );
                                        // 读取失败时通知前端清空设备状态
                                        *state.last_snapshot.lock().unwrap() = None;
                                        let _ = app
                                            .emit("device-updated", Option::<DeviceSnapshot>::None);
                                        let _ = update_tray(&app, None, &load_settings(&app));
                                    }
                                }
                            }
                        } else {
                            *state.last_snapshot.lock().unwrap() = None;
                            let _ = app.emit("device-updated", Option::<DeviceSnapshot>::None);
                            let _ = update_tray(&app, None, &load_settings(&app));
                        }
                    }
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(10));
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

fn tray_icon_bytes(level: u8) -> &'static [u8] {
    match level {
        0 => include_bytes!("../icons/tray-mouse-levels/mouse-0.png"),
        10 => include_bytes!("../icons/tray-mouse-levels/mouse-10.png"),
        20 => include_bytes!("../icons/tray-mouse-levels/mouse-20.png"),
        30 => include_bytes!("../icons/tray-mouse-levels/mouse-30.png"),
        40 => include_bytes!("../icons/tray-mouse-levels/mouse-40.png"),
        50 => include_bytes!("../icons/tray-mouse-levels/mouse-50.png"),
        60 => include_bytes!("../icons/tray-mouse-levels/mouse-60.png"),
        70 => include_bytes!("../icons/tray-mouse-levels/mouse-70.png"),
        80 => include_bytes!("../icons/tray-mouse-levels/mouse-80.png"),
        90 => include_bytes!("../icons/tray-mouse-levels/mouse-90.png"),
        _ => include_bytes!("../icons/tray-mouse-levels/mouse-100.png"),
    }
}

fn update_tray(
    app: &AppHandle,
    snapshot: Option<&DeviceSnapshot>,
    settings: &AppSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    if let Ok(mut active) = app.state::<SessionState>().tray_icon_level.lock() {
        let desired_level = snapshot
            .map(|snapshot| {
                let percentage = mouse_battery_percentage(snapshot).unwrap_or(0);
                (((percentage.saturating_add(5)) / 10).min(10) * 10) as i16
            })
            .unwrap_or(-1);
        if *active != Some(desired_level) {
            if desired_level >= 0 {
                tray.set_icon(Some(tauri::image::Image::from_bytes(tray_icon_bytes(
                    desired_level as u8,
                ))?))?;
            } else {
                tray.set_icon(app.default_window_icon().cloned())?;
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
                false,
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
                false,
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
            false,
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

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or("missing default icon")?,
        )
        .icon_as_template(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => focus_main(app.get_webview_window("main")),
            "quit" => app.exit(0),
            _ => {}
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

            // Spawn background thread that reads the device periodically.
            // This keeps `device_snapshot` instant — the UI never blocks on HID I/O.
            spawn_device_reader(app.handle().clone());

            let start_hidden = load_settings(app.handle()).start_hidden;
            if let Some(window) = app.get_webview_window("main") {
                if start_hidden {
                    if let Err(err) = window.hide() {
                        eprintln!("[mira] hide main window failed: {err}");
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
