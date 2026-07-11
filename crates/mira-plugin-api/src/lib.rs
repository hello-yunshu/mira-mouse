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
const MAX_PLUGIN_NAME_CHARS: usize = 48;
const MAX_UI_LABEL_CHARS: usize = 16;
const MAX_UI_ACTION_LABEL_CHARS: usize = 20;

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
        if !valid_ui_text(&self.name, MAX_PLUGIN_NAME_CHARS) {
            return Err(ApiError::PluginPresentation);
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
            if !valid_presentation_contract(capability) {
                return Err(ApiError::CapabilityPresentation(capability.id.clone()));
            }
            if let Some(summary) = capability.metadata.get("summary") {
                let valid = summary.as_array().is_some_and(|items| {
                    items.len() <= MAX_SUMMARY_ITEMS
                        && items.iter().all(|item| {
                            item.as_object().is_some_and(|item| {
                                (item
                                    .get("label")
                                    .and_then(serde_json::Value::as_str)
                                    .is_some_and(|label| valid_ui_text(label, MAX_UI_LABEL_CHARS))
                                    || item
                                        .get("labelKey")
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(valid_i18n_key))
                                    && item
                                        .get("source")
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(|source| !source.is_empty())
                                    && item.get("options").is_none_or(|options| {
                                        valid_options(options, 32)
                                            || valid_declarative_options(options)
                                    })
                                    && item.get("format").is_none_or(valid_value_format)
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
                        .is_some_and(|label| valid_ui_text(label, MAX_UI_LABEL_CHARS))
                        && item.get("value").is_some_and(|value| {
                            value.is_string() || value.is_number() || value.is_boolean()
                        })
                })
            })
    })
}

fn valid_presentation_contract(capability: &Capability) -> bool {
    let metadata = &capability.metadata;
    // schema v1 的 UI 契约是声明式 fields/zones/stageLayout；新包不得再依赖
    // host 专用的扁平 metadata。保留旧分支仅用于读取历史已签名包，发布校验
    // 已在插件仓库拒绝旧写法。
    if metadata.keys().any(|key| {
        matches!(
            key.as_str(),
            "fields"
                | "zones"
                | "stageLayout"
                | "statusDisplay"
                | "stateMapping"
                | "accentSource"
                | "visibleWhen"
                | "batteryHistory"
                | "summary"
        )
    }) {
        return valid_declarative_presentation(capability);
    }
    if !valid_optional_ui_text(metadata.get("label"), MAX_UI_LABEL_CHARS)
        || !valid_optional_ui_text(metadata.get("actionLabel"), MAX_UI_ACTION_LABEL_CHARS)
        || !valid_binding_labels(metadata.get("bindings"))
    {
        return false;
    }
    if metadata
        .get("format")
        .is_some_and(|format| !valid_value_format(format))
    {
        return false;
    }
    if !valid_numeric_range(metadata) {
        return false;
    }
    match capability.control {
        Control::DpiStages if !capability.read_only => {
            let Some(mutations) = metadata
                .get("mutations")
                .and_then(serde_json::Value::as_object)
            else {
                return false;
            };
            valid_mutation_ref(mutations.get("select"))
                && valid_mutation_ref(mutations.get("value"))
        }
        Control::LightingZone if !capability.read_only => metadata
            .get("lightingRole")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|role| {
                valid_mutation_ref(role.get("mouse")) || valid_mutation_ref(role.get("receiver"))
            }),
        Control::Select | Control::Segmented if !capability.read_only => {
            metadata
                .get("options")
                .is_some_and(|options| valid_options(options, MAX_CONTROL_OPTIONS))
                && valid_mutation_contract(metadata)
        }
        Control::Toggle | Control::Number | Control::Slider | Control::Color | Control::Action
            if !capability.read_only =>
        {
            valid_mutation_contract(metadata)
        }
        _ => true,
    }
}

fn valid_declarative_presentation(capability: &Capability) -> bool {
    let metadata = &capability.metadata;
    let valid_path = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| !path.is_empty() && path.len() <= 160)
    };
    let valid_range = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_object)
            .is_some_and(|range| {
                let min = range.get("min").and_then(serde_json::Value::as_f64);
                let max = range.get("max").and_then(serde_json::Value::as_f64);
                min.zip(max).is_some_and(|(min, max)| min <= max)
                    && range
                        .get("step")
                        .is_none_or(|step| step.as_f64().is_some_and(|step| step > 0.0))
            })
    };
    let valid_field = |field: &serde_json::Value| {
        field.as_object().is_some_and(|field| {
            valid_path(field.get("id"))
                && valid_path(field.get("source"))
                && field
                    .get("editor")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|editor| {
                        matches!(
                            editor,
                            "inline-toggle"
                                | "inline-segmented"
                                | "inline-value"
                                | "inline-action"
                                | "modal-select"
                                | "modal-color"
                                | "modal-range"
                                | "modal-number"
                                | "modal-dpi-stage"
                                | "modal-gradient"
                                | "static-readonly"
                        )
                    })
                && field
                    .get("mutation")
                    .is_none_or(|mutation| valid_mutation_ref(Some(mutation)))
                && field.get("options").is_none_or(valid_declarative_options)
                && field
                    .get("range")
                    .is_none_or(|range| valid_range(Some(range)))
                && field.get("paramSources").is_none_or(|sources| {
                    sources.as_object().is_some_and(|sources| {
                        !sources.is_empty()
                            && sources.iter().all(|(param, source)| {
                                !param.is_empty() && valid_path(Some(source))
                            })
                    })
                })
        })
    };
    let fields_valid = metadata.get("fields").is_none_or(|fields| {
        fields
            .as_array()
            .is_some_and(|fields| fields.len() <= 32 && fields.iter().all(valid_field))
    });
    let zones_valid = metadata.get("zones").is_none_or(|zones| {
        zones.as_array().is_some_and(|zones| {
            zones.len() <= 8
                && zones.iter().all(|zone| {
                    zone.as_object().is_some_and(|zone| {
                        valid_path(zone.get("id"))
                            && zone
                                .get("labelKey")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|key| !key.is_empty())
                            && zone
                                .get("fields")
                                .and_then(serde_json::Value::as_array)
                                .is_some_and(|fields| {
                                    fields.len() <= 32 && fields.iter().all(valid_field)
                                })
                    })
                })
        })
    });
    let battery_history_valid = metadata
        .get("batteryHistory")
        .is_none_or(valid_battery_history_policy)
        && (!metadata.contains_key("batteryHistory") || capability.id == "battery");
    let accent_source_valid = metadata
        .get("accentSource")
        .is_none_or(|source| valid_path(Some(source)));
    if !fields_valid || !zones_valid || !battery_history_valid || !accent_source_valid {
        return false;
    }
    match capability.control {
        Control::DpiStages if !capability.read_only => metadata
            .get("stageLayout")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|layout| {
                valid_path(layout.get("dotsSource"))
                    && valid_path(layout.get("valueSource"))
                    && valid_mutation_ref(layout.get("selectMutation"))
                    && valid_mutation_ref(layout.get("setMutation"))
                    && valid_range(layout.get("range"))
            }),
        Control::LightingZone if !capability.read_only => metadata
            .get("zones")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|zones| {
                zones.iter().any(|zone| {
                    zone.get("fields")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|fields| {
                            fields.iter().any(|field| field.get("mutation").is_some())
                        })
                })
            }),
        _ if !capability.read_only => metadata
            .get("fields")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|fields| {
                fields.iter().any(|field| {
                    field
                        .get("mutation")
                        .is_some_and(|mutation| valid_mutation_ref(Some(mutation)))
                })
            }),
        _ => true,
    }
}

/// 电量历史属于设备语义而非宿主猜测：由 battery capability 显式声明可信连接方式。
fn valid_battery_history_policy(value: &serde_json::Value) -> bool {
    let Some(policy) = value.as_object() else {
        return false;
    };
    if policy.len() != 1 {
        return false;
    }
    let Some(connections) = policy
        .get("validConnections")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    if connections.is_empty() || connections.len() > 4 {
        return false;
    }
    let mut unique = BTreeSet::new();
    connections.iter().all(|connection| {
        connection.as_str().is_some_and(|connection| {
            valid_binding_connection(connection) && unique.insert(connection)
        })
    })
}

fn valid_declarative_options(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|options| {
        options.len() <= 32
            && options.iter().all(|option| {
                option.as_object().is_some_and(|option| {
                    option.get("value").is_some_and(|value| {
                        value.is_string() || value.is_number() || value.is_boolean()
                    }) && option
                        .get("labelKey")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|key| !key.is_empty())
                })
            })
    })
}

fn valid_optional_ui_text(value: Option<&serde_json::Value>, max_chars: usize) -> bool {
    value.is_none_or(|value| {
        value
            .as_str()
            .is_some_and(|text| valid_ui_text(text, max_chars))
    })
}

fn valid_ui_text(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().all(|c| c != '\n' && c != '\r')
        && value.chars().count() <= max_chars
}

fn valid_i18n_key(value: &str) -> bool {
    valid_ui_text(value, 160)
}

fn valid_binding_labels(value: Option<&serde_json::Value>) -> bool {
    value.is_none_or(|value| {
        value.as_array().is_some_and(|items| {
            items.iter().all(|item| {
                item.as_object().is_some_and(|item| {
                    valid_optional_ui_text(item.get("label"), MAX_UI_LABEL_CHARS)
                })
            })
        })
    })
}

fn valid_mutation_contract(metadata: &BTreeMap<String, serde_json::Value>) -> bool {
    if valid_mutation_ref(metadata.get("mutation")) {
        return true;
    }
    if metadata
        .get("mutations")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|mutations| valid_mutation_ref(mutations.get("default")))
    {
        return true;
    }
    metadata.get("bindings").is_some_and(|bindings| {
        valid_binding_sources(bindings) && valid_binding_mutations(bindings)
    })
}

fn valid_mutation_ref(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::String(value)) => !value.is_empty(),
        Some(serde_json::Value::Array(values)) => {
            !values.is_empty()
                && values
                    .iter()
                    .all(|value| value.as_str().is_some_and(|value| !value.is_empty()))
        }
        _ => false,
    }
}

fn valid_binding_sources(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty()
            && items.iter().all(|item| {
                item.as_object().is_some_and(|item| {
                    item.get("source")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|source| !source.is_empty())
                })
            })
    })
}

fn valid_binding_mutations(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty()
            && items.iter().all(|item| {
                item.as_object()
                    .is_some_and(|item| valid_mutation_ref(item.get("mutation")))
            })
    })
}

fn valid_numeric_range(metadata: &BTreeMap<String, serde_json::Value>) -> bool {
    let min = metadata.get("min").map(serde_json::Value::as_f64);
    let max = metadata.get("max").map(serde_json::Value::as_f64);
    let step = metadata.get("step").map(serde_json::Value::as_f64);
    if min.is_some_and(|value| value.is_none())
        || max.is_some_and(|value| value.is_none())
        || step.is_some_and(|value| value.is_none_or(|value| value <= 0.0))
    {
        return false;
    }
    match (min.flatten(), max.flatten()) {
        (Some(min), Some(max)) => min <= max,
        _ => true,
    }
}

fn valid_value_format(value: &serde_json::Value) -> bool {
    matches!(value.as_str(), Some("sleep" | "color"))
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

impl Capability {
    /// 从 metadata 反序列化灯效选项（effectOptions 强类型字段）。
    /// 替代 UI 中通过字符串 key 名访问开放 metadata 的隐式约定。
    pub fn effect_options(&self) -> Option<EffectOptions> {
        self.metadata
            .get("effectOptions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// 从 metadata 反序列化接收器灯效选项（receiverLightingOptions 强类型字段）。
    pub fn receiver_lighting_options(&self) -> Option<ReceiverLightingOptions> {
        self.metadata
            .get("receiverLightingOptions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// 从 metadata 反序列化灯光 mutation 角色映射（lightingRole 强类型字段）。
    /// 供后端夜间模式动态发现 mutation 名。
    pub fn lighting_role(&self) -> Option<LightingRole> {
        let legacy = self
            .metadata
            .get("lightingRole")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        if legacy.is_some() {
            return legacy;
        }
        let zones = self.metadata.get("zones")?.as_array()?;
        let mut role = LightingRole {
            mouse: None,
            receiver: None,
        };
        for zone in zones {
            let Some(zone) = zone.as_object() else {
                continue;
            };
            let Some(id) = zone.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let mutation = zone
                .get("fields")
                .and_then(serde_json::Value::as_array)
                .and_then(|fields| fields.iter().find_map(|field| field.get("mutation")))
                .and_then(|mutation| serde_json::from_value::<MutationDecl>(mutation.clone()).ok());
            match id {
                "mouse" => role.mouse = mutation,
                "receiver" => role.receiver = mutation,
                _ => {}
            }
        }
        (role.mouse.is_some() || role.receiver.is_some()).then_some(role)
    }
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

/// 灯效选项声明（强类型化，替代 HID++ 隐式 metadata 约定）。
/// 由 LightingZone capability 在 metadata.effectOptions 中声明。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectOption {
    /// 灯效数值（如 0=off, 1=fixed, 5=starlight）。
    pub value: f64,
    /// 指向插件 locale 的 i18n key（如 "lighting.fixed"）。
    pub label_key: String,
    /// 该灯效是否需要第二色（如 starlight 需要 extraColor）。
    /// 替代 UI 中硬编码 effect===5 判断。
    #[serde(default)]
    pub requires_extra_color: bool,
}

/// 灯效速度/亮度范围声明。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeSpec {
    pub min: f64,
    pub max: f64,
    #[serde(default)]
    pub step: Option<f64>,
}

/// 灯效选项集（effectOptions 强类型字段）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectOptions {
    /// 声明哪个数值表示"关闭"。替代 UI/后端硬编码 effect===0。
    #[serde(default)]
    pub off_value: Option<f64>,
    pub effect: Vec<EffectOption>,
    #[serde(default)]
    pub speed: Option<RangeSpec>,
    #[serde(default)]
    pub brightness: Option<RangeSpec>,
}

/// 接收器灯效选项条目（labelKey 引用插件 locale）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiverLightingOption {
    pub value: f64,
    pub label_key: String,
}

/// 接收器灯效选项集（receiverLightingOptions 强类型字段）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiverLightingOptions {
    #[serde(default)]
    pub effect: Vec<ReceiverLightingOption>,
    #[serde(default)]
    pub speed: Vec<ReceiverLightingOption>,
    #[serde(default)]
    pub brightness: Vec<ReceiverLightingOption>,
    #[serde(default)]
    pub option: Vec<ReceiverLightingOption>,
}

/// 灯光 mutation 角色映射（lightingRole 强类型字段）。
/// 供后端夜间模式动态发现 mutation 名，替代硬编码 'set-mouse-lighting'。
/// mouse/receiver 可声明单个 mutation 或按优先级排序的候选数组，
/// Host 按数组顺序选取第一个被设备 writableMutations 支持的 mutation。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LightingRole {
    #[serde(default)]
    pub mouse: Option<MutationDecl>,
    #[serde(default)]
    pub receiver: Option<MutationDecl>,
}

/// mutation 声明：单个字符串或按优先级排序的候选数组。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MutationDecl {
    Single(String),
    Many(Vec<String>),
}

impl MutationDecl {
    /// 按声明顺序返回所有候选 mutation 名。
    pub fn candidates(&self) -> Vec<&str> {
        match self {
            MutationDecl::Single(value) => vec![value.as_str()],
            MutationDecl::Many(values) => values.iter().map(String::as_str).collect(),
        }
    }

    /// 选取第一个被 writable 支持的 mutation；若均不支持则返回首个候选。
    pub fn pick<'a>(&'a self, writable: &[String]) -> Option<&'a str> {
        let candidates = self.candidates();
        let first = candidates.first().copied()?;
        candidates
            .into_iter()
            .find(|candidate| writable.iter().any(|w| w == candidate))
            .or(Some(first))
    }
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
    #[error("plugin has an invalid UI presentation contract")]
    PluginPresentation,
    #[error("capability {0} has an invalid placement")]
    CapabilityPlacement(String),
    #[error("capability {0} has an invalid summary declaration")]
    CapabilitySummary(String),
    #[error("capability {0} has invalid control options")]
    CapabilityOptions(String),
    #[error("capability {0} has an invalid UI presentation contract")]
    CapabilityPresentation(String),
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
    fn limits_plugin_display_name_length() {
        let manifest = PluginManifest {
            schema_version: 1,
            plugin_id: "mira.example".into(),
            name: "A plugin name that is intentionally too long for compact host UI".into(),
            version: "1.0.0".into(),
            plugin_api: ">=1.0.0, <2.0.0".parse().unwrap(),
            publisher_key_id: None,
            evidence: EvidenceLevel::FixtureVerified,
            permissions: vec![],
            capabilities: vec![],
            writes_enabled: false,
            exportable_fields: vec![],
            depends_on: vec![],
        };
        assert_eq!(manifest.validate(), Err(ApiError::PluginPresentation));
    }

    #[test]
    fn limits_host_rendered_capability_text() {
        let capability = Capability {
            id: "profile-mgmt-current".into(),
            control: Control::ReadOnlyValue,
            label_key: "capability.profile-mgmt-current".into(),
            read_only: true,
            placements: vec![],
            metadata: BTreeMap::from([(
                "label".into(),
                serde_json::json!("当前配置文件名称特别特别特别特别长"),
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
            Err(ApiError::CapabilityPresentation(
                "profile-mgmt-current".into()
            ))
        );
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
    fn accepts_namespaced_summary_label_keys() {
        assert!(valid_i18n_key("summary.motionSync"));
        assert!(valid_i18n_key("receiverLighting.field.option"));
        assert!(!valid_i18n_key(""));
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
                    {"when": { "path": "connection", "eq": "usb" }, "source": "usbSleep", "mutation": "set-usb-sleep"},
                    {"when": { "path": "connection", "eq": "wireless" }, "source": "wirelessSleep", "mutation": "set-wireless-sleep"},
                    {"when": { "path": "connection", "eq": "bluetooth" }, "source": "bluetoothSleep", "mutation": "set-bluetooth-sleep"},
                    {"when": { "path": "connection", "eq": "virtual" }, "source": "virtualSleep", "mutation": "set-virtual-sleep"}
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
    fn validates_battery_history_connections_as_plugin_semantics() {
        assert!(valid_battery_history_policy(&serde_json::json!({
            "validConnections": ["wireless", "bluetooth"]
        })));
        assert!(!valid_battery_history_policy(&serde_json::json!({
            "validConnections": ["receiver"]
        })));
        assert!(!valid_battery_history_policy(&serde_json::json!({
            "validConnections": ["wireless", "wireless"]
        })));
    }

    #[test]
    fn rejects_unknown_value_formats() {
        let capability = Capability {
            id: "profile-color".into(),
            control: Control::ReadOnlyValue,
            label_key: "capability.profile-color".into(),
            read_only: true,
            placements: vec![],
            metadata: BTreeMap::from([("format".into(), serde_json::json!("colour"))]),
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
            Err(ApiError::CapabilityPresentation("profile-color".into()))
        );
    }

    #[test]
    fn rejects_writable_lighting_without_role_contract() {
        let capability = Capability {
            id: "lighting".into(),
            control: Control::LightingZone,
            label_key: "capability.lighting".into(),
            read_only: false,
            placements: vec![],
            metadata: BTreeMap::new(),
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
            Err(ApiError::CapabilityPresentation("lighting".into()))
        );
    }

    #[test]
    fn accepts_declarative_lighting_and_derives_its_mutation_roles() {
        let capability = Capability {
            id: "lighting".into(),
            control: Control::LightingZone,
            label_key: "capability.lighting".into(),
            read_only: false,
            placements: vec![],
            metadata: BTreeMap::from([(
                "zones".into(),
                serde_json::json!([
                    {"id": "mouse", "labelKey": "lighting.mouse", "fields": [{"id": "enabled", "source": "capabilities.mouse.enabled", "editor": "inline-toggle", "mutation": ["set-mouse", "set-mouse-legacy"]}]},
                    {"id": "receiver", "labelKey": "lighting.receiver", "fields": [{"id": "effect", "source": "capabilities.receiver.effect", "editor": "modal-select", "mutation": "set-receiver", "options": [{"value": 0, "labelKey": "lighting.off"}]}]}
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
            capabilities: vec![capability.clone()],
            writes_enabled: false,
            exportable_fields: vec![],
            depends_on: vec![],
        };
        assert_eq!(manifest.validate(), Ok(()));
        let roles = capability.lighting_role().expect("roles from zones");
        assert_eq!(
            roles.mouse.unwrap().candidates(),
            vec!["set-mouse", "set-mouse-legacy"]
        );
        assert_eq!(roles.receiver.unwrap().candidates(), vec!["set-receiver"]);
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
