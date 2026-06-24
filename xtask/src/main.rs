// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Duration};

const MAX_PLUGIN_BYTES: u64 = 32 * 1024 * 1024;

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
}
#[derive(Subcommand)]
enum Plugins {
    Sync {
        #[arg(long)]
        locked: bool,
        #[arg(long)]
        offline: bool,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lock {
    schema_version: u32,
    plugins: Vec<LockedPlugin>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockedPlugin {
    plugin_id: String,
    repository: String,
    release_tag: String,
    asset: String,
    sha256: String,
    cache_path: String,
    bundle_by_default: bool,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Plugins {
            command: Plugins::Sync { locked, offline },
        } => sync(locked, offline),
        Command::Plugin { args } => {
            let status = std::process::Command::new("cargo")
                .args(["run", "-p", "mira-plugin-cli", "--"])
                .args(args)
                .status()?;
            if !status.success() {
                bail!("plugin command failed");
            }
            Ok(())
        }
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
    format!(
        "https://github.com/{}/releases/download/{}/{}",
        plugin.repository,
        encode_path_segment(&plugin.release_tag),
        plugin.asset
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
