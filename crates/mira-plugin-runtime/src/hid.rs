// SPDX-License-Identifier: AGPL-3.0-or-later
use hidapi::{DeviceInfo, HidApi, HidDevice};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::CString;

use crate::dsl::{Transport, Workflow};
use crate::package::PackageInspection;

/// A device record inside a plugin's `devices.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceDescriptor {
    pub family: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub connection: Option<String>,
    pub evidence: Option<String>,
    #[serde(default)]
    pub topology: Vec<String>,
    pub transport: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevicesFile {
    pub schema_version: u32,
    #[serde(default)]
    pub devices: Vec<DeviceDescriptor>,
    #[serde(default)]
    pub hardware_verified_models: Vec<String>,
}

/// A matched HID path together with the plugin/device that matched it.
#[derive(Debug, Clone)]
pub struct MatchedDevice {
    pub plugin_id: String,
    pub family: String,
    pub evidence: String,
    pub connection: String,
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
}

fn connection_label(conn: Option<&str>) -> String {
    match conn {
        Some(c) => c.to_string(),
        None => "usb".to_string(),
    }
}

fn evidence_label(ev: Option<&str>) -> String {
    match ev {
        Some(e) => e.to_string(),
        None => "unknown".to_string(),
    }
}

/// Enumerate HID devices and return those matching any descriptor in the provided plugins.
#[allow(clippy::type_complexity)] // Keep the public tuple aligned with extracted package contents.
pub fn enumerate_matched_devices(
    api: &HidApi,
    plugins: &[(PackageInspection, DevicesFile, BTreeMap<String, Vec<u8>>)],
) -> Vec<MatchedDevice> {
    let mut matches = Vec::new();
    for device in api.device_list() {
        for (inspection, devices, _) in plugins {
            for descriptor in &devices.devices {
                if descriptor_matches(device, descriptor) {
                    matches.push(MatchedDevice {
                        plugin_id: inspection.plugin_id.clone(),
                        family: descriptor.family.clone(),
                        evidence: evidence_label(descriptor.evidence.as_deref()),
                        connection: connection_label(descriptor.connection.as_deref()),
                        path: device.path().to_string_lossy().into_owned(),
                        vendor_id: device.vendor_id(),
                        product_id: device.product_id(),
                        usage_page: device.usage_page(),
                        usage: device.usage(),
                    });
                }
            }
        }
    }
    matches
}

fn descriptor_matches(info: &DeviceInfo, descriptor: &DeviceDescriptor) -> bool {
    let vendor_match = descriptor
        .vendor_id
        .is_none_or(|vid| vid == info.vendor_id());
    let product_match = descriptor
        .product_id
        .is_none_or(|pid| pid == info.product_id());
    let usage_page_match = descriptor
        .usage_page
        .is_none_or(|up| up == info.usage_page());
    let usage_match = descriptor.usage.is_none_or(|u| u == info.usage());
    vendor_match && product_match && usage_page_match && usage_match
}

/// HID transport backed by `hidapi`. The workflow DSL operates on raw report payloads.
pub struct HidTransport {
    device: HidDevice,
}

impl HidTransport {
    pub fn open(api: &HidApi, path: &str) -> Result<Self, String> {
        let c_path = CString::new(path).map_err(|_| "invalid HID path")?;
        let device = api.open_path(&c_path).map_err(|e| e.to_string())?;
        Ok(Self { device })
    }
}

impl Transport for HidTransport {
    fn write(&mut self, report: &[u8]) -> Result<(), String> {
        self.device.write(report).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn read(&mut self, length: usize) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; length];
        let n = self.device.read(&mut buf).map_err(|e| e.to_string())?;
        buf.truncate(n);
        Ok(buf)
    }

    fn delay(&mut self, milliseconds: u64) -> Result<(), String> {
        std::thread::sleep(std::time::Duration::from_millis(milliseconds));
        Ok(())
    }
}

/// AMaster protocol state and helpers. This is intentionally narrow: it reads the
/// receiver-based device state for hardware testing and leaves full write support
/// to future, protocol-specific extensions.
pub struct AmasterReader<'a> {
    api: &'a HidApi,
    path: String,
}

impl<'a> AmasterReader<'a> {
    pub fn new(api: &'a HidApi, path: &str) -> Self {
        Self {
            api,
            path: path.to_string(),
        }
    }

    /// Run a bounded generic workflow against the matched HID path.
    pub fn run_workflow(&self, workflow: &Workflow) -> Result<Vec<u8>, String> {
        let mut transport = HidTransport::open(self.api, &self.path)?;
        crate::dsl::execute_workflow(workflow, &mut transport, crate::dsl::Limits::default())
            .map_err(|e| e.to_string())
    }
}

/// Convenience: parse `devices.json` from plugin package bytes.
pub fn parse_devices_json(bytes: &[u8]) -> Result<DevicesFile, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("invalid devices.json: {e}"))
}
