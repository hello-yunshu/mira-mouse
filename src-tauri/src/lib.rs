// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Mutex;
use tauri::{Manager, WebviewWindow};

#[derive(Default)]
struct SessionState {
    write_in_progress: Mutex<bool>,
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
