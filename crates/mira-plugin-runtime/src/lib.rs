// SPDX-License-Identifier: AGPL-3.0-or-later
mod dsl;
mod engine;
pub mod hid;
mod onboard_profiles;
mod package;
pub mod protocol;

pub use dsl::{execute_workflow, DslError, Limits, Operation, Transport, Workflow};
pub use engine::ProtocolPackage;
pub use package::{
    canonical_json, extract_package, inspect_package, PackageError, PackageInspection, TrustStore,
};
pub use protocol::{
    execute_plugin_workflow, mutate_device, mutate_device_with_package, read_device,
    read_device_with_package, writable_mutations, writable_mutations_with_package, ConnectionKind,
    DeviceReading, ProtocolContext,
};
