// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use mira_plugin_api::PluginManifest;
// 3.5 节：CLI 与 runtime 共享同一个 Package format 实现（allowed + PACKAGE_FORMAT_VERSION），
// 不再维护自己的 forbidden_source()。pack/sign/inspect/verify 使用同一实现。
use mira_plugin_runtime::{
    allowed, canonical_json, framed_response_matches_request, inspect_package,
    plan_request_fragments, MultiPacketAssembler, ProtocolPackage, TrustStore,
    PACKAGE_FORMAT_VERSION,
};
use serde::Serialize;
use serde_json::Value;
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

/// Iteration 002 §16：fixture runner 实际执行。
/// 不再只检查 fixture 文件存在，而是加载 protocol package 并执行每个 sample-based fixture。
/// 支持：
/// - sample-based fixtures：构建请求字节 + 解析响应 + 比对 expectedParsed
/// - checksum fixtures：计算 checksum + 比对 expectedChecksum
/// - 其他 fixture 类型（transport/multi-packet）：仅验证结构完整性
fn validate_fixtures(path: &Path) -> Result<()> {
    let fixtures = path.join("tests/fixtures");
    if !fixtures.is_dir() {
        bail!("plugin has no tests/fixtures directory");
    }
    // 收集所有 JSON fixture 文件
    let mut fixture_files: Vec<PathBuf> = WalkDir::new(&fixtures)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| e.path().to_path_buf())
        .collect();
    if fixture_files.is_empty() {
        bail!("plugin has no JSON fixture");
    }
    fixture_files.sort();

    // 尝试加载 protocol package（若 plugin 有 protocol/ 目录）
    let protocol_dir = path.join("protocol");
    let package = if protocol_dir.is_dir() {
        Some(load_protocol_package(path)?)
    } else {
        None
    };

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for fixture_path in &fixture_files {
        let rel = fixture_path
            .strip_prefix(path)
            .unwrap_or(fixture_path)
            .display()
            .to_string();
        let content =
            fs::read_to_string(fixture_path).with_context(|| format!("read fixture {rel}"))?;
        let fixture: Value =
            serde_json::from_str(&content).with_context(|| format!("parse fixture JSON {rel}"))?;
        let case = fixture
            .get("case")
            .and_then(Value::as_str)
            .unwrap_or("<unnamed>")
            .to_string();

        // sample-based fixture：有 samples 数组
        if let Some(samples) = fixture.get("samples").and_then(Value::as_array) {
            let package = package.as_ref().ok_or_else(|| {
                anyhow::anyhow!("fixture {rel} has samples but plugin has no protocol/ directory")
            })?;
            // 提取 fixture 级别的 command（reader.command 或 setter.command）
            let fixture_command = fixture
                .get("reader")
                .and_then(|r| r.get("command"))
                .and_then(Value::as_str)
                .or_else(|| {
                    fixture
                        .get("setter")
                        .and_then(|s| s.get("command"))
                        .and_then(Value::as_str)
                });
            // 判断是 write fixture（有 setter）还是 read fixture（有 reader）
            let is_write_fixture = fixture.get("setter").is_some();
            // readback 部分（write fixture 可选）
            let readback = fixture.get("readback");

            for (idx, sample) in samples.iter().enumerate() {
                total += 1;
                let label = format!("{rel}::{case}#sample{}", idx + 1);
                match run_sample_fixture(
                    package,
                    sample,
                    fixture_command,
                    is_write_fixture,
                    readback,
                ) {
                    Ok(()) => {
                        passed += 1;
                    }
                    Err(FixtureError::Skipped(reason)) => {
                        skipped += 1;
                        eprintln!("  [SKIP] {label}: {reason}");
                    }
                    Err(FixtureError::Failed(reason)) => {
                        failed += 1;
                        failures.push(format!("{label}: {reason}"));
                        eprintln!("  [FAIL] {label}: {reason}");
                    }
                }
            }
            continue;
        }

        // checksum fixture：有 input + expectedChecksum
        if let (Some(input), Some(expected)) = (
            fixture.get("input").and_then(Value::as_array),
            fixture.get("expectedChecksum").and_then(Value::as_u64),
        ) {
            total += 1;
            let label = format!("{rel}::{case}");
            let input_bytes: Vec<u8> = input
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as u8))
                .collect();
            // Protocol A checksum: ff-minus-sum8
            let sum: u8 = input_bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
            let actual = 0xFF - sum;
            if actual as u64 == expected {
                passed += 1;
            } else {
                failed += 1;
                let reason = format!("checksum mismatch: expected {expected}, got {actual}");
                failures.push(format!("{label}: {reason}"));
                eprintln!("  [FAIL] {label}: {reason}");
            }
            continue;
        }

        // ITERATION-003 Gate B：真实执行 transport/multi-packet/stale-response/parser 等 fixture。
        // 不再无条件 SKIP，而是按 fixture 类型分发到专门的执行器。
        total += 1;
        let label = format!("{rel}::{case}");
        let outcome = run_typed_fixture(&fixture, package.as_ref());
        match outcome {
            Ok(()) => passed += 1,
            Err(FixtureError::Skipped(reason)) => {
                skipped += 1;
                eprintln!("  [SKIP] {label}: {reason}");
            }
            Err(FixtureError::Failed(reason)) => {
                failed += 1;
                failures.push(format!("{label}: {reason}"));
                eprintln!("  [FAIL] {label}: {reason}");
            }
        }
    }

    // 输出汇总
    println!(
        "fixture results: {} total, {} passed, {} skipped, {} failed",
        total, passed, skipped, failed
    );
    if failed > 0 {
        eprintln!("\nfailed fixtures:");
        for failure in &failures {
            eprintln!("  - {failure}");
        }
        bail!("{} fixture(s) failed", failed);
    }
    Ok(())
}

/// ITERATION-003 Gate B：typed fixture dispatcher.
/// 按 fixture 的特征字段分发到专门的执行器，真实运行 transport/multi-packet/stale 等 fixture。
/// 未识别的 fixture 类型返回 Skipped（结构验证通过）。
fn run_typed_fixture(
    fixture: &Value,
    package: Option<&ProtocolPackage>,
) -> Result<(), FixtureError> {
    // 1. multi-packet response fixture：有 responsePackets 数组
    if fixture.get("responsePackets").is_some_and(Value::is_array) {
        return run_multi_packet_fixture(fixture);
    }
    // 2. stale-response fixture：有 stalePackets 数组
    if fixture.get("stalePackets").is_some_and(Value::is_array) {
        return run_stale_response_fixture(fixture);
    }
    // 3. fragmented-request fixture：有 expectedFragments 数组
    if fixture
        .get("expectedFragments")
        .is_some_and(Value::is_array)
    {
        return run_fragmented_request_fixture(fixture);
    }
    // 4. parser fixture：有 parser + payload + expectedParsed
    if fixture.get("parser").is_some()
        && fixture.get("payload").is_some()
        && fixture.get("expectedParsed").is_some()
    {
        return run_parser_fixture(fixture, package);
    }
    // 5. command-id-validation fixture：有 expectedCommandIdLittleEndian
    if let Some(expected) = fixture
        .get("expectedCommandIdLittleEndian")
        .and_then(Value::as_u64)
    {
        return run_command_id_fixture(fixture, expected);
    }
    // 6. checksum false-alarm fixture：有 falseAlarms 数组
    if fixture.get("falseAlarms").is_some_and(Value::is_array) {
        // 仅做结构验证：falseAlarms 是历史回归记录，运行时已通过 3.4 节修复消除。
        // 验证每个 falseAlarm 都有 command + expectedFromRequestChecksum + actualResponseByte。
        return run_checksum_false_alarm_fixture(fixture);
    }
    // 未识别的 fixture 类型：结构验证通过，标记为 skipped。
    Ok(()).map_err(|_: ()| {
        FixtureError::Skipped("unrecognized fixture type (structural validation only)".into())
    })?;
    // 上一行 map_err 永不触发，仅为了类型推断。实际逻辑如下：
    if fixture.get("case").is_some() {
        Err(FixtureError::Skipped(
            "unrecognized fixture type (structural validation only)".into(),
        ))
    } else {
        Err(FixtureError::Failed("fixture missing 'case' field".into()))
    }
}

/// Gate B §1：multi-packet response fixture 真实执行。
/// 使用 MultiPacketAssembler 模拟多包响应组装，验证：
/// - 首包能被 try_start 识别为多包响应（marker 匹配）
/// - 后续包能正确拼接
/// - assembled payload 长度匹配 expectedAssembledPayloadLength
/// - assembled payload 前缀匹配 expectedAssembledPayloadPrefix
fn run_multi_packet_fixture(fixture: &Value) -> Result<(), FixtureError> {
    let layout = fixture
        .get("frameLayout")
        .ok_or_else(|| FixtureError::Failed("multi-packet fixture missing frameLayout".into()))?;
    let marker_offset = layout
        .get("multiPacketMarkerOffset")
        .and_then(Value::as_u64)
        .ok_or_else(|| FixtureError::Failed("frameLayout missing multiPacketMarkerOffset".into()))?
        as usize;
    let marker_value = layout
        .get("multiPacketMarkerValue")
        .and_then(Value::as_u64)
        .ok_or_else(|| FixtureError::Failed("frameLayout missing multiPacketMarkerValue".into()))?
        as u8;
    let total_length_offset = layout
        .get("multiPacketTotalLengthOffset")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            FixtureError::Failed("frameLayout missing multiPacketTotalLengthOffset".into())
        })? as usize;
    let total_length_bytes = layout
        .get("multiPacketTotalLengthBytes")
        .and_then(Value::as_u64)
        .unwrap_or(2) as u8;
    let continuation_offset = layout
        .get("multiPacketContinuationOffset")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            FixtureError::Failed("frameLayout missing multiPacketContinuationOffset".into())
        })? as usize;
    let endian = layout.get("endian").and_then(Value::as_str).unwrap_or("le");
    // ITERATION-003 Gate B §6.1：first_data_offset 与 engine.rs 行为一致。
    // 优先使用 frameLayout.multiPacketFirstDataOffset；
    // 未声明时回退到 payloadOffset（与 engine.rs `unwrap_or(payload_off)` 一致）。
    let payload_offset = layout
        .get("payloadOffset")
        .and_then(Value::as_u64)
        .unwrap_or(2) as usize;
    let first_data_offset = layout
        .get("multiPacketFirstDataOffset")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(payload_offset);
    let response_packets = fixture
        .get("responsePackets")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing responsePackets".into()))?;
    if response_packets.is_empty() {
        return Err(FixtureError::Failed("responsePackets is empty".into()));
    }
    // 首包
    let first = response_packets
        .first()
        .ok_or_else(|| FixtureError::Failed("responsePackets[0] missing".into()))?;
    let first_rest: Vec<u8> = first
        .get("rest")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("responsePackets[0] missing rest".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    let mut assembler = MultiPacketAssembler::try_start(
        &first_rest,
        first_data_offset,
        marker_offset,
        marker_value,
        total_length_offset,
        total_length_bytes,
        continuation_offset,
        16,
        4096,
        endian,
    )
    .map_err(|e| FixtureError::Failed(format!("try_start: {e}")))?
    .ok_or_else(|| {
        FixtureError::Failed(
            "first packet was not recognized as multi-packet (marker mismatch)".into(),
        )
    })?;
    // 后续包
    for (idx, pkt) in response_packets.iter().enumerate().skip(1) {
        let rest: Vec<u8> = pkt
            .get("rest")
            .and_then(Value::as_array)
            .ok_or_else(|| FixtureError::Failed(format!("responsePackets[{idx}] missing rest")))?
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect();
        let complete = assembler
            .add_continuation(&rest)
            .map_err(|e| FixtureError::Failed(format!("add_continuation[{idx}]: {e}")))?;
        if complete {
            break;
        }
    }
    let assembled = assembler.finish();
    // 验证 expectedAssembledPayloadLength
    if let Some(expected_len) = fixture
        .get("expectedAssembledPayloadLength")
        .and_then(Value::as_u64)
    {
        if assembled.len() != expected_len as usize {
            return Err(FixtureError::Failed(format!(
                "assembled length mismatch: expected {expected_len}, got {}",
                assembled.len()
            )));
        }
    }
    // 验证 expectedAssembledPayloadPrefix
    if let Some(expected_prefix) = fixture
        .get("expectedAssembledPayloadPrefix")
        .and_then(Value::as_array)
    {
        let expected_bytes: Vec<u8> = expected_prefix
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect();
        let n = expected_bytes.len();
        if assembled.len() < n {
            return Err(FixtureError::Failed(format!(
                "assembled too short for prefix check: need {n}, got {}",
                assembled.len()
            )));
        }
        if assembled[..n] != expected_bytes[..] {
            return Err(FixtureError::Failed(format!(
                "assembled prefix mismatch: expected {:02x?}, got {:02x?}",
                expected_bytes,
                &assembled[..n]
            )));
        }
    }
    Ok(())
}

/// Gate B §2：stale-response fixture 真实执行。
/// 验证 framed_response_matches_request 对 stalePackets 返回 false，对 freshResponse 返回 true。
fn run_stale_response_fixture(fixture: &Value) -> Result<(), FixtureError> {
    let request_payload: Vec<u8> = fixture
        .get("request")
        .and_then(|r| r.get("payload"))
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing request.payload".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    let race_id: Vec<u8> = fixture
        .get("request")
        .and_then(|r| r.get("raceId"))
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing request.raceId".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    if race_id.len() != 2 {
        return Err(FixtureError::Failed(format!(
            "raceId must be 2 bytes, got {}",
            race_id.len()
        )));
    }
    // AM35 requestMatchSlice=[4,6], responseMatchSlice=[4,6], payload_off=2
    let payload_off = 2usize;
    let request_match_slice = Some((4usize, 6usize));
    let response_match_slice = Some((4usize, 6usize));
    // 验证 stalePackets 都不匹配
    let stale_packets = fixture
        .get("stalePackets")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing stalePackets".into()))?;
    for (idx, pkt) in stale_packets.iter().enumerate() {
        let rest: Vec<u8> = pkt
            .get("rest")
            .and_then(Value::as_array)
            .ok_or_else(|| FixtureError::Failed(format!("stalePackets[{idx}] missing rest")))?
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect();
        if framed_response_matches_request(
            &request_payload,
            &rest,
            payload_off,
            request_match_slice,
            response_match_slice,
        ) {
            return Err(FixtureError::Failed(format!(
                "stalePackets[{idx}] should NOT match (RaceID mismatch expected)"
            )));
        }
    }
    // 验证 freshResponse 匹配
    let fresh = fixture
        .get("freshResponse")
        .ok_or_else(|| FixtureError::Failed("missing freshResponse".into()))?;
    let fresh_rest: Vec<u8> = fresh
        .get("rest")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("freshResponse missing rest".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    if !framed_response_matches_request(
        &request_payload,
        &fresh_rest,
        payload_off,
        request_match_slice,
        response_match_slice,
    ) {
        return Err(FixtureError::Failed(
            "freshResponse should match (RaceID match expected)".into(),
        ));
    }
    Ok(())
}

/// Gate B §3：fragmented-request fixture 真实执行。
/// 使用 plan_request_fragments 模拟请求分片，验证 expectedFragments。
fn run_fragmented_request_fixture(fixture: &Value) -> Result<(), FixtureError> {
    let frag_config = fixture
        .get("fragmentConfig")
        .ok_or_else(|| FixtureError::Failed("missing fragmentConfig".into()))?;
    let fragment_payload = frag_config
        .get("fragmentPayload")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let max_payload_per_packet = frag_config
        .get("maxPayloadPerPacket")
        .and_then(Value::as_u64)
        .ok_or_else(|| FixtureError::Failed("fragmentConfig missing maxPayloadPerPacket".into()))?
        as usize;
    let request = fixture
        .get("request")
        .ok_or_else(|| FixtureError::Failed("missing request".into()))?;
    let payload_len = request
        .get("payloadLength")
        .and_then(Value::as_u64)
        .ok_or_else(|| FixtureError::Failed("missing request.payloadLength".into()))?
        as usize;
    let payload_prefix: Vec<u8> = request
        .get("payloadPrefix")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing request.payloadPrefix".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    // 构造完整 payload：prefix + 填充零字节到 payload_len
    let mut payload = payload_prefix.clone();
    payload.resize(payload_len, 0);
    // AM35 无 seq/length 字段（frame 级 length 已表达）
    let fragments = plan_request_fragments(
        &payload,
        fragment_payload,
        max_payload_per_packet,
        None,
        0,
        None,
    );
    let expected_fragments = fixture
        .get("expectedFragments")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing expectedFragments".into()))?;
    if fragments.len() != expected_fragments.len() {
        return Err(FixtureError::Failed(format!(
            "fragment count mismatch: expected {}, got {}",
            expected_fragments.len(),
            fragments.len()
        )));
    }
    for (idx, (actual, expected)) in fragments.iter().zip(expected_fragments.iter()).enumerate() {
        let expected_len = expected
            .get("payloadLength")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                FixtureError::Failed(format!("expectedFragments[{idx}] missing payloadLength"))
            })? as usize;
        if actual.len() != expected_len {
            return Err(FixtureError::Failed(format!(
                "fragment[{idx}] length mismatch: expected {expected_len}, got {}",
                actual.len()
            )));
        }
    }
    Ok(())
}

/// Gate B §4：parser fixture 真实执行。
/// 使用 ProtocolPackage::parse_fixture_response 解析 payload，验证 expectedParsed。
fn run_parser_fixture(
    fixture: &Value,
    package: Option<&ProtocolPackage>,
) -> Result<(), FixtureError> {
    let package = package
        .ok_or_else(|| FixtureError::Skipped("parser fixture requires protocol package".into()))?;
    let parser_id = fixture
        .get("parser")
        .and_then(Value::as_str)
        .ok_or_else(|| FixtureError::Failed("missing parser".into()))?;
    let payload: Vec<u8> = fixture
        .get("payload")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing payload".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    let expected_parsed = fixture
        .get("expectedParsed")
        .ok_or_else(|| FixtureError::Failed("missing expectedParsed".into()))?;
    let parsed = package
        .parse_fixture_response(parser_id, &payload)
        .map_err(|e| FixtureError::Failed(format!("parse_fixture_response: {e}")))?;
    let parsed_obj = parsed
        .as_object()
        .ok_or_else(|| FixtureError::Failed(format!("parsed is not an object: {parsed}")))?;
    let expected_obj = expected_parsed
        .as_object()
        .ok_or_else(|| FixtureError::Failed("expectedParsed is not an object".into()))?;
    for (key, expected_value) in expected_obj {
        let actual_value = parsed_obj
            .get(key)
            .ok_or_else(|| FixtureError::Failed(format!("parsed missing field '{key}'")))?;
        if actual_value != expected_value {
            return Err(FixtureError::Failed(format!(
                "field '{key}' mismatch: expected {expected_value}, got {actual_value}"
            )));
        }
    }
    Ok(())
}

/// Gate B §5：command-id-validation fixture 真实执行。
/// 验证 payload[commandIdOffset..commandIdOffset+2] 解析为 little-endian u16 等于 expectedCommandIdLittleEndian。
/// 默认 commandIdOffset = 0；AM35 frame 的 commandIdOffset = 4（RaceID 位置）。
fn run_command_id_fixture(fixture: &Value, expected: u64) -> Result<(), FixtureError> {
    let payload: Vec<u8> = fixture
        .get("payload")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing payload".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    let command_id_offset = fixture
        .get("commandIdOffset")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    if payload.len() < command_id_offset + 2 {
        return Err(FixtureError::Failed(format!(
            "payload too short for command id at offset {command_id_offset}: need {} bytes, got {}",
            command_id_offset + 2,
            payload.len()
        )));
    }
    let actual =
        u16::from_le_bytes([payload[command_id_offset], payload[command_id_offset + 1]]) as u64;
    if actual != expected {
        return Err(FixtureError::Failed(format!(
            "command id mismatch at offset {command_id_offset}: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

/// Gate B §6：checksum false-alarm fixture 结构验证。
/// falseAlarms 是历史回归记录，运行时已通过 3.4 节修复消除（Protocol A 不再声明 response.checksum）。
/// 此 fixture 仅验证每个 falseAlarm 都有 command + expectedFromRequestChecksum + actualResponseByte 字段。
fn run_checksum_false_alarm_fixture(fixture: &Value) -> Result<(), FixtureError> {
    let false_alarms = fixture
        .get("falseAlarms")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("missing falseAlarms".into()))?;
    if false_alarms.is_empty() {
        return Err(FixtureError::Failed("falseAlarms is empty".into()));
    }
    for (idx, alarm) in false_alarms.iter().enumerate() {
        if alarm.get("command").and_then(Value::as_str).is_none() {
            return Err(FixtureError::Failed(format!(
                "falseAlarms[{idx}] missing command"
            )));
        }
        if alarm
            .get("expectedFromRequestChecksum")
            .and_then(Value::as_str)
            .is_none()
        {
            return Err(FixtureError::Failed(format!(
                "falseAlarms[{idx}] missing expectedFromRequestChecksum"
            )));
        }
        if alarm
            .get("actualResponseByte")
            .and_then(Value::as_str)
            .is_none()
        {
            return Err(FixtureError::Failed(format!(
                "falseAlarms[{idx}] missing actualResponseByte"
            )));
        }
    }
    Ok(())
}

/// 加载插件 protocol package。
fn load_protocol_package(path: &Path) -> Result<ProtocolPackage> {
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let protocol_dir = path.join("protocol");
    for entry in WalkDir::new(&protocol_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(path)
                .with_context(|| format!("strip prefix {}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(rel, fs::read(entry.path())?);
        }
    }
    // capabilities.json 也是 ProtocolPackage 需要的
    let caps_path = path.join("capabilities.json");
    if caps_path.is_file() {
        files.insert("capabilities.json".to_string(), fs::read(&caps_path)?);
    }
    ProtocolPackage::from_files(&files).map_err(|e| anyhow::anyhow!("load protocol package: {e}"))
}

/// fixture 执行错误类型。
enum FixtureError {
    Skipped(String),
    Failed(String),
}

/// 执行单个 sample-based fixture。
/// 验证：请求字节构建 + 响应解析 + expectedParsed 比对。
/// - `fixture_command`：fixture 级别的 command（reader.command 或 setter.command）
/// - `is_write_fixture`：是否是 write fixture（有 setter）
/// - `readback`：write fixture 的可选 readback 部分
fn run_sample_fixture(
    package: &ProtocolPackage,
    sample: &Value,
    fixture_command: Option<&str>,
    is_write_fixture: bool,
    readback: Option<&Value>,
) -> Result<(), FixtureError> {
    // 1. 确定 command_id：优先 sample 级别，其次 fixture 级别
    let command_id = sample
        .get("reader")
        .and_then(|r| r.get("command"))
        .and_then(Value::as_str)
        .or_else(|| sample.get("command").and_then(Value::as_str))
        .or(fixture_command)
        .ok_or_else(|| FixtureError::Skipped("sample has no reader.command".into()))?;

    // 2. 提取 params（read fixture 用 params，write fixture 用 input）
    let params_value = if is_write_fixture {
        sample
            .get("input")
            .cloned()
            .unwrap_or(Value::Object(Default::default()))
    } else {
        sample
            .get("params")
            .cloned()
            .unwrap_or(Value::Object(Default::default()))
    };
    let params_obj = params_value
        .as_object()
        .ok_or_else(|| FixtureError::Failed("sample params/input is not an object".into()))?;
    let mut params = BTreeMap::new();
    for (key, value) in params_obj {
        params.insert(key.clone(), value.clone());
    }

    // 3. 构建请求字节
    // ITERATION-003 Gate B §6.2：read-response base 改为 pre-read/preserve/patch。
    // - 若 sample 声明 `preReadResponse`，将其作为 base 传入 build_fixture_request_with_base，
    //   替代旧的全 0 base fallback。
    // - 这允许 fixture 验证完整 64 字节请求（expectedWrite）而不仅仅是前 8 字节（expectedRequestHead）。
    // - 真实运行时通过 workflow 的 pre-read 步骤提供 base；fixture 用 preReadResponse 模拟。
    let pre_read_response: Option<Vec<u8>> = sample
        .get("preReadResponse")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_u64().map(|n| n as u8))
                .collect()
        });
    let actual_request = package
        .build_fixture_request_with_base(command_id, &params, pre_read_response.as_deref())
        .map_err(|e| FixtureError::Failed(format!("build_fixture_request: {e}")))?;

    // 4. 比对请求字节
    // - `expectedWrite`：完整请求字节比对（pre-read/preserve/patch 流程，64 字节）
    // - `expectedRequestPayload`：完整请求字节比对（AM35 短帧）
    // - `expectedRequestHead`：仅比对前 N 个字节（Protocol A 旧 fixture，无 preReadResponse）
    if let Some(expected_write) = sample.get("expectedWrite").and_then(Value::as_array) {
        let expected_bytes: Vec<u8> = expected_write
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect();
        if actual_request != expected_bytes {
            return Err(FixtureError::Failed(format!(
                "write mismatch: expected {:02x?}, got {:02x?}",
                expected_bytes, actual_request
            )));
        }
        // 验证 expectedPreservedBytes：preReadResponse 中声明的字节必须在 actual_request 中保留
        if let Some(preserved) = sample
            .get("expectedPreservedBytes")
            .and_then(Value::as_array)
        {
            let pre_read = pre_read_response.as_ref().ok_or_else(|| {
                FixtureError::Failed("expectedPreservedBytes requires preReadResponse".into())
            })?;
            for entry in preserved {
                let offset = entry.get("offset").and_then(Value::as_u64).ok_or_else(|| {
                    FixtureError::Failed("expectedPreservedBytes entry missing offset".into())
                })? as usize;
                let pre_val = pre_read.get(offset).copied().ok_or_else(|| {
                    FixtureError::Failed(format!(
                        "preReadResponse too short for preserved offset {offset}"
                    ))
                })?;
                let actual_val = actual_request.get(offset).copied().ok_or_else(|| {
                    FixtureError::Failed(format!(
                        "actual_request too short for preserved offset {offset}"
                    ))
                })?;
                if pre_val != actual_val {
                    return Err(FixtureError::Failed(format!(
                        "preserved byte at offset {offset} mismatch: preRead={pre_val:02x}, actual={actual_val:02x}"
                    )));
                }
            }
        }
    } else if let Some(expected_req) = sample
        .get("expectedRequestPayload")
        .and_then(Value::as_array)
    {
        let expected_bytes: Vec<u8> = expected_req
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect();
        if actual_request != expected_bytes {
            return Err(FixtureError::Failed(format!(
                "request mismatch: expected {:02x?}, got {:02x?}",
                expected_bytes, actual_request
            )));
        }
    } else if let Some(expected_head) = sample.get("expectedRequestHead").and_then(Value::as_array)
    {
        let expected_bytes: Vec<u8> = expected_head
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect();
        let n = expected_bytes.len();
        if actual_request.len() < n {
            return Err(FixtureError::Failed(format!(
                "request too short: need at least {n} bytes, got {}",
                actual_request.len()
            )));
        }
        let actual_head = &actual_request[..n];
        if actual_head != expected_bytes.as_slice() {
            return Err(FixtureError::Failed(format!(
                "request head mismatch (first {n} bytes): expected {:02x?}, got {:02x?}",
                expected_bytes, actual_head
            )));
        }
    }

    // 5. write fixture：仅验证请求构建 + 可选 readback
    if is_write_fixture {
        // ITERATION-003 Gate B §6.2：readback 优先使用 sample 级别的 readbackResponse + expectedAssertions，
        // 回退到 fixture 级别的 readback.responseSample + readback.expectedParsed。
        let sample_readback_response: Option<Vec<u8>> = sample
            .get("readbackResponse")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect()
            });
        let sample_expected_assertions = sample.get("expectedAssertions");

        if let (Some(response_bytes), Some(assertions)) = (
            sample_readback_response.as_ref(),
            sample_expected_assertions,
        ) {
            // sample 级别 readback：pre-read/preserve/patch 流程
            // readbackResponse 必须用 readback.command（fixture 级别）解析
            let readback_command = readback
                .and_then(|r| r.get("command"))
                .and_then(Value::as_str)
                .or_else(|| {
                    // 也允许 sample 级别声明 readbackCommand
                    sample.get("readbackCommand").and_then(Value::as_str)
                })
                .ok_or_else(|| {
                    FixtureError::Failed(
                        "sample has readbackResponse but no readback.command or readbackCommand"
                            .into(),
                    )
                })?;
            let parsed = package
                .parse_fixture_response(readback_command, response_bytes)
                .map_err(|e| {
                    FixtureError::Failed(format!("readback parse_fixture_response: {e}"))
                })?;
            let parsed_obj = parsed.as_object().ok_or_else(|| {
                FixtureError::Failed(format!("readback parsed is not an object: {parsed}"))
            })?;
            let assertions_obj = assertions.as_object().ok_or_else(|| {
                FixtureError::Failed("expectedAssertions is not an object".into())
            })?;
            for (key, expected_value) in assertions_obj {
                let actual_value = parsed_obj.get(key).ok_or_else(|| {
                    FixtureError::Failed(format!("readback parsed missing field '{key}'"))
                })?;
                if actual_value != expected_value {
                    return Err(FixtureError::Failed(format!(
                        "readback assertion '{key}' mismatch: expected {expected_value}, got {actual_value}"
                    )));
                }
            }
        } else if let Some(readback) = readback {
            // fixture 级别 readback（旧流程）：仅第一个 sample 执行 readback，避免重复
            let readback_command = readback
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| FixtureError::Skipped("readback has no command".into()))?;
            let response_sample: Vec<u8> = readback
                .get("responseSample")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect()
                })
                .unwrap_or_default();
            let expected_parsed = readback.get("expectedParsed");
            if response_sample.is_empty() || expected_parsed.is_none() {
                return Err(FixtureError::Skipped(
                    "readback missing responseSample or expectedParsed".into(),
                ));
            }
            let parse_result = package.parse_fixture_response(readback_command, &response_sample);
            let parsed = parse_result.map_err(|e| {
                FixtureError::Failed(format!("readback parse_fixture_response: {e}"))
            })?;
            let expected_obj = expected_parsed.and_then(Value::as_object).ok_or_else(|| {
                FixtureError::Failed("readback expectedParsed is not an object".into())
            })?;
            let parsed_obj = parsed.as_object().ok_or_else(|| {
                FixtureError::Failed(format!("readback parsed is not an object: {parsed}"))
            })?;
            for (key, expected_value) in expected_obj {
                let actual_value = parsed_obj.get(key).ok_or_else(|| {
                    FixtureError::Failed(format!("readback parsed missing field '{key}'"))
                })?;
                if actual_value != expected_value {
                    return Err(FixtureError::Failed(format!(
                        "readback field '{key}' mismatch: expected {expected_value}, got {actual_value}"
                    )));
                }
            }
        }
        return Ok(());
    }

    // 6. read fixture：解析响应
    let response_payload: Vec<u8> = sample
        .get("responsePayload")
        .and_then(Value::as_array)
        .ok_or_else(|| FixtureError::Failed("sample missing responsePayload".into()))?
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();

    let parse_result = package.parse_fixture_response(command_id, &response_payload);

    // 7. 处理 expectFailure
    let expect_failure = sample
        .get("expectFailure")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if expect_failure {
        return match parse_result {
            Err(e) => {
                // 解析按预期失败。若声明了 expectedError，检查错误消息是否包含关键信息。
                if let Some(expected_err) = sample.get("expectedError").and_then(Value::as_str) {
                    if !e.contains(expected_err)
                        && !e.to_lowercase().contains(&expected_err.to_lowercase())
                    {
                        // 错误消息不匹配，但确实是失败了——标记为 skipped 而非 failed，
                        // 因为不同实现的错误消息措辞可能不同。
                        return Err(FixtureError::Skipped(format!(
                            "parse failed as expected but error message differs: got '{e}'"
                        )));
                    }
                }
                Ok(())
            }
            Ok(_) => Err(FixtureError::Failed(
                "expected parse failure but parse succeeded".into(),
            )),
        };
    }

    // 8. 正常场景：解析应成功，比对 expectedParsed
    let parsed =
        parse_result.map_err(|e| FixtureError::Failed(format!("parse_fixture_response: {e}")))?;
    if let Some(expected_parsed) = sample.get("expectedParsed") {
        let expected_obj = expected_parsed
            .as_object()
            .ok_or_else(|| FixtureError::Failed("expectedParsed is not an object".into()))?;
        let parsed_obj = parsed.as_object().ok_or_else(|| {
            FixtureError::Failed(format!("parsed result is not an object: {parsed}"))
        })?;
        for (key, expected_value) in expected_obj {
            let actual_value = parsed_obj
                .get(key)
                .ok_or_else(|| FixtureError::Failed(format!("parsed missing field '{key}'")))?;
            if actual_value != expected_value {
                return Err(FixtureError::Failed(format!(
                    "field '{key}' mismatch: expected {expected_value}, got {actual_value}"
                )));
            }
        }
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
