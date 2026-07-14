// SPDX-License-Identifier: AGPL-3.0-or-later
//! 签名、原子、版本化的本地 AI bundle 更新。
//!
//! Bundle = mira-runtime 二进制 + model.rillpack,单一版本号,单一 staging/current/
//! previous 目录,单一回滚。首次安装随 Mira 主程序打包(sidecar + resources),独立更新
//! 仅在用户主动检查时触发。下载复用主程序的 `fetch_bounded` 镜像 fallback。

use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ed25519_dalek::{Signature, Verifier};
use rill_runtime_protocol::{
    ReleaseArtifact, ReleaseArtifactKind, ReleaseIndexPayload, SignedReleaseIndex,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{
    decode_key, fetch_bounded,
    local_ai_runtime::{
        resolve_installation, rill_trust_keys, runtime_executable_name, RuntimeInstallation,
    },
    RILL_PRODUCTION_KEY_ID, RILL_PRODUCTION_PUBLIC_KEY_HEX,
};

const RELEASE_INDEX_URL: &str =
    "https://github.com/hello-yunshu/mira-mouse/releases/latest/download/local-ai-stable-index.json";
const TRUSTED_RELEASE_PREFIX: &str =
    "https://github.com/hello-yunshu/mira-mouse/releases/download/";
const MAX_INDEX_BYTES: u64 = 1024 * 1024;
/// bundle 上限:runtime 64 MiB + model 4 MiB + manifest 余量。
const MAX_BUNDLE_BYTES: u64 = 80 * 1024 * 1024;
const BUNDLE_ARTIFACT_ID: &str = "local-ai-bundle";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const INSTALL_METADATA_SCHEMA: u32 = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAiStatus {
    pub ready: bool,
    /// bundle 版本号(从 install.json 读取)。
    pub bundle_version: Option<String>,
    /// 子进程握手后上报的 runtime 版本(仅显示用,可能随 sidecar 内置版本变化)。
    pub runtime_version: Option<String>,
    pub model_pack_id: Option<String>,
    pub model_pack_version: Option<String>,
    pub rollback_available: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalAiUpdateInfo {
    pub component: String,
    pub current_version: Option<String>,
    pub available_version: String,
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAiInstallResult {
    pub component: String,
    pub version: String,
    pub previous_version: Option<String>,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallMetadata {
    schema_version: u32,
    bundle_version: String,
    runtime_version: String,
    model_pack_id: String,
    model_pack_version: String,
    runtime_sha256: String,
    model_sha256: String,
    publisher_key_id: String,
    installed_at: String,
}

pub fn status(app: &AppHandle) -> LocalAiStatus {
    let root = match local_ai_root(app) {
        Ok(root) => root,
        Err(error) => return status_error(None, error),
    };
    let metadata = read_metadata(&bundle_current_dir(&root).join("install.json"));
    let rollback_available = bundle_previous_dir(&root).is_dir();
    let Some(installation) = resolve_installation(app) else {
        return LocalAiStatus {
            ready: false,
            bundle_version: metadata.as_ref().map(|item| item.bundle_version.clone()),
            runtime_version: metadata.as_ref().map(|item| item.runtime_version.clone()),
            model_pack_id: metadata.as_ref().map(|item| item.model_pack_id.clone()),
            model_pack_version: metadata
                .as_ref()
                .map(|item| item.model_pack_version.clone()),
            rollback_available,
            error: Some("runtimeOrModelNotInstalled".into()),
        };
    };
    match probe_runtime(&installation) {
        Ok(probe) => {
            // The built-in sidecar/model pair has no install.json. Its signed model
            // pack version is the bundle version by construction, so use that as the
            // current version instead of reporting the bundled runtime as uninstalled.
            let bundle_version = metadata
                .as_ref()
                .map(|item| item.bundle_version.clone())
                .or_else(|| Some(probe.model_pack_version.clone()));
            LocalAiStatus {
                ready: true,
                bundle_version,
                runtime_version: Some(probe.runtime_version),
                model_pack_id: Some(probe.model_pack_id),
                model_pack_version: Some(probe.model_pack_version),
                rollback_available,
                error: None,
            }
        }
        Err(error) => LocalAiStatus {
            ready: false,
            bundle_version: metadata.as_ref().map(|item| item.bundle_version.clone()),
            runtime_version: metadata.as_ref().map(|item| item.runtime_version.clone()),
            model_pack_id: metadata.as_ref().map(|item| item.model_pack_id.clone()),
            model_pack_version: metadata
                .as_ref()
                .map(|item| item.model_pack_version.clone()),
            rollback_available,
            error: Some(error),
        },
    }
}

pub fn check_updates(app: &AppHandle) -> Result<Vec<LocalAiUpdateInfo>, String> {
    let index = fetch_and_verify_index()?;
    let current = status(app);
    let artifact = select_bundle_artifact(&index.payload)?;
    Ok(vec![update_info(current.bundle_version, artifact)?])
}

pub fn install_update(app: &AppHandle, _component: &str) -> Result<LocalAiInstallResult, String> {
    // component 参数保留以兼容前端调用签名,实际只支持 bundle。
    let index = fetch_and_verify_index()?;
    let artifact = select_bundle_artifact(&index.payload)?.clone();
    let current = status(app);
    let previous_version = current.bundle_version.clone();
    if let Some(previous) = &previous_version {
        let previous = semver::Version::parse(previous)
            .map_err(|error| format!("invalid installed local AI version: {error}"))?;
        let next = semver::Version::parse(&artifact.version)
            .map_err(|error| format!("invalid local AI release version: {error}"))?;
        if next <= previous {
            return Err("bundle is already up to date".to_string());
        }
    }
    let bytes = download_artifact(&artifact)?;
    let root = local_ai_root(app)?;
    fs::create_dir_all(&root).map_err(|error| format!("create local AI directory: {error}"))?;
    install_bundle(app, &root, &artifact, &bytes)?;
    let next_status = status(app);
    if !next_status.ready {
        let _ = rollback_bundle(&root);
        return Err(format!(
            "local AI activation failed and was rolled back: {}",
            next_status.error.unwrap_or_else(|| "unknown error".into())
        ));
    }
    Ok(LocalAiInstallResult {
        component: "bundle".into(),
        version: artifact.version,
        previous_version,
        ready: next_status.ready,
    })
}

pub fn rollback(app: &AppHandle, _component: &str) -> Result<LocalAiStatus, String> {
    let root = local_ai_root(app)?;
    rollback_bundle(&root)?;
    let next_status = status(app);
    if !next_status.ready {
        let _ = rollback_bundle(&root);
        return Err(format!(
            "rolled-back local AI version failed validation: {}",
            next_status.error.unwrap_or_else(|| "unknown error".into())
        ));
    }
    Ok(next_status)
}

fn status_error(bundle_version: Option<String>, error: String) -> LocalAiStatus {
    LocalAiStatus {
        ready: false,
        bundle_version,
        runtime_version: None,
        model_pack_id: None,
        model_pack_version: None,
        rollback_available: false,
        error: Some(error),
    }
}

fn update_info(
    current_version: Option<String>,
    artifact: &ReleaseArtifact,
) -> Result<LocalAiUpdateInfo, String> {
    let available = semver::Version::parse(&artifact.version)
        .map_err(|error| format!("invalid available local AI version: {error}"))?;
    let update_available = current_version
        .as_deref()
        .map(semver::Version::parse)
        .transpose()
        .map_err(|error| format!("invalid installed local AI version: {error}"))?
        .is_none_or(|current| available > current);
    Ok(LocalAiUpdateInfo {
        component: "bundle".into(),
        current_version,
        available_version: artifact.version.clone(),
        update_available,
    })
}

fn fetch_and_verify_index() -> Result<SignedReleaseIndex, String> {
    let bytes = fetch_bounded(RELEASE_INDEX_URL, MAX_INDEX_BYTES)?;
    let index: SignedReleaseIndex = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse local AI release index: {error}"))?;
    verify_index(&index)?;
    Ok(index)
}

/// 自定义 payload 校验。
///
/// 不复用 `ReleaseIndexPayload::validate_shape()`:协议库 0.5.1 对 Runtime 工件
/// 强制 `id == "rill-runtime"` 且 `size <= 64 MiB`,而 bundle 工件用 `id="local-ai-bundle"`
/// 标识、上限放宽到 [`MAX_BUNDLE_BYTES`]。其余字段约束与协议一致。
fn validate_index_payload(payload: &ReleaseIndexPayload) -> Result<(), String> {
    use rill_runtime_protocol::{RELEASE_INDEX_SCHEMA_VERSION, RUNTIME_API_VERSION};
    if payload.schema_version != RELEASE_INDEX_SCHEMA_VERSION {
        return Err("unsupported release-index schema".into());
    }
    if payload.channel != "stable" {
        return Err("unsupported release channel".into());
    }
    if payload.generated_at.is_empty() || payload.generated_at.len() > 64 {
        return Err("invalid release-index timestamp".into());
    }
    if payload.publisher_key_id.is_empty() || payload.publisher_key_id.len() > 96 {
        return Err("invalid release-index publisher".into());
    }
    if payload.artifacts.is_empty() || payload.artifacts.len() > 64 {
        return Err("invalid release-index artifact count".into());
    }
    for artifact in &payload.artifacts {
        if artifact.id.is_empty() || artifact.id.len() > 96 {
            return Err("invalid artifact id".into());
        }
        if artifact.version.is_empty() || artifact.version.len() > 48 {
            return Err("invalid artifact version".into());
        }
        if artifact.runtime_api_version != RUNTIME_API_VERSION {
            return Err("unsupported artifact runtime API version".into());
        }
        if artifact.url.is_empty() || artifact.url.len() > 2048 {
            return Err("invalid artifact URL".into());
        }
        if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid artifact SHA-256".into());
        }
        if artifact.size == 0 || artifact.size > MAX_BUNDLE_BYTES {
            return Err("invalid artifact size".into());
        }
        // bundle 工件(kind=Runtime + id=BUNDLE_ARTIFACT_ID)必须有平台信息。
        // 普通模型工件(kind=Model)必须平台无关。
        match artifact.kind {
            ReleaseArtifactKind::Runtime => {
                if artifact.target_os.as_deref().is_none_or(str::is_empty)
                    || artifact.target_arch.as_deref().is_none_or(str::is_empty)
                {
                    return Err("bundle artifact requires a target OS and architecture".into());
                }
            }
            ReleaseArtifactKind::Model => {
                if artifact.target_os.is_some() || artifact.target_arch.is_some() {
                    return Err("model artifact must be platform independent".into());
                }
            }
        }
    }
    Ok(())
}

fn verify_index(index: &SignedReleaseIndex) -> Result<(), String> {
    validate_index_payload(&index.payload)?;
    if index.payload.publisher_key_id != RILL_PRODUCTION_KEY_ID {
        return Err("local AI release index uses an untrusted publisher".into());
    }
    chrono::DateTime::parse_from_rfc3339(&index.payload.generated_at)
        .map_err(|error| format!("invalid local AI release timestamp: {error}"))?;
    let signature_bytes = hex::decode(&index.signature)
        .map_err(|_| "invalid local AI release signature encoding".to_string())?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| "invalid local AI release signature length".to_string())?;
    let canonical = canonical_json(&index.payload)?;
    decode_key(RILL_PRODUCTION_PUBLIC_KEY_HEX)
        .verify(&canonical, &signature)
        .map_err(|_| "local AI release-index signature verification failed".to_string())?;

    let mut identities = std::collections::BTreeSet::new();
    for artifact in &index.payload.artifacts {
        semver::Version::parse(&artifact.version)
            .map_err(|error| format!("invalid local AI artifact version: {error}"))?;
        if !artifact.url.starts_with(TRUSTED_RELEASE_PREFIX) {
            return Err("local AI artifact URL is outside the trusted release origin".into());
        }
        let identity = (
            artifact.kind.clone(),
            artifact.id.clone(),
            artifact.target_os.clone(),
            artifact.target_arch.clone(),
        );
        if !identities.insert(identity) {
            return Err("local AI release index has duplicate artifacts".into());
        }
    }
    Ok(())
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    fn sort(value: Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, sort(value)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.into_iter().map(sort).collect()),
            other => other,
        }
    }
    let value = serde_json::to_value(value)
        .map_err(|error| format!("encode local AI release index: {error}"))?;
    serde_json::to_vec(&sort(value))
        .map_err(|error| format!("canonicalize local AI release index: {error}"))
}

/// 选择当前平台的 bundle artifact。
/// bundle 工件用 `ReleaseArtifactKind::Runtime` + id="local-ai-bundle" + 平台匹配标识
/// (协议库 0.5.1 没有 Bundle 变体,复用 Runtime kind)。
fn select_bundle_artifact(payload: &ReleaseIndexPayload) -> Result<&ReleaseArtifact, String> {
    let bundle_matches = payload
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind == ReleaseArtifactKind::Runtime
                && artifact.id == BUNDLE_ARTIFACT_ID
                && artifact.target_os.as_deref() == Some(std::env::consts::OS)
                && artifact.target_arch.as_deref() == Some(std::env::consts::ARCH)
        })
        .collect::<Vec<_>>();
    if let [artifact] = bundle_matches.as_slice() {
        return Ok(*artifact);
    }
    if bundle_matches.len() > 1 {
        return Err("multiple compatible bundle artifacts are published".into());
    }
    Err(format!(
        "no compatible bundle artifact is published for {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

fn download_artifact(artifact: &ReleaseArtifact) -> Result<Vec<u8>, String> {
    if artifact.size > MAX_BUNDLE_BYTES {
        return Err("local AI bundle exceeds the size limit".into());
    }
    let bytes = fetch_bounded(&artifact.url, MAX_BUNDLE_BYTES)?;
    if bytes.len() as u64 != artifact.size {
        return Err(format!(
            "local AI bundle size mismatch: expected {}, got {}",
            artifact.size,
            bytes.len()
        ));
    }
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != artifact.sha256 {
        return Err(format!(
            "local AI bundle SHA-256 mismatch: expected {}, got {actual}",
            artifact.sha256
        ));
    }
    Ok(bytes)
}

/// Bundle zip 内的清单文件:列出每个平台的 runtime 文件名 + sha256,以及模型包 sha256 +
/// 模型 pack_id/version。运行时按平台选择对应 runtime 文件。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundleManifest {
    schema_version: u32,
    bundle_version: String,
    runtime_version: String,
    model_pack_id: String,
    model_pack_version: String,
    /// 每个平台的 runtime 文件名与 sha256。
    runtimes: Vec<BundleRuntimeEntry>,
    model_sha256: String,
    model_filename: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundleRuntimeEntry {
    target_os: String,
    target_arch: String,
    filename: String,
    sha256: String,
}

fn install_bundle(
    _app: &AppHandle,
    root: &Path,
    artifact: &ReleaseArtifact,
    bytes: &[u8],
) -> Result<(), String> {
    let parent = root.join("bundle");
    fs::create_dir_all(&parent).map_err(|error| format!("create bundle directory: {error}"))?;
    let staging = create_staging_dir(&parent)?;
    let result = (|| {
        let manifest = extract_bundle(bytes, &staging)?;
        if manifest.bundle_version != artifact.version {
            return Err("bundle manifest version does not match the signed release index".into());
        }
        let runtime_entry = manifest
            .runtimes
            .iter()
            .find(|entry| {
                entry.target_os == std::env::consts::OS
                    && entry.target_arch == std::env::consts::ARCH
            })
            .ok_or_else(|| {
                format!(
                    "bundle has no runtime for {}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
            })?;
        let runtime_path = staging.join(runtime_executable_name());
        // runtime_entry.filename 可能与 runtime_executable_name() 不同(如带 -linux 后缀),
        // 解压后重命名为统一名称供 resolve_installation 找到。
        let extracted_runtime = staging.join(&runtime_entry.filename);
        if extracted_runtime != runtime_path {
            fs::rename(&extracted_runtime, &runtime_path)
                .map_err(|error| format!("rename bundled runtime: {error}"))?;
        }
        set_executable_permissions(&runtime_path)?;
        verify_platform_signature(&runtime_path)?;

        let model_path = staging.join("model.rillpack");
        let extracted_model = staging.join(&manifest.model_filename);
        if extracted_model != model_path {
            fs::rename(&extracted_model, &model_path)
                .map_err(|error| format!("rename bundled model: {error}"))?;
        }

        // 启动自检:握手验证 runtime + model。
        let probe = probe_runtime(&RuntimeInstallation {
            executable: runtime_path.clone(),
            model_pack: model_path.clone(),
            trust_keys: rill_trust_keys(),
        })?;
        if probe.model_pack_id != manifest.model_pack_id
            || probe.model_pack_version != manifest.model_pack_version
            || probe.runtime_version != manifest.runtime_version
        {
            return Err("bundled runtime or model identity does not match the manifest".into());
        }

        write_metadata(
            &staging.join("install.json"),
            &manifest,
            &runtime_entry.sha256,
        )?;
        activate_directory(
            &staging,
            &bundle_current_dir(root),
            &bundle_previous_dir(root),
        )
    })();
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    // bundle 更新后 controller 的 restart 在 lib.rs 的 install_update 命令层调用,
    // 避免在此处直接访问 controller 造成循环依赖。
    result
}

fn extract_bundle(bytes: &[u8], staging: &Path) -> Result<BundleManifest, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|error| format!("open local AI bundle archive: {error}"))?;
    let manifest_bytes = archive
        .by_name("manifest.json")
        .map_err(|error| format!("read bundle manifest: {error}"))?;
    let manifest: BundleManifest = serde_json::from_reader(manifest_bytes)
        .map_err(|error| format!("parse bundle manifest: {error}"))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported bundle manifest schema version: {}",
            manifest.schema_version
        ));
    }
    validate_bundle_manifest(&manifest)?;
    let expected_entries = std::iter::once("manifest.json".to_string())
        .chain(std::iter::once(manifest.model_filename.clone()))
        .chain(
            manifest
                .runtimes
                .iter()
                .map(|runtime| runtime.filename.clone()),
        )
        .collect::<BTreeSet<_>>();
    let mut seen_entries = BTreeSet::new();
    let mut total_uncompressed = 0_u64;

    // Bundle is a signed, fixed-shape archive. Reject traversal, duplicates,
    // directories and undeclared payloads before anything can escape staging.
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("read bundle entry {index}: {error}"))?;
        let name = entry.name().to_string();
        if entry.is_dir() || !safe_bundle_filename(&name) {
            return Err(format!("unsafe bundle entry path: {name}"));
        }
        if !expected_entries.contains(&name) {
            return Err(format!("undeclared bundle entry: {name}"));
        }
        if !seen_entries.insert(name.clone()) {
            return Err(format!("duplicate bundle entry: {name}"));
        }
        total_uncompressed = total_uncompressed
            .checked_add(entry.size())
            .ok_or_else(|| "bundle uncompressed size overflow".to_string())?;
        if total_uncompressed > MAX_BUNDLE_BYTES {
            return Err("local AI bundle uncompressed payload exceeds the size limit".into());
        }
        let dest = staging.join(&name);
        let mut file = fs::File::create(&dest)
            .map_err(|error| format!("create bundle file {}: {error}", dest.display()))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf)
            .map_err(|error| format!("read bundle entry {name}: {error}"))?;
        // 校验 runtime 条目 sha256。
        if let Some(runtime_entry) = manifest.runtimes.iter().find(|r| r.filename == name) {
            let actual = hex::encode(Sha256::digest(&buf));
            if actual != runtime_entry.sha256 {
                return Err(format!(
                    "bundle runtime {name} SHA-256 mismatch: expected {}, got {actual}",
                    runtime_entry.sha256
                ));
            }
        }
        // 校验 model 条目 sha256。
        if name == manifest.model_filename {
            let actual = hex::encode(Sha256::digest(&buf));
            if actual != manifest.model_sha256 {
                return Err(format!(
                    "bundle model {name} SHA-256 mismatch: expected {}, got {actual}",
                    manifest.model_sha256
                ));
            }
        }
        file.write_all(&buf)
            .map_err(|error| format!("write bundle file {}: {error}", dest.display()))?;
    }
    if seen_entries != expected_entries {
        let missing = expected_entries
            .difference(&seen_entries)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("bundle is missing declared entries: {missing}"));
    }
    Ok(manifest)
}

fn safe_bundle_filename(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_bundle_manifest(manifest: &BundleManifest) -> Result<(), String> {
    semver::Version::parse(&manifest.bundle_version)
        .map_err(|error| format!("invalid bundle manifest version: {error}"))?;
    semver::Version::parse(&manifest.runtime_version)
        .map_err(|error| format!("invalid bundled runtime version: {error}"))?;
    semver::Version::parse(&manifest.model_pack_version)
        .map_err(|error| format!("invalid bundled model version: {error}"))?;
    if manifest.model_pack_id.is_empty()
        || !safe_bundle_filename(&manifest.model_filename)
        || !valid_sha256(&manifest.model_sha256)
        || manifest.runtimes.is_empty()
        || manifest.runtimes.len() > 8
    {
        return Err("invalid bundle manifest fields".into());
    }
    let mut platforms = BTreeSet::new();
    let mut filenames =
        BTreeSet::from(["manifest.json".to_string(), manifest.model_filename.clone()]);
    if filenames.len() != 2 {
        return Err("bundle manifest filenames overlap".into());
    }
    for runtime in &manifest.runtimes {
        if runtime.target_os.is_empty()
            || runtime.target_arch.is_empty()
            || !safe_bundle_filename(&runtime.filename)
            || !valid_sha256(&runtime.sha256)
            || !platforms.insert((&runtime.target_os, &runtime.target_arch))
            || !filenames.insert(runtime.filename.clone())
        {
            return Err("invalid or duplicate bundled runtime entry".into());
        }
    }
    Ok(())
}

fn probe_runtime(
    installation: &RuntimeInstallation,
) -> Result<crate::local_ai_runtime::RuntimeProbe, String> {
    use rill_runtime_protocol::{RuntimeRequest, RUNTIME_API_VERSION};
    // 直接调用 sidecar 一次握手。
    let request = RuntimeRequest::Handshake {
        request_id: "mira-handshake".into(),
        api_version: RUNTIME_API_VERSION,
        client_name: "mira".into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
    };
    let line =
        serde_json::to_vec(&request).map_err(|error| format!("encode handshake: {error}"))?;
    let mut command = Command::new(&installation.executable);
    command
        .arg("serve")
        .arg("--pack")
        .arg(&installation.model_pack);
    for key in &installation.trust_keys {
        command.arg("--trust-key").arg(key);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start local AI runtime: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout unavailable".to_string())?;
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = stderr.take(64 * 1024).read_to_end(&mut sink);
        });
    }
    if let Err(error) = stdin
        .write_all(&line)
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("write local AI handshake: {error}"));
    }

    let (response_tx, response_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout)
            .take((rill_runtime_protocol::MAX_MESSAGE_BYTES as u64).saturating_add(2));
        let mut buf = Vec::new();
        let result = reader
            .read_until(b'\n', &mut buf)
            .map_err(|error| format!("read handshake: {error}"))
            .and_then(|read| {
                if read == 0 {
                    return Err("local AI runtime closed stdout during handshake".into());
                }
                if buf.len() > rill_runtime_protocol::MAX_MESSAGE_BYTES + 1 {
                    return Err("local AI handshake exceeds the message limit".into());
                }
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                Ok(buf)
            });
        let _ = response_tx.send(result);
    });
    let buf = match response_rx.recv_timeout(COMMAND_TIMEOUT) {
        Ok(result) => result?,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("local AI handshake timed out".into());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("local AI handshake reader stopped unexpectedly".into());
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    let response: rill_runtime_protocol::RuntimeResponse = serde_json::from_slice(&buf)
        .map_err(|error| format!("decode handshake response: {error}"))?;
    crate::local_ai_runtime::validate_handshake_response(&response)
}

#[cfg(target_os = "macos")]
fn verify_platform_signature(executable: &Path) -> Result<(), String> {
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--verbose=0"])
        .arg(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map_err(|error| format!("verify runtime code signature: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "downloaded local AI runtime has no valid macOS code signature".into())
}

#[cfg(not(target_os = "macos"))]
fn verify_platform_signature(_executable: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_executable_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("inspect runtime permissions: {error}"))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("set runtime executable permissions: {error}"))
}

#[cfg(not(unix))]
fn set_executable_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn write_metadata(
    path: &Path,
    manifest: &BundleManifest,
    runtime_sha256: &str,
) -> Result<(), String> {
    let metadata = InstallMetadata {
        schema_version: INSTALL_METADATA_SCHEMA,
        bundle_version: manifest.bundle_version.clone(),
        runtime_version: manifest.runtime_version.clone(),
        model_pack_id: manifest.model_pack_id.clone(),
        model_pack_version: manifest.model_pack_version.clone(),
        runtime_sha256: runtime_sha256.to_string(),
        model_sha256: manifest.model_sha256.clone(),
        publisher_key_id: RILL_PRODUCTION_KEY_ID.into(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("encode local AI install metadata: {error}"))?;
    write_synced(path, &bytes)
}

fn read_metadata(path: &Path) -> Option<InstallMetadata> {
    let metadata: InstallMetadata = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (metadata.schema_version == INSTALL_METADATA_SCHEMA
        && metadata.publisher_key_id == RILL_PRODUCTION_KEY_ID)
        .then_some(metadata)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::File::create(path)
        .map_err(|error| format!("create local AI artifact {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write local AI artifact {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync local AI artifact {}: {error}", path.display()))
}

fn create_staging_dir(parent: &Path) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = parent.join(format!(".staging-{}-{nonce}", std::process::id()));
    fs::create_dir(&path).map_err(|error| format!("create local AI staging directory: {error}"))?;
    Ok(path)
}

fn activate_directory(staging: &Path, current: &Path, rollback: &Path) -> Result<(), String> {
    if rollback.exists() {
        fs::remove_dir_all(rollback)
            .map_err(|error| format!("remove stale local AI rollback: {error}"))?;
    }
    let had_current = current.exists();
    if had_current {
        fs::rename(current, rollback)
            .map_err(|error| format!("prepare local AI rollback: {error}"))?;
    }
    if let Err(error) = fs::rename(staging, current) {
        if had_current {
            let _ = fs::rename(rollback, current);
        }
        return Err(format!("activate local AI update atomically: {error}"));
    }
    Ok(())
}

fn rollback_bundle(root: &Path) -> Result<(), String> {
    let current = bundle_current_dir(root);
    let rollback = bundle_previous_dir(root);
    if !rollback.is_dir() {
        return Err("no bundle rollback is available".into());
    }
    let swap = current.with_file_name(format!(".rollback-swap-{}", std::process::id()));
    if swap.exists() {
        fs::remove_dir_all(&swap)
            .map_err(|error| format!("remove stale rollback swap: {error}"))?;
    }
    if current.exists() {
        fs::rename(&current, &swap)
            .map_err(|error| format!("prepare current local AI version: {error}"))?;
    }
    if let Err(error) = fs::rename(&rollback, &current) {
        if swap.exists() {
            let _ = fs::rename(&swap, &current);
        }
        return Err(format!("activate local AI rollback: {error}"));
    }
    if swap.exists() {
        fs::rename(&swap, &rollback)
            .map_err(|error| format!("preserve replaced local AI version: {error}"))?;
    }
    Ok(())
}

fn local_ai_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join("local-ai"))
        .map_err(|error| format!("resolve local AI data directory: {error}"))
}

fn bundle_current_dir(root: &Path) -> PathBuf {
    root.join("bundle").join("current")
}

fn bundle_previous_dir(root: &Path) -> PathBuf {
    root.join("bundle").join("previous")
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use rill_runtime_protocol::{
        ReleaseArtifact, ReleaseArtifactKind, RELEASE_INDEX_SCHEMA_VERSION, RUNTIME_API_VERSION,
    };

    use super::*;

    fn bundle_artifact(version: &str) -> ReleaseArtifact {
        ReleaseArtifact {
            kind: ReleaseArtifactKind::Runtime,
            id: BUNDLE_ARTIFACT_ID.into(),
            version: version.into(),
            runtime_api_version: RUNTIME_API_VERSION,
            target_os: Some(std::env::consts::OS.into()),
            target_arch: Some(std::env::consts::ARCH.into()),
            url: format!(
                "{TRUSTED_RELEASE_PREFIX}local-ai-v{version}/bundle-{}-{}.zip",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            sha256: "ab".repeat(32),
            size: 1024,
        }
    }

    #[test]
    fn update_info_never_downgrades() {
        let artifact = bundle_artifact("0.5.0");
        let info = update_info(Some("0.6.0".into()), &artifact).unwrap();
        assert!(!info.update_available);
    }

    #[test]
    fn artifact_selection_is_platform_exact() {
        let payload = ReleaseIndexPayload {
            schema_version: RELEASE_INDEX_SCHEMA_VERSION,
            channel: "stable".into(),
            generated_at: "2026-07-13T00:00:00Z".into(),
            publisher_key_id: RILL_PRODUCTION_KEY_ID.into(),
            artifacts: vec![bundle_artifact("0.5.0")],
        };
        assert_eq!(select_bundle_artifact(&payload).unwrap().version, "0.5.0");
    }

    #[test]
    fn release_signature_changes_when_payload_changes() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let payload = ReleaseIndexPayload {
            schema_version: RELEASE_INDEX_SCHEMA_VERSION,
            channel: "stable".into(),
            generated_at: "2026-07-13T00:00:00Z".into(),
            publisher_key_id: "test".into(),
            artifacts: vec![bundle_artifact("0.5.0")],
        };
        let canonical = canonical_json(&payload).unwrap();
        let signature = signing.sign(&canonical);
        signing
            .verifying_key()
            .verify(&canonical, &signature)
            .unwrap();
        let mut changed = payload;
        changed.artifacts[0].sha256 = "cd".repeat(32);
        assert!(signing
            .verifying_key()
            .verify(&canonical_json(&changed).unwrap(), &signature)
            .is_err());
    }

    #[test]
    fn activation_keeps_one_rollback() {
        let temporary = tempfile::tempdir().unwrap();
        let current = temporary.path().join("current");
        let rollback = temporary.path().join("previous");
        let staging = temporary.path().join("staging");
        fs::create_dir(&current).unwrap();
        fs::write(current.join("version"), "old").unwrap();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("version"), "new").unwrap();
        activate_directory(&staging, &current, &rollback).unwrap();
        assert_eq!(fs::read_to_string(current.join("version")).unwrap(), "new");
        assert_eq!(fs::read_to_string(rollback.join("version")).unwrap(), "old");
    }

    #[test]
    fn release_contract_uses_the_mira_repository_and_dedicated_rill_key() {
        assert_eq!(
            RELEASE_INDEX_URL,
            "https://github.com/hello-yunshu/mira-mouse/releases/latest/download/local-ai-stable-index.json"
        );
        assert!(TRUSTED_RELEASE_PREFIX.contains("/hello-yunshu/mira-mouse/"));
        assert_eq!(RILL_PRODUCTION_KEY_ID, "mira-rill-2026-001");
        assert_ne!(RILL_PRODUCTION_KEY_ID, crate::PRODUCTION_KEY_ID);
    }

    #[test]
    fn bundle_entry_paths_are_single_safe_filenames() {
        assert!(safe_bundle_filename("mira-runtime"));
        assert!(safe_bundle_filename("model.rillpack"));
        assert!(!safe_bundle_filename("../mira-runtime"));
        assert!(!safe_bundle_filename("nested/mira-runtime"));
        assert!(!safe_bundle_filename("/tmp/mira-runtime"));
    }
}
