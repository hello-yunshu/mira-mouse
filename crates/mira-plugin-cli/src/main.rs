// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use mira_plugin_api::PluginManifest;
// 3.5 节：CLI 与 runtime 共享同一个 Package Format 实现（allowed + PACKAGE_FORMAT_VERSION），
// 不再维护自己的 forbidden_source()。pack/sign/inspect/verify 使用同一实现。
use mira_plugin_runtime::{
    allowed, canonical_json, inspect_package, TrustStore, PACKAGE_FORMAT_VERSION,
};
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
    about = "Validate and package declarative Mira plugins",
    // 3.5 节：CLI 独立版本（packageFormatVersion 独立于 pluginApi）。
    // 插件仓库通过 `mira-plugin --version` 探测已安装 CLI 的版本。
    version,
    long_version = long_version(),
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// 3.5 节：long_version 同时输出 CLI 版本和 package-format-version。
/// package-format-version 来自 runtime crate 的 PACKAGE_FORMAT_VERSION 常量，
/// 确保 CLI 与 runtime 共享同一个 Package Format 版本号。
fn long_version() -> &'static str {
    use std::sync::OnceLock;
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        format!(
            "{}\npackage-format-version: {}",
            env!("CARGO_PKG_VERSION"),
            PACKAGE_FORMAT_VERSION
        )
    })
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
    /// 3.5 节：Verify 子命令 —— 校验插件包的 checksums 和签名。
    /// 与 pack/sign/inspect 使用同一实现（runtime::inspect_package）。
    /// 退出码 0 表示通过，非 0 表示失败（适用于 CI 门禁）。
    Verify {
        package: PathBuf,
        /// 可选的 trusted-keys.json 路径（与 registry/trusted-keys.json 同格式）。
        /// 未提供时使用内置 TrustStore::default()（仅信任测试 key）。
        #[arg(long)]
        trusted_keys: Option<PathBuf>,
        /// 要求包必须包含有效签名（CI 门禁用）。
        #[arg(long)]
        require_signature: bool,
    },
    Sign {
        package: PathBuf,
        /// 32-byte Ed25519 private key in hexadecimal (development only).
        /// Production signing should use --key-pem or PLUGIN_SIGNING_KEY env.
        #[arg(long)]
        key_hex: Option<String>,
        /// Path to a PKCS#8 PEM Ed25519 private key file (production).
        /// Alternatively set PLUGIN_SIGNING_KEY env to the PEM content
        /// (or base64-encoded PEM for CI secret transport).
        #[arg(long)]
        key_pem: Option<PathBuf>,
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
        Command::Verify {
            package,
            trusted_keys,
            require_signature,
        } => {
            // 3.5 节：Verify 使用与 inspect 同一的 inspect_package 实现。
            let trust = load_trust_store(trusted_keys.as_deref())?;
            let file = fs::File::open(&package)?;
            let inspection = inspect_package(file, &trust, require_signature)?;
            if require_signature && !inspection.signature_verified {
                bail!("signature verification failed");
            }
            println!(
                "verified: {} v{} (signature_verified={})",
                inspection.plugin_id, inspection.version, inspection.signature_verified
            );
        }
        Command::Sign {
            package,
            key_hex,
            key_pem,
            output,
        } => {
            let signing_key = resolve_signing_key(key_hex.as_deref(), key_pem.as_deref())?;
            let public_hex = hex::encode(signing_key.verifying_key().to_bytes());
            let signed_bytes = sign_package(&package, &signing_key)?;
            let out_path = output.unwrap_or_else(|| package.clone());
            fs::write(&out_path, &signed_bytes)?;
            println!("signed: {}", out_path.display());
            println!("public key: {}", public_hex);
        }
        Command::New { plugin_id, path } => scaffold(&plugin_id, &path)?,
    }
    Ok(())
}

/// 3.5 节：解析签名密钥。优先级：--key-hex > --key-pem > PLUGIN_SIGNING_KEY env。
/// 生产环境用 PLUGIN_SIGNING_KEY（PEM 内容，或 base64 编码的 PEM 用于 CI secret 传输）。
/// 开发环境用 --key-hex（32 字节十六进制）；均未提供时生成临时测试密钥。
fn resolve_signing_key(key_hex: Option<&str>, key_pem: Option<&Path>) -> Result<SigningKey> {
    if let Some(hex_str) = key_hex {
        let bytes = hex::decode(hex_str)?;
        let array: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("key must be 32 bytes"))?;
        return Ok(SigningKey::from_bytes(&array));
    }
    if let Some(pem_path) = key_pem {
        let pem = fs::read_to_string(pem_path)?;
        return parse_pem_signing_key(&pem);
    }
    if let Ok(env_pem) = std::env::var("PLUGIN_SIGNING_KEY") {
        // CI secret 可能是 base64 编码的 PEM（避免换行问题）。
        let pem = if env_pem.contains("BEGIN PRIVATE KEY") {
            env_pem
        } else {
            use base64::Engine;
            String::from_utf8(base64::engine::general_purpose::STANDARD.decode(&env_pem)?)?
        };
        return parse_pem_signing_key(&pem);
    }
    // 开发模式：生成临时密钥对并打印私钥（仅用于本地测试）。
    use rand::TryRng;
    let mut secret = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut secret)
        .expect("SysRng fill_bytes failed");
    eprintln!("warning: generated ephemeral test key (not for production)");
    eprintln!("private key: {}", hex::encode(secret));
    Ok(SigningKey::from_bytes(&secret))
}

fn parse_pem_signing_key(pem: &str) -> Result<SigningKey> {
    use ed25519_dalek::pkcs8::DecodePrivateKey;
    SigningKey::from_pkcs8_pem(pem)
        .map_err(|e| anyhow::anyhow!("failed to parse PKCS#8 PEM private key: {e}"))
}

/// 3.5 节：从 trusted-keys.json 加载信任仓库。
/// 格式与 registry/trusted-keys.json 一致：
/// `{ "schemaVersion": 1, "keys": [{ "keyId": "...", "publicKey": "<hex>", ... }] }`
/// 仅加载 algorithm == "ed25519" 且当前时间在 [activatedAt, revokedAt) 区间内的 key。
fn load_trust_store(path: Option<&Path>) -> Result<TrustStore> {
    let mut store = TrustStore::default();
    let Some(path) = path else {
        return Ok(store);
    };
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TrustedKeysFile {
        keys: Vec<TrustedKey>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TrustedKey {
        key_id: String,
        algorithm: String,
        public_key: String,
        activated_at: String,
        revoked_at: Option<String>,
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("read trusted keys: {}", path.display()))?;
    let file: TrustedKeysFile = serde_json::from_str(&content)
        .with_context(|| format!("parse trusted keys JSON: {}", path.display()))?;
    let now = chrono::Utc::now();
    for key in file.keys {
        if key.algorithm != "ed25519" {
            continue;
        }
        let activated = chrono::DateTime::parse_from_rfc3339(&key.activated_at).ok();
        if let Some(activated) = activated {
            if now < activated.with_timezone(&chrono::Utc) {
                continue; // not yet active
            }
        }
        if let Some(revoked) = key.revoked_at.as_deref() {
            if let Ok(revoked) = chrono::DateTime::parse_from_rfc3339(revoked) {
                if now >= revoked.with_timezone(&chrono::Utc) {
                    continue; // revoked
                }
            }
        }
        let bytes = hex::decode(&key.public_key)?;
        let array: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;
        store
            .0
            .insert(key.key_id, ed25519_dalek::VerifyingKey::from_bytes(&array)?);
    }
    Ok(store)
}

fn sign_package(package: &Path, signing_key: &SigningKey) -> Result<Vec<u8>> {
    let file = fs::File::open(package)?;
    let mut archive = ZipArchive::new(file)?;

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        // 3.5 节：使用 runtime 共享的 allowlist，不再维护 CLI 自己的 forbidden_source()。
        if !allowed(&name) {
            bail!("forbidden plugin file in package: {name}");
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        files.insert(name, bytes);
    }

    files.remove("checksums.json");
    files.remove("META-INF/signature.ed25519");

    let manifest_bytes = files
        .get("plugin.json")
        .ok_or_else(|| anyhow::anyhow!("missing plugin.json"))?
        .clone();

    let checksums = Checksums {
        schema_version: PACKAGE_FORMAT_VERSION,
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
    // 3.5 节：符号链接仍然硬性拒绝（安全要求），但 allowlist 之外的文件
    //（如 LICENSE/README.md）只跳过、不 bail——`collect_files` 本就会跳过它们，
    // 源目录保留这些文档是合法的，它们只是不进入生产包。
    for entry in WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!("symbolic links are forbidden: {}", entry.path().display());
        }
    }
    Ok(manifest)
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
        schema_version: PACKAGE_FORMAT_VERSION,
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

/// 3.5 节：collect_files 只收集 allowlist 允许的文件。
/// 不再递归复制整个目录——包内文件必须由明确 allowlist 决定。
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
            // 3.5 节：仅收集 allowlist 允许的文件，文档（README.md/LICENSE/docs/*.md）
            // 默认不进入生产包。
            if !allowed(&rel) {
                continue;
            }
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
    fs::create_dir_all(path.join("models"))?;
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "packageFormatVersion": PACKAGE_FORMAT_VERSION,
        "pluginId": plugin_id,
        "name": plugin_id,
        "version": "0.1.0",
        "pluginApi": ">=1.0.0, <2.0.0",
        "publisherKeyId": null,
        "evidence": "fixture-verified",
        "permissions": [],
        "capabilities": [],
        "writesEnabled": false
    });
    fs::write(
        path.join("plugin.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    fs::write(
        path.join("tests/fixtures/example.json"),
        b"{\"kind\":\"read\",\"response\":[]}\n",
    )?;
    // Reserved parent folder for per-model adapter overrides. Plugins ship
    // model-specific JSON under `models/<model>/` in the future; the placeholder
    // keeps the directory non-empty so it survives packaging and version control.
    fs::write(path.join("models/placeholder.json"), b"{}\n")?;
    fs::write(
        path.join("README.md"),
        format!("# {plugin_id}\n\nFixture-only tutorial plugin.\n"),
    )?;
    fs::write(path.join("LICENSE"), "AGPL-3.0-or-later\n")?;
    Ok(())
}
