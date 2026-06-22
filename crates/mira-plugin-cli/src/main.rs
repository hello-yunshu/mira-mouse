// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use mira_plugin_api::PluginManifest;
use mira_plugin_runtime::{canonical_json, inspect_package, TrustStore};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

#[derive(Parser)]
#[command(
    name = "mira-plugin",
    about = "Validate and package declarative Mira plugins"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Validate {
        path: PathBuf,
    },
    Test {
        path: PathBuf,
    },
    Pack {
        path: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    Inspect {
        package: PathBuf,
        #[arg(long)]
        require_signature: bool,
    },
    Sign {
        package: PathBuf,
        #[arg(long)]
        key_hex: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    New {
        plugin_id: String,
        path: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { path } => {
            validate_dir(&path)?;
            println!("valid: {}", path.display());
        }
        Command::Test { path } => {
            validate_dir(&path)?;
            validate_fixtures(&path)?;
            println!("fixture-verified: {}", path.display());
        }
        Command::Pack { path, output } => {
            pack(&path, &output)?;
            println!("packed: {}", output.display());
        }
        Command::Inspect {
            package,
            require_signature,
        } => {
            let file = fs::File::open(&package)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&inspect_package(
                    file,
                    &TrustStore::default(),
                    require_signature
                )?)?
            );
        }
        Command::Sign {
            package,
            key_hex,
            output,
        } => {
            let signed_bytes = sign_package(&package, key_hex.as_deref())?;
            let out_path = output.unwrap_or_else(|| package.clone());
            fs::write(&out_path, &signed_bytes)?;
            println!("signed: {}", out_path.display());
        }
        Command::New { plugin_id, path } => scaffold(&plugin_id, &path)?,
    }
    Ok(())
}

fn sign_package(package: &Path, key_hex: Option<&str>) -> Result<Vec<u8>> {
    let file = fs::File::open(package)?;
    let mut archive = ZipArchive::new(file)?;

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        files.insert(name, bytes);
    }

    let signing_key = match key_hex {
        Some(hex_str) => {
            let bytes = hex::decode(hex_str)?;
            let array: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("key must be 32 bytes"))?;
            SigningKey::from_bytes(&array)
        }
        None => {
            use rand::RngCore;
            let mut secret = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut secret);
            println!("private key: {}", hex::encode(secret));
            SigningKey::from_bytes(&secret)
        }
    };

    let verifying_key = signing_key.verifying_key();
    let public_hex = hex::encode(verifying_key.to_bytes());
    println!("public key: {}", public_hex);

    files.remove("checksums.json");
    files.remove("META-INF/signature.ed25519");

    let manifest_bytes = files
        .get("plugin.json")
        .ok_or_else(|| anyhow::anyhow!("missing plugin.json"))?
        .clone();

    let checksums = Checksums {
        schema_version: 1,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), hex::encode(Sha256::digest(bytes))))
            .collect(),
    };
    let checksums_bytes = serde_json::to_vec_pretty(&checksums)?;

    let mut message = canonical_json(&manifest_bytes)?;
    message.push(b'\n');
    message.extend(canonical_json(&checksums_bytes)?);
    let signature = signing_key.sign(&message).to_bytes().to_vec();

    let mut output = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        for (name, bytes) in &files {
            zip.start_file(name, options)?;
            zip.write_all(bytes)?;
        }
        zip.start_file("checksums.json", options)?;
        zip.write_all(&checksums_bytes)?;
        zip.start_file("META-INF/signature.ed25519", options)?;
        zip.write_all(&signature)?;
        zip.finish()?;
    }
    Ok(output.into_inner())
}

fn validate_dir(path: &Path) -> Result<PluginManifest> {
    let manifest: PluginManifest = serde_json::from_slice(&fs::read(path.join("plugin.json"))?)?;
    manifest.validate()?;
    for entry in WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!("symbolic links are forbidden: {}", entry.path().display());
        }
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(path)?
                .to_string_lossy()
                .replace('\\', "/");
            if forbidden_source(&rel) {
                bail!("forbidden plugin file: {rel}");
            }
        }
    }
    Ok(manifest)
}

fn forbidden_source(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".exe", ".dll", ".dylib", ".so", ".wasm", ".html", ".css", ".js", ".ts", ".py", ".sh",
        ".bat", ".cmd", ".pyc",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
        || lower.contains(".research/")
}

fn validate_fixtures(path: &Path) -> Result<()> {
    let fixtures = path.join("tests/fixtures");
    if !fixtures.is_dir() {
        bail!("plugin has no tests/fixtures directory");
    }
    let count = WalkDir::new(fixtures)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .count();
    if count == 0 {
        bail!("plugin has no JSON fixture");
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Checksums {
    schema_version: u32,
    files: BTreeMap<String, String>,
}

fn pack(path: &Path, output: &Path) -> Result<()> {
    validate_dir(path)?;
    let mut files = collect_files(path)?;
    files.remove("checksums.json");
    files.remove("META-INF/signature.ed25519");
    let checksums = Checksums {
        schema_version: 1,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), hex::encode(Sha256::digest(bytes))))
            .collect(),
    };
    files.insert(
        "checksums.json".into(),
        serde_json::to_vec_pretty(&checksums)?,
    );
    let target =
        fs::File::create(output).with_context(|| format!("create {}", output.display()))?;
    let mut archive = ZipWriter::new(target);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (name, bytes) in files {
        archive.start_file(name, options)?;
        archive.write_all(&bytes)?;
    }
    archive.finish()?;
    Ok(())
}

fn collect_files(path: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(path).follow_links(false).sort_by_file_name() {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(path)?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(rel, fs::read(entry.path())?);
        }
    }
    Ok(files)
}

fn scaffold(plugin_id: &str, path: &Path) -> Result<()> {
    if path.exists() {
        bail!("target already exists");
    }
    fs::create_dir_all(path.join("tests/fixtures"))?;
    let manifest = serde_json::json!({
        "schemaVersion": 1, "pluginId": plugin_id, "name": plugin_id,
        "version": "0.1.0", "pluginApi": ">=1.0.0, <2.0.0",
        "publisherKeyId": null, "evidence": "fixture-verified",
        "permissions": [], "capabilities": [], "writesEnabled": false
    });
    fs::write(
        path.join("plugin.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    fs::write(
        path.join("tests/fixtures/example.json"),
        b"{\"kind\":\"read\",\"response\":[]}\n",
    )?;
    fs::write(
        path.join("README.md"),
        format!("# {plugin_id}\n\nFixture-only tutorial plugin.\n"),
    )?;
    fs::write(path.join("LICENSE"), "AGPL-3.0-or-later\n")?;
    Ok(())
}
