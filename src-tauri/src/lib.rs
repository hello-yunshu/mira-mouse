// SPDX-License-Identifier: AGPL-3.0-or-later
use ed25519_dalek::VerifyingKey;
use mira_plugin_runtime::TrustStore;
use std::sync::Mutex;
use tauri::{Manager, WebviewWindow};

#[derive(Default)]
struct SessionState {
    write_in_progress: Mutex<bool>,
}

// Production plugin signing key for hello-yunshu/mira-mouse-plugins.
// Replace with the real key id and public key after the first production release.
const PRODUCTION_KEY_ID: &str = "mira-plugins-2026-001";
const PRODUCTION_PUBLIC_KEY_HEX: &str =
    "eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3";

fn production_trust_store() -> TrustStore {
    let mut trust = TrustStore::default();
    let bytes = hex::decode(PRODUCTION_PUBLIC_KEY_HEX).expect("invalid hex in production pubkey");
    let key = VerifyingKey::from_bytes(&bytes.try_into().expect("production pubkey must be 32 bytes"))
        .expect("invalid production ed25519 pubkey");
    trust.0.insert(PRODUCTION_KEY_ID.to_string(), key);
    trust
}

#[tauri::command]
fn device_snapshot() -> Option<mira_core::DeviceSnapshot> {
    // No production mock: disconnected is the truthful state until a verified plugin opens a HID session.
    None
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

fn focus_main(window: Option<WebviewWindow>) {
    if let Some(window) = window {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(SessionState::default())
        .manage(production_trust_store())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            focus_main(app.get_webview_window("main"))
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            device_snapshot,
            can_install_update
        ])
        .run(tauri::generate_context!())
        .expect("Mira application runtime failed");
}
