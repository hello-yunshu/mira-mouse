// SPDX-License-Identifier: AGPL-3.0-or-later
mod dsl;
mod engine;
pub mod hid;
mod onboard_profiles;
// 3.5 节：package 模块公开，使 mira-plugin-cli 能共享 allowed() 和 PACKAGE_FORMAT_VERSION。
// 这是主仓库 runtime 和 CLI 共享同一个 Package Format 实现的单一事实源。
pub mod package;
pub mod protocol;

// Re-export mira_plugin_api 的关键类型，便于下游 crate（含测试）构造插件声明。
pub use mira_plugin_api::{
    Capability, CapabilityPlacement, CapabilityProbe, CapabilityRegion, Control, EffectOption,
    EffectOptions, EvidenceLevel, ExportableField, LightingRole, MutationDecl, Permission,
    PluginDependency, PluginManifest, PluginRuntime, RangeSpec, ReceiverLightingOption,
    ReceiverLightingOptions, ReportType, WakeActivitySource, WakeRecoveryContract,
};

pub use dsl::{execute_workflow, DslError, Limits, Operation, Transport, Workflow};
pub use engine::{
    framed_response_matches_request, plan_request_fragments, resolve_marker_offsets,
    MultiPacketAssembler, ProtocolPackage, WorkflowProjection,
};
pub use package::{
    allowed, canonical_json, extract_package, inspect_package, PackageError, PackageInspection,
    TrustStore, PACKAGE_FORMAT_VERSION,
};
pub use protocol::{
    execute_plugin_workflow, map_semantic_to_outputs, mutate_device, mutate_device_with_package,
    normalize_device_outputs_with_package, read_device, read_device_with_package,
    read_device_with_projection, writable_mutations, writable_mutations_with_package,
    ConnectionKind, DeviceReading, FeatureIndexCache, HidEventSink, HidHandleCache, HidIoStats,
    NullHidEventSink, OnboardMemoryCache, ProjectedReading, ProtocolContext, ReadStatus,
    SemanticField,
};
