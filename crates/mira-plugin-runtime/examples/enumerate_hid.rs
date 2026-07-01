// SPDX-License-Identifier: AGPL-3.0-or-later
// Generic real-device verification through the signed plugin workflow.
//
// Usage:
//   cargo run --example enumerate_hid                          # defaults to mira.amaster
//   MIRA_PLUGIN=mira.logitech-hidpp cargo run --example enumerate_hid
//   MIRA_PLUGIN_PATH=/path/to/extracted/plugin cargo run --example enumerate_hid
//
// The tool loads plugins.lock.json, finds the requested plugin entry,
// verifies the package signature against the production + TEST-ONLY trust
// store, enumerates matched HID devices, and runs the signed plugin
// workflow. Set MIRA_WRITE_SMOKE=1 to additionally exercise no-op
// write/readback smoke tests (only for plugins with writesEnabled).
//
// If MIRA_PLUGIN_PATH points to a directory, the example loads the extracted
// plugin files directly without signature verification (useful for local
// plugin development).
use ed25519_dalek::VerifyingKey;
use hidapi::HidApi;
use mira_plugin_runtime::{
    execute_plugin_workflow, extract_package, hid, inspect_package, mutate_device, read_device,
    writable_mutations, ConnectionKind, FeatureIndexCache, HidHandleCache, HidIoStats,
    OnboardMemoryCache, ProtocolContext, TrustStore,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Cursor, path::PathBuf, sync::Mutex};

const PRODUCTION_KEY_ID: &str = "mira-plugins-2026-001";
const PRODUCTION_PUBLIC_KEY_HEX: &str =
    "eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3";

const TEST_KEY_ID: &str = "TEST-ONLY-mira-plugins";
const TEST_PUBLIC_KEY_HEX: &str =
    "00d34dac6e039baada3d3d9aa65390f2887d09d73b396af8434ecb29c233d666";

fn trust_store() -> TrustStore {
    let mut trust = TrustStore::default();
    for (key_id, hex_str) in [
        (PRODUCTION_KEY_ID, PRODUCTION_PUBLIC_KEY_HEX),
        (TEST_KEY_ID, TEST_PUBLIC_KEY_HEX),
    ] {
        let bytes = hex::decode(hex_str).expect("valid public key hex");
        let key = VerifyingKey::from_bytes(&bytes.try_into().expect("32-byte public key"))
            .expect("valid Ed25519 public key");
        trust.0.insert(key_id.to_string(), key);
    }
    trust
}

fn collect_files_recursive(base: &PathBuf, dir: &PathBuf, files: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(dir).expect("read plugin directory") {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if entry.file_type().expect("file type").is_dir() {
            collect_files_recursive(base, &path, files);
        } else if entry.file_type().expect("file type").is_file() {
            let rel = path
                .strip_prefix(base)
                .expect("entry under plugin dir")
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path).expect("read plugin file");
            files.insert(rel, bytes);
        }
    }
}

fn load_extracted_plugin(
    path: &PathBuf,
) -> (
    mira_plugin_runtime::PackageInspection,
    BTreeMap<String, Vec<u8>>,
) {
    let manifest: Value =
        serde_json::from_slice(&fs::read(path.join("plugin.json")).expect("read plugin.json"))
            .expect("parse plugin.json");
    let mut files = BTreeMap::new();
    collect_files_recursive(path, path, &mut files);
    let inspection = mira_plugin_runtime::PackageInspection {
        plugin_id: manifest["pluginId"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        version: manifest["version"].as_str().unwrap_or("0.0.0").to_string(),
        evidence: "development-extracted".to_string(),
        signature_verified: false,
        writes_enabled: manifest["writesEnabled"].as_bool().unwrap_or(false),
        capabilities: serde_json::from_value(manifest["capabilities"].clone()).unwrap_or_default(),
        exportable_fields: serde_json::from_value(manifest["exportableFields"].clone())
            .unwrap_or_default(),
        depends_on: serde_json::from_value(manifest["dependsOn"].clone()).unwrap_or_default(),
        file_count: files.len(),
    };
    println!("source: extracted directory {}", path.display());
    println!("sha256: (not computed for extracted source)");
    (inspection, files)
}

fn load_packaged_plugin(
    path: &PathBuf,
) -> (
    mira_plugin_runtime::PackageInspection,
    BTreeMap<String, Vec<u8>>,
) {
    let bytes = fs::read(path).expect("read plugin package");
    println!("sha256: {}", hex::encode(Sha256::digest(&bytes)));

    let trust = trust_store();
    let inspection = inspect_package(Cursor::new(&bytes), &trust, true).expect("verify package");
    let (_, files) = extract_package(Cursor::new(&bytes), &trust, true).expect("extract package");
    (inspection, files)
}

fn main() {
    let plugin_id = std::env::var("MIRA_PLUGIN").unwrap_or_else(|_| "mira.amaster".to_string());
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock: Value = serde_json::from_slice(
        &fs::read(workspace.join("plugins.lock.json")).expect("read plugins.lock.json"),
    )
    .expect("parse plugins.lock.json");
    let entry = lock["plugins"]
        .as_array()
        .and_then(|plugins| {
            plugins
                .iter()
                .find(|plugin| plugin["pluginId"] == plugin_id)
        })
        .unwrap_or_else(|| panic!("lock entry for {plugin_id} not found"));
    let package_path = std::env::var_os("MIRA_PLUGIN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(entry["cachePath"].as_str().expect("plugin cachePath")));
    println!("plugin: {plugin_id}");

    let (inspection, files) = if package_path.is_dir() {
        load_extracted_plugin(&package_path)
    } else {
        load_packaged_plugin(&package_path)
    };
    println!(
        "version: {} signature_verified={} writes_enabled={} evidence={:?}",
        inspection.version,
        inspection.signature_verified,
        inspection.writes_enabled,
        inspection.evidence
    );
    let devices = hid::parse_devices_json(files.get("devices.json").expect("devices.json"))
        .expect("parse devices.json");
    println!("device descriptors: {}", devices.devices.len());
    for descriptor in &devices.devices {
        println!(
            "  family={} vid={:04x} pid={:04x} connection={} evidence={:?}",
            descriptor.family,
            descriptor.vendor_id.unwrap_or(0),
            descriptor.product_id.unwrap_or(0),
            descriptor.connection.as_deref().unwrap_or("?"),
            descriptor.evidence
        );
    }

    let api = HidApi::new().expect("initialize HID API");
    let plugins = vec![(inspection, devices, files.clone())];
    let matched = hid::enumerate_matched_devices(&api, &plugins);
    println!("matched HID devices: {}", matched.len());
    if matched.is_empty() {
        eprintln!("no HID device matched any descriptor — check VID/PID/usage filters");
        std::process::exit(1);
    }
    for device in &matched {
        println!(
            "  family={} connection={} vid={:04x} pid={:04x} usage_page=0x{:04x} usage=0x{:04x} evidence={:?} path={}",
            device.family,
            device.connection,
            device.vendor_id,
            device.product_id,
            device.usage_page,
            device.usage,
            device.evidence,
            device.path,
        );
    }

    let target = matched
        .iter()
        .find(|device| {
            matches!(
                device.evidence.as_str(),
                "hardware-verified" | "protocol-verified"
            )
        })
        .or_else(|| matched.first())
        .expect("no supported HID device found");
    println!(
        "target: family={} connection={} usage_page=0x{:04x} usage=0x{:04x}",
        target.family, target.connection, target.usage_page, target.usage
    );

    let connection = match target.connection.as_str() {
        "wireless-receiver" | "wireless" => ConnectionKind::Wireless,
        "bluetooth" => ConnectionKind::Bluetooth,
        _ => ConnectionKind::Usb,
    };
    let feature_index_cache = Mutex::new(FeatureIndexCache::default());
    let onboard_memory_cache = Mutex::new(OnboardMemoryCache::default());
    let cached_handles = Mutex::new(HidHandleCache::default());
    let hid_io_stats = Mutex::new(HidIoStats::default());
    let context = ProtocolContext {
        api: &api,
        path: &target.path,
        family: &target.family,
        connection,
        files: &files,
        outputs: BTreeMap::new(),
        feature_index_cache: Some(&feature_index_cache),
        onboard_memory_cache: Some(&onboard_memory_cache),
        cached_handles: Some(&cached_handles),
        hid_io_stats: Some(&hid_io_stats),
    };
    let reading = match read_device(&context) {
        Ok(reading) => reading,
        Err(error) => {
            report_workflow_error("read device", &error, &hid_io_stats);
            std::process::exit(classified_exit_code(&error));
        }
    };

    println!(
        "battery={:?} charging={} batteries={:?} dpi={:?} polling_rate={:?} profile={:?}",
        reading.battery_percent,
        reading.charging,
        reading.batteries,
        reading.dpi,
        reading.polling_rate_hz,
        reading.profile
    );
    println!(
        "capabilities: {}",
        serde_json::to_string_pretty(&reading.capabilities).expect("serialize capabilities")
    );

    let read_context = ProtocolContext {
        api: &api,
        path: &target.path,
        family: &target.family,
        connection,
        files: &files,
        outputs: reading.capabilities.clone(),
        feature_index_cache: Some(&feature_index_cache),
        onboard_memory_cache: Some(&onboard_memory_cache),
        cached_handles: Some(&cached_handles),
        hid_io_stats: Some(&hid_io_stats),
    };
    let allowed = match writable_mutations(&read_context) {
        Ok(allowed) => allowed,
        Err(error) => {
            report_workflow_error("list writable mutations", &error, &hid_io_stats);
            std::process::exit(classified_exit_code(&error));
        }
    };
    println!("writable mutations: {:?}", allowed);
    print_hid_io_stats(&hid_io_stats);

    if let Ok(workflow_id) = std::env::var("MIRA_WORKFLOW") {
        let outputs = match execute_plugin_workflow(&read_context, &workflow_id) {
            Ok(outputs) => outputs,
            Err(error) => {
                report_workflow_error(&format!("execute {workflow_id}"), &error, &hid_io_stats);
                std::process::exit(classified_exit_code(&error));
            }
        };
        println!(
            "workflow {workflow_id}: {}",
            serde_json::to_string_pretty(&outputs).expect("serialize workflow outputs")
        );
        if workflow_id.ends_with("-onboard-read") {
            print_onboard_profile_summary(&outputs);
        }
        print_hid_io_stats(&hid_io_stats);
    }

    if std::env::var("MIRA_WRITE_SMOKE").unwrap_or_default() == "1" {
        let mutate_context = ProtocolContext {
            api: &api,
            path: &target.path,
            family: &target.family,
            connection,
            files: &files,
            outputs: reading.capabilities.clone(),
            feature_index_cache: Some(&feature_index_cache),
            onboard_memory_cache: Some(&onboard_memory_cache),
            cached_handles: Some(&cached_handles),
            hid_io_stats: Some(&hid_io_stats),
        };

        if let Some(target_mode) = std::env::var("MIRA_WRITE_MODE")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|mode| matches!(mode, 1 | 2))
            .filter(|_| {
                allowed
                    .iter()
                    .any(|mutation| mutation == "set-control-mode")
            })
        {
            println!(
                "smoke: set-control-mode {target_mode} ({})",
                if target_mode == 1 {
                    "onboard"
                } else {
                    "software"
                }
            );
            let params = Map::from_iter([("mode".into(), Value::Number(target_mode.into()))]);
            match mutate_device(&mutate_context, "set-control-mode", &params) {
                Ok(value) => println!("  ok: {}", serde_json::to_string(&value).unwrap()),
                Err(error) => eprintln!("  error: {}", error),
            }
        }

        if std::env::var("MIRA_WRITE_ONLY_MODE").unwrap_or_default() == "1" {
            return;
        }

        if let Some(rate) = reading
            .polling_rate_hz
            .filter(|_| allowed.iter().any(|m| m == "set-polling-rate"))
        {
            let target_rate = std::env::var("MIRA_WRITE_RATE")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(rate);
            println!("smoke: set-polling-rate {target_rate} (was {rate})");
            let mut params = Map::new();
            params.insert("rate".into(), Value::Number(target_rate.into()));
            match mutate_device(&mutate_context, "set-polling-rate", &params) {
                Ok(value) => println!("  ok: {}", serde_json::to_string(&value).unwrap()),
                Err(error) => eprintln!("  error: {}", error),
            }
        }

        if let Some(dpi) = reading
            .dpi
            .filter(|_| allowed.iter().any(|m| m == "set-dpi-value"))
        {
            let target_dpi = std::env::var("MIRA_WRITE_DPI")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(dpi);
            let stage = reading
                .dpi_stages
                .as_ref()
                .and_then(|stages| stages.iter().position(|stage| stage.active))
                .map(|index| index + 1)
                .unwrap_or(1);
            println!("smoke: set-dpi-value {target_dpi} (was {dpi}) at UI stage {stage}");
            let mut params = Map::new();
            params.insert("dpi".into(), Value::Number(target_dpi.into()));
            params.insert("stage".into(), Value::Number(stage.into()));
            match mutate_device(&mutate_context, "set-dpi-value", &params) {
                Ok(value) => println!("  ok: {}", serde_json::to_string(&value).unwrap()),
                Err(error) => eprintln!("  error: {}", error),
            }
        }

        if let Ok(color) = std::env::var("MIRA_WRITE_LIGHT") {
            if allowed
                .iter()
                .any(|mutation| mutation == "set-mouse-lighting")
            {
                let current = reading
                    .capabilities
                    .get("mouseLighting")
                    .and_then(Value::as_object);
                let enabled = std::env::var("MIRA_WRITE_LIGHT_ENABLED")
                    .map(|value| value != "0")
                    .unwrap_or(true);
                let current_effect = current
                    .and_then(|lighting| lighting.get("effect"))
                    .and_then(Value::as_u64)
                    .and_then(|value| u8::try_from(value).ok())
                    .unwrap_or(1);
                let effect = std::env::var("MIRA_WRITE_LIGHT_EFFECT")
                    .ok()
                    .and_then(|value| value.parse::<u8>().ok())
                    .unwrap_or({
                        if enabled {
                            if current_effect == 0 {
                                1
                            } else {
                                current_effect
                            }
                        } else {
                            0
                        }
                    });
                let speed = std::env::var("MIRA_WRITE_LIGHT_SPEED")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .or_else(|| {
                        current
                            .and_then(|lighting| lighting.get("speed"))
                            .and_then(Value::as_u64)
                    })
                    .unwrap_or(0);
                let brightness = std::env::var("MIRA_WRITE_LIGHT_BRIGHTNESS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .or_else(|| {
                        current
                            .and_then(|lighting| lighting.get("brightness"))
                            .and_then(Value::as_u64)
                    })
                    .unwrap_or(100);
                let extra_color = current
                    .and_then(|lighting| lighting.get("extraColor"))
                    .and_then(Value::as_str)
                    .unwrap_or("#000000")
                    .to_string();
                println!("smoke: set-mouse-lighting enabled={enabled} color={color}");
                let params = Map::from_iter([
                    ("enabled".into(), Value::Bool(enabled)),
                    ("color".into(), Value::String(color)),
                    ("effect".into(), Value::Number(effect.into())),
                    ("speed".into(), Value::Number(speed.into())),
                    ("brightness".into(), Value::Number(brightness.into())),
                    ("extraColor".into(), Value::String(extra_color)),
                ]);
                match mutate_device(&mutate_context, "set-mouse-lighting", &params) {
                    Ok(value) => println!("  ok: {}", serde_json::to_string(&value).unwrap()),
                    Err(error) => eprintln!("  error: {}", error),
                }
            }
        }
    }
}

fn report_workflow_error(action: &str, error: &str, hid_io_stats: &Mutex<HidIoStats>) {
    eprintln!("{action}: {error}");
    eprintln!("classification: {}", classify_workflow_error(error));
    if classify_workflow_error(error) == "target-offline" {
        eprintln!(
            "the plugin package, signature, and HID match succeeded; the target device behind the receiver is currently offline or asleep"
        );
    }
    print_hid_io_stats(hid_io_stats);
}

fn classify_workflow_error(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("proxy target is offline") {
        "target-offline"
    } else if lower.contains("no such device")
        || lower.contains("device not found")
        || lower.contains("failed to open")
    {
        "device-unavailable"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "device-timeout"
    } else {
        "workflow-error"
    }
}

fn classified_exit_code(error: &str) -> i32 {
    match classify_workflow_error(error) {
        "target-offline" => 2,
        "device-unavailable" => 3,
        "device-timeout" => 4,
        _ => 1,
    }
}

fn print_hid_io_stats(hid_io_stats: &Mutex<HidIoStats>) {
    if let Ok(stats) = hid_io_stats.lock() {
        println!(
            "hid io stats: cache_hits={} cache_misses={} open_attempts={} open_failures={} handles_returned={} lock_failures={}",
            stats.handle_cache_hits,
            stats.handle_cache_misses,
            stats.open_path_attempts,
            stats.open_path_failures,
            stats.handles_returned,
            stats.handle_cache_lock_failures,
        );
    }
}

fn print_onboard_profile_summary(outputs: &BTreeMap<String, Value>) {
    let sector_size = outputs
        .get("onboardDescription")
        .and_then(Value::as_object)
        .and_then(|object| object.get("sectorSize"))
        .and_then(Value::as_u64)
        .and_then(|size| usize::try_from(size).ok())
        .unwrap_or(256);
    // Issue 4 防御：onboard-read workflow 使用固定 15+1 块结构（每块 16 字节），
    // 仅覆盖 sectorSize <= 256。超出时中间区域为空洞（全 0），CRC 校验会失败。
    // 主流 Logitech 设备 sectorSize=256 不触发；未来若出现更大扇区，
    // 需通过 models/ 型号覆盖或引擎循环支持来扩展。
    if sector_size > 256 {
        eprintln!(
            "onboard summary: WARNING sectorSize={sector_size} exceeds 256-byte coverage of the fixed 15+1 chunk workflow; bytes 240..{} will be zero-filled (incomplete data)",
            sector_size.saturating_sub(16)
        );
    }
    let mut profile = vec![0u8; sector_size];
    for index in 0..16 {
        let key = format!("onboardProfileChunk{index:02}");
        let Some(bytes) = outputs
            .get(&key)
            .and_then(Value::as_object)
            .and_then(|object| object.get("bytes"))
            .and_then(Value::as_array)
        else {
            eprintln!("onboard summary: missing {key}.bytes");
            return;
        };
        let offset = if index == 15 {
            sector_size.saturating_sub(16)
        } else {
            index * 16
        };
        for (target, byte) in profile[offset..]
            .iter_mut()
            .zip(bytes.iter().filter_map(Value::as_u64))
        {
            *target = byte as u8;
        }
    }
    if profile.len() < 32 {
        eprintln!("onboard summary: sector is too short: {}", profile.len());
        return;
    }
    let crc_offset = profile.len() - 2;
    let stored_crc = u16::from_be_bytes([profile[crc_offset], profile[crc_offset + 1]]);
    let calculated_crc = crc_ccitt(&profile[..crc_offset]);
    let report_rate = match profile[0] {
        0 => None,
        milliseconds => Some(1000 / u16::from(milliseconds)),
    };
    let dpi: Vec<_> = (0..5)
        .map(|index| {
            let offset = 3 + index * 2;
            u16::from_le_bytes([profile[offset], profile[offset + 1]])
        })
        .collect();
    println!(
        "onboard summary: bytes={} crc=0x{stored_crc:04x}/0x{calculated_crc:04x} valid={} report_rate={report_rate:?} default_dpi_index={} shifted_dpi_index={} dpi={dpi:?}",
        profile.len(),
        stored_crc == calculated_crc,
        profile[1],
        profile[2],
    );
    println!(
        "onboard profile tail: {:02x?}",
        &profile[profile.len().saturating_sub(48)..],
    );
}

fn crc_ccitt(data: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in data {
        let temp = (crc >> 8) ^ u16::from(*byte);
        crc <<= 8;
        let quick = temp ^ (temp >> 4);
        crc ^= quick ^ (quick << 5) ^ (quick << 12);
    }
    crc
}
