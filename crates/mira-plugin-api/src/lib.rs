// SPDX-License-Identifier: AGPL-3.0-or-later
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const PLUGIN_SCHEMA_VERSION: u32 = 1;
pub const PLUGIN_API_VERSION: &str = "1.0.0";
const MAX_DASHBOARD_ITEMS: usize = 6;
const MAX_CONTROL_OPTIONS: usize = 8;
const MAX_SUMMARY_ITEMS: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub plugin_api: VersionReq,
    pub publisher_key_id: Option<String>,
    pub evidence: EvidenceLevel,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub writes_enabled: bool,
    /// #11 配置导入/导出：声明可导出的设备配置字段白名单。
    /// Host 按此白名单读写配置文件，仅导出/导入声明的字段。
    /// 未声明时该插件不参与配置导入/导出。
    #[serde(default)]
    pub exportable_fields: Vec<ExportableField>,
    /// #12 插件间依赖复用：声明当前插件依赖的其他插件。
    /// runtime 解析依赖关系，可复用被依赖插件的传输层定义。
    /// 未声明时插件独立运行（向后兼容）。
    #[serde(default)]
    pub depends_on: Vec<PluginDependency>,
}

/// #11 配置导入/导出：可导出字段声明。
/// 插件声明哪些设备配置字段可以被导入/导出，Host 按白名单操作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportableField {
    /// 字段标识，对应 capability id 或 mutation id。
    pub id: String,
    /// 导入/导出时的字段名（用于配置文件中的 key）。
    pub export_key: String,
    /// 字段类型描述（如 "number"、"select"、"color"、"object"），用于配置文件格式化。
    #[serde(default)]
    pub kind: Option<String>,
    /// 导入时调用的 mutation 名称。
    /// 未声明时，Host 从 capability metadata 的 `mutation` 字段推导。
    #[serde(default)]
    pub mutation: Option<String>,
    /// 导入时 mutation 的参数名（标量字段使用）。
    /// 未声明时默认为 "value"。
    #[serde(default)]
    pub param: Option<String>,
    /// 导出时从设备快照中读取的源路径（如 "capabilities.settings.pollingRate"）。
    /// 未声明时，Host 使用 capability metadata 的 `source` 字段。
    #[serde(default)]
    pub source: Option<String>,
    /// 复合字段的参数源路径映射：参数名 → 快照路径。
    /// 声明后，导出值为 object，导入时将其键值展开为 mutation 参数。
    #[serde(default)]
    pub sources: Option<BTreeMap<String, String>>,
}

/// #12 插件间依赖复用：插件依赖声明。
/// 声明当前插件依赖的其他插件，runtime 可复用被依赖插件的传输层。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginDependency {
    /// 被依赖插件的 plugin_id。
    pub plugin_id: String,
    /// 被依赖插件的版本要求（semver range）。
    #[serde(default)]
    pub version: Option<String>,
    /// 是否复用被依赖插件的传输层定义。
    /// true 时，runtime 从被依赖插件加载 transports.json 作为补充。
    #[serde(default)]
    pub reuse_transport: bool,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.schema_version != PLUGIN_SCHEMA_VERSION {
            return Err(ApiError::SchemaVersion(self.schema_version));
        }
        if !valid_plugin_id(&self.plugin_id) {
            return Err(ApiError::PluginId(self.plugin_id.clone()));
        }
        let current = semver::Version::parse(PLUGIN_API_VERSION).expect("constant version");
        if !self.plugin_api.matches(&current) {
            return Err(ApiError::ApiIncompatible(self.plugin_api.to_string()));
        }
        if self.writes_enabled && self.evidence != EvidenceLevel::HardwareVerified {
            return Err(ApiError::UnsafeWriteEvidence);
        }
        let mut control_groups = BTreeSet::new();
        let mut status_items = 0usize;
        for capability in &self.capabilities {
            if capability
                .placements
                .iter()
                .any(|placement| !(1..=3).contains(&placement.span))
            {
                return Err(ApiError::CapabilityPlacement(capability.id.clone()));
            }
            for placement in &capability.placements {
                match placement.region {
                    CapabilityRegion::Control => {
                        control_groups.insert(
                            placement
                                .group
                                .clone()
                                .unwrap_or_else(|| capability.id.clone()),
                        );
                    }
                    CapabilityRegion::Status => status_items += 1,
                    CapabilityRegion::Hero | CapabilityRegion::Details => {}
                }
            }
            // #4 固件门槛：minFirmware 必须是合法 semver。运行时解析失败会静默
            // fail-closed（能力隐藏），插件作者难以排查；此处预校验快速失败。
            if let Some(min) = &capability.min_firmware {
                if semver::Version::parse(min).is_err() {
                    return Err(ApiError::CapabilityMinFirmware(capability.id.clone()));
                }
            }
            // #3 连接类型：connections 必须是已知连接类型。未知值会被运行时
            // 静默判定为不可见，此处预校验避免插件作者拼写错误。
            if let Some(connections) = &capability.connections {
                if connections.iter().any(|conn| !valid_connection(conn)) {
                    return Err(ApiError::CapabilityConnections(capability.id.clone()));
                }
            }
            if capability
                .metadata
                .get("bindings")
                .is_some_and(|bindings| !valid_bindings(bindings))
            {
                return Err(ApiError::CapabilityBinding(capability.id.clone()));
            }
            if capability
                .metadata
                .get("options")
                .is_some_and(|options| !valid_options(options, MAX_CONTROL_OPTIONS))
            {
                return Err(ApiError::CapabilityOptions(capability.id.clone()));
            }
            if let Some(summary) = capability.metadata.get("summary") {
                let valid = summary.as_array().is_some_and(|items| {
                    items.len() <= MAX_SUMMARY_ITEMS
                        && items.iter().all(|item| {
                            item.as_object().is_some_and(|item| {
                                item.get("label")
                                    .and_then(serde_json::Value::as_str)
                                    .is_some_and(|label| !label.is_empty())
                                    && item
                                        .get("source")
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(|source| !source.is_empty())
                                    && item
                                        .get("options")
                                        .is_none_or(|options| valid_options(options, 32))
                            })
                        })
                });
                if !valid {
                    return Err(ApiError::CapabilitySummary(capability.id.clone()));
                }
            }
        }
        if control_groups.len() > MAX_DASHBOARD_ITEMS || status_items > MAX_DASHBOARD_ITEMS {
            return Err(ApiError::CapabilityLayout);
        }
        Ok(())
    }
}

fn valid_options(value: &serde_json::Value, max_items: usize) -> bool {
    value.as_array().is_some_and(|items| {
        items.len() <= max_items
            && items.iter().all(|item| {
                item.as_object().is_some_and(|item| {
                    item.get("label")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|label| !label.is_empty())
                        && item.get("value").is_some_and(|value| {
                            value.is_string() || value.is_number() || value.is_boolean()
                        })
                })
            })
    })
}

/// 校验连接类型是否为已知值。允许规范值（usb/receiver/bluetooth）
/// 与 runtime 归一化支持的别名（wireless/wireless-receiver）。
fn valid_connection(value: &str) -> bool {
    matches!(
        value,
        "usb" | "receiver" | "bluetooth" | "wireless" | "wireless-receiver"
    )
}

fn valid_binding_connection(value: &str) -> bool {
    matches!(value, "usb" | "wireless" | "bluetooth" | "virtual")
}

fn valid_bindings(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            let Some(item) = item.as_object() else {
                return false;
            };
            let Some(when) = item.get("when") else {
                return true;
            };
            let Some(when) = when.as_object() else {
                return false;
            };
            if when.get("path").and_then(serde_json::Value::as_str) != Some("connection") {
                return true;
            }
            when.get("eq")
                .and_then(serde_json::Value::as_str)
                .is_some_and(valid_binding_connection)
        })
    })
}

fn valid_plugin_id(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceLevel {
    SourceConfirmed,
    FixtureVerified,
    BuildVerified,
    HardwareVerified,
    Inferred,
    Unknown,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Permission {
    Hid {
        report_types: Vec<ReportType>,
        max_report_length: u16,
        max_reports_per_second: u16,
    },
    LinuxDevice {
        vendor_id: u16,
        product_id: u16,
        usage_page: u16,
        usage: u16,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReportType {
    Feature,
    Input,
    Output,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capability {
    pub id: String,
    pub control: Control,
    pub label_key: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub placements: Vec<CapabilityPlacement>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
    /// 能力探测声明：引用 workflow 输出的某个字段，值为 0 表示设备不支持该能力。
    /// Host 读取设备后据此标记 `available`，前端只渲染 available=true 的能力。
    /// 未声明 probe 的能力默认 available=true（向后兼容）。
    #[serde(default)]
    pub probe: Option<CapabilityProbe>,
    /// 连接类型能力分支（#3）：声明该能力仅在指定连接类型下可见。
    /// 可选值："usb"、"receiver"、"bluetooth"。未声明时所有连接类型均可见。
    #[serde(default)]
    pub connections: Option<Vec<String>>,
    /// 固件版本门槛（#4）：声明该能力所需的最低固件版本。
    /// Host 校验设备固件版本，低于此版本时能力被隐藏/禁用。
    /// 格式为 semver（如 "1.2.3"）。未声明时无版本限制。
    #[serde(default)]
    pub min_firmware: Option<String>,
}

/// 能力探测声明，引用 workflow 输出的 `{output, field}`。
/// 当引用字段值为 0 时，该能力被标记为不可用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityProbe {
    /// workflow 输出对象名（如 "dpi"、"lighting"）。
    pub output: String,
    /// 输出对象中的字段名（如 "value"、"featureIndex"）。
    pub field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityPlacement {
    pub region: CapabilityRegion,
    pub group: Option<String>,
    #[serde(default)]
    pub order: i32,
    #[serde(default = "default_span")]
    pub span: u8,
    pub icon: Option<String>,
}

fn default_span() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityRegion {
    Hero,
    Control,
    Status,
    Details,
}

impl CapabilityRegion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hero => "hero",
            Self::Control => "control",
            Self::Status => "status",
            Self::Details => "details",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Control {
    Toggle,
    Segmented,
    Select,
    Slider,
    Number,
    Color,
    GradientStops,
    DpiStages,
    LightingZone,
    ReadOnlyValue,
    Action,
    Info,
}

impl Control {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "Toggle",
            Self::Segmented => "Segmented",
            Self::Select => "Select",
            Self::Slider => "Slider",
            Self::Number => "Number",
            Self::Color => "Color",
            Self::GradientStops => "GradientStops",
            Self::DpiStages => "DpiStages",
            Self::LightingZone => "LightingZone",
            Self::ReadOnlyValue => "ReadOnlyValue",
            Self::Action => "Action",
            Self::Info => "Info",
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ApiError {
    #[error("unsupported plugin schema version {0}")]
    SchemaVersion(u32),
    #[error("invalid plugin id {0}")]
    PluginId(String),
    #[error("plugin API requirement {0} is incompatible")]
    ApiIncompatible(String),
    #[error("stable writes require hardware-verified evidence")]
    UnsafeWriteEvidence,
    #[error("capability {0} has an invalid placement")]
    CapabilityPlacement(String),
    #[error("capability {0} has an invalid summary declaration")]
    CapabilitySummary(String),
    #[error("capability {0} has invalid control options")]
    CapabilityOptions(String),
    #[error("plugin exceeds the host dashboard layout limits")]
    CapabilityLayout,
    #[error("capability {0} declares an invalid minFirmware semver")]
    CapabilityMinFirmware(String),
    #[error("capability {0} declares unknown connection types")]
    CapabilityConnections(String),
    #[error("capability {0} has invalid binding conditions")]
    CapabilityBinding(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_writes_without_hardware_evidence() {
        let manifest = PluginManifest {
            schema_version: 1,
            plugin_id: "mira.example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            plugin_api: ">=1.0.0, <2.0.0".parse().unwrap(),
            publisher_key_id: None,
            evidence: EvidenceLevel::FixtureVerified,
            permissions: vec![],
            capabilities: vec![],
            writes_enabled: true,
            exportable_fields: vec![],
            depends_on: vec![],
        };
        assert_eq!(manifest.validate(), Err(ApiError::UnsafeWriteEvidence));
    }

    #[test]
    fn limits_host_rendered_summary_declarations() {
        let capability = Capability {
            id: "polling-rate".into(),
            control: Control::Select,
            label_key: "capability.polling-rate".into(),
            read_only: true,
            placements: vec![],
            metadata: BTreeMap::from([(
                "summary".into(),
                serde_json::json!([
                    {"label": "1", "source": "one"},
                    {"label": "2", "source": "two"},
                    {"label": "3", "source": "three"},
                    {"label": "4", "source": "four"},
                    {"label": "5", "source": "five"}
                ]),
            )]),
            probe: None,
            connections: None,
            min_firmware: None,
        };
        let manifest = PluginManifest {
            schema_version: 1,
            plugin_id: "mira.example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            plugin_api: ">=1.0.0, <2.0.0".parse().unwrap(),
            publisher_key_id: None,
            evidence: EvidenceLevel::FixtureVerified,
            permissions: vec![],
            capabilities: vec![capability],
            writes_enabled: false,
            exportable_fields: vec![],
            depends_on: vec![],
        };
        assert_eq!(
            manifest.validate(),
            Err(ApiError::CapabilitySummary("polling-rate".into()))
        );
    }

    #[test]
    fn rejects_display_labels_in_connection_bindings() {
        let capability = Capability {
            id: "sleep-time".into(),
            control: Control::Number,
            label_key: "capability.sleep-time".into(),
            read_only: false,
            placements: vec![],
            metadata: BTreeMap::from([(
                "bindings".into(),
                serde_json::json!([
                    {
                        "when": { "path": "connection", "eq": "无线" },
                        "label": "2.4G 休眠",
                        "source": "capabilities.settings.wirelessSleepValue"
                    }
                ]),
            )]),
            probe: None,
            connections: None,
            min_firmware: None,
        };
        let manifest = PluginManifest {
            schema_version: 1,
            plugin_id: "mira.example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            plugin_api: ">=1.0.0, <2.0.0".parse().unwrap(),
            publisher_key_id: None,
            evidence: EvidenceLevel::FixtureVerified,
            permissions: vec![],
            capabilities: vec![capability],
            writes_enabled: false,
            exportable_fields: vec![],
            depends_on: vec![],
        };
        assert_eq!(
            manifest.validate(),
            Err(ApiError::CapabilityBinding("sleep-time".into()))
        );
    }

    #[test]
    fn accepts_host_connection_values_in_bindings() {
        let capability = Capability {
            id: "sleep-time".into(),
            control: Control::Number,
            label_key: "capability.sleep-time".into(),
            read_only: false,
            placements: vec![],
            metadata: BTreeMap::from([(
                "bindings".into(),
                serde_json::json!([
                    {"when": { "path": "connection", "eq": "usb" }, "source": "usbSleep"},
                    {"when": { "path": "connection", "eq": "wireless" }, "source": "wirelessSleep"},
                    {"when": { "path": "connection", "eq": "bluetooth" }, "source": "bluetoothSleep"},
                    {"when": { "path": "connection", "eq": "virtual" }, "source": "virtualSleep"}
                ]),
            )]),
            probe: None,
            connections: None,
            min_firmware: None,
        };
        let manifest = PluginManifest {
            schema_version: 1,
            plugin_id: "mira.example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            plugin_api: ">=1.0.0, <2.0.0".parse().unwrap(),
            publisher_key_id: None,
            evidence: EvidenceLevel::FixtureVerified,
            permissions: vec![],
            capabilities: vec![capability],
            writes_enabled: false,
            exportable_fields: vec![],
            depends_on: vec![],
        };
        assert_eq!(manifest.validate(), Ok(()));
    }

    #[test]
    fn limits_dashboard_control_groups() {
        let capabilities = (0..7)
            .map(|index| Capability {
                id: format!("control-{index}"),
                control: Control::ReadOnlyValue,
                label_key: format!("capability.control-{index}"),
                read_only: true,
                placements: vec![CapabilityPlacement {
                    region: CapabilityRegion::Control,
                    group: Some(format!("group-{index}")),
                    order: index,
                    span: 1,
                    icon: None,
                }],
                metadata: BTreeMap::new(),
                probe: None,
                connections: None,
                min_firmware: None,
            })
            .collect();
        let manifest = PluginManifest {
            schema_version: 1,
            plugin_id: "mira.example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            plugin_api: ">=1.0.0, <2.0.0".parse().unwrap(),
            publisher_key_id: None,
            evidence: EvidenceLevel::FixtureVerified,
            permissions: vec![],
            capabilities,
            writes_enabled: false,
            exportable_fields: vec![],
            depends_on: vec![],
        };
        assert_eq!(manifest.validate(), Err(ApiError::CapabilityLayout));
    }
}
