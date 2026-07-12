// SPDX-License-Identifier: AGPL-3.0-or-later
mod dsl;
mod engine;
pub mod hid;
mod onboard_profiles;
mod package;
pub mod protocol;

// Re-export mira_plugin_api 的关键类型，便于下游 crate（含测试）构造插件声明。
pub use mira_plugin_api::{
    Capability, CapabilityPlacement, CapabilityProbe, CapabilityRegion, Control, EffectOption,
    EffectOptions, EvidenceLevel, ExportableField, LightingRole, MutationDecl, Permission,
    PluginDependency, PluginManifest, RangeSpec, ReceiverLightingOption, ReceiverLightingOptions,
    ReportType,
};

pub use dsl::{execute_workflow, DslError, Limits, Operation, Transport, Workflow};
pub use engine::{ProtocolPackage, WorkflowProjection};
pub use package::{
    canonical_json, extract_package, inspect_package, PackageError, PackageInspection, TrustStore,
};
pub use protocol::{
    execute_plugin_workflow, map_semantic_to_outputs, mutate_device, mutate_device_with_package,
    normalize_device_outputs_with_package, read_device, read_device_with_package,
    read_device_with_projection, writable_mutations, writable_mutations_with_package,
    ConnectionKind, DeviceReading, FeatureIndexCache, HidHandleCache, HidIoStats,
    OnboardMemoryCache, ProjectedReading, ProtocolContext, SemanticField,
};
