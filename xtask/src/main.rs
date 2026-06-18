// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

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
    if !offline {
        bail!("network sync is disabled until a real immutable repository URL and release are configured");
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
        let source = PathBuf::from(&plugin.cache_path);
        let bytes =
            fs::read(&source).with_context(|| format!("read locked cache {}", source.display()))?;
        let actual = hex::encode(Sha256::digest(&bytes));
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
