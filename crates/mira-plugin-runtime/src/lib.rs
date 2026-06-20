// SPDX-License-Identifier: AGPL-3.0-or-later
mod dsl;
mod engine;
pub mod hid;
mod package;
pub mod protocol;

pub use dsl::{execute_workflow, DslError, Limits, Operation, Transport, Workflow};
pub use package::{
    canonical_json, extract_package, inspect_package, PackageError, PackageInspection, TrustStore,
};
pub use protocol::{read_device, ConnectionKind, DeviceReading, ProtocolContext};
