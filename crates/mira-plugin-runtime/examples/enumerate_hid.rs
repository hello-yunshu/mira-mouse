// SPDX-License-Identifier: AGPL-3.0-or-later
// Generic real-device verification through the signed plugin workflow.
use ed25519_dalek::VerifyingKey;
use hidapi::HidApi;
use mira_plugin_runtime::{
    extract_package, hid, inspect_package, read_device, ConnectionKind, ProtocolContext, TrustStore,
};
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor};

const PRODUCTION_KEY_ID: &str = "mira-plugins-2026-001";
const PRODUCTION_PUBLIC_KEY_HEX: &str =
    "eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3";
const PLUGIN_PATH: &str = "src-tauri/resources/plugins/mira-amaster-1.2.0.mira-plugin";

fn production_trust_store() -> TrustStore {
    let bytes = hex::decode(PRODUCTION_PUBLIC_KEY_HEX).expect("valid public key hex");
    let key = VerifyingKey::from_bytes(&bytes.try_into().expect("32-byte public key"))
        .expect("valid Ed25519 public key");
    let mut trust = TrustStore::default();
    trust.0.insert(PRODUCTION_KEY_ID.to_string(), key);
    trust
}

fn main() {
    let bytes = fs::read(PLUGIN_PATH).expect("read plugin package");
    println!("sha256: {}", hex::encode(Sha256::digest(&bytes)));

    let trust = production_trust_store();
    let inspection = inspect_package(Cursor::new(&bytes), &trust, true).expect("verify package");
    println!(
        "plugin: {} v{} signature_verified={}",
        inspection.plugin_id, inspection.version, inspection.signature_verified
    );
    let (_, files) = extract_package(Cursor::new(&bytes), &trust, true).expect("extract package");
    let devices = hid::parse_devices_json(files.get("devices.json").expect("devices.json"))
        .expect("parse devices.json");

    let api = HidApi::new().expect("initialize HID API");
    let plugins = vec![(inspection, devices, files.clone())];
    let matched = hid::enumerate_matched_devices(&api, &plugins);
    let target = matched
        .iter()
        .find(|device| device.evidence == "hardware-verified")
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
    let reading = read_device(&ProtocolContext {
        api: &api,
        path: &target.path,
        family: &target.family,
        connection,
        files: &files,
    })
    .expect("execute signed plugin workflow");

    println!(
        "battery={:?} dpi={:?} polling_rate={:?} profile={:?}",
        reading.battery_percent, reading.dpi, reading.polling_rate_hz, reading.profile
    );
    println!(
        "capabilities: {}",
        serde_json::to_string_pretty(&reading.capabilities).expect("serialize capabilities")
    );
}
