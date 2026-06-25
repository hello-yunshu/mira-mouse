// SPDX-License-Identifier: AGPL-3.0-or-later
use hidapi::{DeviceInfo, HidApi, HidDevice};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
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
    /// 设备型号名，用于型号覆盖加载（模式 C）。
    /// 当 evidence 为 "hardware-verified" 且 devices.json 的 hardwareVerifiedModels
    /// 只有一个型号时，设置为该型号名；否则为 None。
    /// 未来可通过 workflow 探测或 VID/PID 查表精确识别多型号场景。
    pub model: Option<String>,
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
    let mut matches: Vec<MatchedDevice> = Vec::new();
    // P4: 用 HashMap 索引替代线性查找，将去重从 O(n²) 降为 O(n)。
    // key 为 (plugin_id, family, path)，value 为 matches 中的索引。
    let mut dedup_index: HashMap<(String, String, String), usize> = HashMap::new();
    for device in api.device_list() {
        for (inspection, devices, _) in plugins {
            for descriptor in &devices.devices {
                if descriptor_matches(device, descriptor) {
                    // 型号识别：当 evidence 为 "hardware-verified" 且只有一个已验证型号时，
                    // 可以确定设备型号。多型号场景需要未来通过 workflow 探测扩展。
                    let model = if evidence_label(descriptor.evidence.as_deref())
                        == "hardware-verified"
                        && devices.hardware_verified_models.len() == 1
                    {
                        Some(devices.hardware_verified_models[0].clone())
                    } else {
                        None
                    };
                    let candidate = MatchedDevice {
                        plugin_id: inspection.plugin_id.clone(),
                        family: descriptor.family.clone(),
                        evidence: evidence_label(descriptor.evidence.as_deref()),
                        connection: connection_label(descriptor.connection.as_deref()),
                        path: device.path().to_string_lossy().into_owned(),
                        vendor_id: device.vendor_id(),
                        product_id: device.product_id(),
                        usage_page: device.usage_page(),
                        usage: device.usage(),
                        model,
                    };
                    let key = (
                        candidate.plugin_id.clone(),
                        candidate.family.clone(),
                        candidate.path.clone(),
                    );
                    if let Some(&idx) = dedup_index.get(&key) {
                        if evidence_rank(&candidate.evidence)
                            > evidence_rank(&matches[idx].evidence)
                        {
                            matches[idx] = candidate;
                        }
                    } else {
                        dedup_index.insert(key, matches.len());
                        matches.push(candidate);
                    }
                }
            }
        }
    }
    matches
}

fn evidence_rank(evidence: &str) -> u8 {
    match evidence {
        "hardware-verified" => 4,
        "protocol-verified" => 3,
        "source-confirmed" => 2,
        "fixture-verified" => 1,
        _ => 0,
    }
}

fn descriptor_matches(info: &DeviceInfo, descriptor: &DeviceDescriptor) -> bool {
    matches_descriptor(
        info.vendor_id(),
        info.product_id(),
        info.usage_page(),
        info.usage(),
        descriptor,
    )
}

/// Pure matching logic extracted from `descriptor_matches` for testability.
/// Each descriptor field is optional: `None` means "match any value".
fn matches_descriptor(
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    descriptor: &DeviceDescriptor,
) -> bool {
    let vendor_match = descriptor.vendor_id.is_none_or(|vid| vid == vendor_id);
    let product_match = descriptor.product_id.is_none_or(|pid| pid == product_id);
    let usage_page_match = descriptor.usage_page.is_none_or(|up| up == usage_page);
    let usage_match = descriptor.usage.is_none_or(|u| u == usage);
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

#[cfg(test)]
mod tests {
    use super::{evidence_rank, matches_descriptor, DeviceDescriptor};

    #[test]
    fn protocol_evidence_outranks_unverified_source_evidence() {
        assert!(evidence_rank("protocol-verified") > evidence_rank("source-confirmed"));
    }

    fn descriptor(
        vendor_id: Option<u16>,
        product_id: Option<u16>,
        usage_page: Option<u16>,
        usage: Option<u16>,
    ) -> DeviceDescriptor {
        DeviceDescriptor {
            family: "test".into(),
            vendor_id,
            product_id,
            usage_page,
            usage,
            connection: None,
            evidence: None,
            topology: Vec::new(),
            transport: None,
        }
    }

    #[test]
    fn empty_descriptor_matches_any_device() {
        let desc = descriptor(None, None, None, None);
        assert!(matches_descriptor(0x1234, 0xC001, 0x0001, 0x0002, &desc));
    }

    #[test]
    fn exact_vendor_product_match() {
        let desc = descriptor(Some(0x1234), Some(0xC001), None, None);
        assert!(matches_descriptor(0x1234, 0xC001, 0xFF, 0xFF, &desc));
        assert!(!matches_descriptor(0x1234, 0xC002, 0xFF, 0xFF, &desc));
        assert!(!matches_descriptor(0x5678, 0xC001, 0xFF, 0xFF, &desc));
    }

    #[test]
    fn usage_page_and_usage_filter_interface() {
        let desc = descriptor(Some(0x1234), None, Some(0x0001), Some(0x0002));
        assert!(matches_descriptor(0x1234, 0xFFFF, 0x0001, 0x0002, &desc));
        assert!(!matches_descriptor(0x1234, 0xFFFF, 0x0001, 0x0003, &desc));
        assert!(!matches_descriptor(0x1234, 0xFFFF, 0x0002, 0x0002, &desc));
    }

    #[test]
    fn vendor_only_match_ignores_product() {
        let desc = descriptor(Some(0x1234), None, None, None);
        assert!(matches_descriptor(0x1234, 0x0001, 0, 0, &desc));
        assert!(matches_descriptor(0x1234, 0xFFFF, 0xFF, 0xFF, &desc));
        assert!(!matches_descriptor(0x0000, 0x0001, 0, 0, &desc));
    }
}
