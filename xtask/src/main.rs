// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap, env, fs, io::Cursor, path::PathBuf, process::Command as StdCommand,
    time::Duration,
};
use zip::ZipArchive;

const MAX_PLUGIN_BYTES: u64 = 32 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 1024 * 1024;
const MAX_RUNTIME_BYTES: u64 = 64 * 1024 * 1024;
const RILL_RUNTIME_VERSION: &str = "0.7.1";
const RILL_ML_RELEASE_BASE: &str = "https://github.com/hello-yunshu/rill-ml/releases/download";

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Plugins {
        #[command(subcommand)]
        command: Plugins,
    },
    Plugin {
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// 从 rill-ml releases 下载预编译 `rill-runtime` 并拷到 `src-tauri/binaries/`，
    /// 供 Tauri `externalBin` sidecar 打包使用。文件名需带 target-triple 后缀。
    DistSidecar {
        /// 下载目标，缺省为 host target。
        #[arg(long)]
        target: Option<String>,
        /// 保留兼容性参数（预编译二进制始终为 release）。
        #[arg(long, default_value_t = true)]
        release: bool,
    },
}
#[derive(Subcommand)]
enum Plugins {
    Sync {
        #[arg(long)]
        locked: bool,
        #[arg(long)]
        offline: bool,
    },
    CheckLock {
        #[arg(long)]
        release_tag: Option<String>,
    },
    UpdateLock {
        #[arg(long)]
        release_tag: String,
        #[arg(long, default_value = "hello-yunshu/mira-mouse-plugins")]
        repository: String,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Lock {
    schema_version: u32,
    release_ready: bool,
    plugins: Vec<LockedPlugin>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LockedPlugin {
    plugin_id: String,
    repository: String,
    release_tag: String,
    version: String,
    asset: String,
    sha256: String,
    publisher_key_id: String,
    plugin_api: String,
    cache_path: String,
    bundle_by_default: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginManifest {
    plugin_id: String,
    version: String,
    publisher_key_id: Option<String>,
    plugin_api: String,
}
struct PublishedPlugin {
    manifest: PluginManifest,
    asset: String,
    bytes: Vec<u8>,
    sha256: String,
}
#[derive(Deserialize)]
struct ReleaseResponse {
    assets: Vec<ReleaseAsset>,
}
#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Plugins {
            command: Plugins::Sync { locked, offline },
        } => sync(locked, offline),
        Command::Plugins {
            command: Plugins::CheckLock { release_tag },
        } => check_lock(release_tag),
        Command::Plugins {
            command:
                Plugins::UpdateLock {
                    release_tag,
                    repository,
                },
        } => update_lock(&repository, &release_tag),
        Command::Plugin { args } => {
            let status = StdCommand::new("cargo")
                .args(["run", "-p", "mira-plugin-cli", "--"])
                .args(args)
                .status()?;
            if !status.success() {
                bail!("plugin command failed");
            }
            Ok(())
        }
        Command::DistSidecar { target, release } => dist_sidecar(target.as_deref(), release),
    }
}

fn sync(locked: bool, offline: bool) -> Result<()> {
    if !locked {
        bail!("plugin synchronization requires --locked");
    }
    let lock: Lock = serde_json::from_slice(&fs::read("plugins.lock.json")?)?;
    if lock.schema_version != 1 {
        bail!("unsupported lock schema");
    }
    fs::create_dir_all("src-tauri/resources/plugins")?;
    for plugin in lock.plugins.into_iter().filter(|p| p.bundle_by_default) {
        if plugin.sha256.starts_with("BLOCKED_") || plugin.cache_path.starts_with("BLOCKED_") {
            bail!("{} has unresolved release metadata", plugin.plugin_id);
        }
        let bytes = load_locked_plugin(&plugin, offline)?;
        let actual = sha256_hex(&bytes);
        if actual != plugin.sha256 {
            bail!("locked hash mismatch for {}", plugin.plugin_id);
        }
        fs::write(
            PathBuf::from("src-tauri/resources/plugins").join(plugin.asset),
            bytes,
        )?;
    }
    Ok(())
}

fn check_lock(release_tag: Option<String>) -> Result<()> {
    let lock = read_lock()?;
    let mut failures = Vec::new();
    for plugin in lock.plugins.iter().filter(|p| p.bundle_by_default) {
        if plugin.sha256.starts_with("BLOCKED_") || plugin.cache_path.starts_with("BLOCKED_") {
            failures.push(format!(
                "{} has unresolved release metadata",
                plugin.plugin_id
            ));
            continue;
        }
        let tag = release_tag.as_deref().unwrap_or(&plugin.release_tag);
        let url =
            release_asset_url_parts(&plugin.repository, tag, &format!("{}.sha256", plugin.asset));
        let remote = download_text(&url)
            .with_context(|| format!("download checksum for {}", plugin.plugin_id))?;
        let remote = remote.trim();
        if remote != plugin.sha256 {
            failures.push(format!(
                "{} {} is out of sync with {}: lock={}, remote={}",
                plugin.plugin_id, plugin.asset, tag, plugin.sha256, remote
            ));
        }
    }
    if !failures.is_empty() {
        bail!(
            "plugin lock is out of sync with published release assets:\n{}\nRun `cargo run --package xtask -- plugins update-lock --release-tag <tag>` or merge the plugin-sync PR.",
            failures.join("\n")
        );
    }
    println!("plugin lock: remote checksums match");
    Ok(())
}

fn update_lock(repository: &str, release_tag: &str) -> Result<()> {
    let mut lock = read_lock()?;
    let published = download_published_plugins(repository, release_tag)?;
    let mut updated = Vec::new();
    for plugin in published {
        let Some(entry) = lock
            .plugins
            .iter_mut()
            .find(|entry| entry.plugin_id == plugin.manifest.plugin_id)
        else {
            println!(
                "Skipping {}: not in plugins.lock.json",
                plugin.manifest.plugin_id
            );
            continue;
        };
        if !entry.bundle_by_default {
            println!("Skipping {}: bundleByDefault is false", entry.plugin_id);
            continue;
        }
        let publisher_key_id =
            plugin.manifest.publisher_key_id.clone().with_context(|| {
                format!("{} is missing publisherKeyId", plugin.manifest.plugin_id)
            })?;
        if publisher_key_id.starts_with("TEST-ONLY") {
            bail!(
                "{} uses a TEST-ONLY publisher key",
                plugin.manifest.plugin_id
            );
        }

        let resource_path = format!("src-tauri/resources/plugins/{}", plugin.asset);
        if entry.asset != plugin.asset {
            let old_path = PathBuf::from(&entry.cache_path);
            let _ = fs::remove_file(old_path);
        }
        if let Some(parent) = PathBuf::from(&resource_path).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&resource_path, &plugin.bytes)
            .with_context(|| format!("write plugin cache {resource_path}"))?;

        entry.repository = repository.to_string();
        entry.release_tag = release_tag.to_string();
        entry.version = plugin.manifest.version.clone();
        entry.asset = plugin.asset.clone();
        entry.sha256 = plugin.sha256.clone();
        entry.publisher_key_id = publisher_key_id;
        entry.plugin_api = plugin.manifest.plugin_api.clone();
        entry.cache_path = resource_path;
        updated.push(format!("{}@{}", entry.plugin_id, entry.version));
    }

    let bundled_assets = lock
        .plugins
        .iter()
        .filter(|plugin| plugin.bundle_by_default)
        .map(|plugin| format!("resources/plugins/{}", plugin.asset))
        .collect::<Vec<_>>();
    lock.release_ready = lock
        .plugins
        .iter()
        .filter(|p| p.bundle_by_default)
        .all(|p| {
            p.sha256.len() == 64
                && p.sha256.chars().all(|c| c.is_ascii_hexdigit())
                && p.repository == repository
                && !p.publisher_key_id.starts_with("TEST-ONLY")
        });
    write_lock(&lock)?;
    write_tauri_resources(&bundled_assets)?;

    if updated.is_empty() {
        println!("plugin lock: no bundled plugins updated");
    } else {
        println!("plugin lock: updated {}", updated.join(", "));
    }
    Ok(())
}

fn load_locked_plugin(plugin: &LockedPlugin, offline: bool) -> Result<Vec<u8>> {
    let source = PathBuf::from(&plugin.cache_path);
    let cached = fs::read(&source).ok();
    if let Some(bytes) = cached {
        if sha256_hex(&bytes) == plugin.sha256 {
            return Ok(bytes);
        }
        if offline {
            bail!("locked hash mismatch for {}", plugin.plugin_id);
        }
    } else if offline {
        return fs::read(&source)
            .with_context(|| format!("read locked cache {}", source.display()));
    }

    let url = release_asset_url(plugin);
    let bytes = download_bounded(&url, MAX_PLUGIN_BYTES)?;
    if sha256_hex(&bytes) != plugin.sha256 {
        bail!("downloaded hash mismatch for {}", plugin.plugin_id);
    }
    if let Some(parent) = source.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&source, &bytes)
        .with_context(|| format!("write locked cache {}", source.display()))?;
    Ok(bytes)
}

fn release_asset_url(plugin: &LockedPlugin) -> String {
    release_asset_url_parts(&plugin.repository, &plugin.release_tag, &plugin.asset)
}

fn release_asset_url_parts(repository: &str, release_tag: &str, asset: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{}/{}",
        repository,
        encode_path_segment(release_tag),
        encode_path_segment(asset)
    )
}

fn download_bounded(url: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent("mira-xtask-plugin-sync/0.1")
        .build()?
        .get(url)
        .send()
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("download {url}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        bail!("download exceeds {max_bytes} byte limit");
    }
    let bytes = response.bytes().context("read plugin download")?;
    if bytes.len() as u64 > max_bytes {
        bail!("download exceeds {max_bytes} byte limit");
    }
    Ok(bytes.to_vec())
}

fn download_text(url: &str) -> Result<String> {
    String::from_utf8(download_bounded(url, MAX_PLUGIN_BYTES)?).context("download was not UTF-8")
}

fn read_lock() -> Result<Lock> {
    let lock: Lock = serde_json::from_slice(&fs::read("plugins.lock.json")?)?;
    if lock.schema_version != 1 {
        bail!("unsupported lock schema");
    }
    Ok(lock)
}

fn write_lock(lock: &Lock) -> Result<()> {
    fs::write(
        "plugins.lock.json",
        serde_json::to_string_pretty(lock)? + "\n",
    )?;
    Ok(())
}

fn write_tauri_resources(resources: &[String]) -> Result<()> {
    let path = "src-tauri/tauri.conf.json";
    let text = fs::read_to_string(path)?;
    let config: serde_json::Value = serde_json::from_str(&text)?;
    let existing_resources = config
        .get("bundle")
        .and_then(|bundle| bundle.get("resources"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str);
    let existing_resources = existing_resources.map(str::to_string).collect::<Vec<_>>();
    let merged_resources = merge_bundle_resources(resources, &existing_resources);
    let key = "    \"resources\": [";
    let start = text
        .find(key)
        .context("tauri.conf.json missing bundle resources array")?;
    let body_start = start + key.len();
    let body_end = text[body_start..]
        .find("\n    ]")
        .map(|offset| body_start + offset + "\n    ]".len())
        .context("tauri.conf.json resources array is not formatted as expected")?;

    let mut replacement = String::from(key);
    for (index, resource) in merged_resources.iter().enumerate() {
        let comma = if index + 1 == merged_resources.len() {
            ""
        } else {
            ","
        };
        replacement.push_str(&format!(
            "\n      {}{}",
            serde_json::to_string(resource)?,
            comma
        ));
    }
    replacement.push_str("\n    ]");

    let mut updated = String::new();
    updated.push_str(&text[..start]);
    updated.push_str(&replacement);
    updated.push_str(&text[body_end..]);
    fs::write(path, updated)?;
    Ok(())
}

fn merge_bundle_resources(plugin_resources: &[String], existing: &[String]) -> Vec<String> {
    let mut merged = plugin_resources.to_vec();
    for resource in existing {
        // Plugin entries are authoritative from plugins.lock.json. Preserve every
        // other packaged resource, including the signed local-AI model pack.
        if !resource.starts_with("resources/plugins/") && !merged.contains(resource) {
            merged.push(resource.clone());
        }
    }
    merged
}

fn download_published_plugins(repository: &str, release_tag: &str) -> Result<Vec<PublishedPlugin>> {
    let api = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        repository,
        encode_path_segment(release_tag)
    );
    let release: ReleaseResponse = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent("mira-xtask-plugin-sync/0.1")
        .build()?
        .get(&api)
        .send()
        .with_context(|| format!("download {api}"))?
        .error_for_status()
        .with_context(|| format!("download {api}"))?
        .json()
        .context("parse release metadata")?;

    let mut assets = BTreeMap::new();
    for asset in release.assets {
        assets.insert(asset.name, asset.browser_download_url);
    }

    let mut plugins = Vec::new();
    let plugin_assets = assets
        .keys()
        .filter(|name| name.ends_with(".mira-plugin"))
        .cloned()
        .collect::<Vec<_>>();
    for asset in plugin_assets {
        let checksum_asset = format!("{asset}.sha256");
        let checksum_url = assets
            .get(&checksum_asset)
            .with_context(|| format!("release is missing {checksum_asset}"))?;
        let asset_url = assets
            .get(&asset)
            .with_context(|| format!("release is missing {asset}"))?;
        let bytes = download_bounded(asset_url, MAX_PLUGIN_BYTES)
            .with_context(|| format!("download {asset}"))?;
        let expected = download_text(checksum_url)
            .with_context(|| format!("download {checksum_asset}"))?
            .trim()
            .to_string();
        let actual = sha256_hex(&bytes);
        if actual != expected {
            bail!("{asset} checksum mismatch: asset={actual}, sha256={expected}");
        }
        let manifest = extract_manifest(&bytes).with_context(|| format!("extract {asset}"))?;
        plugins.push(PublishedPlugin {
            manifest,
            asset,
            bytes,
            sha256: expected,
        });
    }
    Ok(plugins)
}

fn extract_manifest(bytes: &[u8]) -> Result<PluginManifest> {
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).context("open plugin archive")?;
    let manifest = archive
        .by_name("plugin.json")
        .context("read plugin.json from plugin archive")?;
    serde_json::from_reader(manifest).context("parse plugin.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// 从 rill-ml GitHub Releases 下载预编译的 `rill-runtime`（已启用 WASM handler），
/// 按 Tauri sidecar 约定拷到 `src-tauri/binaries/`。Tauri `externalBin` 要求文件名形如
/// `rill-runtime-<target-triple>[.exe]`，
/// 其中 target-triple 必须与构建 Tauri 应用时使用的 `--target` 一致。
fn dist_sidecar(target: Option<&str>, _release: bool) -> Result<()> {
    let target_triple = match target {
        Some(t) => t.to_string(),
        None => host_target_triple()?,
    };
    let is_windows = target_triple.contains("windows");
    let binary_name = if is_windows {
        format!("rill-runtime-{target_triple}.exe")
    } else {
        format!("rill-runtime-{target_triple}")
    };

    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|_| env::current_dir())?;
    let workspace_root = manifest_dir
        .ancestors()
        .nth(1)
        .context("xtask must be run from its own crate dir")?
        .to_path_buf();
    let dest_dir = workspace_root.join("src-tauri/binaries");
    fs::create_dir_all(&dest_dir)?;

    let (target_os, target_arch) = rill_ml_platform(&target_triple)?;
    let bytes = download_runtime_binary(target_os, target_arch)?;
    let dest = dest_dir.join(&binary_name);
    fs::write(&dest, &bytes).with_context(|| format!("write sidecar to {}", dest.display()))?;
    #[cfg(target_os = "macos")]
    if target_triple.contains("apple-darwin") {
        let status = StdCommand::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&dest)
            .status()
            .context("ad-hoc sign rill-runtime")?;
        if !status.success() {
            bail!("codesign rill-runtime failed");
        }
    }
    println!(
        "dist-sidecar: downloaded {} → {}",
        target_triple,
        dest.display()
    );
    Ok(())
}

/// 将 Rust target triple 映射到 rill-ml release 资产的 (os, arch) 命名。
fn rill_ml_platform(target_triple: &str) -> Result<(&'static str, &'static str)> {
    match target_triple {
        "aarch64-apple-darwin" => Ok(("macos", "aarch64")),
        "x86_64-apple-darwin" => Ok(("macos", "x86_64")),
        "x86_64-unknown-linux-gnu" => Ok(("linux", "x86_64")),
        "x86_64-pc-windows-msvc" => Ok(("windows", "x86_64")),
        other => bail!("unsupported target triple for rill-runtime download: {other}"),
    }
}

/// 从 rill-ml releases 下载 stable-index.json 并解析为 JSON。
fn fetch_rill_ml_index() -> Result<serde_json::Value> {
    let url = format!("{RILL_ML_RELEASE_BASE}/v{RILL_RUNTIME_VERSION}/stable-index.json");
    let bytes = download_bounded(&url, MAX_INDEX_BYTES)?;
    serde_json::from_slice(&bytes).context("parse rill-ml stable-index.json")
}

/// 在 rill-ml stable-index.json 中查找指定平台的 runtime artifact。
struct RuntimeArtifactInfo {
    url: String,
    sha256: String,
    size: u64,
}

fn find_runtime_artifact(
    index: &serde_json::Value,
    target_os: &str,
    target_arch: &str,
) -> Result<RuntimeArtifactInfo> {
    let artifacts = index
        .get("payload")
        .and_then(|p| p.get("artifacts"))
        .and_then(|a| a.as_array())
        .context("stable-index.json missing payload.artifacts")?;
    for artifact in artifacts {
        let kind = artifact.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let id = artifact.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let os = artifact.get("targetOs").and_then(|v| v.as_str());
        let arch = artifact.get("targetArch").and_then(|v| v.as_str());
        if kind == "runtime"
            && id == "rill-runtime"
            && os == Some(target_os)
            && arch == Some(target_arch)
        {
            return Ok(RuntimeArtifactInfo {
                url: artifact
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                sha256: artifact
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                size: artifact.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
            });
        }
    }
    bail!("runtime artifact for {target_os}-{target_arch} not found in rill-ml stable-index.json");
}

/// 下载指定平台的 rill-runtime 预编译二进制，并通过 stable-index.json 校验 SHA-256 和大小。
fn download_runtime_binary(target_os: &str, target_arch: &str) -> Result<Vec<u8>> {
    let index = fetch_rill_ml_index()?;
    let artifact = find_runtime_artifact(&index, target_os, target_arch)?;
    if artifact.size == 0 || artifact.size > MAX_RUNTIME_BYTES {
        bail!(
            "runtime binary size {} is invalid or exceeds the {} byte limit",
            artifact.size,
            MAX_RUNTIME_BYTES
        );
    }
    let bytes = download_bounded(&artifact.url, MAX_RUNTIME_BYTES)?;
    if bytes.len() as u64 != artifact.size {
        bail!(
            "runtime binary size mismatch: expected {}, got {}",
            artifact.size,
            bytes.len()
        );
    }
    let actual = sha256_hex(&bytes);
    if actual != artifact.sha256 {
        bail!(
            "runtime binary SHA-256 mismatch: expected {}, got {}",
            artifact.sha256,
            actual
        );
    }
    Ok(bytes)
}

/// 通过 `rustc -vV` 解析 host target triple。
fn host_target_triple() -> Result<String> {
    let output = StdCommand::new("rustc")
        .args(["-vV"])
        .output()
        .context("run rustc -vV")?;
    if !output.status.success() {
        bail!("rustc -vV failed");
    }
    let text = String::from_utf8(output.stdout).context("rustc -vV output")?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("host: ") {
            return Ok(rest.trim().to_string());
        }
    }
    bail!("could not parse host target triple from rustc -vV output");
}

#[cfg(test)]
mod tests {
    use super::merge_bundle_resources;

    #[test]
    fn plugin_lock_updates_preserve_non_plugin_bundle_resources() {
        let merged = merge_bundle_resources(
            &["resources/plugins/new.mira-plugin".into()],
            &[
                "resources/plugins/old.mira-plugin".into(),
                "resources/local-ai/model.rillpack".into(),
                "resources/local-ai/handler.rillhandler".into(),
            ],
        );
        assert_eq!(
            merged,
            vec![
                "resources/plugins/new.mira-plugin",
                "resources/local-ai/model.rillpack",
                "resources/local-ai/handler.rillhandler"
            ]
        );
    }
}
