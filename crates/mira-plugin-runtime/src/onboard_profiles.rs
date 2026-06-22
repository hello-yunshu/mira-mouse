//! Normalizes profile-related outputs from HID++ Onboard Profiles (`0x8100`)
//! and Profile Management (`0x8101`) into runtime-friendly values.
//!
//! The HID++ plugin exposes profile information through several workflow
//! outputs. This module centralizes extraction so `protocol.rs` does not need
//! to know the exact JSON shape of every future profile feature.
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Returns the currently active profile index, if any source reports one.
///
/// Priority:
/// 1. `0x8101` Profile Management `profileMgmtCurrent.profileIndex`
/// 2. Legacy `settings.profile` (used by AMaster / HID++ report rate output)
/// 3. Legacy `dpi.profile` (used by AMaster protocol A)
pub fn active_profile_index(outputs: &BTreeMap<String, Value>) -> Option<u8> {
    object(outputs, "profileMgmtCurrent")
        .and_then(|current| number(current, "profileIndex"))
        .or_else(|| object(outputs, "settings").and_then(|settings| number(settings, "profile")))
        .or_else(|| object(outputs, "dpi").and_then(|dpi| number(dpi, "profile")))
}

/// Returns the number of stored profiles reported by `0x8101` Profile
/// Management, if the device exposes that feature.
pub fn profile_count(outputs: &BTreeMap<String, Value>) -> Option<u8> {
    object(outputs, "profileMgmtCount").and_then(|count| number(count, "profileCount"))
}

/// Returns capability metadata from `0x8101` Profile Management.
pub fn profile_management_info(outputs: &BTreeMap<String, Value>) -> Option<ProfileManagementInfo> {
    let info = object(outputs, "profileMgmtInfo")?;
    Some(ProfileManagementInfo {
        feature_version: number(info, "featureVersion")?,
        max_profile_count: number(info, "maxProfileCount"),
        profile_name_length: number(info, "profileNameLength"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileManagementInfo {
    pub feature_version: u8,
    pub max_profile_count: Option<u8>,
    pub profile_name_length: Option<u8>,
}

fn object<'a>(outputs: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    outputs.get(key)?.as_object()
}

fn number(object: &Map<String, Value>, key: &str) -> Option<u8> {
    object
        .get(key)?
        .as_u64()
        .and_then(|value| u8::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefers_profile_management_current() {
        let outputs = BTreeMap::from([
            ("profileMgmtCurrent".into(), json!({"profileIndex": 2})),
            ("settings".into(), json!({"profile": 1})),
            ("dpi".into(), json!({"profile": 0})),
        ]);
        assert_eq!(active_profile_index(&outputs), Some(2));
    }

    #[test]
    fn falls_back_to_settings_profile() {
        let outputs = BTreeMap::from([
            ("settings".into(), json!({"profile": 3})),
            ("dpi".into(), json!({"profile": 0})),
        ]);
        assert_eq!(active_profile_index(&outputs), Some(3));
    }

    #[test]
    fn falls_back_to_dpi_profile() {
        let outputs = BTreeMap::from([("dpi".into(), json!({"profile": 4}))]);
        assert_eq!(active_profile_index(&outputs), Some(4));
    }

    #[test]
    fn returns_none_without_profile_outputs() {
        let outputs = BTreeMap::from([("battery".into(), json!({"percentage": 100}))]);
        assert_eq!(active_profile_index(&outputs), None);
    }

    #[test]
    fn extracts_profile_count() {
        let outputs = BTreeMap::from([("profileMgmtCount".into(), json!({"profileCount": 5}))]);
        assert_eq!(profile_count(&outputs), Some(5));
    }

    #[test]
    fn extracts_profile_management_info() {
        let outputs = BTreeMap::from([(
            "profileMgmtInfo".into(),
            json!({"featureVersion": 1, "maxProfileCount": 5, "profileNameLength": 24}),
        )]);
        let info = profile_management_info(&outputs).expect("info");
        assert_eq!(info.feature_version, 1);
        assert_eq!(info.max_profile_count, Some(5));
        assert_eq!(info.profile_name_length, Some(24));
    }

    #[test]
    fn profile_management_info_requires_version() {
        let outputs = BTreeMap::from([("profileMgmtInfo".into(), json!({"maxProfileCount": 5}))]);
        assert_eq!(profile_management_info(&outputs), None);
    }
}
