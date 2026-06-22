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
        };
        assert_eq!(
            manifest.validate(),
            Err(ApiError::CapabilitySummary("polling-rate".into()))
        );
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
        };
        assert_eq!(manifest.validate(), Err(ApiError::CapabilityLayout));
    }
}
