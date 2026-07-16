// SPDX-License-Identifier: AGPL-3.0-or-later
//! Signed, independently versioned local-AI artifact updates.
//!
//! Rill runtime, Mira model and Mira handler are published as three artifacts.
//! They update independently, but activation is transactional: a candidate
//! deployment is assembled in staging, fully handshaken, then atomically
//! swapped with `current`. One complete previous deployment remains rollbackable.

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
    ReleaseArtifact, ReleaseArtifactKind, ReleaseIndexPayload, RuntimeResponseV2,
    SignedReleaseIndex, HANDLER_API_VERSION, RUNTIME_ARTIFACT_ID,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{
    decode_key, fetch_bounded,
    local_ai_runtime::{
        resolve_installation, rill_handler_trust_keys, rill_trust_keys, runtime_executable_name,
        RuntimeInstallation, MIRA_HANDLER_ID,
    },
    RILL_V2_PRODUCTION_KEY_ID, RILL_V2_PRODUCTION_PUBLIC_KEY_HEX,
};

const RELEASE_INDEX_URLS: &[&str] = &[
    "https://github.com/hello-yunshu/mira-mouse/releases/download/local-ai-stable/local-ai-stable-index.json",
    // Bootstrap/failover for clients released before the dedicated pointer
    // existed. Only transport failures fall through; a fetched but invalid
    // signed index is always rejected.
    "https://github.com/hello-yunshu/mira-mouse/releases/latest/download/local-ai-stable-index.json",
];
const TRUSTED_RELEASE_PREFIXES: &[&str] = &[
    "https://github.com/hello-yunshu/mira-mouse/releases/download/",
    "https://github.com/hello-yunshu/rill-ml/releases/download/",
];
const MODEL_ARTIFACT_ID: &str = "mira-battery-model";
const MAX_INDEX_BYTES: u64 = 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const INSTALL_METADATA_SCHEMA: u32 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAiStatus {
    pub ready: bool,
    /// Compatibility field used by the current UI. This is the version that
    /// triggered the latest transactional deployment, not a coupled bundle ABI.
    pub bundle_version: Option<String>,
    pub runtime_version: Option<String>,
    pub model_pack_id: Option<String>,
    pub model_pack_version: Option<String>,
    pub handler_id: Option<String>,
    pub handler_version: Option<String>,
    pub handler_api_version: Option<u32>,
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
    deployment_version: String,
    runtime_version: String,
    model_pack_id: String,
    model_pack_version: String,
    handler_id: String,
    handler_version: String,
    handler_api_version: u32,
    runtime_sha256: String,
    model_sha256: String,
    handler_sha256: String,
    publisher_key_id: String,
    installed_at: String,
}

#[derive(Clone, Copy)]
struct SelectedArtifacts<'a> {
    runtime: &'a ReleaseArtifact,
    model: &'a ReleaseArtifact,
    handler: &'a ReleaseArtifact,
}

#[derive(Clone, Copy)]
struct UpdatePlan {
    runtime: bool,
    model: bool,
    handler: bool,
}

impl UpdatePlan {
    fn any(self) -> bool {
        self.runtime || self.model || self.handler
    }
}

pub fn status(app: &AppHandle) -> LocalAiStatus {
    let root = match local_ai_root(app) {
        Ok(root) => root,
        Err(error) => return status_error(None, error),
    };
    let metadata = read_metadata(&deployment_current_dir(&root).join("install.json"));
    let rollback_available = deployment_previous_dir(&root).is_dir();
    let Some(installation) = resolve_installation(app) else {
        return LocalAiStatus {
            ready: false,
            bundle_version: metadata
                .as_ref()
                .map(|item| item.deployment_version.clone()),
            runtime_version: metadata.as_ref().map(|item| item.runtime_version.clone()),
            model_pack_id: metadata.as_ref().map(|item| item.model_pack_id.clone()),
            model_pack_version: metadata
                .as_ref()
                .map(|item| item.model_pack_version.clone()),
            handler_id: metadata.as_ref().map(|item| item.handler_id.clone()),
            handler_version: metadata.as_ref().map(|item| item.handler_version.clone()),
            handler_api_version: metadata.as_ref().map(|item| item.handler_api_version),
            rollback_available,
            error: Some("runtimeModelOrHandlerNotInstalled".into()),
        };
    };
    match probe_runtime(&installation) {
        Ok(probe) => LocalAiStatus {
            ready: true,
            bundle_version: metadata
                .as_ref()
                .map(|item| item.deployment_version.clone())
                .or_else(|| Some(probe.handler_version.clone())),
            runtime_version: Some(probe.runtime_version),
            model_pack_id: Some(probe.model_pack_id),
            model_pack_version: Some(probe.model_pack_version),
            handler_id: Some(probe.handler_id),
            handler_version: Some(probe.handler_version),
            handler_api_version: Some(probe.handler_api_version),
            rollback_available,
            error: None,
        },
        Err(error) => LocalAiStatus {
            ready: false,
            bundle_version: metadata
                .as_ref()
                .map(|item| item.deployment_version.clone()),
            runtime_version: metadata.as_ref().map(|item| item.runtime_version.clone()),
            model_pack_id: metadata.as_ref().map(|item| item.model_pack_id.clone()),
            model_pack_version: metadata
                .as_ref()
                .map(|item| item.model_pack_version.clone()),
            handler_id: metadata.as_ref().map(|item| item.handler_id.clone()),
            handler_version: metadata.as_ref().map(|item| item.handler_version.clone()),
            handler_api_version: metadata.as_ref().map(|item| item.handler_api_version),
            rollback_available,
            error: Some(error),
        },
    }
}

pub fn check_updates(app: &AppHandle) -> Result<Vec<LocalAiUpdateInfo>, String> {
    let index = fetch_and_verify_index()?;
    let artifacts = select_artifacts(&index.payload)?;
    let current = status(app);
    Ok(vec![update_info(&current, artifacts)?])
}

pub fn install_update(app: &AppHandle, component: &str) -> Result<LocalAiInstallResult, String> {
    if component != "bundle" {
        return Err("unsupported local AI update component".into());
    }
    let index = fetch_and_verify_index()?;
    let artifacts = select_artifacts(&index.payload)?;
    let current = status(app);
    let plan = update_plan(&current, artifacts)?;
    if !plan.any() {
        return Err("local AI components are already up to date".into());
    }

    let previous_version = current.bundle_version.clone();
    let source = resolve_installation(app);
    let root = local_ai_root(app)?;
    let parent = root.join("bundle");
    fs::create_dir_all(&parent).map_err(|error| format!("create deployment directory: {error}"))?;
    let staging = create_staging_dir(&parent)?;
    let result = stage_deployment(source.as_ref(), &staging, artifacts, plan)
        .and_then(|()| validate_staged_deployment(&staging, artifacts, plan))
        .and_then(|probe| {
            let deployment_version = selected_deployment_version(artifacts, plan).to_string();
            write_metadata(
                &staging.join("install.json"),
                &deployment_version,
                &staging,
                &probe,
            )?;
            activate_directory(
                &staging,
                &deployment_current_dir(&root),
                &deployment_previous_dir(&root),
            )?;
            Ok(deployment_version)
        });
    let deployment_version = match result {
        Ok(version) => version,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

    let next_status = status(app);
    if !next_status.ready {
        let _ = rollback_deployment(&root);
        return Err(format!(
            "local AI activation failed and was rolled back: {}",
            next_status.error.unwrap_or_else(|| "unknown error".into())
        ));
    }
    Ok(LocalAiInstallResult {
        component: "bundle".into(),
        version: deployment_version,
        previous_version,
        ready: true,
    })
}

pub fn rollback(app: &AppHandle, component: &str) -> Result<LocalAiStatus, String> {
    if component != "bundle" {
        return Err("unsupported local AI rollback component".into());
    }
    let root = local_ai_root(app)?;
    rollback_deployment(&root)?;
    let next_status = status(app);
    if !next_status.ready {
        let _ = rollback_deployment(&root);
        return Err(format!(
            "rolled-back local AI deployment failed validation: {}",
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
        handler_id: None,
        handler_version: None,
        handler_api_version: None,
        rollback_available: false,
        error: Some(error),
    }
}

fn update_plan(
    current: &LocalAiStatus,
    artifacts: SelectedArtifacts<'_>,
) -> Result<UpdatePlan, String> {
    Ok(UpdatePlan {
        runtime: is_newer(
            &artifacts.runtime.version,
            current.runtime_version.as_deref(),
        )?,
        model: is_newer(
            &artifacts.model.version,
            current.model_pack_version.as_deref(),
        )?,
        handler: is_newer(
            &artifacts.handler.version,
            current.handler_version.as_deref(),
        )?,
    })
}

fn update_info(
    current: &LocalAiStatus,
    artifacts: SelectedArtifacts<'_>,
) -> Result<LocalAiUpdateInfo, String> {
    let plan = update_plan(current, artifacts)?;
    Ok(LocalAiUpdateInfo {
        component: "bundle".into(),
        current_version: current.bundle_version.clone(),
        available_version: selected_deployment_version(artifacts, plan).to_string(),
        update_available: plan.any(),
    })
}

fn selected_deployment_version(artifacts: SelectedArtifacts<'_>, plan: UpdatePlan) -> &str {
    if plan.runtime {
        &artifacts.runtime.version
    } else if plan.handler {
        &artifacts.handler.version
    } else {
        &artifacts.model.version
    }
}

fn is_newer(available: &str, current: Option<&str>) -> Result<bool, String> {
    let available = semver::Version::parse(available)
        .map_err(|error| format!("invalid available local AI version: {error}"))?;
    match current {
        None => Ok(true),
        Some(current) => {
            let current = semver::Version::parse(current)
                .map_err(|error| format!("invalid installed local AI version: {error}"))?;
            Ok(available > current)
        }
    }
}

fn fetch_and_verify_index() -> Result<SignedReleaseIndex, String> {
    let mut transport_errors = Vec::new();
    for url in RELEASE_INDEX_URLS {
        let bytes = match fetch_bounded(url, MAX_INDEX_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                transport_errors.push(format!("{url}: {error}"));
                continue;
            }
        };
        let index: SignedReleaseIndex = serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse local AI release index from {url}: {error}"))?;
        verify_index(&index)?;
        return Ok(index);
    }
    Err(format!(
        "fetch local AI release index: {}",
        transport_errors.join("; ")
    ))
}

fn verify_index(index: &SignedReleaseIndex) -> Result<(), String> {
    index
        .payload
        .validate_shape()
        .map_err(|message| format!("invalid local AI release index: {message}"))?;
    if index.payload.publisher_key_id != RILL_V2_PRODUCTION_KEY_ID {
        return Err("local AI release index uses an untrusted publisher".into());
    }
    chrono::DateTime::parse_from_rfc3339(&index.payload.generated_at)
        .map_err(|error| format!("invalid local AI release timestamp: {error}"))?;
    let signature_bytes = hex::decode(&index.signature)
        .map_err(|_| "invalid local AI release signature encoding".to_string())?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| "invalid local AI release signature length".to_string())?;
    decode_key(RILL_V2_PRODUCTION_PUBLIC_KEY_HEX)
        .verify(&canonical_json(&index.payload)?, &signature)
        .map_err(|_| "local AI release-index signature verification failed".to_string())?;

    let mut identities = BTreeSet::new();
    for artifact in &index.payload.artifacts {
        semver::Version::parse(&artifact.version)
            .map_err(|error| format!("invalid local AI artifact version: {error}"))?;
        if !TRUSTED_RELEASE_PREFIXES
            .iter()
            .any(|prefix| artifact.url.starts_with(prefix))
        {
            return Err("local AI artifact URL is outside the trusted release origins".into());
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
    let selected = select_artifacts(&index.payload)?;
    validate_handler_runtime_compatibility(selected.handler, &selected.runtime.version)
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

fn select_artifacts(payload: &ReleaseIndexPayload) -> Result<SelectedArtifacts<'_>, String> {
    let runtime = select_unique(
        payload,
        |artifact| {
            artifact.kind == ReleaseArtifactKind::Runtime
                && artifact.id == RUNTIME_ARTIFACT_ID
                && artifact.target_os.as_deref() == Some(std::env::consts::OS)
                && artifact.target_arch.as_deref() == Some(std::env::consts::ARCH)
        },
        "runtime",
    )?;
    let model = select_unique(
        payload,
        |artifact| artifact.kind == ReleaseArtifactKind::Model && artifact.id == MODEL_ARTIFACT_ID,
        "model",
    )?;
    let handler = select_unique(
        payload,
        |artifact| artifact.kind == ReleaseArtifactKind::Handler && artifact.id == MIRA_HANDLER_ID,
        "handler",
    )?;
    Ok(SelectedArtifacts {
        runtime,
        model,
        handler,
    })
}

fn select_unique<'a>(
    payload: &'a ReleaseIndexPayload,
    predicate: impl Fn(&ReleaseArtifact) -> bool,
    label: &str,
) -> Result<&'a ReleaseArtifact, String> {
    let matches = payload
        .artifacts
        .iter()
        .filter(|artifact| predicate(artifact))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [artifact] => Ok(*artifact),
        [] => Err(format!("no compatible {label} artifact is published")),
        _ => Err(format!(
            "multiple compatible {label} artifacts are published"
        )),
    }
}

fn validate_handler_runtime_compatibility(
    handler: &ReleaseArtifact,
    runtime_version: &str,
) -> Result<(), String> {
    if handler.handler_api_version != Some(HANDLER_API_VERSION) {
        return Err("handler artifact uses an unsupported handler API".into());
    }
    let minimum = handler
        .min_runtime_version
        .as_deref()
        .ok_or_else(|| "handler artifact is missing its minimum runtime version".to_string())?;
    let minimum = semver::Version::parse(minimum)
        .map_err(|error| format!("invalid handler minimum runtime version: {error}"))?;
    let runtime = semver::Version::parse(runtime_version)
        .map_err(|error| format!("invalid runtime artifact version: {error}"))?;
    if runtime < minimum {
        return Err("published handler requires a newer Rill runtime".into());
    }
    Ok(())
}

fn stage_deployment(
    source: Option<&RuntimeInstallation>,
    staging: &Path,
    artifacts: SelectedArtifacts<'_>,
    plan: UpdatePlan,
) -> Result<(), String> {
    stage_component(
        source.map(|item| item.executable.as_path()),
        &staging.join(runtime_executable_name()),
        artifacts.runtime,
        plan.runtime,
    )?;
    set_executable_permissions(&staging.join(runtime_executable_name()))?;
    verify_platform_signature(&staging.join(runtime_executable_name()))?;
    stage_component(
        source.map(|item| item.model_pack.as_path()),
        &staging.join("model.rillpack"),
        artifacts.model,
        plan.model,
    )?;
    stage_component(
        source.map(|item| item.handler_pack.as_path()),
        &staging.join("handler.rillhandler"),
        artifacts.handler,
        plan.handler,
    )
}

fn stage_component(
    source: Option<&Path>,
    destination: &Path,
    artifact: &ReleaseArtifact,
    download: bool,
) -> Result<(), String> {
    if let (false, Some(source)) = (download, source) {
        crate::local_ai_runtime::ensure_safe_runtime_file(source)?;
        fs::copy(source, destination).map_err(|error| {
            format!(
                "copy installed local AI artifact {} to staging: {error}",
                source.display()
            )
        })?;
        fs::File::open(destination)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("sync staged artifact {}: {error}", destination.display()))
    } else {
        let bytes = download_artifact(artifact)?;
        write_synced(destination, &bytes)
    }
}

fn download_artifact(artifact: &ReleaseArtifact) -> Result<Vec<u8>, String> {
    if artifact.size == 0 || artifact.size > MAX_ARTIFACT_BYTES {
        return Err("local AI artifact exceeds the size limit".into());
    }
    let bytes = fetch_bounded(&artifact.url, MAX_ARTIFACT_BYTES)?;
    if bytes.len() as u64 != artifact.size {
        return Err(format!(
            "local AI artifact size mismatch: expected {}, got {}",
            artifact.size,
            bytes.len()
        ));
    }
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != artifact.sha256 {
        return Err(format!(
            "local AI artifact SHA-256 mismatch: expected {}, got {actual}",
            artifact.sha256
        ));
    }
    Ok(bytes)
}

fn validate_staged_deployment(
    staging: &Path,
    artifacts: SelectedArtifacts<'_>,
    plan: UpdatePlan,
) -> Result<crate::local_ai_runtime::RuntimeProbe, String> {
    let installation = RuntimeInstallation {
        executable: staging.join(runtime_executable_name()),
        model_pack: staging.join("model.rillpack"),
        handler_pack: staging.join("handler.rillhandler"),
        model_trust_keys: rill_trust_keys(),
        handler_trust_keys: rill_handler_trust_keys(),
    };
    let probe = probe_runtime(&installation)?;
    if probe.model_pack_id != MODEL_ARTIFACT_ID || probe.handler_id != MIRA_HANDLER_ID {
        return Err("staged local AI artifact identities do not match Mira's contract".into());
    }
    if plan.runtime && probe.runtime_version != artifacts.runtime.version {
        return Err("staged runtime version does not match the signed release index".into());
    }
    if plan.model && probe.model_pack_version != artifacts.model.version {
        return Err("staged model version does not match the signed release index".into());
    }
    if plan.handler && probe.handler_version != artifacts.handler.version {
        return Err("staged handler version does not match the signed release index".into());
    }
    if probe.handler_api_version != HANDLER_API_VERSION {
        return Err("staged handler API version is unsupported".into());
    }
    validate_handler_runtime_compatibility(artifacts.handler, &probe.runtime_version)?;
    Ok(probe)
}

fn probe_runtime(
    installation: &RuntimeInstallation,
) -> Result<crate::local_ai_runtime::RuntimeProbe, String> {
    use rill_runtime_protocol::{RuntimeRequest, RUNTIME_API_VERSION};

    crate::local_ai_runtime::ensure_safe_runtime_file(&installation.executable)?;
    crate::local_ai_runtime::ensure_safe_runtime_file(&installation.model_pack)?;
    crate::local_ai_runtime::ensure_safe_runtime_file(&installation.handler_pack)?;
    let request = RuntimeRequest::Handshake {
        request_id: "mira-handshake".into(),
        api_version: RUNTIME_API_VERSION,
        client_name: "mira".into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
    };
    let line = serde_json::to_vec(&request)
        .map_err(|error| format!("encode local AI handshake: {error}"))?;
    let mut command = Command::new(&installation.executable);
    command
        .arg("serve")
        .arg("--pack")
        .arg(&installation.model_pack)
        .arg("--handler")
        .arg(&installation.handler_pack);
    for key in &installation.model_trust_keys {
        command.arg("--trust-key").arg(key);
    }
    for key in &installation.handler_trust_keys {
        command.arg("--handler-trust-key").arg(key);
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
        .ok_or_else(|| "local AI runtime stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "local AI runtime stdout unavailable".to_string())?;
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
    let response: RuntimeResponseV2 = serde_json::from_slice(&buf)
        .map_err(|error| format!("decode local AI handshake response: {error}"))?;
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
        .ok_or_else(|| "downloaded Rill runtime has no valid macOS code signature".into())
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
    deployment_version: &str,
    staging: &Path,
    probe: &crate::local_ai_runtime::RuntimeProbe,
) -> Result<(), String> {
    let metadata = InstallMetadata {
        schema_version: INSTALL_METADATA_SCHEMA,
        deployment_version: deployment_version.into(),
        runtime_version: probe.runtime_version.clone(),
        model_pack_id: probe.model_pack_id.clone(),
        model_pack_version: probe.model_pack_version.clone(),
        handler_id: probe.handler_id.clone(),
        handler_version: probe.handler_version.clone(),
        handler_api_version: probe.handler_api_version,
        runtime_sha256: file_sha256(&staging.join(runtime_executable_name()))?,
        model_sha256: file_sha256(&staging.join("model.rillpack"))?,
        handler_sha256: file_sha256(&staging.join("handler.rillhandler"))?,
        publisher_key_id: RILL_V2_PRODUCTION_KEY_ID.into(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("encode local AI install metadata: {error}"))?;
    write_synced(path, &bytes)
}

fn file_sha256(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| hex::encode(Sha256::digest(bytes)))
        .map_err(|error| format!("hash local AI artifact {}: {error}", path.display()))
}

fn read_metadata(path: &Path) -> Option<InstallMetadata> {
    let metadata: InstallMetadata = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (metadata.schema_version == INSTALL_METADATA_SCHEMA
        && metadata.publisher_key_id == RILL_V2_PRODUCTION_KEY_ID)
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

fn rollback_deployment(root: &Path) -> Result<(), String> {
    let current = deployment_current_dir(root);
    let rollback = deployment_previous_dir(root);
    if !rollback.is_dir() {
        return Err("no local AI rollback is available".into());
    }
    let swap = current.with_file_name(format!(".rollback-swap-{}", std::process::id()));
    if swap.exists() {
        fs::remove_dir_all(&swap)
            .map_err(|error| format!("remove stale rollback swap: {error}"))?;
    }
    if current.exists() {
        fs::rename(&current, &swap)
            .map_err(|error| format!("prepare current local AI deployment: {error}"))?;
    }
    if let Err(error) = fs::rename(&rollback, &current) {
        if swap.exists() {
            let _ = fs::rename(&swap, &current);
        }
        return Err(format!("activate local AI rollback: {error}"));
    }
    if swap.exists() {
        fs::rename(&swap, &rollback)
            .map_err(|error| format!("preserve replaced local AI deployment: {error}"))?;
    }
    Ok(())
}

fn local_ai_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join("local-ai"))
        .map_err(|error| format!("resolve local AI data directory: {error}"))
}

fn deployment_current_dir(root: &Path) -> PathBuf {
    root.join("bundle").join("current")
}

fn deployment_previous_dir(root: &Path) -> PathBuf {
    root.join("bundle").join("previous")
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use rill_runtime_protocol::{
        ReleaseArtifact, ReleaseArtifactKind, RELEASE_INDEX_SCHEMA_VERSION, RUNTIME_API_VERSION,
    };

    use super::*;

    fn runtime_artifact(version: &str) -> ReleaseArtifact {
        ReleaseArtifact {
            kind: ReleaseArtifactKind::Runtime,
            id: RUNTIME_ARTIFACT_ID.into(),
            version: version.into(),
            runtime_api_version: RUNTIME_API_VERSION,
            target_os: Some(std::env::consts::OS.into()),
            target_arch: Some(std::env::consts::ARCH.into()),
            handler_api_version: None,
            min_runtime_version: None,
            url: format!(
                "{}local-ai-v{version}/rill-runtime-{}-{}",
                TRUSTED_RELEASE_PREFIXES[0],
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            sha256: "ab".repeat(32),
            size: 1024,
        }
    }

    fn model_artifact(version: &str) -> ReleaseArtifact {
        ReleaseArtifact {
            kind: ReleaseArtifactKind::Model,
            id: MODEL_ARTIFACT_ID.into(),
            version: version.into(),
            runtime_api_version: RUNTIME_API_VERSION,
            target_os: None,
            target_arch: None,
            handler_api_version: None,
            min_runtime_version: None,
            url: format!(
                "{}local-ai-v{version}/model.rillpack",
                TRUSTED_RELEASE_PREFIXES[0]
            ),
            sha256: "bc".repeat(32),
            size: 1024,
        }
    }

    fn handler_artifact(version: &str) -> ReleaseArtifact {
        ReleaseArtifact {
            kind: ReleaseArtifactKind::Handler,
            id: MIRA_HANDLER_ID.into(),
            version: version.into(),
            runtime_api_version: RUNTIME_API_VERSION,
            target_os: None,
            target_arch: None,
            handler_api_version: Some(HANDLER_API_VERSION),
            min_runtime_version: Some("0.7.1".into()),
            url: format!(
                "{}local-ai-v{version}/handler.rillhandler",
                TRUSTED_RELEASE_PREFIXES[0]
            ),
            sha256: "cd".repeat(32),
            size: 1024,
        }
    }

    fn selected<'a>(
        runtime: &'a ReleaseArtifact,
        model: &'a ReleaseArtifact,
        handler: &'a ReleaseArtifact,
    ) -> SelectedArtifacts<'a> {
        SelectedArtifacts {
            runtime,
            model,
            handler,
        }
    }

    fn current(runtime: &str, model: &str, handler: &str) -> LocalAiStatus {
        LocalAiStatus {
            ready: true,
            bundle_version: Some(handler.into()),
            runtime_version: Some(runtime.into()),
            model_pack_id: Some(MODEL_ARTIFACT_ID.into()),
            model_pack_version: Some(model.into()),
            handler_id: Some(MIRA_HANDLER_ID.into()),
            handler_version: Some(handler.into()),
            handler_api_version: Some(HANDLER_API_VERSION),
            rollback_available: false,
            error: None,
        }
    }

    #[test]
    fn independent_versions_never_downgrade() {
        let runtime = runtime_artifact("0.7.1");
        let model = model_artifact("0.8.2");
        let handler = handler_artifact("0.8.2");
        let plan = update_plan(
            &current("0.8.0", "0.9.0", "0.9.0"),
            selected(&runtime, &model, &handler),
        )
        .unwrap();
        assert!(!plan.any());
    }

    #[test]
    fn handler_can_update_without_runtime_or_model() {
        let runtime = runtime_artifact("0.7.1");
        let model = model_artifact("0.8.2");
        let handler = handler_artifact("0.9.0");
        let info = update_info(
            &current("0.7.1", "0.8.2", "0.8.2"),
            selected(&runtime, &model, &handler),
        )
        .unwrap();
        assert!(info.update_available);
        assert_eq!(info.available_version, "0.9.0");
    }

    #[test]
    fn artifact_selection_requires_all_three_contracts() {
        let payload = ReleaseIndexPayload {
            schema_version: RELEASE_INDEX_SCHEMA_VERSION,
            channel: "stable".into(),
            generated_at: "2026-07-15T00:00:00Z".into(),
            publisher_key_id: RILL_V2_PRODUCTION_KEY_ID.into(),
            artifacts: vec![
                runtime_artifact("0.7.1"),
                model_artifact("0.8.2"),
                handler_artifact("0.8.2"),
            ],
        };
        let selected = select_artifacts(&payload).unwrap();
        assert_eq!(selected.runtime.id, RUNTIME_ARTIFACT_ID);
        assert_eq!(selected.model.id, MODEL_ARTIFACT_ID);
        assert_eq!(selected.handler.id, MIRA_HANDLER_ID);
    }

    #[test]
    fn incompatible_handler_runtime_pair_is_rejected() {
        let mut handler = handler_artifact("0.8.2");
        handler.min_runtime_version = Some("0.8.0".into());
        assert!(validate_handler_runtime_compatibility(&handler, "0.7.1").is_err());
    }

    #[test]
    fn release_signature_changes_when_payload_changes() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let payload = ReleaseIndexPayload {
            schema_version: RELEASE_INDEX_SCHEMA_VERSION,
            channel: "stable".into(),
            generated_at: "2026-07-15T00:00:00Z".into(),
            publisher_key_id: "test".into(),
            artifacts: vec![runtime_artifact("0.7.1")],
        };
        let canonical = canonical_json(&payload).unwrap();
        let signature = signing.sign(&canonical);
        signing
            .verifying_key()
            .verify(&canonical, &signature)
            .unwrap();
        let mut changed = payload;
        changed.artifacts[0].sha256 = "ef".repeat(32);
        assert!(signing
            .verifying_key()
            .verify(&canonical_json(&changed).unwrap(), &signature)
            .is_err());
    }

    #[test]
    fn activation_keeps_one_complete_rollback() {
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
    fn release_contract_uses_mira_origin_and_rill_index_key() {
        assert!(TRUSTED_RELEASE_PREFIXES[0].contains("/hello-yunshu/mira-mouse/"));
        assert!(TRUSTED_RELEASE_PREFIXES[1].contains("/hello-yunshu/rill-ml/"));
        assert!(RELEASE_INDEX_URLS[0].contains("/releases/download/local-ai-stable/"));
        assert!(RELEASE_INDEX_URLS[1].contains("/releases/latest/"));
        assert_eq!(RILL_V2_PRODUCTION_KEY_ID, "mira-rill-2026-002");
        assert_ne!(RILL_V2_PRODUCTION_KEY_ID, crate::PRODUCTION_KEY_ID);
    }
}
