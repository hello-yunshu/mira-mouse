// SPDX-License-Identifier: AGPL-3.0-or-later
// Generic real-device verification through the signed plugin workflow.
//
// Usage:
//   cargo run --example enumerate_hid                          # defaults to mira.amaster
//   MIRA_PLUGIN=mira.logitech-hidpp cargo run --example enumerate_hid
//
// The tool loads plugins.lock.json, finds the requested plugin entry,
// verifies the package signature against the production + TEST-ONLY trust
// store, enumerates matched HID devices, and runs the signed plugin
// workflow. Set MIRA_WRITE_SMOKE=1 to additionally exercise no-op
// write/readback smoke tests (only for plugins with writesEnabled).
use ed25519_dalek::VerifyingKey;
use hidapi::HidApi;
use mira_plugin_runtime::{
    execute_plugin_workflow, extract_package, hid, inspect_package, mutate_device, read_device,
    writable_mutations, ConnectionKind, ProtocolContext, TrustStore,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Cursor, path::PathBuf};

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
    let bytes = fs::read(&package_path).expect("read plugin package");
    println!("plugin: {plugin_id}");
    println!("sha256: {}", hex::encode(Sha256::digest(&bytes)));

    let trust = trust_store();
    let inspection = inspect_package(Cursor::new(&bytes), &trust, true).expect("verify package");
    println!(
        "version: {} signature_verified={} writes_enabled={} evidence={:?}",
        inspection.version,
        inspection.signature_verified,
        inspection.writes_enabled,
        inspection.evidence
    );
    let (_, files) = extract_package(Cursor::new(&bytes), &trust, true).expect("extract package");
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
    let context = ProtocolContext {
        api: &api,
        path: &target.path,
        family: &target.family,
        connection,
        files: &files,
        outputs: BTreeMap::new(),
    };
    let reading = read_device(&context).expect("execute signed plugin workflow");

    println!(
        "battery={:?} dpi={:?} polling_rate={:?} profile={:?}",
        reading.battery_percent, reading.dpi, reading.polling_rate_hz, reading.profile
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
    };
    let allowed = writable_mutations(&read_context).expect("list writable mutations");
    println!("writable mutations: {:?}", allowed);

    if let Ok(workflow_id) = std::env::var("MIRA_WORKFLOW") {
        let outputs = execute_plugin_workflow(&read_context, &workflow_id)
            .unwrap_or_else(|error| panic!("execute {workflow_id}: {error}"));
        println!(
            "workflow {workflow_id}: {}",
            serde_json::to_string_pretty(&outputs).expect("serialize workflow outputs")
        );
        if workflow_id.ends_with("-onboard-read") {
            print_onboard_profile_summary(&outputs);
        }
    }

    if std::env::var("MIRA_WRITE_SMOKE").unwrap_or_default() == "1" {
        let mutate_context = ProtocolContext {
            api: &api,
            path: &target.path,
            family: &target.family,
            connection,
            files: &files,
            outputs: reading.capabilities.clone(),
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
                let enabled = std::env::var("MIRA_WRITE_LIGHT_ENABLED")
                    .map(|value| value != "0")
                    .unwrap_or(true);
                println!("smoke: set-mouse-lighting enabled={enabled} color={color}");
                let params = Map::from_iter([
                    ("enabled".into(), Value::Bool(enabled)),
                    ("color".into(), Value::String(color)),
                ]);
                match mutate_device(&mutate_context, "set-mouse-lighting", &params) {
                    Ok(value) => println!("  ok: {}", serde_json::to_string(&value).unwrap()),
                    Err(error) => eprintln!("  error: {}", error),
                }
            }
        }
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
