// SPDX-License-Identifier: AGPL-3.0-or-later
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use mira_plugin_api::PluginManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Seek},
};
use thiserror::Error;
use zip::ZipArchive;

const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Default)]
pub struct TrustStore(pub HashMap<String, VerifyingKey>);

#[derive(Debug, Clone, Serialize)]
pub struct PackageInspection {
    pub plugin_id: String,
    pub version: String,
    pub evidence: String,
    pub signature_verified: bool,
    pub writes_enabled: bool,
    pub capabilities: Vec<mira_plugin_api::Capability>,
    pub exportable_fields: Vec<mira_plugin_api::ExportableField>,
    pub depends_on: Vec<mira_plugin_api::PluginDependency>,
    pub file_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Checksums {
    schema_version: u32,
    files: BTreeMap<String, String>,
}

/// Canonicalise a JSON document by recursively sorting object keys.
///
/// `serde_json::Map` defaults to `BTreeMap` (keys already sorted), so the
/// object branch is a no-op in the current configuration. The recursive walk
/// is kept so canonicalisation still holds if `preserve_order` is enabled in
/// the future, and so array elements are normalised consistently.
pub fn canonical_json(bytes: &[u8]) -> Result<Vec<u8>, PackageError> {
    fn sort(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                Value::Object(map.into_iter().map(|(k, v)| (k, sort(v))).collect())
            }
            Value::Array(items) => Value::Array(items.into_iter().map(sort).collect()),
            other => other,
        }
    }
    let value: Value = serde_json::from_slice(bytes)?;
    Ok(serde_json::to_vec(&sort(value))?)
}

/// Extract and verify a plugin package, returning both inspection metadata and the
/// raw file map. Callers that only need the summary can use `inspect_package`.
pub fn extract_package<R: Read + Seek>(
    reader: R,
    trust: &TrustStore,
    require_signature: bool,
) -> Result<(PackageInspection, BTreeMap<String, Vec<u8>>), PackageError> {
    let mut archive = ZipArchive::new(reader)?;
    if archive.len() > MAX_FILES {
        return Err(PackageError::Limit("file count"));
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        validate_path(&name)?;
        if entry.is_dir() {
            continue;
        }
        if !allowed(&name) {
            return Err(PackageError::Forbidden(name));
        }
        if entry.size() > MAX_FILE_BYTES {
            return Err(PackageError::Limit("file size"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or(PackageError::Limit("total size"))?;
        if total > MAX_TOTAL_BYTES {
            return Err(PackageError::Limit("total size"));
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        if files.insert(name.clone(), bytes).is_some() {
            return Err(PackageError::Duplicate(name));
        }
    }
    let manifest_bytes = files
        .get("plugin.json")
        .ok_or(PackageError::Missing("plugin.json"))?;
    let manifest: PluginManifest = serde_json::from_slice(manifest_bytes)?;
    manifest
        .validate()
        .map_err(|e| PackageError::Manifest(e.to_string()))?;
    let checksums_bytes = files
        .get("checksums.json")
        .ok_or(PackageError::Missing("checksums.json"))?;
    let checksums: Checksums = serde_json::from_slice(checksums_bytes)?;
    if checksums.schema_version != 1 {
        return Err(PackageError::Manifest("checksum schema".into()));
    }
    for (name, expected) in &checksums.files {
        if name == "checksums.json" || name == "META-INF/signature.ed25519" {
            return Err(PackageError::ChecksumScope(name.clone()));
        }
        let bytes = files
            .get(name)
            .ok_or_else(|| PackageError::MissingOwned(name.clone()))?;
        let actual = hex::encode(Sha256::digest(bytes));
        if &actual != expected {
            return Err(PackageError::Digest(name.clone()));
        }
    }
    let payload_names: Vec<_> = files
        .keys()
        .filter(|n| *n != "checksums.json" && *n != "META-INF/signature.ed25519")
        .cloned()
        .collect();
    let expected_names: Vec<_> = checksums.files.keys().cloned().collect();
    if payload_names != expected_names {
        return Err(PackageError::ChecksumCoverage);
    }
    let signature_verified = match files.get("META-INF/signature.ed25519") {
        Some(raw) => {
            let key_id = manifest
                .publisher_key_id
                .as_ref()
                .ok_or(PackageError::UnknownKey)?;
            let key = trust.0.get(key_id).ok_or(PackageError::UnknownKey)?;
            let signature = Signature::from_slice(raw).map_err(|_| PackageError::Signature)?;
            let mut message = canonical_json(manifest_bytes)?;
            message.push(b'\n');
            message.extend(canonical_json(checksums_bytes)?);
            key.verify(&message, &signature)
                .map_err(|_| PackageError::Signature)?;
            true
        }
        None if require_signature => {
            return Err(PackageError::Missing("META-INF/signature.ed25519"))
        }
        None => false,
    };
    let inspection = PackageInspection {
        plugin_id: manifest.plugin_id,
        version: manifest.version,
        evidence: format!("{:?}", manifest.evidence),
        signature_verified,
        writes_enabled: manifest.writes_enabled,
        capabilities: manifest.capabilities,
        exportable_fields: manifest.exportable_fields,
        depends_on: manifest.depends_on,
        file_count: files.len(),
    };
    Ok((inspection, files))
}

pub fn inspect_package<R: Read + Seek>(
    reader: R,
    trust: &TrustStore,
    require_signature: bool,
) -> Result<PackageInspection, PackageError> {
    extract_package(reader, trust, require_signature).map(|(inspection, _)| inspection)
}

fn validate_path(name: &str) -> Result<(), PackageError> {
    if name.starts_with('/')
        || name.contains('\\')
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(PackageError::UnsafePath(name.into()));
    }
    Ok(())
}

fn allowed(name: &str) -> bool {
    matches!(
        name,
        "plugin.json"
            | "checksums.json"
            | "README.md"
            | "LICENSE"
            | "devices.json"
            | "capabilities.json"
            | "META-INF/signature.ed25519"
    ) || ["protocol/", "locales/", "tests/fixtures/", "models/"]
        .iter()
        .any(|prefix| name.starts_with(prefix) && name.ends_with(".json"))
}

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsafe package path {0}")]
    UnsafePath(String),
    #[error("forbidden package file {0}")]
    Forbidden(String),
    #[error("duplicate package file {0}")]
    Duplicate(String),
    #[error("package exceeded {0} limit")]
    Limit(&'static str),
    #[error("missing package file {0}")]
    Missing(&'static str),
    #[error("missing package file {0}")]
    MissingOwned(String),
    #[error("invalid manifest: {0}")]
    Manifest(String),
    #[error("checksum excludes forbidden path {0}")]
    ChecksumScope(String),
    #[error("checksum mismatch for {0}")]
    Digest(String),
    #[error("checksum coverage does not exactly match payload")]
    ChecksumCoverage,
    #[error("unknown publisher key")]
    UnknownKey,
    #[error("signature verification failed")]
    Signature,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::{Cursor, Write};
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    fn manifest(key_id: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "pluginId": "mira.example",
            "name": "Example",
            "version": "1.0.0",
            "pluginApi": ">=1.0.0, <2.0.0",
            "publisherKeyId": key_id,
            "evidence": "fixture-verified",
            "permissions": [],
            "capabilities": [{
                "id": "mode",
                "control": "Segmented",
                "labelKey": "capability.mode",
                "readOnly": true,
                "metadata": {"section": "control"}
            }],
            "writesEnabled": false
        }))
        .unwrap()
    }

    fn archive(
        extra: Option<(&str, &[u8])>,
        corrupt_digest: bool,
        signed: bool,
    ) -> (Vec<u8>, TrustStore) {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let key_id = signed.then_some("test-only-key");
        let manifest = manifest(key_id);
        let mut payload = BTreeMap::from([("plugin.json".to_string(), manifest.clone())]);
        if let Some((name, bytes)) = extra {
            payload.insert(name.into(), bytes.to_vec());
        }
        let checksums = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "files": payload.iter().map(|(name, bytes)| {
                let digest = if corrupt_digest { "00".repeat(32) } else { hex::encode(Sha256::digest(bytes)) };
                (name.clone(), digest)
            }).collect::<BTreeMap<_, _>>()
        })).unwrap();
        let signature = signed.then(|| {
            let mut message = canonical_json(&manifest).unwrap();
            message.push(b'\n');
            message.extend(canonical_json(&checksums).unwrap());
            signing.sign(&message).to_bytes().to_vec()
        });
        let mut output = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut output);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, bytes) in payload {
                zip.start_file(name, options).unwrap();
                zip.write_all(&bytes).unwrap();
            }
            zip.start_file("checksums.json", options).unwrap();
            zip.write_all(&checksums).unwrap();
            if let Some(bytes) = signature {
                zip.start_file("META-INF/signature.ed25519", options)
                    .unwrap();
                zip.write_all(&bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        let mut trust = TrustStore::default();
        if signed {
            trust
                .0
                .insert("test-only-key".into(), signing.verifying_key());
        }
        (output.into_inner(), trust)
    }

    #[test]
    fn verifies_exact_digest_and_test_signature() {
        let (bytes, trust) = archive(None, false, true);
        let result = inspect_package(Cursor::new(bytes), &trust, true).unwrap();
        assert_eq!(result.plugin_id, "mira.example");
        assert!(result.signature_verified);
        assert_eq!(result.capabilities.len(), 1);
        assert_eq!(result.capabilities[0].control.as_str(), "Segmented");
    }

    #[test]
    fn rejects_forbidden_files_before_use() {
        let (bytes, trust) = archive(Some(("protocol/run.js", b"alert(1)")), false, false);
        assert!(matches!(
            inspect_package(Cursor::new(bytes), &trust, false),
            Err(PackageError::Forbidden(_))
        ));
    }

    /// The `models/` directory is the reserved parent folder for per-model
    /// adapter overrides. JSON payloads under it must be accepted so that
    /// future plugins can ship model-specific files; non-JSON entries remain
    /// forbidden, matching the `protocol/` policy.
    #[test]
    fn allows_models_directory_json_but_rejects_other_extensions() {
        let (allowed_bytes, trust) = archive(Some(("models/placeholder.json", b"{}")), false, true);
        let inspection = inspect_package(Cursor::new(allowed_bytes), &trust, true).unwrap();
        assert_eq!(inspection.plugin_id, "mira.example");

        let (forbidden_bytes, trust) = archive(Some(("models/run.js", b"alert(1)")), false, false);
        assert!(matches!(
            inspect_package(Cursor::new(forbidden_bytes), &trust, false),
            Err(PackageError::Forbidden(_))
        ));
    }

    #[test]
    fn rejects_bad_digest_and_missing_signature() {
        let (bad, trust) = archive(None, true, false);
        assert!(matches!(
            inspect_package(Cursor::new(bad), &trust, false),
            Err(PackageError::Digest(_))
        ));
        let (unsigned, trust) = archive(None, false, false);
        assert!(matches!(
            inspect_package(Cursor::new(unsigned), &trust, true),
            Err(PackageError::Missing("META-INF/signature.ed25519"))
        ));
    }

    /// Build a zip archive from an explicit list of (name, bytes) entries.
    /// Unlike `archive`, this does not add checksums or a signature, making
    /// it suitable for testing low-level path/size validation.
    fn raw_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut output);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, bytes) in entries {
                zip.start_file(name, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        output.into_inner()
    }

    /// Path traversal attempts must be rejected before any file is read.
    #[test]
    fn rejects_path_traversal_entries() {
        for bad_name in &[
            "../escape.json",
            "/absolute.json",
            "back\\slash.json",
            "protocol/..//escape.json",
            "protocol/../escape.json",
            "protocol/./nested.json",
            "//double.json",
        ] {
            let bytes = raw_zip(&[(bad_name, b"{}")]);
            let result = inspect_package(Cursor::new(bytes), &TrustStore::default(), false);
            assert!(
                matches!(result, Err(PackageError::UnsafePath(_))),
                "expected UnsafePath for {bad_name}, got {result:?}"
            );
        }
    }

    /// Archives exceeding the 512-file limit must be rejected.
    #[test]
    fn rejects_too_many_files() {
        // Use directory entries (cheaper than files) to exceed MAX_FILES.
        // `archive.len()` counts all entries including directories.
        let mut output = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut output);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for i in 0..=MAX_FILES {
                zip.add_directory(format!("d{i}/"), options).unwrap();
            }
            zip.finish().unwrap();
        }
        let bytes = output.into_inner();
        let result = inspect_package(Cursor::new(bytes), &TrustStore::default(), false);
        assert!(
            matches!(result, Err(PackageError::Limit("file count"))),
            "expected file count limit, got {result:?}"
        );
    }

    /// A single file exceeding the 4 MB per-file limit must be rejected.
    #[test]
    fn rejects_oversized_single_file() {
        let big = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
        let bytes = raw_zip(&[("protocol/big.json", &big)]);
        let result = inspect_package(Cursor::new(bytes), &TrustStore::default(), false);
        assert!(
            matches!(result, Err(PackageError::Limit("file size"))),
            "expected file size limit, got {result:?}"
        );
    }

    /// An unsigned archive with a valid manifest but wrong signing key
    /// must fail signature verification, not silently accept.
    #[test]
    fn rejects_signature_with_wrong_key() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let wrong_key = SigningKey::from_bytes(&[99; 32]);
        let manifest = manifest(Some("test-only-key"));
        let checksums = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "files": {
                "plugin.json": hex::encode(Sha256::digest(&manifest))
            }
        }))
        .unwrap();
        // Sign with `wrong_key` but register `signing` in the trust store.
        let mut message = canonical_json(&manifest).unwrap();
        message.push(b'\n');
        message.extend(canonical_json(&checksums).unwrap());
        let signature = wrong_key.sign(&message).to_bytes().to_vec();
        let bytes = raw_zip(&[
            ("plugin.json", &manifest),
            ("checksums.json", &checksums),
            ("META-INF/signature.ed25519", &signature),
        ]);
        let mut trust = TrustStore::default();
        trust
            .0
            .insert("test-only-key".into(), signing.verifying_key());
        let result = inspect_package(Cursor::new(bytes), &trust, true);
        assert!(
            matches!(result, Err(PackageError::Signature)),
            "expected Signature error, got {result:?}"
        );
    }

    /// `canonical_json` must produce stable output regardless of key order
    /// in the input, so that signature verification is deterministic.
    #[test]
    fn canonical_json_is_key_order_independent() {
        let a = canonical_json(br#"{"b": 2, "a": 1}"#).unwrap();
        let b = canonical_json(br#"{"a": 1, "b": 2}"#).unwrap();
        assert_eq!(a, b);
    }
}
