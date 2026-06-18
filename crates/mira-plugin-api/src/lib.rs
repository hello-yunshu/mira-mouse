// SPDX-License-Identifier: AGPL-3.0-or-later
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const PLUGIN_SCHEMA_VERSION: u32 = 1;
pub const PLUGIN_API_VERSION: &str = "1.0.0";

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
        Ok(())
    }
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
    pub metadata: BTreeMap<String, serde_json::Value>,
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
}
