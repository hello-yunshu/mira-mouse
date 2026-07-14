// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mira local-AI runtime: signed-model battery prediction via RillML.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};

use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use mira_protocol::{BatteryModelConfig, BATTERY_USAGE_CAPABILITY};
use rill_runtime::{
    package::{build_signed_model_pack, load_model_pack, ModelPackError},
    RuntimeEngine, TrustStore,
};
use rill_runtime_protocol::{
    ModelPackManifest, ReleaseIndexPayload, RuntimeRequest, RuntimeResponse, SignedReleaseIndex,
    MAX_MESSAGE_BYTES, MODEL_PACK_FORMAT_VERSION, RUNTIME_API_VERSION,
};
use thiserror::Error;

mod battery;
mod handler;

use handler::MiraInvokeHandler;

#[derive(Debug, Parser)]
#[command(
    name = "mira-runtime",
    version,
    about = "Mira signed-model local inference runtime"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve newline-delimited JSON requests over stdin/stdout.
    Serve {
        #[arg(long)]
        pack: PathBuf,
        /// Trusted Ed25519 public key as KEY_ID=64_HEX_CHARS. May be repeated.
        #[arg(long = "trust-key", required = true)]
        trust_keys: Vec<String>,
    },
    /// Verify and print metadata for a signed model package.
    InspectPack {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long = "trust-key", required = true)]
        trust_keys: Vec<String>,
    },
    /// Build a signed model pack from a BatteryModelConfig JSON file.
    ///
    /// Used by CI to produce `model.rillpack` artifacts. The signing key is
    /// read from the `MODEL_PACK_SIGNING_KEY` environment variable (64 hex chars).
    BuildPack {
        /// Output path for the signed .rillpack file.
        #[arg(long)]
        output: PathBuf,
        /// Path to a JSON file containing BatteryModelConfig.
        /// If omitted, the default config is used.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Model pack identifier (e.g. "mira-battery-model").
        #[arg(long, default_value = "mira-battery-model")]
        pack_id: String,
        /// Model pack version (e.g. "0.8.1").
        #[arg(long)]
        pack_version: String,
        /// Minimum runtime version required to load this pack.
        #[arg(long, default_value = "0.5.0")]
        min_runtime_version: String,
        /// Publisher key ID matching the signing key.
        #[arg(long, default_value = "mira-rill-2026-001")]
        key_id: String,
    },
    /// Sign a Mira local-AI release-index payload. The updater intentionally
    /// uses a bundle artifact shape that is stricter than, but not identical
    /// to, the generic Rill runtime artifact schema.
    SignIndex {
        #[arg(long)]
        payload: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Error)]
enum CliError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("model package error: {0}")]
    Pack(#[from] ModelPackError),
    #[error("invalid trusted key: {0}")]
    TrustKey(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IPC message exceeds {MAX_MESSAGE_BYTES} bytes")]
    MessageTooLarge,
    #[error("handler error: {0}")]
    Handler(String),
    #[error("signing key error: {0}")]
    SigningKey(String),
    #[error("model config error: {0}")]
    ModelConfig(String),
}

fn main() -> ExitCode {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("mira-runtime: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Serve { pack, trust_keys } => {
            let trust = parse_trust_store(&trust_keys)?;
            let (loaded, _) = load_model_pack(File::open(pack)?, &trust)?;
            let handler =
                MiraInvokeHandler::from_model(&loaded.model).map_err(CliError::Handler)?;
            serve(RuntimeEngine::new(loaded).with_invoke_handler(Arc::new(handler)))
        }
        Command::InspectPack { pack, trust_keys } => {
            let trust = parse_trust_store(&trust_keys)?;
            let (_, inspection) = load_model_pack(File::open(pack)?, &trust)?;
            println!("{}", serde_json::to_string_pretty(&inspection)?);
            Ok(())
        }
        Command::BuildPack {
            output,
            config,
            pack_id,
            pack_version,
            min_runtime_version,
            key_id,
        } => {
            build_pack(
                &output,
                config.as_deref(),
                &pack_id,
                &pack_version,
                &min_runtime_version,
                &key_id,
            )?;
            eprintln!(
                "mira-runtime: signed model pack written to {}",
                output.display()
            );
            Ok(())
        }
        Command::SignIndex { payload, output } => {
            let payload: ReleaseIndexPayload = serde_json::from_slice(&std::fs::read(payload)?)?;
            let canonical = canonical_json_value(&payload)?;
            let signature =
                hex::encode(signing_key_from_environment()?.sign(&canonical).to_bytes());
            let index = SignedReleaseIndex { payload, signature };
            std::fs::write(output, serde_json::to_vec_pretty(&index)?)?;
            Ok(())
        }
    }
}

fn canonical_json_value<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, CliError> {
    fn sort(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, sort(value)))
                    .collect(),
            ),
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.into_iter().map(sort).collect())
            }
            other => other,
        }
    }
    Ok(serde_json::to_vec(&sort(serde_json::to_value(value)?))?)
}

fn signing_key_from_environment() -> Result<SigningKey, CliError> {
    let key_hex = std::env::var("MODEL_PACK_SIGNING_KEY")
        .map_err(|_| CliError::SigningKey("MODEL_PACK_SIGNING_KEY env var not set".into()))?;
    let key_bytes = hex::decode(&key_hex)
        .map_err(|_| CliError::SigningKey("signing key is not valid hexadecimal".into()))?;
    let key_array: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| CliError::SigningKey("signing key must be 32 bytes (64 hex chars)".into()))?;
    Ok(SigningKey::from_bytes(&key_array))
}

/// Build a signed model pack from a BatteryModelConfig.
///
/// The signing key is read from the `MODEL_PACK_SIGNING_KEY` environment
/// variable (64 hex chars = 32 bytes). This keeps the private key out of
/// command-line history and aligns with GitHub Actions secret usage.
fn build_pack(
    output: &PathBuf,
    config_path: Option<&std::path::Path>,
    pack_id: &str,
    pack_version: &str,
    min_runtime_version: &str,
    key_id: &str,
) -> Result<(), CliError> {
    // Load model config from file or use default.
    let model_config: BatteryModelConfig = match config_path {
        Some(path) => {
            let content = std::fs::read_to_string(path).map_err(CliError::Io)?;
            serde_json::from_str(&content).map_err(CliError::Json)?
        }
        None => BatteryModelConfig::default(),
    };
    model_config
        .validate()
        .map_err(|e| CliError::ModelConfig(e.to_string()))?;

    let model_value = serde_json::to_value(&model_config)?;

    // Read signing key from environment variable.
    let signing_key = signing_key_from_environment()?;

    let manifest = ModelPackManifest {
        format_version: MODEL_PACK_FORMAT_VERSION,
        id: pack_id.to_string(),
        version: pack_version.to_string(),
        runtime_api_version: RUNTIME_API_VERSION,
        min_runtime_version: min_runtime_version.to_string(),
        publisher_key_id: key_id.to_string(),
        capabilities: vec![BATTERY_USAGE_CAPABILITY.to_string()],
    };

    let packed_bytes = build_signed_model_pack(&manifest, &model_value, &signing_key)?;
    std::fs::write(output, &packed_bytes).map_err(CliError::Io)?;

    // Verify the pack can be loaded back with the corresponding public key.
    let verifying_key = signing_key.verifying_key();
    let mut trust_keys = BTreeMap::new();
    trust_keys.insert(key_id.to_string(), verifying_key);
    let trust_store = TrustStore(trust_keys);
    let file = File::open(output)?;
    let (_, inspection) = load_model_pack(file, &trust_store)?;
    eprintln!(
        "mira-runtime: pack verified — id={}, version={}, capabilities={:?}",
        inspection.id, inspection.version, inspection.capabilities
    );
    Ok(())
}

fn parse_trust_store(values: &[String]) -> Result<TrustStore, CliError> {
    let mut keys = BTreeMap::new();
    for value in values {
        let (key_id, encoded) = value
            .split_once('=')
            .ok_or_else(|| CliError::TrustKey("expected KEY_ID=HEX".into()))?;
        if key_id.is_empty() || key_id.len() > 96 {
            return Err(CliError::TrustKey("invalid key id".into()));
        }
        let bytes = hex::decode(encoded)
            .map_err(|_| CliError::TrustKey(format!("{key_id} is not valid hexadecimal")))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CliError::TrustKey(format!("{key_id} must contain 32 bytes")))?;
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|_| CliError::TrustKey(format!("{key_id} is not a valid Ed25519 key")))?;
        if keys.insert(key_id.to_string(), key).is_some() {
            return Err(CliError::TrustKey(format!("duplicate key id {key_id}")));
        }
    }
    Ok(TrustStore(keys))
}

fn serve(engine: RuntimeEngine) -> Result<(), CliError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout.lock());
    let mut line = Vec::new();
    loop {
        line.clear();
        let bytes_read = (&mut input)
            .take((MAX_MESSAGE_BYTES + 2) as u64)
            .read_until(b'\n', &mut line)?;
        if bytes_read == 0 {
            break;
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if line.len() > MAX_MESSAGE_BYTES {
            return Err(CliError::MessageTooLarge);
        }
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_slice::<RuntimeRequest>(&line) {
            Ok(request) => engine.handle(request),
            Err(_) => RuntimeResponse::Error {
                request_id: String::new(),
                api_version: RUNTIME_API_VERSION,
                code: "invalidJson".into(),
                message: "request is not valid protocol JSON".into(),
                retryable: false,
            },
        };
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}
