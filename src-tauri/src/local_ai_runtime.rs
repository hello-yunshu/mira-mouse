// SPDX-License-Identifier: AGPL-3.0-or-later
//! 本地 AI Runtime 客户端。
//!
//! 历史上 Mira 每次预测都 spawn 一个短命的专用 runtime 子进程;现在改为常驻
//! `LocalAiController`(见 `local_ai_controller.rs`),`predict_batteries` 仅保留为对外
//! 签名兼容的入口,内部委托给 controller。本模块仍提供路径解析、握手验证与文件安全
//! 检查等共用工具。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use mira_protocol::BATTERY_USAGE_CAPABILITY;
use rill_runtime_protocol::{RuntimeResponseV2, HANDLER_API_VERSION, RUNTIME_API_VERSION};
use tauri::{AppHandle, Manager};

use crate::{
    battery_history::BatterySample, RILL_HANDLER_KEY_ID, RILL_HANDLER_PUBLIC_KEY_HEX,
    RILL_PRODUCTION_KEY_ID, RILL_PRODUCTION_PUBLIC_KEY_HEX, RILL_V2_PRODUCTION_KEY_ID,
    RILL_V2_PRODUCTION_PUBLIC_KEY_HEX,
};

pub(crate) const MIRA_HANDLER_ID: &str = "mira.battery.handler";

#[derive(Debug, Clone)]
pub(crate) struct RuntimeInstallation {
    pub(crate) executable: PathBuf,
    pub(crate) model_pack: PathBuf,
    pub(crate) handler_pack: PathBuf,
    pub(crate) model_trust_keys: Vec<String>,
    pub(crate) handler_trust_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeProbe {
    pub(crate) runtime_version: String,
    pub(crate) model_pack_id: String,
    pub(crate) model_pack_version: String,
    pub(crate) handler_id: String,
    pub(crate) handler_version: String,
    pub(crate) handler_api_version: u32,
}

/// 对外入口:通过 `LocalAiController` 发送批量预测请求。
/// 签名保持与旧实现一致,调用方([battery_history.rs](crate::battery_history))无需改动。
/// 任何失败返回空 map,由上层回退确定性预测。
pub fn predict_batteries(
    app: &AppHandle,
    batches: &[(String, Vec<&BatterySample>)],
    now: DateTime<Utc>,
) -> BTreeMap<String, f64> {
    let state = app.state::<crate::SessionState>();
    state.local_ai_controller.predict(app, batches, now)
}

/// 解析 Runtime 可执行文件与模型包路径。
///
/// 优先级:
/// 1. debug 构建的环境变量覆盖(`RILL_RUNTIME_PATH` / `RILL_MODEL_PACK_PATH` /
///    `RILL_HANDLER_PATH`)
/// 2. 独立更新后的 bundle:`app_local_data_dir/local-ai/bundle/current/`
/// 3. 随 Mira 打包的内置资源:Tauri sidecar + 独立签名的 model/handler pack
///
/// 任一关键文件缺失返回 `None`,由 controller 决定如何回退。
pub(crate) fn resolve_installation(app: &AppHandle) -> Option<RuntimeInstallation> {
    #[cfg(debug_assertions)]
    if let (Ok(executable), Ok(model_pack), Ok(handler_pack)) = (
        std::env::var("RILL_RUNTIME_PATH"),
        std::env::var("RILL_MODEL_PACK_PATH"),
        std::env::var("RILL_HANDLER_PATH"),
    ) {
        let mut model_trust_keys = rill_trust_keys();
        if let Ok(key) = std::env::var("RILL_TRUST_KEY") {
            if valid_trust_key_argument(&key) {
                model_trust_keys.push(key);
            }
        }
        let mut handler_trust_keys = rill_handler_trust_keys();
        if let Ok(key) = std::env::var("RILL_HANDLER_TRUST_KEY") {
            if valid_trust_key_argument(&key) {
                handler_trust_keys.push(key);
            }
        }
        let installation = RuntimeInstallation {
            executable: PathBuf::from(executable),
            model_pack: PathBuf::from(model_pack),
            handler_pack: PathBuf::from(handler_pack),
            model_trust_keys,
            handler_trust_keys,
        };
        if installation.executable.is_file()
            && installation.model_pack.is_file()
            && installation.handler_pack.is_file()
        {
            return Some(installation);
        }
    }

    let model_trust_keys = rill_trust_keys();
    let handler_trust_keys = rill_handler_trust_keys();
    let root = app.path().app_local_data_dir().ok()?.join("local-ai");
    let bundle_current = root.join("bundle").join("current");

    // 优先使用 bundle 更新后的版本。
    let bundle_runtime = bundle_current.join(runtime_executable_name());
    let bundle_model = bundle_current.join("model.rillpack");
    let bundle_handler = bundle_current.join("handler.rillhandler");
    if bundle_runtime.is_file() && bundle_model.is_file() && bundle_handler.is_file() {
        return Some(RuntimeInstallation {
            executable: bundle_runtime,
            model_pack: bundle_model,
            handler_pack: bundle_handler,
            model_trust_keys,
            handler_trust_keys,
        });
    }

    // fallback 到随 Mira 打包的内置资源。
    let sidecar_exec = resolve_sidecar_executable(app)?;
    let builtin_model = resolve_builtin_model_pack(app)?;
    let builtin_handler = resolve_builtin_handler_pack(app)?;
    if sidecar_exec.is_file() && builtin_model.is_file() && builtin_handler.is_file() {
        return Some(RuntimeInstallation {
            executable: sidecar_exec,
            model_pack: builtin_model,
            handler_pack: builtin_handler,
            model_trust_keys,
            handler_trust_keys,
        });
    }
    None
}

/// 解析 Tauri sidecar 可执行文件路径。Tauri 会把 `externalBin` 配置的二进制按
/// 当前构建 target 重命名后放到资源目录。
pub(crate) fn resolve_sidecar_executable(app: &AppHandle) -> Option<PathBuf> {
    // Tauri externalBin executables are placed beside the main executable
    // (`Contents/MacOS` on macOS, the application directory elsewhere), not
    // under resource_dir. Keep a resource-dir fallback for development layouts.
    if let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|executable| sidecar_path_beside(&executable))
        .filter(|path| path.is_file())
    {
        return Some(path);
    }
    let resource_dir = app.path().resource_dir().ok()?;
    let name = runtime_executable_name();
    Some(resource_dir.join(name))
}

/// 解析随 Mira 打包的内置 `model.rillpack`。
pub(crate) fn resolve_builtin_model_pack(app: &AppHandle) -> Option<PathBuf> {
    let resource_dir = app.path().resource_dir().ok()?;
    Some(builtin_model_path(&resource_dir))
}

pub(crate) fn resolve_builtin_handler_pack(app: &AppHandle) -> Option<PathBuf> {
    let resource_dir = app.path().resource_dir().ok()?;
    Some(builtin_handler_path(&resource_dir))
}

fn sidecar_path_beside(main_executable: &Path) -> Option<PathBuf> {
    main_executable
        .parent()
        .map(|directory| directory.join(runtime_executable_name()))
}

fn builtin_model_path(resource_dir: &Path) -> PathBuf {
    resource_dir
        .join("resources")
        .join("local-ai")
        .join("model.rillpack")
}

fn builtin_handler_path(resource_dir: &Path) -> PathBuf {
    resource_dir
        .join("resources")
        .join("local-ai")
        .join("handler.rillhandler")
}

pub(crate) fn runtime_executable_name() -> &'static str {
    if cfg!(windows) {
        "rill-runtime.exe"
    } else {
        "rill-runtime"
    }
}

pub(crate) fn rill_trust_keys() -> Vec<String> {
    vec![
        format!("{RILL_PRODUCTION_KEY_ID}={RILL_PRODUCTION_PUBLIC_KEY_HEX}"),
        format!("{RILL_V2_PRODUCTION_KEY_ID}={RILL_V2_PRODUCTION_PUBLIC_KEY_HEX}"),
    ]
}

pub(crate) fn rill_handler_trust_keys() -> Vec<String> {
    vec![format!(
        "{RILL_HANDLER_KEY_ID}={RILL_HANDLER_PUBLIC_KEY_HEX}"
    )]
}

#[cfg(debug_assertions)]
fn valid_trust_key_argument(value: &str) -> bool {
    let Some((key_id, encoded)) = value.split_once('=') else {
        return false;
    };
    !key_id.is_empty()
        && key_id.len() <= 96
        && encoded.len() == 64
        && encoded.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 检查路径是常规文件(非符号链接等)。沿用 sidecar / model.rillpack 的安全约束。
pub(crate) fn ensure_safe_runtime_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect local AI file {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "local AI path is not a regular file: {}",
            path.display()
        ));
    }
    Ok(())
}

/// 握手响应验证,供 `LocalAiController` 启动时复用。
pub(crate) fn validate_handshake_response(
    response: &RuntimeResponseV2,
) -> Result<RuntimeProbe, String> {
    match response {
        RuntimeResponseV2::Handshake {
            request_id,
            api_version,
            runtime_version,
            model_pack_id,
            model_pack_version,
            capabilities,
            handler_id,
            handler_version,
            handler_api_version,
            effective_capabilities,
        } if request_id == "mira-handshake"
            && *api_version == RUNTIME_API_VERSION
            && handler_id == MIRA_HANDLER_ID
            && !handler_version.is_empty()
            && *handler_api_version == HANDLER_API_VERSION
            && capabilities
                .iter()
                .any(|capability| capability == BATTERY_USAGE_CAPABILITY)
            && effective_capabilities
                .iter()
                .any(|capability| capability == BATTERY_USAGE_CAPABILITY) =>
        {
            Ok(RuntimeProbe {
                runtime_version: runtime_version.clone(),
                model_pack_id: model_pack_id.clone(),
                model_pack_version: model_pack_version.clone(),
                handler_id: handler_id.clone(),
                handler_version: handler_version.clone(),
                handler_api_version: *handler_api_version,
            })
        }
        RuntimeResponseV2::Error { code, message, .. } => {
            Err(format!("local AI handshake failed ({code}): {message}"))
        }
        _ => Err("local AI handshake contract mismatch".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_protocol::PredictionSource;
    use rill_runtime_protocol::RUNTIME_API_VERSION;

    fn responses(source: PredictionSource, remaining_hours: Option<f64>) -> Vec<RuntimeResponseV2> {
        let handshake = RuntimeResponseV2::Handshake {
            request_id: "mira-handshake".into(),
            api_version: RUNTIME_API_VERSION,
            runtime_version: "0.7.1".into(),
            model_pack_id: "mira.battery.default".into(),
            model_pack_version: "0.5.0".into(),
            capabilities: vec![BATTERY_USAGE_CAPABILITY.into()],
            handler_id: MIRA_HANDLER_ID.into(),
            handler_version: "0.8.2".into(),
            handler_api_version: HANDLER_API_VERSION,
            effective_capabilities: vec![BATTERY_USAGE_CAPABILITY.into()],
        };
        let output = mira_protocol::BatteryPredictionOutput {
            remaining_hours,
            source,
            reason: "test".into(),
            training_samples: 20,
            validation_samples: 10,
            baseline_mae: Some(2.0),
            candidate_mae: Some(1.0),
        };
        let prediction = RuntimeResponseV2::Result {
            request_id: "mira-battery-predict".into(),
            api_version: RUNTIME_API_VERSION,
            output: serde_json::to_value(&output).unwrap(),
        };
        vec![handshake, prediction]
    }

    #[test]
    fn handshake_validates_with_battery_capability() {
        let output = responses(PredictionSource::LocalAi, Some(12.5));
        validate_handshake_response(&output[0]).unwrap();
    }

    #[test]
    fn rill_trust_key_is_valid_and_separate_from_plugin_signing() {
        assert_ne!(
            RILL_PRODUCTION_PUBLIC_KEY_HEX,
            crate::PRODUCTION_PUBLIC_KEY_HEX
        );
        assert_ne!(RILL_HANDLER_PUBLIC_KEY_HEX, RILL_PRODUCTION_PUBLIC_KEY_HEX);
        assert_ne!(
            RILL_V2_PRODUCTION_PUBLIC_KEY_HEX,
            RILL_PRODUCTION_PUBLIC_KEY_HEX
        );
        crate::decode_key(RILL_PRODUCTION_PUBLIC_KEY_HEX);
        crate::decode_key(RILL_V2_PRODUCTION_PUBLIC_KEY_HEX);
        crate::decode_key(RILL_HANDLER_PUBLIC_KEY_HEX);
        assert_eq!(
            rill_trust_keys(),
            vec![
                format!("{RILL_PRODUCTION_KEY_ID}={RILL_PRODUCTION_PUBLIC_KEY_HEX}"),
                format!("{RILL_V2_PRODUCTION_KEY_ID}={RILL_V2_PRODUCTION_PUBLIC_KEY_HEX}"),
            ]
        );
        assert_eq!(
            rill_handler_trust_keys(),
            vec![format!(
                "{RILL_HANDLER_KEY_ID}={RILL_HANDLER_PUBLIC_KEY_HEX}"
            )]
        );
    }

    #[test]
    fn packaged_paths_match_tauri_external_bin_and_resource_layout() {
        let executable = if cfg!(windows) {
            Path::new("app").join("mira.exe")
        } else {
            Path::new("app").join("mira")
        };
        assert_eq!(
            sidecar_path_beside(&executable).unwrap(),
            Path::new("app").join(runtime_executable_name())
        );
        assert_eq!(
            builtin_model_path(Path::new("Resources")),
            Path::new("Resources/resources/local-ai/model.rillpack")
        );
        assert_eq!(
            builtin_handler_path(Path::new("Resources")),
            Path::new("Resources/resources/local-ai/handler.rillhandler")
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn development_trust_key_is_strictly_parsed() {
        assert!(valid_trust_key_argument(&format!(
            "dev={}",
            "aa".repeat(32)
        )));
        assert!(!valid_trust_key_argument("dev=abc"));
    }
}
