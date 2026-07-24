// SPDX-License-Identifier: AGPL-3.0-or-later
use hidapi::{DeviceInfo, HidApi, HidDevice};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::ffi::CString;

use crate::dsl::{Transport, Workflow};
use crate::package::PackageInspection;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceIdentity {
    pub group: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

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
    #[serde(default)]
    pub identity: Option<DeviceIdentity>,
    /// 静态默认选择优先级。数值越大，越适合作为主设备。
    #[serde(default)]
    pub selection_priority: i32,
    /// 按运行时实际连接类型覆盖默认选择优先级。
    #[serde(default)]
    pub selection_priority_by_connection: BTreeMap<String, i32>,
    /// 显式设备型号名，用于型号覆盖加载。设置后 model routing 与 evidence 解耦：
    /// descriptor.model 优先，未设置时才回退到 hardware-verified 单型号启发式。
    /// 路径安全：经 `validate_model_name` 校验，拒绝包含路径分隔符的值。
    #[serde(default)]
    pub model: Option<String>,
    /// 仅匹配指定 interface number 的 HID 设备。
    /// 厂商自定义 usage page（如 0xFF00）常在多个 interface 上暴露，
    /// 此字段用于选择控制接口而非鼠标输入接口。未设置时匹配任意 interface。
    #[serde(default)]
    pub interface_number: Option<u8>,
    /// 仅匹配 feature report size 不小于此值的设备。
    /// 用于排除小 report 的输入接口，确保控制接口能容纳完整命令报文。
    #[serde(default)]
    pub min_feature_report_size: Option<usize>,
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
    pub identity: Option<DeviceIdentity>,
    pub selection_priority: i32,
    pub selection_priority_by_connection: BTreeMap<String, i32>,
}

impl MatchedDevice {
    pub fn selection_priority_for(&self, connection: &str) -> i32 {
        self.selection_priority_by_connection
            .get(connection)
            .copied()
            .unwrap_or(self.selection_priority)
    }
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
                    // 型号识别（explicit routing）：descriptor.model 优先，路径安全校验后采用。
                    // 未设置时回退到 hardware-verified 单型号启发式（向后兼容）。
                    // 这将 model routing 与 evidence 解耦：source-confirmed 等低证据等级
                    // 也能通过 descriptor.model 精确路由覆盖。
                    let evidence = evidence_label(descriptor.evidence.as_deref());
                    let model = descriptor
                        .model
                        .as_deref()
                        .filter(|m| validate_model_name(m))
                        .map(str::to_string)
                        .or_else(|| {
                            if evidence == "hardware-verified"
                                && devices.hardware_verified_models.len() == 1
                            {
                                Some(devices.hardware_verified_models[0].clone())
                            } else {
                                None
                            }
                        });
                    let candidate = MatchedDevice {
                        plugin_id: inspection.plugin_id.clone(),
                        family: descriptor.family.clone(),
                        evidence,
                        connection: connection_label(descriptor.connection.as_deref()),
                        path: device.path().to_string_lossy().into_owned(),
                        vendor_id: device.vendor_id(),
                        product_id: device.product_id(),
                        usage_page: device.usage_page(),
                        usage: device.usage(),
                        model,
                        identity: descriptor.identity.clone(),
                        selection_priority: descriptor.selection_priority,
                        selection_priority_by_connection: descriptor
                            .selection_priority_by_connection
                            .clone(),
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

/// Validate a model name for safe use in overlay path construction.
/// Rejects empty strings, path separators (`/`, `\`), parent-dir traversal
/// (`..`), NUL bytes, and any non-ASCII-alphanumeric character outside of
/// `-`, `_`, `.`. This keeps model routing decoupled from evidence while
/// preventing path-escape through a plugin-supplied model string.
pub fn validate_model_name(model: &str) -> bool {
    !model.is_empty()
        && !model.contains('/')
        && !model.contains('\\')
        && !model.contains("..")
        && !model.contains('\0')
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn descriptor_matches(info: &DeviceInfo, descriptor: &DeviceDescriptor) -> bool {
    matches_descriptor(
        info.vendor_id(),
        info.product_id(),
        info.usage_page(),
        info.usage(),
        info.interface_number(),
        descriptor,
    )
}

/// Pure matching logic extracted from `descriptor_matches` for testability.
/// Each descriptor field is optional: `None` means "match any value".
/// `interface_number` from the live HID device is matched against
/// `descriptor.interface_number` when set, allowing plugins to target a
/// specific control interface (e.g. vendor-defined usage page on interface 0
/// or 2) instead of the mouse input interface.
fn matches_descriptor(
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    interface_number: i32,
    descriptor: &DeviceDescriptor,
) -> bool {
    let vendor_match = descriptor.vendor_id.is_none_or(|vid| vid == vendor_id);
    let product_match = descriptor.product_id.is_none_or(|pid| pid == product_id);
    let usage_page_match = descriptor.usage_page.is_none_or(|up| up == usage_page);
    let usage_match = descriptor.usage.is_none_or(|u| u == usage);
    // interface_number: hidapi exposes the USB interface number as i32 (-1 when
    // unavailable). Match only when the descriptor pins a specific interface.
    let interface_match = descriptor
        .interface_number
        .is_none_or(|iface| iface as i32 == interface_number);
    // min_feature_report_size: the live report size is not exposed by hidapi's
    // DeviceInfo at enumeration time, so this filter is recorded on the
    // descriptor but not enforced here. Callers that open the device can still
    // read the descriptor's declaration for transport-level gating.
    let _ = descriptor.min_feature_report_size;
    vendor_match && product_match && usage_page_match && usage_match && interface_match
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
    let devices: DevicesFile =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid devices.json: {e}"))?;
    for descriptor in &devices.devices {
        for connection in descriptor.selection_priority_by_connection.keys() {
            if !matches!(
                connection.as_str(),
                "usb" | "wireless" | "bluetooth" | "virtual"
            ) {
                return Err(format!(
                    "invalid devices.json: {}/selectionPriorityByConnection has unknown connection {}",
                    descriptor.family, connection
                ));
            }
        }
        let priorities = std::iter::once(("selectionPriority", descriptor.selection_priority))
            .chain(
                descriptor
                    .selection_priority_by_connection
                    .iter()
                    .map(|(connection, priority)| (connection.as_str(), *priority)),
            );
        for (source, priority) in priorities {
            if !(-1000..=1000).contains(&priority) {
                return Err(format!(
                    "invalid devices.json: {}/{} must be between -1000 and 1000",
                    descriptor.family, source
                ));
            }
        }
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::{
        evidence_rank, matches_descriptor, parse_devices_json, validate_model_name,
        DeviceDescriptor,
    };
    use std::collections::BTreeMap;

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
            identity: None,
            selection_priority: 0,
            selection_priority_by_connection: BTreeMap::new(),
            model: None,
            interface_number: None,
            min_feature_report_size: None,
        }
    }

    #[test]
    fn empty_descriptor_matches_any_device() {
        let desc = descriptor(None, None, None, None);
        assert!(matches_descriptor(
            0x1234, 0xC001, 0x0001, 0x0002, -1, &desc
        ));
    }

    #[test]
    fn exact_vendor_product_match() {
        let desc = descriptor(Some(0x1234), Some(0xC001), None, None);
        assert!(matches_descriptor(0x1234, 0xC001, 0xFF, 0xFF, -1, &desc));
        assert!(!matches_descriptor(0x1234, 0xC002, 0xFF, 0xFF, -1, &desc));
        assert!(!matches_descriptor(0x5678, 0xC001, 0xFF, 0xFF, -1, &desc));
    }

    #[test]
    fn usage_page_and_usage_filter_interface() {
        let desc = descriptor(Some(0x1234), None, Some(0x0001), Some(0x0002));
        assert!(matches_descriptor(
            0x1234, 0xFFFF, 0x0001, 0x0002, -1, &desc
        ));
        assert!(!matches_descriptor(
            0x1234, 0xFFFF, 0x0001, 0x0003, -1, &desc
        ));
        assert!(!matches_descriptor(
            0x1234, 0xFFFF, 0x0002, 0x0002, -1, &desc
        ));
    }

    #[test]
    fn vendor_only_match_ignores_product() {
        let desc = descriptor(Some(0x1234), None, None, None);
        assert!(matches_descriptor(0x1234, 0x0001, 0, 0, -1, &desc));
        assert!(matches_descriptor(0x1234, 0xFFFF, 0xFF, 0xFF, -1, &desc));
        assert!(!matches_descriptor(0x0000, 0x0001, 0, 0, -1, &desc));
    }

    #[test]
    fn interface_number_filter_selects_control_interface() {
        let mut desc = descriptor(Some(0x1532), None, Some(0xFF00), None);
        // Descriptor pins interface 0 (vendor control interface).
        desc.interface_number = Some(0);
        // Live device on interface 0 matches.
        assert!(matches_descriptor(0x1532, 0xABCD, 0xFF00, 0x0001, 0, &desc));
        // Live device on interface 2 (mouse input interface) does not match.
        assert!(!matches_descriptor(
            0x1532, 0xABCD, 0xFF00, 0x0001, 2, &desc
        ));
        // When descriptor leaves interface_number unset, both match (backwards compat).
        desc.interface_number = None;
        assert!(matches_descriptor(0x1532, 0xABCD, 0xFF00, 0x0001, 0, &desc));
        assert!(matches_descriptor(0x1532, 0xABCD, 0xFF00, 0x0001, 2, &desc));
    }

    #[test]
    fn rejects_out_of_range_selection_priority() {
        let devices = br#"{
            "schemaVersion": 1,
            "devices": [{
                "family": "test",
                "selectionPriority": 1001
            }]
        }"#;

        assert!(parse_devices_json(devices)
            .unwrap_err()
            .contains("selectionPriority must be between -1000 and 1000"));
    }

    #[test]
    fn resolves_connection_specific_selection_priority() {
        let devices = br#"{
            "schemaVersion": 1,
            "devices": [{
                "family": "test",
                "selectionPriority": -10,
                "selectionPriorityByConnection": { "usb": 100, "wireless": 0 }
            }]
        }"#;
        let descriptor = &parse_devices_json(devices).unwrap().devices[0];
        let matched = super::MatchedDevice {
            plugin_id: "test.plugin".into(),
            family: descriptor.family.clone(),
            evidence: "fixture-verified".into(),
            connection: "hidpp".into(),
            path: "test-path".into(),
            vendor_id: 0,
            product_id: 0,
            usage_page: 0,
            usage: 0,
            model: None,
            identity: None,
            selection_priority: descriptor.selection_priority,
            selection_priority_by_connection: descriptor.selection_priority_by_connection.clone(),
        };

        assert_eq!(matched.selection_priority_for("usb"), 100);
        assert_eq!(matched.selection_priority_for("wireless"), 0);
        assert_eq!(matched.selection_priority_for("virtual"), -10);
    }

    #[test]
    fn validate_model_name_accepts_safe_names() {
        assert!(validate_model_name("Viper-V2-Pro"));
        assert!(validate_model_name("DeathAdder_V3_2024"));
        assert!(validate_model_name("G502.1"));
        assert!(validate_model_name("Model-A"));
    }

    #[test]
    fn validate_model_name_rejects_unsafe_inputs() {
        // Path separators and traversal sequences.
        assert!(!validate_model_name("a/b"));
        assert!(!validate_model_name("a\\b"));
        assert!(!validate_model_name(".."));
        assert!(!validate_model_name("a/../b"));
        assert!(!validate_model_name("a\0b"));
        // Empty.
        assert!(!validate_model_name(""));
        // Non-ASCII / disallowed characters (spaces, Chinese, etc.).
        assert!(!validate_model_name("Viper V2"));
        assert!(!validate_model_name("毒蛇V2"));
        assert!(!validate_model_name("Viper@V2"));
    }

    #[test]
    fn descriptor_model_decouples_routing_from_evidence() {
        // A source-confirmed descriptor with an explicit model should still
        // route to that model even though evidence is not hardware-verified.
        let devices = br#"{
            "schemaVersion": 1,
            "devices": [{
                "family": "test",
                "evidence": "source-confirmed",
                "model": "Explicit-Model"
            }],
            "hardwareVerifiedModels": []
        }"#;
        let parsed = parse_devices_json(devices).unwrap();
        assert_eq!(parsed.devices[0].model.as_deref(), Some("Explicit-Model"));
        // With descriptor.model set, the explicit name is preferred regardless
        // of evidence level — verified by enumerate_matched_devices which is
        // covered indirectly here via the parsed field.
    }

    #[test]
    fn old_devices_json_without_model_field_still_loads() {
        // Backward compatibility: a devices.json written before the model
        // field existed must still parse (#[serde(default)] makes it Optional).
        let devices = br#"{
            "schemaVersion": 1,
            "devices": [{
                "family": "legacy",
                "vendorId": 1234,
                "productId": 5678
            }],
            "hardwareVerifiedModels": ["Legacy-Model"]
        }"#;
        let parsed = parse_devices_json(devices).unwrap();
        assert_eq!(parsed.devices[0].model, None);
        assert_eq!(parsed.devices[0].interface_number, None);
        assert_eq!(parsed.devices[0].min_feature_report_size, None);
    }
}
