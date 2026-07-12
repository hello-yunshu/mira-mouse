// SPDX-License-Identifier: AGPL-3.0-or-later
use auto_launch::AutoLaunchBuilder;
use chrono::{Local, NaiveTime, Timelike};
use ed25519_dalek::VerifyingKey;
use hidapi::HidApi;
use mira_core::{
    Connection, DeviceIdentity, DeviceSnapshot, LowBatteryCrossing, PluginCapability,
    PluginCapabilityPlacement,
};

mod battery_history;
mod tray;
use battery_history::{AbnormalDrainNotifyState, BatteryHistoryResponse, BatteryHistoryState};
use mira_plugin_runtime::{
    extract_package, hid, inspect_package, mutate_device_with_package,
    normalize_device_outputs_with_package, read_device_with_package, read_device_with_projection,
    writable_mutations_with_package, Capability, ConnectionKind, Control, DeviceReading,
    ExportableField, FeatureIndexCache, HidHandleCache, HidIoStats, OnboardMemoryCache,
    PackageInspection, ProtocolContext, ProtocolPackage, SemanticField, TrustStore,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::OsStr,
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    process::Command,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use sys_locale::get_locale;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Theme, WebviewWindow,
};
use tauri_plugin_notification::NotificationExt;

type CachedPlugins = Vec<(
    PackageInspection,
    hid::DevicesFile,
    std::collections::BTreeMap<String, Vec<u8>>,
)>;
type PluginLocales = BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>;
const AUTOSTART_HIDDEN_ARG: &str = "--hidden";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceReadDiagnostic {
    plugin_id: String,
    family: String,
    evidence: String,
    connection: String,
    device_key: String,
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    error_kind: String,
    error: String,
}

const PLUGIN_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/hello-yunshu/mira-mouse-plugins/main/registry/index.json";
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_PLUGIN_BYTES: u64 = 32 * 1024 * 1024;
/// GitHub 资源镜像配置。每项为 `(名称, URL 重写函数)`。
/// `fetch_bounded` 按顺序尝试，重写函数返回 `Some(url)` 表示该镜像支持此 URL，
/// 返回 `None` 表示跳过该镜像。
///
/// 优先级（基于 2026-07 实测）：
/// 1. **hey.run**：用户专属加速域名，对所有 `hello-yunshu` 仓库的 release/raw 均支持
///    （主仓库 release ~3.4MB/s，比 GitHub 原始快 3 倍），国内外都快。
/// 2. **direct**：GitHub 原始，hey.run 不可用时（如非 hello-yunshu 仓库）的 fallback。
/// 3. **gh-proxy.com**：公共代理，全类型支持（~2MB/s），hey.run 和 direct 都失败时的最后兜底。
///
/// 注意：`github.akams.cn`（返回 HTML）、`ghproxy.net`（超时）经实测不可用，未纳入。
type MirrorRewriter = fn(&str) -> Option<String>;

const GITHUB_MIRRORS: &[(&str, MirrorRewriter)] = &[
    ("hey.run", hey_run_mirror),
    ("direct", direct_mirror),
    ("gh-proxy.com", gh_proxy_mirror),
];
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn windows_hidden_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// 将连接类型字符串归一化为规范值（"usb"/"receiver"/"bluetooth"）。
/// devices.json 中接收器连接值可能写作 "wireless" 或 "wireless-receiver"，
/// 统一映射为 "receiver" 以匹配插件 capabilities 声明的 connections 列表。
/// 修复 #3：消除文档词汇表（"receiver"）与 runtime 实际值（"wireless"）的不一致。
fn normalize_connection(value: &str) -> &str {
    match value {
        "wireless" | "wireless-receiver" => "receiver",
        "usb" => "usb",
        "bluetooth" => "bluetooth",
        _ => value,
    }
}

fn find_firmware_version(value: &serde_json::Value) -> Option<semver::Version> {
    match value {
        serde_json::Value::String(text) => semver::Version::parse(text).ok(),
        serde_json::Value::Array(items) => items.iter().find_map(find_firmware_version),
        serde_json::Value::Object(object) => {
            for key in ["firmwareVersion", "version", "versionName", "semver"] {
                if let Some(version) = object.get(key).and_then(find_firmware_version) {
                    return Some(version);
                }
            }
            object.values().find_map(find_firmware_version)
        }
        _ => None,
    }
}

fn firmware_available(
    outputs: &BTreeMap<String, serde_json::Value>,
    min_firmware: Option<&str>,
) -> bool {
    let Some(min_firmware) = min_firmware else {
        return true;
    };
    let Ok(required) = semver::Version::parse(min_firmware) else {
        return false;
    };
    outputs
        .values()
        .find_map(find_firmware_version)
        .is_some_and(|current| current >= required)
}

/// 根据插件声明和 workflow 输出构建能力列表。
/// `outputs` 是设备读取的 workflow 输出，用于 probe 判断：
/// 当 capability.probe 引用的字段值为 0 时，available=false。
/// `connection` 是当前设备连接类型（"usb"/"receiver"/"bluetooth"），
/// 用于 #3 连接类型能力分支筛选。
fn plugin_capabilities(
    inspection: &PackageInspection,
    outputs: &BTreeMap<String, serde_json::Value>,
    connection: &str,
    package_files: Option<&BTreeMap<String, Vec<u8>>>,
    family: Option<&str>,
    writable_mutations: &[String],
) -> Vec<PluginCapability> {
    let normalized_connection = normalize_connection(connection);
    inspection
        .capabilities
        .iter()
        .map(|capability| {
            // 能力动态协商：根据 probe 声明检查 workflow 输出。
            // probe 引用的字段值为 0 → 设备不支持该能力 → available=false。
            // 无 probe 声明 → 默认 available=true（向后兼容）。
            // 修复 #1：支持整数和浮点（0/0.0 均视为不支持），非数字或字段缺失默认可用。
            let probe_available = capability.probe.as_ref().is_none_or(|probe| {
                match outputs.get(&probe.output) {
                    None => {
                        // probe 引用的 output 不存在：产生该 output 的 workflow 未执行
                        //（可能因 skip_if_zero 被跳过，说明设备不支持该能力）。
                        // fail-closed 标记不可用，避免 false-positive。
                        false
                    }
                    Some(value) => value
                        .as_object()
                        .and_then(|object| object.get(&probe.field))
                        .map(|field_value| {
                            field_value
                                .as_u64()
                                .map(|v| v != 0)
                                .or_else(|| field_value.as_f64().map(|v| v != 0.0))
                                .unwrap_or(true)
                        })
                        .unwrap_or(true),
                }
            });
            // #3 连接类型能力分支：归一化后比较，兼容 "wireless"/"wireless-receiver" 别名。
            let connection_available = capability.connections.as_ref().is_none_or(|allowed| {
                allowed
                    .iter()
                    .any(|conn| normalize_connection(conn) == normalized_connection)
            });
            let firmware_available =
                firmware_available(outputs, capability.min_firmware.as_deref());
            let mut metadata = capability.metadata.clone();
            enrich_range_from_mutation_inputs(&mut metadata, package_files, family);
            enrich_options_from_mutation_inputs(
                &mut metadata,
                package_files,
                family,
                writable_mutations,
            );
            PluginCapability {
                id: capability.id.clone(),
                control: capability.control.as_str().into(),
                label_key: capability.label_key.clone(),
                read_only: capability.read_only,
                placements: capability
                    .placements
                    .iter()
                    .map(|placement| PluginCapabilityPlacement {
                        region: placement.region.as_str().into(),
                        group: placement.group.clone(),
                        order: placement.order,
                        span: placement.span,
                        icon: placement.icon.clone(),
                    })
                    .collect(),
                metadata,
                available: probe_available && connection_available && firmware_available,
                connections: capability.connections.clone(),
                min_firmware: capability.min_firmware.clone(),
            }
        })
        .collect()
}

fn collect_mutation_refs(value: Option<&serde_json::Value>, refs: &mut Vec<String>) {
    match value {
        Some(serde_json::Value::String(value)) => refs.push(value.clone()),
        Some(serde_json::Value::Array(values)) => {
            refs.extend(
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string)),
            );
        }
        _ => {}
    }
}

fn range_mutation_refs(metadata: &BTreeMap<String, serde_json::Value>) -> Vec<String> {
    let mut refs = Vec::new();
    if let Some(mutations) = metadata
        .get("mutations")
        .and_then(serde_json::Value::as_object)
    {
        collect_mutation_refs(mutations.get("value"), &mut refs);
        collect_mutation_refs(mutations.get("default"), &mut refs);
        if refs.is_empty() {
            for value in mutations.values() {
                collect_mutation_refs(Some(value), &mut refs);
            }
        }
    }
    if refs.is_empty() {
        collect_mutation_refs(metadata.get("mutation"), &mut refs);
    }
    refs
}

fn range_param_names(metadata: &BTreeMap<String, serde_json::Value>) -> Vec<&str> {
    if metadata
        .get("mutations")
        .and_then(serde_json::Value::as_object)
        .and_then(|mutations| mutations.get("value"))
        .is_some()
    {
        return vec!["dpi", "value"];
    }
    metadata
        .get("param")
        .and_then(serde_json::Value::as_str)
        .map(|param| vec![param])
        .unwrap_or_else(|| vec!["value"])
}

fn mutation_input_range(
    workflows: &serde_json::Value,
    family: Option<&str>,
    mutation_ref: &str,
    params: &[&str],
) -> Option<(u64, u64, Option<u64>)> {
    let mutations = workflows.get("mutations")?.as_object()?;
    let mut ids = Vec::new();
    if let Some(family) = family {
        ids.push(format!("{family}-{mutation_ref}"));
    }
    ids.push(mutation_ref.to_string());
    for id in ids {
        let Some(inputs) = mutations
            .get(&id)
            .and_then(|mutation| mutation.get("inputs"))
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        for param in params {
            let Some(input) = inputs.get(*param).and_then(serde_json::Value::as_object) else {
                continue;
            };
            let Some(min) = input.get("min").and_then(serde_json::Value::as_u64) else {
                continue;
            };
            let Some(max) = input.get("max").and_then(serde_json::Value::as_u64) else {
                continue;
            };
            let step = input.get("step").and_then(serde_json::Value::as_u64);
            return Some((min, max, step));
        }
    }
    None
}

fn mutation_input_allowed(
    workflows: &serde_json::Value,
    family: Option<&str>,
    mutation_ref: &str,
    param: &str,
) -> Option<Vec<u64>> {
    let mutations = workflows.get("mutations")?.as_object()?;
    let mut ids = Vec::new();
    if let Some(family) = family {
        ids.push(format!("{family}-{mutation_ref}"));
    }
    ids.push(mutation_ref.to_string());
    for id in ids {
        let Some(allowed) = mutations
            .get(&id)
            .and_then(|mutation| mutation.get("inputs"))
            .and_then(|inputs| inputs.get(param))
            .and_then(|input| input.get("allowed"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        let values: Vec<_> = allowed
            .iter()
            .filter_map(serde_json::Value::as_u64)
            .collect();
        if !values.is_empty() {
            return Some(values);
        }
    }
    None
}

fn enrich_range_from_mutation_inputs(
    metadata: &mut BTreeMap<String, serde_json::Value>,
    package_files: Option<&BTreeMap<String, Vec<u8>>>,
    family: Option<&str>,
) {
    if metadata.contains_key("min") && metadata.contains_key("max") {
        return;
    }
    let Some(bytes) = package_files.and_then(|files| files.get("protocol/workflows.json")) else {
        return;
    };
    let Ok(workflows) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return;
    };
    let params = range_param_names(metadata);
    for mutation_ref in range_mutation_refs(metadata) {
        let Some((min, max, step)) =
            mutation_input_range(&workflows, family, &mutation_ref, &params)
        else {
            continue;
        };
        metadata
            .entry("min".into())
            .or_insert_with(|| serde_json::Value::Number(min.into()));
        metadata
            .entry("max".into())
            .or_insert_with(|| serde_json::Value::Number(max.into()));
        if let Some(step) = step {
            metadata
                .entry("step".into())
                .or_insert_with(|| serde_json::Value::Number(step.into()));
        }
        return;
    }
}

fn enrich_options_from_mutation_inputs(
    metadata: &mut BTreeMap<String, serde_json::Value>,
    package_files: Option<&BTreeMap<String, Vec<u8>>>,
    family: Option<&str>,
    writable_mutations: &[String],
) {
    if !metadata.contains_key("options") {
        return;
    }
    let Some(param) = metadata
        .get("param")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let Some(bytes) = package_files.and_then(|files| files.get("protocol/workflows.json")) else {
        return;
    };
    let Ok(workflows) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return;
    };
    let mut refs = range_mutation_refs(metadata);
    if let Some(writable) = refs.iter().position(|mutation| {
        writable_mutations
            .iter()
            .any(|candidate| candidate == mutation)
    }) {
        refs.swap(0, writable);
    }
    for mutation_ref in refs {
        let Some(allowed) = mutation_input_allowed(&workflows, family, &mutation_ref, &param)
        else {
            continue;
        };
        let labels = metadata
            .get("options")
            .and_then(serde_json::Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| {
                        Some((
                            option.get("value")?.as_u64()?,
                            option.get("label")?.as_str()?.to_string(),
                        ))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let options: Vec<_> = allowed
            .into_iter()
            .map(|value| {
                serde_json::json!({
                    "value": value,
                    "label": labels.get(&value).cloned().unwrap_or_else(|| value.to_string())
                })
            })
            .collect();
        metadata.insert("options".into(), serde_json::Value::Array(options));
        return;
    }
}

/// 托盘菜单签名：捕获影响菜单结构的所有字段。
/// 签名相同时跳过菜单重建，仅更新 title/tooltip。
#[derive(Clone, PartialEq)]
struct TrayMenuSignature {
    /// 是否有设备连接（None = 未连接，Some = 已连接）
    connected: bool,
    /// 电池信息列表：(label, percentage, charging)
    batteries: Vec<(String, u8, bool)>,
    /// 是否显示连接状态菜单项
    show_connection: bool,
    /// 连接标签（如 "USB"、"无线"）
    connection_label: String,
    /// 设备显示名
    display_name: String,
}

/// 安静灯光（夜间模式）当前所处的阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NightPhase {
    /// 日间：灯光按用户设置正常工作。
    Day,
    /// 夜间：鼠标灯光已被关闭，等待退出时段时恢复。
    Night,
}

/// 进入夜间模式时保存的鼠标灯光状态，用于退出时可靠恢复。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedMouseLight {
    /// 鼠标灯光颜色（#RRGGBB 格式，来自 capabilities.settings.mouseLightStartColor）。
    color: String,
    /// 灯效编号（0=off, 1=fixed, 3=cycle, 4=wave, 5=starlight, 10=breathing, 11=ripple, 12=custom）。
    /// 旧版持久化文件可能没有此字段，default=1（fixed）以兼容 amaster 等只支持 color+enabled 的插件。
    #[serde(default = "default_mouse_light_effect")]
    effect: u8,
    /// 灯效速度（0-255）。default=0。
    #[serde(default)]
    speed: u8,
    /// 亮度百分比（0-100）。default=100。
    #[serde(default = "default_mouse_light_brightness")]
    brightness: u8,
    /// starlight 第二色（#RRGGBB），其他灯效无此字段。
    #[serde(default)]
    extra_color: Option<String>,
}

fn default_mouse_light_effect() -> u8 {
    1
}

fn default_mouse_light_brightness() -> u8 {
    100
}

/// 进入夜间模式时保存的接收器灯光状态，用于退出时可靠恢复。
/// set-receiver-lighting mutation 没有 enabled 字段，关闭时 effect=0，
/// 因此需要保存完整状态以恢复。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedReceiverLight {
    effect: u8,
    speed: u8,
    brightness: u8,
    option: u8,
    /// 接收器灯光颜色（#RRGGBB 格式）。
    color: String,
}

/// 安静灯光运行时状态。仅存在于内存中，不直接持久化。
/// 持久化通过 NightModeStore 完成（仅保存 saved 状态）。
#[derive(Debug, Clone, PartialEq)]
struct NightModeRuntime {
    /// 当前所处的阶段。
    phase: NightPhase,
    /// 进入夜间模式时保存的鼠标灯光状态。None 表示未保存。
    saved_mouse_light: Option<SavedMouseLight>,
    /// 进入夜间模式时保存的接收器灯光状态。None 表示未保存。
    saved_receiver_light: Option<SavedReceiverLight>,
    /// 当前转换尝试是否已展示过失败通知。
    /// 调度器每分钟重试时，避免对同一 (current, target) 组合重复弹通知；
    /// 阶段成功切换后由 `update_night_mode_phase` 重置。
    notified: bool,
}

impl Default for NightModeRuntime {
    fn default() -> Self {
        Self {
            phase: NightPhase::Day,
            saved_mouse_light: None,
            saved_receiver_light: None,
            notified: false,
        }
    }
}

/// 安静灯光持久化状态。保存在 config 目录下的 night_mode_state.json。
/// 仅持久化 saved 状态，phase 在启动时根据 saved 状态推导。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NightModeStore {
    /// 进入夜间模式时保存的鼠标灯光状态。用于 Mira 重启后恢复。
    saved_mouse_light: Option<SavedMouseLight>,
    /// 进入夜间模式时保存的接收器灯光状态。用于 Mira 重启后恢复。
    saved_receiver_light: Option<SavedReceiverLight>,
}

/// 每设备指数退避状态。
/// 连续读取失败时增加退避时间，避免对休眠/离线设备的无效轮询。
/// 重置事件：热插拔、用户刷新、窗口聚焦、系统恢复、mutation。
#[derive(Default)]
struct DeviceBackoff {
    /// 连续失败次数
    consecutive_failures: u32,
    /// 下次允许重试的时间
    next_retry_at: Option<Instant>,
}

impl DeviceBackoff {
    /// 根据连续失败次数计算退避时间。
    /// 首次失败 60s，第二次 5min，第三次 15min，后续 30min 上限。
    fn backoff_duration(failures: u32) -> Duration {
        match failures {
            0 => Duration::ZERO,
            1 => Duration::from_secs(60),
            2 => Duration::from_secs(300),
            3 => Duration::from_secs(900),
            _ => Duration::from_secs(1800),
        }
    }

    /// 是否正在退避中（还未到重试时间）
    fn is_backing_off(&self) -> bool {
        self.next_retry_at.is_some_and(|t| Instant::now() < t)
    }

    /// 记录失败，增加退避
    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.next_retry_at =
            Some(Instant::now() + Self::backoff_duration(self.consecutive_failures));
    }

    /// 清除退避（读取成功或重置事件）
    fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.next_retry_at = None;
    }
}

/// 清除所有设备的退避状态。
/// 在窗口聚焦、用户手动刷新、系统恢复等事件时调用。
fn reset_all_backoff(state: &SessionState) {
    if let Ok(mut backoff) = state.backoff_state.lock() {
        for b in backoff.values_mut() {
            b.reset();
        }
    }
}

/// 记录或更新设备诊断指标。
/// 用脱敏 device key 索引，首次访问时初始化设备身份字段。
fn record_device_metric<F>(state: &SessionState, device: &hid::MatchedDevice, update: F)
where
    F: FnOnce(&mut DeviceMetrics),
{
    let key = sanitized_device_key(device);
    if let Ok(mut metrics) = state.device_metrics.lock() {
        let entry = metrics.entry(key.clone()).or_insert_with(|| DeviceMetrics {
            device_key: key,
            plugin_id: device.plugin_id.clone(),
            family: device.family.clone(),
            connection: device.connection.clone(),
            ..Default::default()
        });
        update(entry);
    }
}

/// 每种读取计划的执行统计。
#[derive(Default, Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanStats {
    presence: u64,
    quick: u64,
    battery: u64,
    full: u64,
}

impl PlanStats {
    fn bump(&mut self, plan: ReadPlan) {
        match plan {
            ReadPlan::PresenceOnly => self.presence += 1,
            ReadPlan::Quick => self.quick += 1,
            ReadPlan::BatteryOnly => self.battery += 1,
            ReadPlan::Full => self.full += 1,
        }
    }
}

/// 每设备诊断指标（本地、脱敏、有界）。
/// 不导出原始 HID path、序列号或用户路径。
#[derive(Default, Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceMetrics {
    /// 脱敏 device key（VID/PID/usage 格式，不含路径）
    device_key: String,
    plugin_id: String,
    family: String,
    connection: String,
    /// 每种计划的执行次数
    plan_counts: PlanStats,
    /// 每种计划的成功次数
    plan_success: PlanStats,
    /// 每种计划的失败次数
    plan_failures: PlanStats,
    /// 投影成功次数（Quick/BatteryOnly 投影命中）
    projection_success: u64,
    /// 投影回退次数（投影失败回退到完整读取）
    projection_fallback: u64,
    /// Full 被避免次数（Quick/BatteryOnly 投影成功替代 Full）
    full_avoided: u64,
    /// 最近读取耗时（毫秒）
    last_read_ms: Option<u64>,
    /// 当前退避连续失败次数
    backoff_failures: u32,
    /// 是否正在退避
    backing_off: bool,
}

/// 全局诊断指标。
#[derive(Default, Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GlobalMetrics {
    /// 显式调度请求总数（参数编辑、用户刷新、系统恢复等）
    schedule_requests: u64,
    /// 睡眠恢复次数
    sleep_resume_count: u64,
}

#[derive(Default, Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HidIoDiagnostics {
    reports_executed: u64,
    handle_cache_hits: u64,
    handle_cache_misses: u64,
    open_path_attempts: u64,
    open_path_failures: u64,
}

impl From<HidIoStats> for HidIoDiagnostics {
    fn from(stats: HidIoStats) -> Self {
        Self {
            reports_executed: stats.reports_executed,
            handle_cache_hits: stats.handle_cache_hits,
            handle_cache_misses: stats.handle_cache_misses,
            open_path_attempts: stats.open_path_attempts,
            open_path_failures: stats.open_path_failures,
        }
    }
}

/// 宿主诊断汇总：每设备 + 全局。
/// 通过 export_diagnostics 命令暴露给 UI，不联网。
#[derive(Default, Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HostDiagnostics {
    global: GlobalMetrics,
    devices: Vec<DeviceMetrics>,
    hid_io: HidIoDiagnostics,
}

#[derive(Default)]
struct SessionState {
    write_in_progress: Mutex<bool>,
    /// 与 `write_in_progress` 配对，让并发写入排队等待而非立即失败。
    write_cond: Condvar,
    device_io: Mutex<()>,
    /// 缓存所有已连接设备的快照，按 HID 路径索引。
    /// 支持多设备并行管理：每轮轮询更新对应设备的快照，
    /// 断开的设备从 map 中移除。`device_snapshot` 命令返回 primary 设备。
    last_snapshot: Mutex<BTreeMap<String, DeviceSnapshot>>,
    /// 用户在首页选择的 HID 设备路径；为空时回退到 primary 设备。
    selected_device_path: Mutex<Option<String>>,
    /// Last sanitized read error per matched HID path. Exposed only through
    /// diagnostics and the manual HID scan to explain Windows access/timeouts.
    last_read_errors: Mutex<BTreeMap<String, DeviceReadDiagnostic>>,
    plugins: Mutex<Option<CachedPlugins>>,
    /// 托盘图标控制器：内部维护动态图标缓存和 diff 逻辑。
    /// 替换旧的 tray_icon_level / tray_is_charging / tray_uses_dark 三个缓存字段。
    tray_renderer: Mutex<tray::renderer::PlatformTrayController>,
    /// 缓存托盘菜单签名，避免每轮轮询都重建菜单（仅图标做了 diff）。
    /// 签名相同时跳过菜单重建，仅更新 title/tooltip（轻量文本操作）。
    tray_menu_signature: Mutex<Option<TrayMenuSignature>>,
    /// 缓存系统主题检测结果，避免每次 update_tray 都 fork 进程。
    /// 由 ThemeChanged 事件和窗口聚焦事件更新；首次读取时 tray_theme_is_dark 会兜底检测。
    system_dark: Mutex<Option<bool>>,
    /// Channel used to wake the background reader thread for an immediate refresh.
    /// Send `()` to trigger a read; the reader drains pending signals before reading
    /// to avoid redundant work when multiple events fire in quick succession.
    refresh_tx: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// Strongest explicitly requested plan waiting for the reader. Repeated
    /// focus/tray/manual events coalesce instead of creating redundant reads.
    forced_read_plan: Mutex<Option<ReadPlan>>,
    applied_software_profiles: Mutex<BTreeSet<String>>,
    /// 低电量跨阈值检测：仅在电量从高跨入低时通知一次，避免反复弹窗。
    low_battery: Mutex<LowBatteryCrossing>,
    /// 缓存应用设置，避免每次轮询都读磁盘。
    /// 由 settings_set 命令和首次加载时更新。
    cached_settings: Mutex<Option<AppSettings>>,
    /// 缓存 HidApi 实例，避免每次设备读取/写入都重新枚举所有 HID 设备。
    /// 调用方负责在持锁期间完成 HID 操作；device_io 锁已序列化大部分访问。
    cached_hidapi: Mutex<Option<HidApi>>,
    /// 缓存 inspect_bundled_plugins 结果。内置插件在构建时打包，运行时不变，
    /// 因此只需计算一次（SHA-256 + 签名验证）。
    cached_bundled_plugins: Mutex<Option<Vec<BundledPluginInfo>>>,
    /// 缓存插件 locale 文件（locales/*.json），按 plugin_id 索引。
    /// 结构: pluginId → locale code → key → translation。
    /// 前端启动时通过 plugin_locales 命令获取，合并到 i18n 命名空间。
    cached_plugin_locales: Mutex<Option<PluginLocales>>,
    /// 缓存 load_software_profiles 结果。由 save_software_profiles 写入后失效。
    cached_software_profiles: Mutex<Option<SoftwareProfileStore>>,
    /// 缓存 ProtocolPackage 解析结果，按 plugin_id 索引。
    /// 每次 read_device_once / device_mutate_blocking 都会调用 from_files，
    /// 而 JSON 文件内容在插件加载后不变，缓存可避免重复解析。
    /// 由插件加载（启动 / install_plugin_update）清空。
    cached_packages: Mutex<HashMap<String, Arc<ProtocolPackage>>>,
    /// #9 防抖式 TTL 缓存：记录每个设备最近一次 HID 读取的时间戳（按 HID 路径索引）。
    /// 500ms 内的重复读取复用 last_snapshot，跳过 HID 往返。
    /// 写入后（device_mutate_blocking）和设备断开（clear_snapshots）时清除。
    last_read_at: Mutex<HashMap<String, Instant>>,
    /// 每设备指数退避状态，按 HID 路径索引。
    /// 连续读取失败时增加退避时间，避免对休眠/离线设备的无效轮询。
    /// 重置事件：热插拔、用户刷新、窗口聚焦、系统恢复、mutation。
    backoff_state: Mutex<HashMap<String, DeviceBackoff>>,
    /// 每个已枚举设备的快照阶段。该状态独立于 UI 快照内容，避免用
    /// `plugin_capabilities.is_empty()` 猜测是否完成过 Full 读取。
    snapshot_readiness: Mutex<HashMap<String, SnapshotReadiness>>,
    /// 每设备诊断指标，按脱敏 device key 索引。
    /// 本地、脱敏、有界：不导出原始 HID path、序列号或用户路径。
    device_metrics: Mutex<BTreeMap<String, DeviceMetrics>>,
    /// 全局诊断指标（调度请求、睡眠恢复等）。
    global_metrics: Mutex<GlobalMetrics>,
    /// 缓存 logitech-hidpp feature index 查询结果，按设备路径索引。
    /// feature index 在设备连接期间不变，缓存命中时跳过 root-get-feature 的 HID 往返。
    /// 设备断开时由 clear_snapshots 清空。
    feature_index_cache: Mutex<FeatureIndexCache>,
    /// 缓存 onboard memory 读取结果，按设备路径索引。
    /// 写入 mutation 预读时命中缓存则跳过 16 chunk HID 往返；验证读后更新缓存。
    /// 设备断开时由 clear_snapshots 清空。
    onboard_memory_cache: Mutex<OnboardMemoryCache>,
    /// 已打开的 HID 设备句柄缓存，按设备路径索引。
    /// 命中时复用句柄跳过 open_path 系统调用；未命中时 open_path 并在执行成功后归还。
    /// 设备断开时由 clear_snapshots 清空。device_io 锁已序列化 HID 访问，缓存读写仅持锁极短时段。
    cached_handles: Mutex<HidHandleCache>,
    /// 本地 HID I/O 诊断计数；只记录计数，不保存路径或报告内容。
    hid_io_stats: Mutex<HidIoStats>,
    /// 安静灯光（夜间模式）运行时状态：当前阶段 + 进入夜间时保存的灯光状态。
    /// 由 spawn_night_mode_scheduler 后台线程读写，settings_set 唤醒该线程。
    night_mode: Mutex<NightModeRuntime>,
    /// Channel used to wake the night mode scheduler thread for an immediate
    /// re-evaluation (settings change, manual toggle, etc.).
    night_mode_tx: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// debug 构建中记录上一次 matched device 数量，仅在变化时输出日志。
    #[cfg(debug_assertions)]
    last_matched_count: Mutex<usize>,
    /// macOS 原生通知点击后待执行的跳转动作。
    /// macOS 上 `tauri-plugin-notification` 不暴露点击回调，改用 pending action +
    /// 窗口 focus 消费的方式：发通知时写入 action，前端聚焦窗口时取走并执行跳转。
    /// Windows/Linux 直接用 `notify-rust` 的 `wait_for_action` 处理点击，不写入此字段。
    pending_notification_action: Mutex<Option<String>>,
    /// 电量使用情况：内存缓存 + 去重追踪。启动时从 battery_history.json 加载。
    battery_history: BatteryHistoryState,
    /// 异常耗电通知节流：同一设备同一部件 24 小时内最多通知一次。
    abnormal_drain_notify: AbnormalDrainNotifyState,
}

fn hid_io_stats_ref(state: &SessionState) -> Option<&Mutex<HidIoStats>> {
    Some(&state.hid_io_stats)
}

/// Send an immediate-refresh signal to the background reader thread.
/// No-op if the reader has not been spawned yet.
///
/// 退避重置由调用方按需执行，不在本函数中无条件重置：
/// 避免高频 UI 交互（tab 切换、编辑按钮）反复重置休眠设备退避，
/// 违背设计文档第 420-427 行的退避重置事件清单。
fn request_refresh_with_plan(state: &SessionState, plan: ReadPlan) {
    // 全局诊断：记录调度请求。
    if let Ok(mut g) = state.global_metrics.lock() {
        g.schedule_requests += 1;
    }
    if let Ok(mut pending) = state.forced_read_plan.lock() {
        *pending = Some(pending.map_or(plan, |current| current.max(plan)));
    }
    if let Ok(tx) = state.refresh_tx.lock() {
        if let Some(sender) = tx.as_ref() {
            let _ = sender.send(());
        }
    }
}

/// mutation 后请求 Quick 读取验证。
/// mutation 已在 `device_mutate_blocking` 中单独重置当前设备退避，
/// 不重置其他设备——一个设备的写入不应影响其他设备的退避状态。
fn request_refresh(state: &SessionState) {
    request_refresh_with_plan(state, ReadPlan::Quick);
}

/// 窗口聚焦/托盘点击/Dock 点击等窗口事件触发的 Presence 刷新。
/// 这些事件属于设计文档定义的退避重置事件（窗口重新打开），需重置所有设备退避。
fn request_presence_refresh(state: &SessionState) {
    reset_all_backoff(state);
    request_refresh_with_plan(state, ReadPlan::PresenceOnly);
}

/// Send a wake-up signal to the night mode scheduler thread.
fn request_night_mode_eval(state: &SessionState) {
    if let Ok(tx) = state.night_mode_tx.lock() {
        if let Some(sender) = tx.as_ref() {
            let _ = sender.send(());
        }
    }
}

/// 安静灯光阶段转换：根据目标阶段执行灯光关闭或恢复。
///
/// 支持多灯光对象（鼠标灯光 + 接收器灯光），根据 `settings.night_mode_target_*`
/// 决定操作哪些灯光。进入夜间时保存原状态，退出时恢复。
///
/// 此函数在调度器线程中调用，通过 `device_mutate_blocking` 执行 HID 写入。
/// 失败时通过系统通知告知用户（遵循项目约定：不在界面内弹错误）。
fn apply_night_mode_transition(app: &AppHandle, target_phase: NightPhase) {
    let state = app.state::<SessionState>();
    // 读取当前运行时状态（持锁时间极短，仅 clone）。
    let (current_phase, saved_mouse, saved_receiver, already_notified) = {
        let guard = state.night_mode.lock().unwrap_or_else(|e| e.into_inner());
        (
            guard.phase,
            guard.saved_mouse_light.clone(),
            guard.saved_receiver_light.clone(),
            guard.notified,
        )
    };
    if current_phase == target_phase {
        return;
    }

    let settings = cached_settings(app);
    let lang = effective_language(&settings.language);
    // 同一转换尝试已通知过失败时，本轮静默重试，不再重复弹通知。
    let can_notify = !already_notified;
    let mut did_notify = false;

    match target_phase {
        NightPhase::Night => {
            // Day → Night：关闭勾选的灯光对象。
            let snapshot = state.last_snapshot.lock().ok().and_then(|guard| {
                selected_snapshot_entry(&state, &guard).map(|(_, snapshot)| snapshot.clone())
            });

            let Some(snapshot) = snapshot else {
                // 设备未连接：不更新阶段，保持 Day，下一轮调度会重试。
                return;
            };

            // 从插件 capability 的 lightingRole 强类型字段动态查询 mutation 名。
            let (mouse_mutation, receiver_mutation) = resolve_lighting_mutations(&snapshot);
            let can_mouse = mouse_mutation
                .as_deref()
                .is_some_and(|mutation| snapshot_supports_mutation(&snapshot, mutation));
            let can_receiver = receiver_mutation
                .as_deref()
                .is_some_and(|mutation| snapshot_supports_mutation(&snapshot, mutation));

            // 如果设备不支持任何勾选的灯光对象：静默跳过，仅更新阶段。
            let mouse_wanted = settings.night_mode_target_mouse && can_mouse;
            let receiver_wanted = settings.night_mode_target_receiver && can_receiver;
            if !mouse_wanted && !receiver_wanted {
                update_night_mode_phase(app, NightPhase::Night, None, None);
                return;
            }

            // 以运行时已有的 saved 为初始值：部分失败重试时，已成功关闭的灯光
            // 不会因重新读快照（enabled=false）被跳过而丢失 saved。
            let mut new_saved_mouse = saved_mouse.clone();
            let mut new_saved_receiver = saved_receiver.clone();
            let mut any_failed = false;

            // 鼠标灯光（仅在尚未保存时才尝试关闭）
            if mouse_wanted && new_saved_mouse.is_none() {
                let Some(mouse_mutation) = mouse_mutation.as_deref() else {
                    return;
                };
                if let Some((is_on, saved)) = read_mouse_light_state(&snapshot) {
                    if is_on {
                        // 关闭灯光：off effect 由插件声明，同时传递 speed/brightness 以满足
                        // HID++ mutation inputs 的校验（即使设备走 memory 路径也兼容）。
                        let off_effect = mouse_lighting_off_effect(&snapshot);
                        let mut params = serde_json::Map::from_iter([
                            (
                                "color".into(),
                                serde_json::Value::String(saved.color.clone()),
                            ),
                            ("enabled".into(), serde_json::Value::Bool(false)),
                            (
                                "effect".into(),
                                serde_json::Value::Number(off_effect.into()),
                            ),
                            (
                                "speed".into(),
                                serde_json::Value::Number(saved.speed.into()),
                            ),
                            (
                                "brightness".into(),
                                serde_json::Value::Number(saved.brightness.into()),
                            ),
                        ]);
                        if let Some(ref ec) = saved.extra_color {
                            params
                                .insert("extraColor".into(), serde_json::Value::String(ec.clone()));
                        }
                        match device_mutate_blocking(app, mouse_mutation, &params) {
                            Ok(_) => {
                                new_saved_mouse = Some(saved);
                            }
                            Err(error) => {
                                eprintln!(
                                    "[mira] night mode: failed to turn off mouse lighting: {error}"
                                );
                                if can_notify {
                                    let _ = app
                                        .notification()
                                        .builder()
                                        .title(tr_night_mode_title(lang))
                                        .body(tr_night_mode_off_failed(lang, &error))
                                        .show();
                                    did_notify = true;
                                }
                                any_failed = true;
                            }
                        }
                    }
                    // 灯光本就关闭：不保存状态，退出时不强行开灯。
                }
            }

            // 接收器灯光（effect=0 表示关闭，仅在尚未保存时才尝试关闭）
            if receiver_wanted && new_saved_receiver.is_none() {
                let Some(receiver_mutation) = receiver_mutation.as_deref() else {
                    return;
                };
                if let Some(saved) = read_receiver_light_state(&snapshot) {
                    if saved.effect != 0 {
                        let params = serde_json::Map::from_iter([
                            ("effect".into(), serde_json::Value::Number(0u8.into())),
                            (
                                "speed".into(),
                                serde_json::Value::Number(saved.speed.into()),
                            ),
                            (
                                "brightness".into(),
                                serde_json::Value::Number(saved.brightness.into()),
                            ),
                            (
                                "option".into(),
                                serde_json::Value::Number(saved.option.into()),
                            ),
                            (
                                "color".into(),
                                serde_json::Value::String(saved.color.clone()),
                            ),
                        ]);
                        match device_mutate_blocking(app, receiver_mutation, &params) {
                            Ok(_) => {
                                new_saved_receiver = Some(saved);
                            }
                            Err(error) => {
                                eprintln!("[mira] night mode: failed to turn off receiver lighting: {error}");
                                if can_notify {
                                    let _ = app
                                        .notification()
                                        .builder()
                                        .title(tr_night_mode_title(lang))
                                        .body(tr_night_mode_off_failed_receiver(lang, &error))
                                        .show();
                                    did_notify = true;
                                }
                                any_failed = true;
                            }
                        }
                    }
                    // effect 本就为 0：不保存状态。
                }
            }

            if !any_failed {
                // 全部成功（或灯光本就关闭）：更新阶段为 Night。
                update_night_mode_phase(
                    app,
                    NightPhase::Night,
                    new_saved_mouse,
                    new_saved_receiver,
                );
            } else {
                // 有失败：保持阶段为 Day 以便重试，但持久化已成功部分的 saved，
                // 避免重试时已关闭的灯光因 enabled=false 被跳过而丢失 saved。
                update_night_mode_phase(app, NightPhase::Day, new_saved_mouse, new_saved_receiver);
            }
        }
        NightPhase::Day => {
            // Night → Day：恢复保存的灯光状态。
            if saved_mouse.is_none() && saved_receiver.is_none() {
                update_night_mode_phase(app, NightPhase::Day, None, None);
                return;
            }

            let snapshot = state.last_snapshot.lock().ok().and_then(|guard| {
                selected_snapshot_entry(&state, &guard).map(|(_, snapshot)| snapshot.clone())
            });

            let Some(snapshot) = snapshot else {
                // 设备未连接：不更新阶段，保持 Night + saved，下一轮重试恢复。
                return;
            };

            let mut any_failed = false;

            // 恢复阶段同样动态查询插件声明的 mutation 名（与关闭阶段保持一致）
            let (mouse_mutation, receiver_mutation) = resolve_lighting_mutations(&snapshot);

            // 恢复鼠标灯光
            if let (Some(saved), Some(mouse_mutation)) =
                (saved_mouse.as_ref(), mouse_mutation.as_deref())
            {
                if snapshot_supports_mutation(&snapshot, mouse_mutation) {
                    let mut params = serde_json::Map::from_iter([
                        (
                            "color".into(),
                            serde_json::Value::String(saved.color.clone()),
                        ),
                        ("enabled".into(), serde_json::Value::Bool(true)),
                        (
                            "effect".into(),
                            serde_json::Value::Number(saved.effect.into()),
                        ),
                        (
                            "speed".into(),
                            serde_json::Value::Number(saved.speed.into()),
                        ),
                        (
                            "brightness".into(),
                            serde_json::Value::Number(saved.brightness.into()),
                        ),
                    ]);
                    if let Some(ref ec) = saved.extra_color {
                        params.insert("extraColor".into(), serde_json::Value::String(ec.clone()));
                    }
                    if let Err(error) = device_mutate_blocking(app, mouse_mutation, &params) {
                        eprintln!("[mira] night mode: failed to restore mouse lighting: {error}");
                        if can_notify {
                            let _ = app
                                .notification()
                                .builder()
                                .title(tr_night_mode_title(lang))
                                .body(tr_night_mode_restore_failed(lang, &error))
                                .show();
                            did_notify = true;
                        }
                        any_failed = true;
                    }
                }
            }

            // 恢复接收器灯光
            if let (Some(saved), Some(receiver_mutation)) =
                (saved_receiver.as_ref(), receiver_mutation.as_deref())
            {
                if snapshot_supports_mutation(&snapshot, receiver_mutation) {
                    let params = serde_json::Map::from_iter([
                        (
                            "effect".into(),
                            serde_json::Value::Number(saved.effect.into()),
                        ),
                        (
                            "speed".into(),
                            serde_json::Value::Number(saved.speed.into()),
                        ),
                        (
                            "brightness".into(),
                            serde_json::Value::Number(saved.brightness.into()),
                        ),
                        (
                            "option".into(),
                            serde_json::Value::Number(saved.option.into()),
                        ),
                        (
                            "color".into(),
                            serde_json::Value::String(saved.color.clone()),
                        ),
                    ]);
                    if let Err(error) = device_mutate_blocking(app, receiver_mutation, &params) {
                        eprintln!(
                            "[mira] night mode: failed to restore receiver lighting: {error}"
                        );
                        if can_notify {
                            let _ = app
                                .notification()
                                .builder()
                                .title(tr_night_mode_title(lang))
                                .body(tr_night_mode_restore_failed_receiver(lang, &error))
                                .show();
                            did_notify = true;
                        }
                        any_failed = true;
                    }
                }
            }

            if !any_failed {
                update_night_mode_phase(app, NightPhase::Day, None, None);
            }
            // 有失败：不更新阶段，下一轮重试。
        }
    }

    // 已展示失败通知时标记，使后续每分钟的重试静默，直到阶段成功切换后重置。
    if did_notify {
        if let Ok(mut guard) = state.night_mode.lock() {
            guard.notified = true;
        }
    }
}

/// 更新运行时阶段并同步持久化 saved 状态。
fn update_night_mode_phase(
    app: &AppHandle,
    phase: NightPhase,
    saved_mouse: Option<SavedMouseLight>,
    saved_receiver: Option<SavedReceiverLight>,
) {
    let state = app.state::<SessionState>();
    let store = NightModeStore {
        saved_mouse_light: saved_mouse.clone(),
        saved_receiver_light: saved_receiver.clone(),
    };
    {
        let mut guard = state.night_mode.lock().unwrap_or_else(|e| e.into_inner());
        // 阶段真正切换时清除失败通知标记，使下一次转换能再次通知；
        // 部分失败重试时 phase 不变，保留标记以抑制重复弹通知。
        if guard.phase != phase {
            guard.notified = false;
        }
        guard.phase = phase;
        guard.saved_mouse_light = saved_mouse;
        guard.saved_receiver_light = saved_receiver;
    }
    if let Err(error) = save_night_mode_state(app, &store) {
        eprintln!("[mira] night mode: failed to persist state: {error}");
    }
}

/// 安静灯光调度器线程：每分钟检查一次是否需要切换阶段。
///
/// 线程在 mpsc::recv_timeout 上等待，而非固定 sleep：
/// - settings_set 变更夜间模式设置时发送 wake 信号，立即重新评估。
/// - 无信号时每 60 秒检查一次，确保在时段边界及时切换。
fn spawn_night_mode_scheduler(app: AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    *app.state::<SessionState>()
        .night_mode_tx
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(tx);

    std::thread::spawn(move || loop {
        let settings = cached_settings(&app);
        let now = Local::now().time();
        let state = app.state::<SessionState>();
        // 先取出缓存值并释放锁，再 fallback 到 tray_theme_is_dark。
        // 后者内部会再次获取同一 mutex，若在持锁状态下调用会死锁
        //（std::sync::Mutex 不可重入）。
        let system_dark = {
            let cache = state.system_dark.lock().unwrap_or_else(|e| e.into_inner());
            *cache
        }
        .or_else(|| Some(tray_theme_is_dark(&app)));
        let snapshot = state
            .last_snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .next()
            .cloned();
        let target = should_be_night(&settings, now, system_dark, snapshot.as_ref());

        apply_night_mode_transition(&app, target);

        // 计算到下一分钟的整点，确保在时段边界及时检查。
        // 最多等 60 秒，避免设置变更时等待过久。
        let wait = {
            let secs_to_next_minute =
                60u64 - (Local::now().time().num_seconds_from_midnight() as u64 % 60);
            Duration::from_secs(secs_to_next_minute.min(60))
        };

        match rx.recv_timeout(wait) {
            Ok(()) => {
                while rx.try_recv().is_ok() {}
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    });
}

/// 解析或复用插件对应的 ProtocolPackage。
///
/// 插件文件在加载后内容不变，重复解析 JSON 会浪费 CPU。缓存按 `plugin_id::model`
/// 索引（型号覆盖加载 #2：不同型号产生不同的合并结果），在插件加载
/// （启动 / install_plugin_update）时整体清空。
fn get_or_parse_package(
    state: &SessionState,
    inspection: &PackageInspection,
    model: Option<&str>,
    files: &BTreeMap<String, Vec<u8>>,
    plugins: &CachedPlugins,
) -> Result<Arc<ProtocolPackage>, String> {
    // 缓存键：plugin_id + "::" + model，确保不同型号的合并结果独立缓存。
    let cache_key = match model {
        Some(m) if !m.is_empty() => format!("{}::{m}", inspection.plugin_id),
        _ => inspection.plugin_id.to_string(),
    };
    let dependencies = dependency_transport_files(inspection, plugins)?;
    if let Ok(mut cache) = state.cached_packages.lock() {
        if let Some(package) = cache.get(&cache_key) {
            return Ok(package.clone());
        }
        let package = Arc::new(ProtocolPackage::from_files_with_model_and_dependencies(
            files,
            model,
            &dependencies,
        )?);
        cache.insert(cache_key, package.clone());
        return Ok(package);
    }
    // 锁失败（中毒）—— 直接解析，不缓存。
    Ok(Arc::new(
        ProtocolPackage::from_files_with_model_and_dependencies(files, model, &dependencies)?,
    ))
}

/// 清空插件派生缓存。在插件集合变化时调用。
fn invalidate_package_cache(state: &SessionState) {
    if let Ok(mut cache) = state.cached_packages.lock() {
        cache.clear();
    }
    if let Ok(mut cache) = state.cached_plugin_locales.lock() {
        *cache = None;
    }
}

fn dependency_transport_files<'a>(
    inspection: &PackageInspection,
    plugins: &'a CachedPlugins,
) -> Result<Vec<&'a BTreeMap<String, Vec<u8>>>, String> {
    // 所有依赖（无论 reuseTransport 是否为 true）都校验存在性与版本要求；
    // 仅 reuseTransport=true 时才收集被依赖插件的 transports.json。
    // 当前实现不递归解析依赖的依赖（roadmap #12 仅承诺直接依赖的 transport 复用），
    // 因此不会出现循环依赖导致的无限递归；若未来支持传递依赖，需在此增加环检测。
    let mut transport_files = Vec::new();
    for dependency in &inspection.depends_on {
        let (found, _, files) = plugins
            .iter()
            .find(|(candidate, _, _)| candidate.plugin_id == dependency.plugin_id)
            .ok_or_else(|| {
                format!(
                    "plugin {} depends on missing plugin {}",
                    inspection.plugin_id, dependency.plugin_id
                )
            })?;
        if let Some(requirement) = &dependency.version {
            let requirement = semver::VersionReq::parse(requirement)
                .map_err(|error| format!("invalid dependency version requirement: {error}"))?;
            let version = semver::Version::parse(&found.version)
                .map_err(|error| format!("invalid dependency version: {error}"))?;
            if !requirement.matches(&version) {
                return Err(format!(
                    "plugin {} dependency {} version {} does not satisfy {}",
                    inspection.plugin_id, found.plugin_id, found.version, requirement
                ));
            }
        }
        if dependency.reuse_transport {
            transport_files.push(files);
        }
    }
    Ok(transport_files)
}

/// #9 防抖式 TTL：500ms 内的重复读取复用快照。
/// 防止窗口聚焦、托盘点击等短时间多次 RefreshNow 信号触发重复 HID 往返。
const READ_DEBOUNCE_TTL: Duration = Duration::from_millis(500);

/// 从多设备快照 map 中选择 primary 设备。
/// 优先返回真正开放写入的设备，否则返回第一个。
#[cfg(test)]
fn primary_snapshot(snapshots: &BTreeMap<String, DeviceSnapshot>) -> Option<&DeviceSnapshot> {
    primary_snapshot_entry(snapshots).map(|(_, snapshot)| snapshot)
}

fn primary_snapshot_entry(
    snapshots: &BTreeMap<String, DeviceSnapshot>,
) -> Option<(&String, &DeviceSnapshot)> {
    snapshots
        .iter()
        .find(|(_, snapshot)| snapshot_allows_writes(snapshot))
        .or_else(|| snapshots.iter().next())
}

fn selected_snapshot_entry<'a>(
    state: &SessionState,
    snapshots: &'a BTreeMap<String, DeviceSnapshot>,
) -> Option<(&'a String, &'a DeviceSnapshot)> {
    let selected = state
        .selected_device_path
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    selected
        .as_ref()
        .and_then(|path| snapshots.get_key_value(path))
        .or_else(|| primary_snapshot_entry(snapshots))
}

fn selected_snapshot(state: &SessionState) -> Option<DeviceSnapshot> {
    state.last_snapshot.lock().ok().and_then(|guard| {
        selected_snapshot_entry(state, &guard).map(|(_, snapshot)| snapshot.clone())
    })
}

fn selected_device_path(state: &SessionState) -> Option<String> {
    state
        .last_snapshot
        .lock()
        .ok()
        .and_then(|guard| selected_snapshot_entry(state, &guard).map(|(path, _)| path.clone()))
}

/// 替换整个快照 map，并返回内容是否变化。
fn store_snapshots(state: &SessionState, snapshots: &BTreeMap<String, DeviceSnapshot>) -> bool {
    if let Ok(mut selected) = state.selected_device_path.lock() {
        if selected
            .as_ref()
            .is_some_and(|path| !snapshots.contains_key(path))
        {
            *selected = None;
        }
    }
    let mut guard = state
        .last_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let changed = *guard != *snapshots;
    *guard = snapshots.clone();
    drop(guard);
    if changed {
        // 设备状态变化（连接/断开/灯光状态变化）时唤醒夜间模式调度器，
        // 确保设备连接后立即关闭/恢复灯光，不必等下一轮 60 秒检查。
        request_night_mode_eval(state);
    }
    changed
}

/// 更新单个设备的快照，避免 clone 整个 map 引发的读-改-写竞态。
/// 修复 #10：device_mutate_blocking 写入后只更新当前设备，不覆盖其他设备的快照。
fn store_snapshot(state: &SessionState, device_path: String, snapshot: DeviceSnapshot) {
    let mut guard = state
        .last_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.insert(device_path, snapshot);
    drop(guard);
}

/// 清空所有快照与设备会话缓存。
fn clear_snapshots(state: &SessionState) {
    if let Ok(mut selected) = state.selected_device_path.lock() {
        *selected = None;
    }
    let mut guard = state
        .last_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.clear();
    drop(guard);
    // #9 设备断开时清除 TTL 缓存。
    if let Ok(mut cache) = state.last_read_at.lock() {
        cache.clear();
    }
    // P1 设备断开时清除 feature index 缓存，确保重连后重新查询。
    if let Ok(mut cache) = state.feature_index_cache.lock() {
        cache.clear();
    }
    // UX3 设备断开时清除 onboard memory 缓存，避免重连后使用过期数据。
    if let Ok(mut cache) = state.onboard_memory_cache.lock() {
        cache.clear();
    }
    // 设备断开时清除 HID 句柄缓存，避免复用已失效的句柄。
    if let Ok(mut cache) = state.cached_handles.lock() {
        cache.clear();
    }
    // 设备断开时清除退避状态，重连后不受历史失败影响。
    if let Ok(mut backoff) = state.backoff_state.lock() {
        backoff.clear();
    }
    // 设备断开时清除诊断指标，避免累积离线设备数据。
    if let Ok(mut metrics) = state.device_metrics.lock() {
        metrics.clear();
    }
    if let Ok(mut errors) = state.last_read_errors.lock() {
        errors.clear();
    }
}

fn sanitized_device_key(device: &hid::MatchedDevice) -> String {
    format!(
        "device-{:04x}-{:04x}-{:02x}-{:02x}",
        device.vendor_id, device.product_id, device.usage_page, device.usage
    )
}

fn classify_device_read_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("used by another process")
        || lower.contains("access is denied")
        || lower.contains("access denied")
        || lower.contains("sharing violation")
        || lower.contains("busy")
    {
        "access".into()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout".into()
    } else if lower.contains("input report")
        || lower.contains("output report")
        || lower.contains("feature report")
    {
        "hid-io".into()
    } else {
        "protocol".into()
    }
}

fn device_read_diagnostic(device: &hid::MatchedDevice, error: String) -> DeviceReadDiagnostic {
    DeviceReadDiagnostic {
        plugin_id: device.plugin_id.clone(),
        family: device.family.clone(),
        evidence: device.evidence.clone(),
        connection: device.connection.clone(),
        device_key: sanitized_device_key(device),
        vendor_id: device.vendor_id,
        product_id: device.product_id,
        usage_page: device.usage_page,
        usage: device.usage,
        error_kind: classify_device_read_error(&error),
        error,
    }
}

fn record_device_read_error(state: &SessionState, device: &hid::MatchedDevice, error: String) {
    if let Ok(mut errors) = state.last_read_errors.lock() {
        errors.insert(device.path.clone(), device_read_diagnostic(device, error));
    }
}

fn clear_device_read_error(state: &SessionState, device_path: &str) {
    if let Ok(mut errors) = state.last_read_errors.lock() {
        errors.remove(device_path);
    }
}

// Production plugin signing key for hello-yunshu/mira-mouse-plugins.
// Replace with the real key id and public key after the first production release.
const PRODUCTION_KEY_ID: &str = "mira-plugins-2026-001";
const PRODUCTION_PUBLIC_KEY_HEX: &str =
    "eb80fdde2dc7ba507b6c8afbbf5a7de82e6219967edf1914ddb979d5601d39b3";

// Test packages are trusted in all builds until the first production release.
const TEST_KEY_ID: &str = "TEST-ONLY-mira-plugins";
const TEST_PUBLIC_KEY_HEX: &str =
    "00d34dac6e039baada3d3d9aa65390f2887d09d73b396af8434ecb29c233d666";

fn decode_key(hex_str: &str) -> VerifyingKey {
    let bytes = hex::decode(hex_str).expect("invalid hex in pubkey");
    let array: [u8; 32] = bytes.try_into().expect("pubkey must be 32 bytes");
    VerifyingKey::from_bytes(&array).expect("invalid ed25519 pubkey")
}

fn production_trust_store() -> TrustStore {
    let mut trust = TrustStore::default();
    trust.0.insert(
        PRODUCTION_KEY_ID.to_string(),
        decode_key(PRODUCTION_PUBLIC_KEY_HEX),
    );
    trust
        .0
        .insert(TEST_KEY_ID.to_string(), decode_key(TEST_PUBLIC_KEY_HEX));
    trust
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AboutInfo {
    name: String,
    version: String,
    identifier: String,
    platform: String,
    architecture: String,
    rust_version: String,
    build_date: String,
    git_commit: String,
    bundled_plugins: Vec<BundledPluginInfo>,
    contact: ContactLinks,
    updater_active: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSnapshotEntry {
    pub device_key: String,
    pub snapshot: DeviceSnapshot,
    pub selected: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundledPluginInfo {
    plugin_id: String,
    version: String,
    asset: String,
    sha256: String,
    publisher_key_id: String,
    release_tag: String,
    bundle_by_default: bool,
    signature_verified: bool,
    evidence: String,
    source: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ContactLinks {
    github: Option<String>,
    repository: Option<String>,
    x: Option<String>,
    telegram: Option<String>,
    developer_name: Option<String>,
    copyright: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    language: String,
    theme: String,
    autostart: bool,
    start_hidden: bool,
    tray_show_battery_title: bool,
    tray_include_receiver_battery: bool,
    tray_show_connection: bool,
    /// 托盘鼠标图标颜色： "white"（白色轮廓）、"black"（黑色轮廓）、"auto"（跟随菜单栏/系统外观）。
    /// 默认 "auto"。
    tray_icon_color: String,
    /// 托盘渲染模式："auto"（macOS 原生/其他平台动态图片）、"dynamic-image"、"static"。
    /// 默认 "auto"。当前未在设置页暴露，仅后端字段。
    tray_render_mode: String,
    low_battery_threshold: u8,
    night_mode_enabled: bool,
    night_mode_start: String,
    night_mode_end: String,
    /// 安静灯光触发场景：特定时间（时间组，与 trigger_theme 互斥）。
    night_mode_trigger_time: bool,
    /// 安静灯光触发场景：跟随系统主题（时间组，与 trigger_time 互斥）。
    night_mode_trigger_theme: bool,
    /// 跟随系统主题的方向：true=深色模式时关灯，false=浅色模式时关灯。
    night_mode_theme_dark: bool,
    /// 安静灯光触发场景：仅在充电时（状态组，可多选）。
    night_mode_trigger_charging: bool,
    /// 安静灯光触发场景：电量低于阈值时（状态组，可多选，复用 low_battery_threshold）。
    night_mode_trigger_low_battery: bool,
    /// 安静灯光控制对象：鼠标灯光。
    night_mode_target_mouse: bool,
    /// 安静灯光控制对象：接收器灯光（仅设备支持时可设置）。
    night_mode_target_receiver: bool,
    telemetry_disabled: bool,
    automatic_update_checks: bool,
    automatic_update_install: bool,
    automatic_plugin_update_checks: bool,
    /// 电量使用情况：是否记录电量历史。默认开启。
    battery_history_enabled: bool,
    /// 电量历史保留天数。默认 30 天。
    battery_history_retention_days: u16,
    /// 异常耗电提醒：当设备掉电速度明显高于平时时通过现有通知方式提醒。
    unusual_drain_alerts: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SoftwareProfileStore {
    schema_version: u32,
    devices: BTreeMap<String, SoftwareProfile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SoftwareProfile {
    mutations: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            theme: "system".into(),
            autostart: false,
            start_hidden: false,
            tray_show_battery_title: true,
            tray_include_receiver_battery: false,
            tray_show_connection: true,
            tray_icon_color: "auto".into(),
            tray_render_mode: "auto".into(),
            low_battery_threshold: 20,
            night_mode_enabled: false,
            night_mode_start: "22:00".into(),
            night_mode_end: "07:00".into(),
            night_mode_trigger_time: true,
            night_mode_trigger_theme: false,
            night_mode_theme_dark: true,
            night_mode_trigger_charging: false,
            night_mode_trigger_low_battery: false,
            night_mode_target_mouse: true,
            night_mode_target_receiver: false,
            telemetry_disabled: true,
            automatic_update_checks: true,
            automatic_update_install: false,
            automatic_plugin_update_checks: true,
            battery_history_enabled: true,
            battery_history_retention_days: 30,
            unusual_drain_alerts: false,
        }
    }
}

impl AppSettings {
    fn normalized(mut self) -> Self {
        let defaults = Self::default();
        if !matches!(self.language.as_str(), "auto" | "zh-CN" | "en") {
            self.language = defaults.language;
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = defaults.theme;
        }
        if !matches!(self.tray_icon_color.as_str(), "white" | "black" | "auto") {
            self.tray_icon_color = defaults.tray_icon_color;
        }
        if !matches!(
            self.tray_render_mode.as_str(),
            "auto" | "native-macos" | "dynamic-image" | "static"
        ) {
            self.tray_render_mode = defaults.tray_render_mode;
        }
        if !(5..=50).contains(&self.low_battery_threshold) {
            self.low_battery_threshold = defaults.low_battery_threshold;
        }
        if !(1..=90).contains(&self.battery_history_retention_days) {
            self.battery_history_retention_days = defaults.battery_history_retention_days;
        }
        if !is_clock_time(&self.night_mode_start) {
            self.night_mode_start = defaults.night_mode_start;
        }
        if !is_clock_time(&self.night_mode_end) {
            self.night_mode_end = defaults.night_mode_end;
        }
        // 时间组互斥：trigger_time 和 trigger_theme 不能同时为 true。
        // 如果用户同时启用了两个，保留 trigger_time，禁用 trigger_theme。
        if self.night_mode_trigger_time && self.night_mode_trigger_theme {
            self.night_mode_trigger_theme = false;
        }
        // 灯光对象至少要勾选一个；如果都未勾选，恢复默认（鼠标灯光）。
        if !self.night_mode_target_mouse && !self.night_mode_target_receiver {
            self.night_mode_target_mouse = defaults.night_mode_target_mouse;
        }
        self.telemetry_disabled = true;
        self
    }
}

fn is_clock_time(value: &str) -> bool {
    let Some((hour, minute)) = value.split_once(':') else {
        return false;
    };
    hour.len() == 2
        && minute.len() == 2
        && hour.parse::<u8>().is_ok_and(|value| value < 24)
        && minute.parse::<u8>().is_ok_and(|value| value < 60)
}

/// 将 "HH:MM" 解析为 NaiveTime。已通过 is_clock_time 校验的值不会失败。
fn parse_clock_time(value: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(value, "%H:%M").ok()
}

/// 判断当前时间是否处于夜间时段 [start, end)。
///
/// 支持跨午夜时段（如 22:00→07:00）：当 start > end 时，
/// 时间 >= start 或 < end 都算在夜间。
/// 当 start < end 时，正常区间判断。
/// 当 start == end 时，视为全天夜间（返回 true）。
fn is_in_night_window(start: NaiveTime, end: NaiveTime, now: NaiveTime) -> bool {
    if start == end {
        return true;
    }
    if start < end {
        now >= start && now < end
    } else {
        // 跨午夜：22:00→07:00 → now >= 22:00 || now < 07:00
        now >= start || now < end
    }
}

/// 根据设置和当前设备/系统状态判断是否应当处于夜间阶段。
///
/// 触发场景（任一满足即返回 Night，全部不满足返回 Day）：
/// - 时间组（互斥）：特定时间 / 跟随系统主题
/// - 状态组（可多选）：仅在充电时 / 电量低于阈值时
///
/// `system_dark` 为 None 时视为浅色模式（不触发跟随系统主题）。
/// `snapshot` 为 None 时跳过状态类触发（设备未连接）。
fn should_be_night(
    settings: &AppSettings,
    now: NaiveTime,
    system_dark: Option<bool>,
    snapshot: Option<&DeviceSnapshot>,
) -> NightPhase {
    if !settings.night_mode_enabled {
        return NightPhase::Day;
    }

    // 时间组（互斥）：trigger_time 优先于 trigger_theme。
    let time_trigger = if settings.night_mode_trigger_time {
        let (Some(start), Some(end)) = (
            parse_clock_time(&settings.night_mode_start),
            parse_clock_time(&settings.night_mode_end),
        ) else {
            return NightPhase::Day;
        };
        is_in_night_window(start, end, now)
    } else if settings.night_mode_trigger_theme {
        // theme_dark=true 表示深色模式时关灯；system_dark 为 None 时视为不匹配。
        system_dark == Some(settings.night_mode_theme_dark)
    } else {
        false
    };

    // 状态组（可多选）：需要设备快照。
    let charging_trigger = settings.night_mode_trigger_charging
        && snapshot
            .map(tray::state::mouse_battery_charging)
            .unwrap_or(false);
    let low_battery_trigger = settings.night_mode_trigger_low_battery
        && snapshot
            .and_then(tray::state::mouse_battery_percentage)
            .map(|p| p < settings.low_battery_threshold)
            .unwrap_or(false);

    if time_trigger || charging_trigger || low_battery_trigger {
        NightPhase::Night
    } else {
        NightPhase::Day
    }
}

/// 从设备快照的标准 `mouseLighting` capability 中读取鼠标灯光状态。
///
/// 返回 (enabled, color_hex)：
/// - enabled: 来自 capabilities.mouseLighting.enabled
/// - color_hex: 来自 capabilities.mouseLighting.color（运行时 rgb parser 输出 #RRGGBB 字符串）
///
/// 如果字段缺失或类型不符，返回 None（调用方应跳过夜间模式操作）。
fn read_mouse_light_state(snapshot: &DeviceSnapshot) -> Option<(bool, SavedMouseLight)> {
    let lighting = snapshot.capabilities.get("mouseLighting")?;
    let off_effect = mouse_lighting_off_effect(snapshot);
    let enabled = lighting
        .get("enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let color = lighting
        .get("color")
        .and_then(|v| v.as_str())
        .filter(|s| s.starts_with('#') && s.len() == 7)
        .map(|s| s.to_string());
    // 读取扩展灯效参数（HID++ 插件提供；amaster 等旧插件无这些字段时使用默认值）。
    let effect = lighting
        .get("effect")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8);
    let speed = lighting
        .get("speed")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8)
        .unwrap_or(0);
    let brightness = lighting
        .get("brightness")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8)
        .unwrap_or(100);
    let extra_color = lighting
        .get("extraColor")
        .and_then(|v| v.as_str())
        .filter(|s| s.starts_with('#') && s.len() == 7)
        .map(|s| s.to_string());
    // 判断灯光是否开启：优先用插件声明的 off effect（HID++ 设备），无 effect 时回退到 enabled。
    let is_on = effect.map(|e| e != off_effect).unwrap_or(enabled);
    if is_on {
        // 灯光开启：必须有有效颜色才能正确保存并恢复，否则跳过该目标，
        // 避免 fallback #000000 在退出夜间恢复时覆盖设备原色。
        let color = color?;
        Some((
            true,
            SavedMouseLight {
                color,
                effect: effect.unwrap_or(1),
                speed,
                brightness,
                extra_color,
            },
        ))
    } else {
        // 灯光关闭：颜色无意义，用 #000000 仅作为关闭写入的占位参数。
        Some((
            false,
            SavedMouseLight {
                color: color.unwrap_or_else(|| "#000000".to_string()),
                effect: effect.unwrap_or(off_effect),
                speed,
                brightness,
                extra_color,
            },
        ))
    }
}

fn mouse_lighting_off_effect(snapshot: &DeviceSnapshot) -> u8 {
    snapshot
        .plugin_capabilities
        .iter()
        .find(|capability| capability.control == "LightingZone")
        .and_then(|capability| capability.metadata.get("effectOptions"))
        .and_then(|effect_options| effect_options.get("offValue"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or(0)
}

fn launch_args_request_hidden<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter().any(|arg| {
        matches!(
            arg.as_ref().to_str(),
            Some(AUTOSTART_HIDDEN_ARG | "--minimized")
        )
    })
}

fn should_start_hidden_for_args<I, S>(settings: &AppSettings, args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    settings.start_hidden && launch_args_request_hidden(args)
}

fn should_start_hidden(settings: &AppSettings) -> bool {
    should_start_hidden_for_args(settings, std::env::args_os())
}

fn autostart_args(settings: &AppSettings) -> Vec<&'static str> {
    if settings.start_hidden {
        vec![AUTOSTART_HIDDEN_ARG]
    } else {
        Vec::new()
    }
}

/// 从设备快照的 capabilities 中读取接收器灯光状态。
///
/// 返回 SavedReceiverLight，用于进入夜间时保存、退出时恢复。
/// set-receiver-lighting mutation 没有 enabled 字段，关闭时 effect=0，
/// 因此需要保存 effect/speed/brightness/option/color 完整状态。
///
/// 如果字段缺失或类型不符，返回 None。
fn read_receiver_light_state(snapshot: &DeviceSnapshot) -> Option<SavedReceiverLight> {
    let receiver = snapshot.capabilities.get("receiverLighting")?;
    let effect = receiver.get("effect")?.as_u64()? as u8;
    let speed = receiver.get("speed")?.as_u64()? as u8;
    let brightness = receiver.get("brightness")?.as_u64()? as u8;
    let option = receiver
        .get("option")
        .or_else(|| receiver.get("type"))
        .and_then(|value| value.as_u64())
        .unwrap_or(effect as u64) as u8;
    let color = receiver
        .get("color")
        .and_then(|v| v.as_str())
        .filter(|s| s.starts_with('#') && s.len() == 7)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "#000000".to_string());
    Some(SavedReceiverLight {
        effect,
        speed,
        brightness,
        option,
        color,
    })
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn defaults_match_the_frontend_contract() {
        let settings = AppSettings::default();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.low_battery_threshold, 20);
        assert!(settings.telemetry_disabled);
        assert!(!settings.start_hidden);
        assert!(settings.automatic_update_checks);
        assert!(!settings.automatic_update_install);
        assert!(settings.automatic_plugin_update_checks);
        assert_eq!(settings.battery_history_retention_days, 30);
    }

    #[test]
    fn launch_arguments_only_hide_startup_window_when_enabled_in_settings() {
        assert!(!launch_args_request_hidden(["mira"]));
        assert!(!launch_args_request_hidden(["mira", "--updated"]));
        assert!(launch_args_request_hidden(["mira", AUTOSTART_HIDDEN_ARG]));
        assert!(launch_args_request_hidden(["mira", "--minimized"]));

        let mut settings = AppSettings {
            start_hidden: false,
            ..Default::default()
        };
        assert!(!should_start_hidden_for_args(
            &settings,
            ["mira", AUTOSTART_HIDDEN_ARG]
        ));
        settings.start_hidden = true;
        assert!(should_start_hidden_for_args(
            &settings,
            ["mira", AUTOSTART_HIDDEN_ARG]
        ));
    }

    #[test]
    fn invalid_saved_values_are_repaired() {
        let settings = AppSettings {
            theme: String::new(),
            tray_icon_color: "blue".into(),
            low_battery_threshold: 0,
            night_mode_start: "99:99".into(),
            night_mode_end: String::new(),
            telemetry_disabled: false,
            ..AppSettings::default()
        }
        .normalized();
        assert_eq!(settings, AppSettings::default());
    }

    #[test]
    fn seeds_standard_values_for_software_profile_reapply() {
        let reading = DeviceReading {
            dpi: Some(1850),
            dpi_stages: Some(vec![mira_core::DpiStage {
                value: 1850,
                color: "#9a8bd0".into(),
                enabled: true,
                active: true,
            }]),
            polling_rate_hz: Some(1000),
            capabilities: BTreeMap::from([("controlMode".into(), serde_json::json!({"mode": 2}))]),
            ..DeviceReading::default()
        };
        assert_eq!(control_mode(&reading), Some(2));
        // 契约化：capability 声明 mutation 名（mutationDecl 支持 string 或 string[]），
        // Host 不再硬编码 "set-dpi-value" / "set-polling-rate"。
        let capabilities = vec![
            Capability {
                id: "dpi".into(),
                control: Control::DpiStages,
                label_key: "capability.dpi".into(),
                read_only: false,
                placements: vec![],
                metadata: BTreeMap::from([(
                    "mutations".into(),
                    serde_json::json!({
                        "select": "set-dpi-stage",
                        "value": ["set-dpi-value", "set-dpi-value-extended"],
                    }),
                )]),
                probe: None,
                connections: None,
                min_firmware: None,
            },
            Capability {
                id: "polling-rate".into(),
                control: Control::Select,
                label_key: "capability.polling-rate".into(),
                read_only: false,
                placements: vec![],
                metadata: BTreeMap::from([(
                    "mutation".into(),
                    serde_json::json!("set-polling-rate"),
                )]),
                probe: None,
                connections: None,
                min_firmware: None,
            },
        ];
        let mut profile = SoftwareProfile::default();
        seed_standard_software_mutations(
            &mut profile,
            &reading,
            &["set-dpi-value".into(), "set-polling-rate".into()],
            &capabilities,
        );
        assert_eq!(profile.mutations["set-dpi-value"]["dpi"], 1850);
        assert_eq!(profile.mutations["set-polling-rate"]["rate"], 1000);
    }

    #[test]
    fn seed_standard_mutations_skips_when_capability_missing() {
        // 契约化验证：插件未声明 DpiStages/polling-rate capability 时，Host 不应
        // 凭空 seed 任何 mutation（避免硬编码字符串回退）。
        let reading = DeviceReading {
            dpi: Some(1850),
            polling_rate_hz: Some(1000),
            capabilities: BTreeMap::from([("controlMode".into(), serde_json::json!({"mode": 2}))]),
            ..DeviceReading::default()
        };
        let mut profile = SoftwareProfile::default();
        seed_standard_software_mutations(
            &mut profile,
            &reading,
            &["set-dpi-value".into(), "set-polling-rate".into()],
            &[],
        );
        assert!(profile.mutations.is_empty());
    }

    #[test]
    fn seed_standard_mutations_picks_first_supported_candidate() {
        // mutationDecl 候选数组：设备只支持第二个候选时，应选取第二个。
        let reading = DeviceReading {
            dpi: Some(3200),
            ..DeviceReading::default()
        };
        let capabilities = vec![Capability {
            id: "dpi".into(),
            control: Control::DpiStages,
            label_key: "capability.dpi".into(),
            read_only: false,
            placements: vec![],
            metadata: BTreeMap::from([(
                "mutations".into(),
                serde_json::json!({
                    "value": ["set-dpi-value-legacy", "set-dpi-value-v2"],
                }),
            )]),
            probe: None,
            connections: None,
            min_firmware: None,
        }];
        let mut profile = SoftwareProfile::default();
        seed_standard_software_mutations(
            &mut profile,
            &reading,
            &["set-dpi-value-v2".into()],
            &capabilities,
        );
        assert_eq!(profile.mutations["set-dpi-value-v2"]["dpi"], 3200);
        assert!(!profile.mutations.contains_key("set-dpi-value-legacy"));
    }

    #[test]
    fn default_tray_icon_is_decodable_and_transparent() {
        let icon = tauri::image::Image::from_bytes(tray::static_icon::static_tray_app_icon_bytes())
            .unwrap();
        assert_eq!((icon.width(), icon.height()), (64, 64));

        let mut alpha = icon.rgba().iter().skip(3).step_by(4);
        assert!(alpha.clone().any(|value| *value == 0));
        assert!(alpha.any(|value| *value > 0));
    }

    #[test]
    fn dark_app_icons_are_decodable_and_transparent() {
        let tray_icon = tauri::image::Image::from_bytes(
            tray::static_icon::static_tray_app_icon_bytes_for_theme(true),
        )
        .unwrap();
        assert_eq!((tray_icon.width(), tray_icon.height()), (64, 64));
        let mut tray_alpha = tray_icon.rgba().iter().skip(3).step_by(4);
        assert!(tray_alpha.clone().any(|value| *value == 0));
        assert!(tray_alpha.any(|value| *value > 0));

        let app_icon = tauri::image::Image::from_bytes(app_icon_bytes_for_theme(true)).unwrap();
        assert_eq!((app_icon.width(), app_icon.height()), (512, 512));
        let mut app_alpha = app_icon.rgba().iter().skip(3).step_by(4);
        assert!(app_alpha.clone().any(|value| *value == 0));
        assert!(app_alpha.any(|value| *value > 0));
    }

    #[test]
    fn device_writes_queue_and_release_after_completion() {
        let state = SessionState::default();
        std::thread::scope(|s| {
            let guard = begin_device_write(&state).unwrap();
            let handle = s.spawn(|| {
                // 并发写入排队等待，而非立即失败。
                let _queued = begin_device_write(&state).unwrap();
            });
            // 给排队线程一点时间进入等待。
            std::thread::sleep(Duration::from_millis(50));
            drop(guard);
            // guard 释放后，排队的写入应能获取锁并完成。
            handle.join().unwrap();
        });
    }

    #[test]
    fn tray_title_uses_primary_and_optional_receiver_batteries() {
        let mut snapshot = DeviceSnapshot {
            display_name: "AM INFINITY 8K MOUSE".into(),
            connection: mira_core::Connection::Wireless,
            battery_percent: Some(64),
            charging: false,
            batteries: vec![
                mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: "鼠标".into(),
                    percentage: 64,
                    charging: false,
                },
                mira_core::DeviceBattery {
                    id: "receiver".into(),
                    label: "接收器".into(),
                    percentage: 100,
                    charging: false,
                },
            ],
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: Default::default(),
            plugin_capabilities: Vec::new(),
            writable_mutations: Vec::new(),
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
            history_identity: None,
        };
        let mut settings = AppSettings::default();
        assert_eq!(battery_title(&snapshot, &settings).as_deref(), Some("64%"));
        assert!(!tray::state::mouse_battery_charging(&snapshot));
        snapshot.batteries[0].charging = true;
        assert!(tray::state::mouse_battery_charging(&snapshot));
        settings.tray_include_receiver_battery = true;
        assert_eq!(
            battery_title(&snapshot, &settings).as_deref(),
            Some("鼠 64% · 接 100%")
        );
        settings.tray_show_battery_title = false;
        assert_eq!(battery_title(&snapshot, &settings), None);
    }

    #[test]
    fn tray_battery_labels_translate_mock_keys() {
        assert_eq!(
            tr_battery_label("zh-CN", "mouse", "mock.mouseLabel"),
            "鼠标"
        );
        assert_eq!(
            tr_battery_label("zh-CN", "receiver", "mock.receiverLabel"),
            "接收器"
        );
        assert_eq!(tr_battery_label("en", "mouse", "mock.mouseLabel"), "Mouse");
        assert_eq!(
            tr_battery_label("en", "receiver", "mock.receiverLabel"),
            "Receiver"
        );
        assert_eq!(
            tr_battery_item(
                "zh-CN",
                &tr_battery_label("zh-CN", "receiver", "mock.receiverLabel"),
                100,
                true,
            ),
            "接收器电量：100% · 充电中"
        );
    }

    fn primary_test_snapshot(
        name: &str,
        evidence: &str,
        readonly: bool,
        writable_mutations: Vec<String>,
    ) -> DeviceSnapshot {
        DeviceSnapshot {
            display_name: name.into(),
            connection: mira_core::Connection::Wireless,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: Default::default(),
            plugin_capabilities: Vec::new(),
            writable_mutations,
            evidence: evidence.into(),
            readonly,
            plugin_id: None,
            history_identity: None,
        }
    }

    #[test]
    fn primary_snapshot_prefers_truly_writable_device() {
        let snapshots = BTreeMap::from([
            (
                "a".into(),
                primary_test_snapshot(
                    "readonly verified",
                    "hardware-verified",
                    true,
                    vec!["set-dpi".into()],
                ),
            ),
            (
                "b".into(),
                primary_test_snapshot(
                    "writable verified",
                    "hardware-verified",
                    false,
                    vec!["set-dpi".into()],
                ),
            ),
        ]);
        assert_eq!(
            primary_snapshot(&snapshots).map(|snapshot| snapshot.display_name.as_str()),
            Some("writable verified")
        );
    }

    #[test]
    fn primary_snapshot_falls_back_when_no_device_can_write() {
        let snapshots = BTreeMap::from([
            (
                "a".into(),
                primary_test_snapshot("first readonly", "hardware-verified", true, Vec::new()),
            ),
            (
                "b".into(),
                primary_test_snapshot("source only", "source-confirmed", false, Vec::new()),
            ),
        ]);
        assert_eq!(
            primary_snapshot(&snapshots).map(|snapshot| snapshot.display_name.as_str()),
            Some("first readonly")
        );
    }

    #[test]
    fn plugin_updates_only_offer_newer_semver() {
        let current = BTreeMap::from([
            ("mira.amaster".into(), "1.3.3".into()),
            ("mira.logitech-hidpp".into(), "0.6.1".into()),
        ]);
        let registry = PluginRegistry {
            schema_version: 1,
            plugins: vec![
                PluginRegistryEntry {
                    plugin_id: "mira.amaster".into(),
                    version: "1.3.5".into(),
                    release_tag: "plugin/amaster/v1.3.5".into(),
                    url: "https://github.com/hello-yunshu/mira-mouse-plugins/releases/download/test/amaster.mira-plugin".into(),
                    sha256: "0".repeat(64),
                    publisher_key_id: PRODUCTION_KEY_ID.into(),
                    notes: None,
                },
                PluginRegistryEntry {
                    plugin_id: "mira.logitech-hidpp".into(),
                    version: "0.6.1".into(),
                    release_tag: "plugin/logitech-hidpp/v0.6.1".into(),
                    url: "https://github.com/hello-yunshu/mira-mouse-plugins/releases/download/test/logitech.mira-plugin".into(),
                    sha256: "1".repeat(64),
                    publisher_key_id: PRODUCTION_KEY_ID.into(),
                    notes: None,
                },
            ],
        };
        let updates = plugin_updates_for_versions(&current, registry).unwrap();
        assert!(
            updates
                .iter()
                .find(|item| item.plugin_id == "mira.amaster")
                .unwrap()
                .update_available
        );
        assert!(
            !updates
                .iter()
                .find(|item| item.plugin_id == "mira.logitech-hidpp")
                .unwrap()
                .update_available
        );
    }

    #[test]
    fn mirror_candidates_main_repo_release_uses_hey_run_domain_replacement() {
        // mira-mouse 主仓库 release：hey.run 仅替换域名，3 个镜像全支持
        let url = "https://github.com/hello-yunshu/mira-mouse/releases/download/app/v0.6.6/Mira_macOS_0.6.6.app.tar.gz";
        let candidates = mirror_candidates(url);
        assert_eq!(candidates.len(), GITHUB_MIRRORS.len());
        // hey.run 优先级最高
        assert_eq!(candidates[0].0, "hey.run");
        assert_eq!(
            candidates[0].1,
            "https://github.hey.run/hello-yunshu/mira-mouse/releases/download/app/v0.6.6/Mira_macOS_0.6.6.app.tar.gz"
        );
        // direct：原始 URL
        assert_eq!(candidates[1].0, "direct");
        assert_eq!(candidates[1].1, url);
        // gh-proxy：前缀拼接
        assert!(candidates[2]
            .1
            .starts_with("https://gh-proxy.com/https://github.com/"));
    }

    #[test]
    fn mirror_candidates_plugin_release_uses_hey_run_domain_replacement() {
        // mira-mouse-plugins release：hey.run 仅替换域名，3 个镜像全支持
        let url = "https://github.com/hello-yunshu/mira-mouse-plugins/releases/download/release/v2026-07-11/mira-amaster-1.6.1.mira-plugin";
        let candidates = mirror_candidates(url);
        assert_eq!(candidates.len(), GITHUB_MIRRORS.len());
        // hey.run 优先级最高
        assert_eq!(candidates[0].0, "hey.run");
        assert_eq!(
            candidates[0].1,
            "https://github.hey.run/hello-yunshu/mira-mouse-plugins/releases/download/release/v2026-07-11/mira-amaster-1.6.1.mira-plugin"
        );
        // direct：原始 URL
        assert_eq!(candidates[1].0, "direct");
        assert_eq!(candidates[1].1, url);
        // gh-proxy：前缀拼接
        assert!(candidates[2]
            .1
            .starts_with("https://gh-proxy.com/https://github.com/"));
    }

    #[test]
    fn mirror_candidates_plugin_raw_uses_hey_run_raw_rewrite() {
        // mira-mouse-plugins raw：hey.run 重写为 /raw/ 格式
        let url = "https://raw.githubusercontent.com/hello-yunshu/mira-mouse-plugins/main/registry/index.json";
        let candidates = mirror_candidates(url);
        assert_eq!(candidates.len(), GITHUB_MIRRORS.len());
        // hey.run 优先级最高
        assert_eq!(candidates[0].0, "hey.run");
        assert_eq!(
            candidates[0].1,
            "https://github.hey.run/hello-yunshu/mira-mouse-plugins/raw/main/registry/index.json"
        );
        // direct：原始 URL
        assert_eq!(candidates[1].0, "direct");
        assert_eq!(candidates[1].1, url);
    }

    #[test]
    fn mirror_candidates_main_repo_raw_uses_hey_run_raw_rewrite() {
        // mira-mouse 主仓库 raw：hey.run 重写为 /raw/ 格式
        let url = "https://raw.githubusercontent.com/hello-yunshu/mira-mouse/main/README.md";
        let candidates = mirror_candidates(url);
        assert_eq!(candidates.len(), GITHUB_MIRRORS.len());
        // hey.run 优先级最高
        assert_eq!(candidates[0].0, "hey.run");
        assert_eq!(
            candidates[0].1,
            "https://github.hey.run/hello-yunshu/mira-mouse/raw/main/README.md"
        );
        // direct：原始 URL
        assert_eq!(candidates[1].0, "direct");
        assert_eq!(candidates[1].1, url);
    }

    #[test]
    fn mirror_candidates_non_hello_yunshu_repo_skips_hey_run() {
        // 非 hello-yunshu 仓库：hey.run 不支持，仅 direct + gh-proxy = 2 个镜像
        let url = "https://github.com/tauri-apps/tauri/releases/download/v2/tauri.tgz";
        let candidates = mirror_candidates(url);
        assert_eq!(candidates.len(), 2);
        assert!(!candidates.iter().any(|(name, _)| *name == "hey.run"));
        // direct 优先级最高（hey.run 被跳过）
        assert_eq!(candidates[0].0, "direct");
        assert_eq!(candidates[1].0, "gh-proxy.com");
    }

    #[test]
    fn mirror_candidates_passthrough_non_github_urls() {
        // 非 GitHub URL 不添加镜像前缀，原样返回单个候选
        let candidates = mirror_candidates("https://example.com/file.txt");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "direct");
        assert_eq!(candidates[0].1, "https://example.com/file.txt");
    }

    #[test]
    fn exportable_value_reads_capability_source() {
        let snapshot = DeviceSnapshot {
            display_name: "Test Mouse".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: Some(1600),
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: vec![PluginCapability {
                id: "dpi".into(),
                control: "Number".into(),
                label_key: "capability.dpi".into(),
                read_only: false,
                placements: Vec::new(),
                metadata: BTreeMap::from([(
                    "source".into(),
                    serde_json::Value::String("dpi".into()),
                )]),
                available: true,
                connections: None,
                min_firmware: None,
            }],
            writable_mutations: Vec::new(),
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
            history_identity: None,
        };
        assert_eq!(
            exportable_value(
                &snapshot,
                &ExportableField {
                    id: "dpi".into(),
                    export_key: "dpi".into(),
                    kind: None,
                    mutation: None,
                    param: None,
                    source: None,
                    sources: None,
                }
            ),
            Some(serde_json::json!(1600))
        );
    }

    #[test]
    fn mutation_for_exportable_uses_capability_metadata() {
        let capability = mira_plugin_runtime::Capability {
            id: "dpi".into(),
            control: mira_plugin_runtime::Control::Number,
            label_key: "capability.dpi".into(),
            read_only: false,
            placements: Vec::new(),
            metadata: BTreeMap::from([
                ("mutation".into(), serde_json::json!("set-dpi-value")),
                ("param".into(), serde_json::json!("dpi")),
            ]),
            probe: None,
            connections: None,
            min_firmware: None,
        };
        let inspection = PackageInspection {
            plugin_id: "test.plugin".into(),
            version: "1.0.0".into(),
            evidence: "hardware-verified".into(),
            signature_verified: true,
            writes_enabled: true,
            capabilities: vec![capability],
            exportable_fields: vec![],
            depends_on: vec![],
            file_count: 0,
        };
        assert_eq!(
            mutation_for_exportable(
                &inspection,
                &ExportableField {
                    id: "dpi".into(),
                    export_key: "dpi".into(),
                    kind: None,
                    mutation: None,
                    param: None,
                    source: None,
                    sources: None,
                }
            ),
            ("set-dpi-value".into(), "dpi".into())
        );
    }

    #[test]
    fn import_exportable_skips_receiver_lighting_when_mutation_is_not_writable() {
        let inspection = PackageInspection {
            plugin_id: "mira.amaster".into(),
            version: "1.3.5".into(),
            evidence: "hardware-verified".into(),
            signature_verified: true,
            writes_enabled: true,
            capabilities: Vec::new(),
            exportable_fields: Vec::new(),
            depends_on: Vec::new(),
            file_count: 0,
        };
        let snapshot = DeviceSnapshot {
            display_name: "AM35".into(),
            connection: mira_core::Connection::Wireless,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: Vec::new(),
            writable_mutations: vec!["set-mouse-lighting".into()],
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
            history_identity: None,
        };
        let field = ExportableField {
            id: "receiver-lighting".into(),
            export_key: "receiverLighting".into(),
            kind: Some("object".into()),
            mutation: Some("set-receiver-lighting".into()),
            param: None,
            source: None,
            sources: Some(BTreeMap::from([(
                "color".into(),
                "capabilities.receiverLighting.color".into(),
            )])),
        };

        assert!(import_params_for_exportable(
            &inspection,
            &snapshot,
            &field,
            &serde_json::json!({"color": "#AABBCC"})
        )
        .is_none());
    }

    #[test]
    fn import_exportable_keeps_receiver_lighting_when_mutation_is_writable() {
        let inspection = PackageInspection {
            plugin_id: "mira.amaster".into(),
            version: "1.3.5".into(),
            evidence: "hardware-verified".into(),
            signature_verified: true,
            writes_enabled: true,
            capabilities: Vec::new(),
            exportable_fields: Vec::new(),
            depends_on: Vec::new(),
            file_count: 0,
        };
        let snapshot = DeviceSnapshot {
            display_name: "Protocol A".into(),
            connection: mira_core::Connection::Wireless,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: Vec::new(),
            writable_mutations: vec!["set-receiver-lighting".into()],
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
            history_identity: None,
        };
        let field = ExportableField {
            id: "receiver-lighting".into(),
            export_key: "receiverLighting".into(),
            kind: Some("object".into()),
            mutation: Some("set-receiver-lighting".into()),
            param: None,
            source: None,
            sources: None,
        };

        let (mutation, params) = import_params_for_exportable(
            &inspection,
            &snapshot,
            &field,
            &serde_json::json!({"effect": 3, "color": "#AABBCC"}),
        )
        .expect("writable receiver lighting import");
        assert_eq!(mutation, "set-receiver-lighting");
        assert_eq!(params.get("effect"), Some(&serde_json::json!(3)));
        assert_eq!(params.get("color"), Some(&serde_json::json!("#AABBCC")));
    }
}

#[cfg(test)]
mod night_mode_tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;

    #[test]
    fn parse_clock_time_accepts_valid_values() {
        assert!(parse_clock_time("00:00").is_some());
        assert!(parse_clock_time("23:59").is_some());
        assert!(parse_clock_time("22:00").is_some());
        assert!(parse_clock_time("07:30").is_some());
    }

    #[test]
    fn parse_clock_time_rejects_invalid_values() {
        assert!(parse_clock_time("24:00").is_none());
        assert!(parse_clock_time("12:60").is_none());
        assert!(parse_clock_time("abc").is_none());
        assert!(parse_clock_time("").is_none());
        // Note: chrono's %H:%M parser accepts single-digit hours/minutes ("1:2"),
        // but is_clock_time enforces two-digit format, so settings are always
        // normalized to "01:02" before parse_clock_time is called.
    }

    #[test]
    fn night_window_wrap_around_midnight() {
        // 22:00 → 07:00 跨午夜
        let start = parse_clock_time("22:00").unwrap();
        let end = parse_clock_time("07:00").unwrap();

        // 夜间时段内
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("22:00").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("23:30").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("00:00").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("03:00").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("06:59").unwrap()
        ));

        // 日间时段
        assert!(!is_in_night_window(
            start,
            end,
            parse_clock_time("07:00").unwrap()
        ));
        assert!(!is_in_night_window(
            start,
            end,
            parse_clock_time("12:00").unwrap()
        ));
        assert!(!is_in_night_window(
            start,
            end,
            parse_clock_time("21:59").unwrap()
        ));
    }

    #[test]
    fn night_window_same_day() {
        // 09:00 → 17:00 同日
        let start = parse_clock_time("09:00").unwrap();
        let end = parse_clock_time("17:00").unwrap();

        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("09:00").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("12:00").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("16:59").unwrap()
        ));

        assert!(!is_in_night_window(
            start,
            end,
            parse_clock_time("08:59").unwrap()
        ));
        assert!(!is_in_night_window(
            start,
            end,
            parse_clock_time("17:00").unwrap()
        ));
        assert!(!is_in_night_window(
            start,
            end,
            parse_clock_time("23:00").unwrap()
        ));
    }

    #[test]
    fn night_window_full_day_when_start_equals_end() {
        let start = parse_clock_time("12:00").unwrap();
        let end = parse_clock_time("12:00").unwrap();
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("00:00").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("12:00").unwrap()
        ));
        assert!(is_in_night_window(
            start,
            end,
            parse_clock_time("23:59").unwrap()
        ));
    }

    #[test]
    fn target_phase_respects_disabled_setting() {
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = false;
        settings.night_mode_start = "22:00".into();
        settings.night_mode_end = "07:00".into();
        // 即使处于夜间时段，禁用时也返回 Day。
        assert_eq!(
            should_be_night(&settings, parse_clock_time("23:00").unwrap(), None, None),
            NightPhase::Day
        );
    }

    #[test]
    fn target_phase_computes_correctly_when_enabled() {
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = true;
        settings.night_mode_trigger_time = true;
        settings.night_mode_start = "22:00".into();
        settings.night_mode_end = "07:00".into();

        assert_eq!(
            should_be_night(&settings, parse_clock_time("23:00").unwrap(), None, None),
            NightPhase::Night
        );
        assert_eq!(
            should_be_night(&settings, parse_clock_time("03:00").unwrap(), None, None),
            NightPhase::Night
        );
        assert_eq!(
            should_be_night(&settings, parse_clock_time("12:00").unwrap(), None, None),
            NightPhase::Day
        );
    }

    #[test]
    fn target_phase_falls_back_to_day_on_invalid_time() {
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = true;
        settings.night_mode_trigger_time = true;
        settings.night_mode_start = "invalid".into();
        settings.night_mode_end = "07:00".into();
        assert_eq!(
            should_be_night(&settings, parse_clock_time("23:00").unwrap(), None, None),
            NightPhase::Day
        );
    }

    #[test]
    fn should_be_night_theme_trigger_dark_mode() {
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = true;
        settings.night_mode_trigger_time = false;
        settings.night_mode_trigger_theme = true;
        settings.night_mode_theme_dark = true; // 深色模式时关灯
                                               // 系统深色模式 → Night
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                Some(true),
                None
            ),
            NightPhase::Night
        );
        // 系统浅色模式 → Day
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                Some(false),
                None
            ),
            NightPhase::Day
        );
        // system_dark 为 None → Day
        assert_eq!(
            should_be_night(&settings, parse_clock_time("12:00").unwrap(), None, None),
            NightPhase::Day
        );
    }

    #[test]
    fn should_be_night_theme_trigger_light_mode() {
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = true;
        settings.night_mode_trigger_time = false;
        settings.night_mode_trigger_theme = true;
        settings.night_mode_theme_dark = false; // 浅色模式时关灯
                                                // 系统浅色模式 → Night
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                Some(false),
                None
            ),
            NightPhase::Night
        );
        // 系统深色模式 → Day
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                Some(true),
                None
            ),
            NightPhase::Day
        );
    }

    #[test]
    fn should_be_night_charging_trigger() {
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = true;
        settings.night_mode_trigger_time = false;
        settings.night_mode_trigger_charging = true;
        let mut snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: Some(80),
            charging: true,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            evidence: String::new(),
            readonly: false,
            writable_mutations: Vec::new(),
            plugin_capabilities: Vec::new(),
            plugin_id: None,
            history_identity: None,
        };
        // 充电中 → Night
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                None,
                Some(&snapshot)
            ),
            NightPhase::Night
        );
        // 未充电 → Day
        snapshot.charging = false;
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                None,
                Some(&snapshot)
            ),
            NightPhase::Day
        );
        snapshot.batteries.push(mira_core::DeviceBattery {
            id: "mouse".into(),
            label: "鼠标".into(),
            percentage: 80,
            charging: true,
        });
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                None,
                Some(&snapshot)
            ),
            NightPhase::Night
        );
    }

    #[test]
    fn should_be_night_low_battery_trigger() {
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = true;
        settings.night_mode_trigger_time = false;
        settings.night_mode_trigger_low_battery = true;
        settings.low_battery_threshold = 20;
        let mut snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: Some(15),
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            evidence: String::new(),
            readonly: false,
            writable_mutations: Vec::new(),
            plugin_capabilities: Vec::new(),
            plugin_id: None,
            history_identity: None,
        };
        // 电量 15% < 阈值 20% → Night
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                None,
                Some(&snapshot)
            ),
            NightPhase::Night
        );
        // 电量 20% == 阈值 → Day（不小于）
        snapshot.battery_percent = Some(20);
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                None,
                Some(&snapshot)
            ),
            NightPhase::Day
        );
        snapshot.battery_percent = None;
        snapshot.batteries.push(mira_core::DeviceBattery {
            id: "mouse".into(),
            label: "鼠标".into(),
            percentage: 15,
            charging: false,
        });
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                None,
                Some(&snapshot)
            ),
            NightPhase::Night
        );
    }

    #[test]
    fn should_be_night_or_combination() {
        // 特定时间（白天）OR 仅在充电时（充电中）→ Night
        let mut settings = AppSettings::default();
        settings.night_mode_enabled = true;
        settings.night_mode_trigger_time = true;
        settings.night_mode_trigger_charging = true;
        settings.night_mode_start = "22:00".into();
        settings.night_mode_end = "07:00".into();
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: Some(80),
            charging: true,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            evidence: String::new(),
            readonly: false,
            writable_mutations: Vec::new(),
            plugin_capabilities: Vec::new(),
            plugin_id: None,
            history_identity: None,
        };
        // 白天但充电中 → Night（OR）
        assert_eq!(
            should_be_night(
                &settings,
                parse_clock_time("12:00").unwrap(),
                None,
                Some(&snapshot)
            ),
            NightPhase::Night
        );
    }

    #[test]
    fn read_receiver_light_state_extracts_fields() {
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::from([(
                "receiverLighting".into(),
                serde_json::json!({
                    "effect": 3,
                    "speed": 2,
                    "brightness": 80,
                    "option": 1,
                    "color": "#00FF00"
                }),
            )]),
            evidence: String::new(),
            readonly: false,
            writable_mutations: Vec::new(),
            plugin_capabilities: Vec::new(),
            plugin_id: None,
            history_identity: None,
        };
        let saved = read_receiver_light_state(&snapshot).unwrap();
        assert_eq!(saved.effect, 3);
        assert_eq!(saved.speed, 2);
        assert_eq!(saved.brightness, 80);
        assert_eq!(saved.option, 1);
        assert_eq!(saved.color, "#00FF00");
    }

    #[test]
    fn read_receiver_light_state_defaults_option_to_effect() {
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::from([(
                "receiverLighting".into(),
                serde_json::json!({
                    "effect": 2,
                    "speed": 3,
                    "brightness": 4,
                    "color": "#00FF00"
                }),
            )]),
            evidence: String::new(),
            readonly: false,
            writable_mutations: Vec::new(),
            plugin_capabilities: Vec::new(),
            plugin_id: None,
            history_identity: None,
        };
        let saved = read_receiver_light_state(&snapshot).unwrap();
        assert_eq!(saved.option, 2);
    }

    #[test]
    fn read_receiver_light_state_returns_none_when_missing() {
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            evidence: String::new(),
            readonly: false,
            writable_mutations: Vec::new(),
            plugin_capabilities: Vec::new(),
            plugin_id: None,
            history_identity: None,
        };
        assert!(read_receiver_light_state(&snapshot).is_none());
    }

    #[test]
    fn normalized_enforces_time_trigger_mutual_exclusion() {
        let mut settings = AppSettings::default();
        settings.night_mode_trigger_time = true;
        settings.night_mode_trigger_theme = true;
        let normalized = settings.normalized();
        assert!(normalized.night_mode_trigger_time);
        assert!(!normalized.night_mode_trigger_theme);
    }

    #[test]
    fn normalized_ensures_at_least_one_light_target() {
        let mut settings = AppSettings::default();
        settings.night_mode_target_mouse = false;
        settings.night_mode_target_receiver = false;
        let normalized = settings.normalized();
        assert!(normalized.night_mode_target_mouse);
    }

    #[test]
    fn read_mouse_light_state_extracts_enabled_and_color() {
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::from([(
                "mouseLighting".into(),
                serde_json::json!({
                    "enabled": true,
                    "color": "#FF8800"
                }),
            )]),
            plugin_capabilities: Vec::new(),
            writable_mutations: vec!["set-mouse-lighting".into()],
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
            history_identity: None,
        };
        let (is_on, saved) = read_mouse_light_state(&snapshot).unwrap();
        assert!(is_on);
        assert_eq!(saved.color, "#FF8800");
        // 无 effect 字段时默认为 fixed(1)
        assert_eq!(saved.effect, 1);
    }

    #[test]
    fn read_mouse_light_state_returns_none_when_fields_missing() {
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: Vec::new(),
            writable_mutations: Vec::new(),
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
            history_identity: None,
        };
        assert!(read_mouse_light_state(&snapshot).is_none());
    }

    #[test]
    fn read_mouse_light_state_handles_missing_color_with_fallback() {
        let make_snapshot = |enabled: bool| DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::from([(
                "mouseLighting".into(),
                serde_json::json!({ "enabled": enabled }),
            )]),
            plugin_capabilities: Vec::new(),
            writable_mutations: Vec::new(),
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: None,
            history_identity: None,
        };
        // enabled=true 但颜色缺失：返回 None，跳过该目标，
        // 避免 fallback #000000 在退出夜间恢复时覆盖设备原色。
        assert!(read_mouse_light_state(&make_snapshot(true)).is_none());
        // enabled=false 且颜色缺失：返回 fallback 颜色（仅作为关闭写入的占位参数）。
        let (is_on, saved) = read_mouse_light_state(&make_snapshot(false)).unwrap();
        assert!(!is_on);
        assert_eq!(saved.color, "#000000");
    }

    #[test]
    fn read_mouse_light_state_uses_declared_off_effect() {
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::from([(
                "mouseLighting".into(),
                serde_json::json!({
                    "enabled": true,
                    "effect": 9,
                    "color": "#2266AA"
                }),
            )]),
            plugin_capabilities: vec![PluginCapability {
                id: "mouse-lighting".into(),
                control: "LightingZone".into(),
                label_key: "capability.mouse-lighting".into(),
                read_only: false,
                placements: Vec::new(),
                metadata: BTreeMap::from([(
                    "effectOptions".into(),
                    serde_json::json!({ "offValue": 9 }),
                )]),
                available: true,
                connections: None,
                min_firmware: None,
            }],
            writable_mutations: vec!["set-mouse-lighting".into()],
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: Some("mira.logitech-hidpp".into()),
            history_identity: None,
        };

        let (is_on, saved) = read_mouse_light_state(&snapshot).unwrap();
        assert!(!is_on);
        assert_eq!(saved.effect, 9);
        assert_eq!(mouse_lighting_off_effect(&snapshot), 9);
    }

    #[test]
    fn resolve_lighting_mutations_uses_lighting_role_only() {
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: vec![PluginCapability {
                id: "mouse-lighting".into(),
                control: "LightingZone".into(),
                label_key: "capability.mouse-lighting".into(),
                read_only: false,
                placements: Vec::new(),
                metadata: BTreeMap::from([
                    (
                        "lightingRole".into(),
                        serde_json::json!({ "mouse": "set-mouse-lighting" }),
                    ),
                    (
                        "mutations".into(),
                        serde_json::json!({
                            "mouse": ["set-mouse-lighting-onboard", "set-mouse-lighting"]
                        }),
                    ),
                ]),
                available: true,
                connections: None,
                min_firmware: None,
            }],
            writable_mutations: vec!["set-mouse-lighting-onboard".into()],
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: Some("mira.logitech-hidpp".into()),
            history_identity: None,
        };

        let (mouse, receiver) = resolve_lighting_mutations(&snapshot);
        assert_eq!(mouse.as_deref(), Some("set-mouse-lighting"));
        assert!(receiver.is_none());
    }

    #[test]
    fn resolve_lighting_mutations_falls_back_to_zones() {
        // 插件只声明 zones（声明式），没有 lightingRole：后端应从 zones 提取 mutation。
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: vec![PluginCapability {
                id: "lighting".into(),
                control: "LightingZone".into(),
                label_key: "capability.lighting".into(),
                read_only: false,
                placements: Vec::new(),
                metadata: BTreeMap::from([(
                    "zones".into(),
                    serde_json::json!([
                        {
                            "id": "mouse",
                            "labelKey": "lighting.mouse",
                            "fields": [
                                {"id": "enabled", "mutation": "set-mouse-lighting"},
                                {"id": "color", "mutation": "set-mouse-lighting"}
                            ]
                        },
                        {
                            "id": "receiver",
                            "labelKey": "lighting.receiver",
                            "fields": [
                                {"id": "effect", "mutation": "set-receiver-lighting"}
                            ]
                        }
                    ]),
                )]),
                available: true,
                connections: None,
                min_firmware: None,
            }],
            writable_mutations: vec!["set-mouse-lighting".into(), "set-receiver-lighting".into()],
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: Some("mira.test".into()),
            history_identity: None,
        };

        let (mouse, receiver) = resolve_lighting_mutations(&snapshot);
        assert_eq!(mouse.as_deref(), Some("set-mouse-lighting"));
        assert_eq!(receiver.as_deref(), Some("set-receiver-lighting"));
    }

    #[test]
    fn resolve_lighting_mutations_zones_picks_supported_candidate() {
        // zone 中多个 fields 声明了不同 mutation，应选取第一个被 writable 支持的。
        let snapshot = DeviceSnapshot {
            display_name: "Test".into(),
            connection: mira_core::Connection::Usb,
            battery_percent: None,
            charging: false,
            batteries: Vec::new(),
            dpi: None,
            dpi_stages: None,
            polling_rate_hz: None,
            supported_polling_rates_hz: None,
            profile: None,
            confirmed_light_color: None,
            capabilities: BTreeMap::new(),
            plugin_capabilities: vec![PluginCapability {
                id: "lighting".into(),
                control: "LightingZone".into(),
                label_key: "capability.lighting".into(),
                read_only: false,
                placements: Vec::new(),
                metadata: BTreeMap::from([(
                    "zones".into(),
                    serde_json::json!([
                        {
                            "id": "mouse",
                            "fields": [
                                {"id": "onboard", "mutation": "set-mouse-lighting-onboard"},
                                {"id": "runtime", "mutation": "set-mouse-lighting"}
                            ]
                        }
                    ]),
                )]),
                available: true,
                connections: None,
                min_firmware: None,
            }],
            // 设备只支持 runtime mutation，不支持 onboard
            writable_mutations: vec!["set-mouse-lighting".into()],
            evidence: "hardware-verified".into(),
            readonly: false,
            plugin_id: Some("mira.test".into()),
            history_identity: None,
        };

        let (mouse, receiver) = resolve_lighting_mutations(&snapshot);
        assert_eq!(mouse.as_deref(), Some("set-mouse-lighting"));
        assert!(receiver.is_none());
    }

    #[test]
    fn night_mode_runtime_default_is_day_with_no_saved_state() {
        let runtime = NightModeRuntime::default();
        assert_eq!(runtime.phase, NightPhase::Day);
        assert!(runtime.saved_mouse_light.is_none());
    }

    #[test]
    fn night_mode_store_round_trips_through_json() {
        let store = NightModeStore {
            saved_mouse_light: Some(SavedMouseLight {
                color: "#AB12CD".into(),
                effect: 5,
                speed: 128,
                brightness: 80,
                extra_color: Some("#00FF00".into()),
            }),
            saved_receiver_light: Some(SavedReceiverLight {
                effect: 3,
                speed: 2,
                brightness: 80,
                option: 1,
                color: "#FF0000".into(),
            }),
        };
        let json = serde_json::to_string(&store).unwrap();
        let restored: NightModeStore = serde_json::from_str(&json).unwrap();
        let m = restored.saved_mouse_light.unwrap();
        assert_eq!(m.color, "#AB12CD");
        assert_eq!(m.effect, 5);
        assert_eq!(m.speed, 128);
        assert_eq!(m.brightness, 80);
        assert_eq!(m.extra_color.as_deref(), Some("#00FF00"));
        let r = restored.saved_receiver_light.unwrap();
        assert_eq!(r.effect, 3);
        assert_eq!(r.color, "#FF0000");
    }

    #[test]
    fn night_mode_store_default_has_no_saved_state() {
        let store = NightModeStore::default();
        assert!(store.saved_mouse_light.is_none());
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use mira_plugin_runtime::{Capability, CapabilityProbe, Control};

    fn make_capability(
        id: &str,
        probe: Option<CapabilityProbe>,
        connections: Option<Vec<String>>,
    ) -> Capability {
        Capability {
            id: id.into(),
            control: Control::Toggle,
            label_key: format!("{id}.label"),
            read_only: false,
            placements: Vec::new(),
            metadata: BTreeMap::new(),
            probe,
            connections,
            min_firmware: None,
        }
    }

    fn make_inspection(capabilities: Vec<Capability>) -> PackageInspection {
        PackageInspection {
            plugin_id: "test.plugin".into(),
            version: "1.0.0".into(),
            evidence: "hardware-verified".into(),
            signature_verified: true,
            writes_enabled: true,
            capabilities,
            exportable_fields: vec![],
            depends_on: vec![],
            file_count: 0,
        }
    }

    #[test]
    fn normalize_connection_maps_aliases() {
        // 修复 #3：wireless/wireless-receiver 应归一化为 receiver
        assert_eq!(normalize_connection("wireless"), "receiver");
        assert_eq!(normalize_connection("wireless-receiver"), "receiver");
        assert_eq!(normalize_connection("usb"), "usb");
        assert_eq!(normalize_connection("bluetooth"), "bluetooth");
        assert_eq!(normalize_connection("unknown"), "unknown");
    }

    #[test]
    fn firmware_gate_marks_capability_unavailable_below_minimum() {
        let mut cap = make_capability("advanced", None, None);
        cap.min_firmware = Some("2.0.0".into());
        let inspection = make_inspection(vec![cap]);
        let outputs =
            BTreeMap::from([("firmware".into(), serde_json::json!({"version": "1.9.9"}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(!result[0].available);
    }

    #[test]
    fn firmware_gate_accepts_matching_version() {
        let mut cap = make_capability("advanced", None, None);
        cap.min_firmware = Some("2.0.0".into());
        let inspection = make_inspection(vec![cap]);
        let outputs =
            BTreeMap::from([("firmware".into(), serde_json::json!({"version": "2.1.0"}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(result[0].available);
    }

    #[test]
    fn dpi_range_is_enriched_from_workflow_mutation_inputs() {
        let mut cap = make_capability("dpi", None, None);
        cap.control = Control::DpiStages;
        cap.metadata = BTreeMap::from([(
            "mutations".into(),
            serde_json::json!({ "select": "set-dpi-stage", "value": "set-dpi-value" }),
        )]);
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let files = BTreeMap::from([(
            "protocol/workflows.json".into(),
            serde_json::to_vec(&serde_json::json!({
                "mutations": {
                    "test-family-set-dpi-stage": {
                        "inputs": { "stage": { "kind": "integer", "min": 1, "max": 8, "step": 1 } }
                    },
                    "test-family-set-dpi-value": {
                        "inputs": { "dpi": { "kind": "integer", "min": 50, "max": 30000, "step": 50 } }
                    }
                }
            }))
            .unwrap(),
        )]);

        let result = plugin_capabilities(
            &inspection,
            &outputs,
            "usb",
            Some(&files),
            Some("test-family"),
            &[],
        );

        assert_eq!(result[0].metadata.get("min"), Some(&serde_json::json!(50)));
        assert_eq!(
            result[0].metadata.get("max"),
            Some(&serde_json::json!(30000))
        );
        assert_eq!(result[0].metadata.get("step"), Some(&serde_json::json!(50)));
    }

    #[test]
    fn select_options_follow_the_current_writable_mutation_allowed_values() {
        let mut cap = make_capability("polling-rate", None, None);
        cap.control = Control::Select;
        cap.metadata = BTreeMap::from([
            (
                "mutation".into(),
                serde_json::json!(["set-polling-rate", "set-polling-rate-extended"]),
            ),
            ("param".into(), serde_json::json!("rate")),
            (
                "options".into(),
                serde_json::json!([
                    { "value": 125, "label": "125 Hz" },
                    { "value": 250, "label": "250 Hz" },
                    { "value": 500, "label": "500 Hz" },
                    { "value": 1000, "label": "1000 Hz" },
                    { "value": 2000, "label": "2000 Hz" },
                    { "value": 4000, "label": "4000 Hz" },
                    { "value": 8000, "label": "8000 Hz" }
                ]),
            ),
        ]);
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let files = BTreeMap::from([(
            "protocol/workflows.json".into(),
            serde_json::to_vec(&serde_json::json!({
                "mutations": {
                    "test-family-set-polling-rate": {
                        "inputs": { "rate": { "kind": "integer", "allowed": [125, 250, 500, 1000] } }
                    },
                    "test-family-set-polling-rate-extended": {
                        "inputs": { "rate": { "kind": "integer", "allowed": [125, 250, 500, 1000, 2000, 4000, 8000] } }
                    }
                }
            }))
            .unwrap(),
        )]);

        let result = plugin_capabilities(
            &inspection,
            &outputs,
            "usb",
            Some(&files),
            Some("test-family"),
            &["set-polling-rate".into()],
        );

        assert_eq!(
            result[0].metadata.get("options"),
            Some(&serde_json::json!([
                { "value": 125, "label": "125 Hz" },
                { "value": 250, "label": "250 Hz" },
                { "value": 500, "label": "500 Hz" },
                { "value": 1000, "label": "1000 Hz" }
            ]))
        );
    }

    #[test]
    fn capability_filter_uses_runtime_reported_connection() {
        let cap = make_capability("wired-only", None, Some(vec!["usb".into()]));
        let inspection = make_inspection(vec![cap]);
        let devices = hid::DevicesFile {
            schema_version: 1,
            devices: Vec::new(),
            hardware_verified_models: Vec::new(),
        };
        let device = hid::MatchedDevice {
            plugin_id: "test.plugin".into(),
            family: "mouse".into(),
            evidence: "hardware-verified".into(),
            connection: "wireless".into(),
            path: "test-path".into(),
            vendor_id: 1,
            product_id: 2,
            usage_page: 3,
            usage: 4,
            model: None,
            identity: None,
        };
        let reading = DeviceReading {
            connection: Some(ConnectionKind::Usb),
            ..DeviceReading::default()
        };

        let snapshot = build_device_snapshot(
            reading,
            &inspection,
            &devices,
            &device,
            SnapshotRuntimeContext {
                package_files: None,
                fallback_connection: mira_core::Connection::Wireless,
                writable_mutations: Vec::new(),
                mutation_result: None,
            },
        );

        assert_eq!(snapshot.connection, mira_core::Connection::Usb);
        assert!(snapshot.plugin_capabilities[0].available);
    }

    #[test]
    fn probe_zero_integer_marks_unavailable() {
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({"value": 0}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(!result[0].available);
    }

    #[test]
    fn probe_zero_float_marks_unavailable() {
        // 修复 #1：浮点 0.0 也应标记为不可用
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({"value": 0.0}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(!result[0].available);
    }

    #[test]
    fn probe_nonzero_marks_available() {
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({"value": 1600}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(result[0].available);
    }

    #[test]
    fn probe_missing_output_marks_unavailable() {
        // probe 引用的 output 整个不存在（workflow 未执行/被 skip_if_zero 跳过）：
        // fail-closed 标记不可用，避免设备不支持却显示为可用。
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(!result[0].available);
    }

    #[test]
    fn probe_missing_field_defaults_available() {
        // 向后兼容：output 存在但字段缺失时默认可用
        let cap = make_capability(
            "dpi",
            Some(CapabilityProbe {
                output: "dpi".into(),
                field: "value".into(),
            }),
            None,
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({}))]);
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(result[0].available);
    }

    #[test]
    fn no_probe_defaults_available() {
        let cap = make_capability("dpi", None, None);
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(result[0].available);
    }

    #[test]
    fn connections_receiver_alias_matches_wireless() {
        // 修复 #3：声明 ["receiver"] 应匹配 "wireless" 连接
        let cap = make_capability("lighting", None, Some(vec!["receiver".into()]));
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "wireless", None, None, &[]);
        assert!(result[0].available);
    }

    #[test]
    fn connections_receiver_alias_matches_wireless_receiver() {
        let cap = make_capability("lighting", None, Some(vec!["receiver".into()]));
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result =
            plugin_capabilities(&inspection, &outputs, "wireless-receiver", None, None, &[]);
        assert!(result[0].available);
    }

    #[test]
    fn connections_mismatch_marks_unavailable() {
        let cap = make_capability("lighting", None, Some(vec!["usb".into()]));
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "wireless", None, None, &[]);
        assert!(!result[0].available);
    }

    #[test]
    fn connections_multiple_matches() {
        let cap = make_capability(
            "lighting",
            None,
            Some(vec!["receiver".into(), "usb".into()]),
        );
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "usb", None, None, &[]);
        assert!(result[0].available);
    }

    #[test]
    fn no_connections_defaults_available() {
        let cap = make_capability("lighting", None, None);
        let inspection = make_inspection(vec![cap]);
        let outputs = BTreeMap::new();
        let result = plugin_capabilities(&inspection, &outputs, "wireless", None, None, &[]);
        assert!(result[0].available);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockFile {
    #[allow(dead_code)]
    schema_version: u32,
    release_ready: bool,
    plugins: Vec<LockPlugin>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockPlugin {
    plugin_id: String,
    #[allow(dead_code)]
    repository: String,
    release_tag: String,
    version: String,
    asset: String,
    sha256: String,
    publisher_key_id: String,
    #[allow(dead_code)]
    plugin_api: String,
    bundle_by_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginRegistry {
    schema_version: u32,
    plugins: Vec<PluginRegistryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginRegistryEntry {
    plugin_id: String,
    version: String,
    release_tag: String,
    url: String,
    sha256: String,
    publisher_key_id: String,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginUpdateInfo {
    plugin_id: String,
    current_version: String,
    available_version: Option<String>,
    release_tag: Option<String>,
    notes: Option<String>,
    update_available: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginInstallResult {
    plugin_id: String,
    version: String,
    previous_version: String,
    restarted_runtime: bool,
}

/// 下载资源，自动在多个 GitHub 镜像之间切换。
/// 国内访问 GitHub 常出现连接失败、超时或下载断流，本函数按镜像列表顺序尝试，
/// 任意候选成功即返回；所有候选失败时返回汇总错误。
fn fetch_bounded(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let candidates = mirror_candidates(url);
    let mut errors = Vec::new();
    for (name, candidate) in &candidates {
        match try_fetch_bounded(candidate, max_bytes) {
            Ok(bytes) => return Ok(bytes),
            Err(error) => errors.push(format!("{name} ({candidate}): {error}")),
        }
    }
    Err(format!(
        "all {} download source(s) failed: {}",
        candidates.len(),
        errors.join("; ")
    ))
}

/// 为 GitHub 资源 URL 构造镜像候选列表（原始 + 国内代理）。
/// 遍历 `GITHUB_MIRRORS` 中的重写函数，返回 `Some(url)` 的镜像按顺序纳入候选。
/// 非 GitHub URL 仅生成原始 URL 单个候选（不走镜像）。
fn mirror_candidates(url: &str) -> Vec<(&'static str, String)> {
    let is_github = url.starts_with("https://github.com/")
        || url.starts_with("https://raw.githubusercontent.com/")
        || url.starts_with("https://objects.githubusercontent.com/");
    if !is_github {
        return vec![("direct", url.to_string())];
    }
    GITHUB_MIRRORS
        .iter()
        .filter_map(|(name, rewrite)| rewrite(url).map(|rewritten| (*name, rewritten)))
        .collect()
}

/// 原始 GitHub URL，不做任何重写。
fn direct_mirror(url: &str) -> Option<String> {
    Some(url.to_string())
}

/// `github.hey.run` 用户专属加速域名。仅对 `hello-yunshu` 仓库生效：
/// - 所有 `hello-yunshu` 仓库的 release：替换域名（实测 ~3.4MB/s，比 GitHub 原始快 3 倍）。
/// - mira-mouse / mira-mouse-plugins 的 raw：替换域名并插入 `/raw/` 段。
///
/// 非 hello-yunshu 仓库不支持，返回 `None` 跳过。
fn hey_run_mirror(url: &str) -> Option<String> {
    // hello-yunshu 仓库的 release URL：仅替换域名
    // 覆盖 mira-mouse 和 mira-mouse-plugins（以及未来新增的同组织仓库）
    if url.starts_with("https://github.com/hello-yunshu/") && url.contains("/releases/") {
        return Some(url.replacen("https://github.com/", "https://github.hey.run/", 1));
    }
    // mira-mouse-plugins raw：替换域名 + 插入 /raw/
    if let Some(rest) =
        url.strip_prefix("https://raw.githubusercontent.com/hello-yunshu/mira-mouse-plugins/")
    {
        return Some(format!(
            "https://github.hey.run/hello-yunshu/mira-mouse-plugins/raw/{rest}"
        ));
    }
    // mira-mouse 主仓库 raw：替换域名 + 插入 /raw/
    if let Some(rest) =
        url.strip_prefix("https://raw.githubusercontent.com/hello-yunshu/mira-mouse/")
    {
        return Some(format!(
            "https://github.hey.run/hello-yunshu/mira-mouse/raw/{rest}"
        ));
    }
    // 非 hello-yunshu 仓库：hey.run 不支持，跳过
    None
}

/// `gh-proxy.com` 公共代理，前缀拼接方式。对所有 GitHub URL 生效。
fn gh_proxy_mirror(url: &str) -> Option<String> {
    Some(format!("https://gh-proxy.com/{url}"))
}

/// 从单个 URL 下载，带速度监控。
/// 连接超时(10s)、HTTP 错误、下载速度过低(<100KB/s 持续 5s)、总超时(180s)都会返回错误，
/// 由 `fetch_bounded` 触发到下一个镜像的 fallback。
fn try_fetch_bounded(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let mut response = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(180))
        .user_agent("Mira-Mouse-Updater")
        .build()
        .map_err(|error| format!("build HTTP client: {error}"))?
        .get(url)
        .send()
        .map_err(|error| format!("connect: {error}"))?
        .error_for_status()
        .map_err(|error| format!("HTTP status: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(format!("download exceeds {max_bytes} byte limit"));
    }
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut total_read: u64 = 0;
    let check_interval = std::time::Duration::from_secs(5);
    let min_speed: u64 = 100 * 1024; // 100 KB/s：正常镜像均 > 500KB/s
    let mut last_check = std::time::Instant::now();
    let mut last_total: u64 = 0;
    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|error| format!("read download: {error}"))?;
        if n == 0 {
            break;
        }
        total_read += n as u64;
        if total_read > max_bytes {
            return Err(format!("download exceeds {max_bytes} byte limit"));
        }
        bytes.extend_from_slice(&buffer[..n]);
        // 速度监控：每隔 5s 检查一次，速度低于阈值则中止当前下载
        if last_check.elapsed() >= check_interval {
            let elapsed = last_check.elapsed().as_secs_f64();
            let bytes_in_interval = total_read - last_total;
            let speed = bytes_in_interval as f64 / elapsed;
            if speed < min_speed as f64 {
                return Err(format!("download too slow: {:.0} B/s", speed));
            }
            last_check = std::time::Instant::now();
            last_total = total_read;
        }
    }
    Ok(bytes)
}

fn fetch_plugin_registry() -> Result<PluginRegistry, String> {
    let bytes = fetch_bounded(PLUGIN_REGISTRY_URL, MAX_REGISTRY_BYTES)?;
    let registry: PluginRegistry = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse plugin registry: {error}"))?;
    if registry.schema_version != 1 {
        return Err(format!(
            "unsupported plugin registry schema {}",
            registry.schema_version
        ));
    }
    Ok(registry)
}

fn plugin_updates_for_versions(
    current: &BTreeMap<String, String>,
    registry: PluginRegistry,
) -> Result<Vec<PluginUpdateInfo>, String> {
    let remote = registry
        .plugins
        .into_iter()
        .map(|entry| (entry.plugin_id.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    current
        .iter()
        .map(|(plugin_id, current_version)| {
            let current_semver = semver::Version::parse(current_version)
                .map_err(|error| format!("invalid installed version for {plugin_id}: {error}"))?;
            let candidate = remote.get(plugin_id);
            let update_available = candidate
                .map(|entry| {
                    semver::Version::parse(&entry.version)
                        .map(|version| version > current_semver)
                        .map_err(|error| {
                            format!("invalid registry version for {plugin_id}: {error}")
                        })
                })
                .transpose()?
                .unwrap_or(false);
            Ok(PluginUpdateInfo {
                plugin_id: plugin_id.clone(),
                current_version: current_version.clone(),
                available_version: candidate.map(|entry| entry.version.clone()),
                release_tag: candidate.map(|entry| entry.release_tag.clone()),
                notes: candidate.and_then(|entry| entry.notes.clone()),
                update_available,
            })
        })
        .collect()
}

fn installed_plugins_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("plugins"))
        .map_err(|error| format!("resolve plugin data directory: {error}"))
}

fn read_lock_file() -> Option<LockFile> {
    serde_json::from_slice(include_bytes!("../../plugins.lock.json")).ok()
}

fn bundled_plugins_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("resources").join("plugins"))
        .filter(|dir| dir.exists())
        .or_else(|| {
            let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/plugins");
            if local.exists() {
                Some(local)
            } else {
                None
            }
        })
}

fn inspect_bundled_plugins(app: &AppHandle, trust: &TrustStore) -> Vec<BundledPluginInfo> {
    let Some(lock) = read_lock_file() else {
        return Vec::new();
    };
    let Some(plugins_dir) = bundled_plugins_dir(app) else {
        return Vec::new();
    };
    let trust_store = production_trust_store();
    let combined_trust = if trust.0.is_empty() {
        &trust_store
    } else {
        trust
    };
    lock.plugins
        .into_iter()
        .filter(|plugin| plugin.bundle_by_default)
        .filter_map(|plugin| {
            let asset_path = plugins_dir.join(&plugin.asset);
            let bytes = fs::read(&asset_path).ok()?;
            let actual_sha = hex::encode(Sha256::digest(&bytes));
            let signature_verified = if actual_sha == plugin.sha256 {
                inspect_package(std::io::Cursor::new(&bytes), combined_trust, true)
                    .map(|inspection| inspection.signature_verified)
                    .unwrap_or(false)
            } else {
                false
            };
            Some(BundledPluginInfo {
                plugin_id: plugin.plugin_id,
                version: plugin.version,
                asset: plugin.asset,
                sha256: plugin.sha256,
                publisher_key_id: plugin.publisher_key_id,
                release_tag: plugin.release_tag,
                bundle_by_default: plugin.bundle_by_default,
                signature_verified,
                evidence: if signature_verified {
                    "signature-verified".to_string()
                } else {
                    "signature-unverified".to_string()
                },
                source: "bundled".to_string(),
            })
        })
        .collect()
}

/// Return cached bundled plugin info, computing it once on first access.
/// Bundled plugins are fixed at build time, so the SHA-256 and signature
/// verification only need to run once for the app's lifetime.
fn cached_bundled_plugins(app: &AppHandle) -> Vec<BundledPluginInfo> {
    let state = app.state::<SessionState>();
    if let Ok(mut cache) = state.cached_bundled_plugins.lock() {
        if let Some(ref plugins) = *cache {
            return plugins.clone();
        }
        let plugins = inspect_bundled_plugins(app, &production_trust_store());
        *cache = Some(plugins.clone());
        return plugins;
    }
    // Lock failed (poisoned) — compute directly without caching.
    inspect_bundled_plugins(app, &production_trust_store())
}

/// 从插件包的文件映射中提取 locale 数据。
/// 扫描 `locales/*.json` 文件，解析为 locale code → key → translation 映射。
fn extract_plugin_locales(
    files: &BTreeMap<String, Vec<u8>>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut locales = BTreeMap::new();
    for (path, bytes) in files {
        // 匹配 locales/zh-CN.json 或 locales/en.json 格式
        if let Some(locale) = path
            .strip_prefix("locales/")
            .and_then(|rest| rest.strip_suffix(".json"))
        {
            if let Ok(dict) = serde_json::from_slice::<BTreeMap<String, String>>(bytes) {
                locales.insert(locale.to_string(), dict);
            }
        }
    }
    locales
}

/// 从当前真正参与运行的插件集合读取 locale。
///
/// 不能先读内置包、再用文件名猜测已安装包：当已安装的新版插件覆盖内置
/// 版本时，那种顺序会把旧 locale 送给前端，进而让新声明的 labelKey 原样
/// 显示。`SessionState.plugins` 已是签名校验并按版本解析后的最终集合，直接
/// 使用其中的文件映射可保证 locale 与设备协议包完全一致。
fn load_all_plugin_locales(app: &AppHandle) -> PluginLocales {
    let mut result: PluginLocales = BTreeMap::new();
    let state = app.state::<SessionState>();
    let Ok(plugins) = state.plugins.lock() else {
        return result;
    };
    let Some(plugins) = plugins.as_ref() else {
        return result;
    };
    for (inspection, _, files) in plugins {
        let locales = extract_plugin_locales(files);
        if !locales.is_empty() {
            result.insert(inspection.plugin_id.clone(), locales);
        }
    }
    result
}

/// 返回缓存的插件 locale 数据，首次访问时计算。
fn cached_plugin_locales(app: &AppHandle) -> PluginLocales {
    let state = app.state::<SessionState>();
    if let Ok(mut cache) = state.cached_plugin_locales.lock() {
        if let Some(ref locales) = *cache {
            return locales.clone();
        }
        let locales = load_all_plugin_locales(app);
        *cache = Some(locales.clone());
        return locales;
    }
    load_all_plugin_locales(app)
}

#[tauri::command]
async fn plugin_locales(app: AppHandle) -> PluginLocales {
    cached_plugin_locales(&app)
}

fn active_plugins_info(app: &AppHandle) -> Vec<BundledPluginInfo> {
    let mut info = cached_bundled_plugins(app);
    let Ok(versions) = active_plugin_versions(app) else {
        return info;
    };
    let trust = production_trust_store();
    let Ok(directory) = installed_plugins_dir(app) else {
        return info;
    };
    for (plugin_id, version) in versions {
        if info
            .iter()
            .any(|plugin| plugin.plugin_id == plugin_id && plugin.version == version)
        {
            continue;
        }
        let Some(path) = find_installed_plugin_path(&directory, &plugin_id, &trust) else {
            continue;
        };
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok((inspection, files)) = extract_package(Cursor::new(&bytes), &trust, true) else {
            continue;
        };
        let publisher_key_id = files
            .get("plugin.json")
            .and_then(|manifest| serde_json::from_slice::<serde_json::Value>(manifest).ok())
            .and_then(|manifest| {
                manifest
                    .get("publisherKeyId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let installed = BundledPluginInfo {
            plugin_id: inspection.plugin_id.clone(),
            version: inspection.version,
            asset: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("installed.mira-plugin")
                .to_string(),
            sha256: hex::encode(Sha256::digest(&bytes)),
            publisher_key_id,
            release_tag: String::new(),
            bundle_by_default: false,
            signature_verified: inspection.signature_verified,
            evidence: inspection.evidence,
            source: "installed".to_string(),
        };
        if let Some(index) = info.iter().position(|plugin| plugin.plugin_id == plugin_id) {
            info[index] = installed;
        } else {
            info.push(installed);
        }
    }
    info
}

/// Load all bundled plugin packages that can be verified and extract their
/// `devices.json` descriptors. Used by the HID discovery path.
fn load_bundled_plugin_devices(
    app: &AppHandle,
) -> Vec<(
    mira_plugin_runtime::PackageInspection,
    hid::DevicesFile,
    std::collections::BTreeMap<String, Vec<u8>>,
)> {
    let Some(lock) = read_lock_file() else {
        return Vec::new();
    };
    let Some(plugins_dir) = bundled_plugins_dir(app) else {
        return Vec::new();
    };
    let trust = production_trust_store();
    let mut plugins: CachedPlugins = lock
        .plugins
        .into_iter()
        .filter(|plugin| plugin.bundle_by_default)
        .filter_map(|plugin| {
            let result = (|| -> Result<_, String> {
                let asset_path = plugins_dir.join(&plugin.asset);
                let bytes = fs::read(&asset_path)
                    .map_err(|error| format!("read {}: {error}", asset_path.display()))?;
                let actual_sha = hex::encode(Sha256::digest(&bytes));
                if actual_sha != plugin.sha256 {
                    return Err(format!(
                        "SHA-256 mismatch for {}: expected {}, got {actual_sha}",
                        plugin.asset, plugin.sha256
                    ));
                }
                let (_, files) = extract_package(Cursor::new(&bytes), &trust, true)
                    .map_err(|error| format!("extract {}: {error}", plugin.asset))?;
                let devices_bytes = files
                    .get("devices.json")
                    .ok_or_else(|| format!("{} has no devices.json", plugin.asset))?;
                let devices = hid::parse_devices_json(devices_bytes)?;
                let inspection = inspect_package(Cursor::new(&bytes), &trust, true)
                    .map_err(|error| format!("inspect {}: {error}", plugin.asset))?;
                if inspection.plugin_id != plugin.plugin_id || inspection.version != plugin.version
                {
                    return Err(format!(
                        "identity mismatch for {}: lock has {} {}, package has {} {}",
                        plugin.asset,
                        plugin.plugin_id,
                        plugin.version,
                        inspection.plugin_id,
                        inspection.version
                    ));
                }
                Ok((inspection, devices, files))
            })();
            match result {
                Ok(plugin) => Some(plugin),
                Err(error) => {
                    eprintln!("[mira] plugin load failed: {error}");
                    None
                }
            }
        })
        .collect();

    if let Ok(installed_dir) = installed_plugins_dir(app) {
        if let Ok(entries) = fs::read_dir(&installed_dir) {
            for backup in entries.flatten().map(|entry| entry.path()).filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.contains(".rollback."))
            }) {
                let recovered = fs::read(&backup)
                    .ok()
                    .and_then(|bytes| inspect_package(Cursor::new(bytes), &trust, true).ok());
                if let Some(inspection) = recovered {
                    if find_installed_plugin_path(&installed_dir, &inspection.plugin_id, &trust)
                        .is_none()
                    {
                        let target = installed_dir.join(format!(
                            "{}-{}.mira-plugin",
                            inspection.plugin_id, inspection.version
                        ));
                        if let Err(error) = fs::rename(&backup, target) {
                            eprintln!("[mira] plugin rollback recovery failed: {error}");
                        }
                    }
                }
            }
        }
        if let Ok(entries) = fs::read_dir(installed_dir) {
            for path in entries.flatten().map(|entry| entry.path()).filter(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("mira-plugin")
                    && !path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.contains(".rollback."))
            }) {
                let loaded = (|| -> Result<_, String> {
                    let bytes = fs::read(&path)
                        .map_err(|error| format!("read {}: {error}", path.display()))?;
                    let (inspection, files) = extract_package(Cursor::new(&bytes), &trust, true)
                        .map_err(|error| format!("verify {}: {error}", path.display()))?;
                    let devices = hid::parse_devices_json(
                        files
                            .get("devices.json")
                            .ok_or("installed plugin has no devices.json")?,
                    )?;
                    Ok((inspection, devices, files))
                })();
                match loaded {
                    Ok(installed) => {
                        let installed_version = semver::Version::parse(&installed.0.version).ok();
                        let replace = plugins
                            .iter()
                            .position(|plugin| plugin.0.plugin_id == installed.0.plugin_id)
                            .filter(|index| {
                                let current =
                                    semver::Version::parse(&plugins[*index].0.version).ok();
                                installed_version > current
                            });
                        if let Some(index) = replace {
                            plugins[index] = installed;
                        } else if !plugins
                            .iter()
                            .any(|plugin| plugin.0.plugin_id == installed.0.plugin_id)
                        {
                            plugins.push(installed);
                        }
                    }
                    Err(error) => eprintln!("[mira] installed plugin ignored: {error}"),
                }
            }
        }
    }
    plugins
}

fn load_contact_links() -> ContactLinks {
    // Project metadata is part of the application build, so embedding it keeps
    // development and packaged builds on the same path.
    let text = include_str!("../../config/project-metadata.toml");
    let mut github = None;
    let mut repository = None;
    let mut x = None;
    let mut telegram = None;
    let mut developer_name = None;
    let mut copyright = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("github_profile_url = ") {
            github = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("main_repository_url = ") {
            repository = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("x_profile_url = ") {
            x = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("telegram_profile_url = ") {
            telegram = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("developer_display_name = ") {
            developer_name = parse_toml_string(value);
        } else if let Some(value) = trimmed.strip_prefix("copyright_holder = ") {
            copyright = parse_toml_string(value);
        }
    }
    ContactLinks {
        github,
        repository,
        x,
        telegram,
        developer_name,
        copyright,
    }
}

fn parse_toml_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "\"\"" || trimmed == "''" {
        return None;
    }
    trimmed
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .map(|s| s.to_string())
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|v| v.strip_suffix('\''))
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

fn load_settings(app: &AppHandle) -> AppSettings {
    settings_path(app)
        .and_then(|path| fs::read_to_string(&path).ok())
        .and_then(|text| serde_json::from_str::<AppSettings>(&text).ok())
        .map(AppSettings::normalized)
        .unwrap_or_default()
}

/// Return cached settings if available, otherwise load from disk and cache.
/// The cache is populated on first access and updated whenever `settings_set`
/// is called. This avoids reading `settings.json` from disk on every poll.
fn cached_settings(app: &AppHandle) -> AppSettings {
    let state = app.state::<SessionState>();
    if let Ok(mut cache) = state.cached_settings.lock() {
        if let Some(settings) = cache.as_ref() {
            return settings.clone();
        }
        let settings = load_settings(app);
        *cache = Some(settings.clone());
        return settings;
    }
    // Lock failed (poisoned) — fall back to direct disk read.
    load_settings(app)
}

/// Update the cached settings. Called by `settings_set` after a successful save.
fn update_cached_settings(app: &AppHandle, settings: &AppSettings) {
    if let Ok(mut cache) = app.state::<SessionState>().cached_settings.lock() {
        *cache = Some(settings.clone());
    }
}

fn save_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app).ok_or_else(|| "config dir unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &text).map_err(|e| format!("write settings: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("commit settings: {e}"))?;
    Ok(())
}

fn night_mode_state_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("night_mode_state.json"))
}

fn load_night_mode_state(app: &AppHandle) -> NightModeStore {
    night_mode_state_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_night_mode_state(app: &AppHandle, store: &NightModeStore) -> Result<(), String> {
    let Some(path) = night_mode_state_path(app) else {
        return Err("config dir unavailable".to_string());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }
    let text = serde_json::to_string_pretty(store).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &text).map_err(|e| format!("write night mode state: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("commit night mode state: {e}"))?;
    Ok(())
}

fn software_profiles_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("device-profiles.json"))
}

fn load_software_profiles(app: &AppHandle) -> SoftwareProfileStore {
    software_profiles_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| SoftwareProfileStore {
            schema_version: 1,
            ..SoftwareProfileStore::default()
        })
}

/// Return cached software profiles, loading from disk on first access.
/// The cache is invalidated by `save_software_profiles` so subsequent reads
/// after a write always reflect the latest state.
fn cached_software_profiles(app: &AppHandle) -> SoftwareProfileStore {
    let state = app.state::<SessionState>();
    if let Ok(mut cache) = state.cached_software_profiles.lock() {
        if let Some(ref profiles) = *cache {
            return profiles.clone();
        }
        let profiles = load_software_profiles(app);
        *cache = Some(profiles.clone());
        return profiles;
    }
    load_software_profiles(app)
}

fn save_software_profiles(app: &AppHandle, profiles: &SoftwareProfileStore) -> Result<(), String> {
    let path = software_profiles_path(app).ok_or_else(|| "config dir unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config dir: {error}"))?;
    }
    let text = serde_json::to_string_pretty(profiles)
        .map_err(|error| format!("serialize device profiles: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|error| format!("write device profiles: {error}"))?;
    fs::rename(&tmp, &path).map_err(|error| format!("commit device profiles: {error}"))?;
    // Update the cache so the next read doesn't hit the disk.
    if let Ok(mut cache) = app.state::<SessionState>().cached_software_profiles.lock() {
        *cache = Some(profiles.clone());
    }
    Ok(())
}

fn device_config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("device-config.json"))
}

fn read_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.').try_fold(value, |current, segment| {
        current.as_object().and_then(|object| object.get(segment))
    })
}

fn exportable_value(
    snapshot: &DeviceSnapshot,
    field: &ExportableField,
) -> Option<serde_json::Value> {
    // 复合字段：从多个快照路径组合为 object
    if let Some(sources) = &field.sources {
        let snapshot_value = serde_json::to_value(snapshot).ok()?;
        let mut object = serde_json::Map::new();
        for (param, path) in sources {
            if let Some(value) = read_json_path(&snapshot_value, path) {
                object.insert(param.clone(), value.clone());
            }
        }
        return Some(serde_json::Value::Object(object));
    }
    // 显式声明的源路径
    if let Some(source) = &field.source {
        let snapshot_value = serde_json::to_value(snapshot).ok()?;
        return read_json_path(&snapshot_value, source).cloned();
    }
    // 回退：直接从 capabilities map 取值
    if let Some(value) = snapshot.capabilities.get(&field.id) {
        return Some(value.clone());
    }
    // 回退：从 capability metadata 的 source 路径取值
    let snapshot_value = serde_json::to_value(snapshot).ok()?;
    snapshot
        .plugin_capabilities
        .iter()
        .find(|capability| capability.id == field.id)
        .and_then(|capability| capability.metadata.get("source"))
        .and_then(serde_json::Value::as_str)
        .and_then(|path| read_json_path(&snapshot_value, path))
        .cloned()
}

fn current_plugin_for_selected_snapshot(
    app: &AppHandle,
) -> Result<(PackageInspection, DeviceSnapshot), String> {
    let state = app.state::<SessionState>();
    let (selected_path, snapshot) = {
        let guard = state
            .last_snapshot
            .lock()
            .map_err(|_| "device snapshot state unavailable".to_string())?;
        selected_snapshot_entry(&state, &guard)
            .map(|(path, snapshot)| (path.clone(), snapshot.clone()))
            .ok_or_else(|| "no device snapshot is available".to_string())?
    };
    let plugins_guard = state.plugins.lock().map_err(|_| "state lock failed")?;
    let plugins = plugins_guard.as_ref().ok_or("plugins not loaded")?;
    let mut hidapi_guard = state
        .cached_hidapi
        .lock()
        .map_err(|_| "HidApi cache unavailable")?;
    if hidapi_guard.is_none() {
        *hidapi_guard = Some(HidApi::new().map_err(|e| e.to_string())?);
    }
    let api = hidapi_guard.as_mut().unwrap();
    let _ = api.refresh_devices();
    let matched = hid::enumerate_matched_devices(api, plugins);
    let device = matched
        .iter()
        .find(|device| device.path == selected_path)
        .ok_or("selected device is no longer connected")?;
    let inspection = plugins
        .iter()
        .find(|(inspection, _, _)| inspection.plugin_id == device.plugin_id)
        .map(|(inspection, _, _)| inspection.clone())
        .ok_or("matched plugin is unavailable")?;
    Ok((inspection, snapshot))
}

#[tauri::command]
fn device_config_export(
    app: tauri::AppHandle,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    let (inspection, snapshot) = current_plugin_for_selected_snapshot(&app)?;
    if inspection.exportable_fields.is_empty() {
        return Err(format!(
            "plugin {} does not declare exportable fields",
            inspection.plugin_id
        ));
    }
    let mut fields = serde_json::Map::new();
    for field in &inspection.exportable_fields {
        if let Some(value) = exportable_value(&snapshot, field) {
            fields.insert(field.export_key.clone(), value);
        }
    }
    let config = serde_json::json!({
        "schemaVersion": 1,
        "pluginId": inspection.plugin_id,
        "pluginVersion": inspection.version,
        "device": snapshot.display_name,
        "fields": fields,
    });
    // #11 支持用户指定导出路径（文件选择器），未指定时用默认 app config 路径。
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => device_config_path(&app).ok_or_else(|| "config dir unavailable".to_string())?,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config dir: {error}"))?;
    }
    let text =
        serde_json::to_string_pretty(&config).map_err(|error| format!("serialize: {error}"))?;
    fs::write(&path, text).map_err(|error| format!("write device config: {error}"))?;
    Ok(config)
}

fn mutation_for_exportable(
    inspection: &PackageInspection,
    field: &ExportableField,
) -> (String, String) {
    // 优先使用字段声明的 mutation 和 param
    if let Some(mutation) = &field.mutation {
        let param = field.param.clone().unwrap_or_else(|| "value".to_string());
        return (mutation.clone(), param);
    }
    // 回退：从 capability metadata 推导
    let field_id = &field.id;
    for capability in &inspection.capabilities {
        let mutation = capability
            .metadata
            .get("mutation")
            .and_then(serde_json::Value::as_str);
        if &capability.id == field_id {
            if let Some(mutation) = mutation {
                let param = capability
                    .metadata
                    .get("param")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("value");
                return (mutation.to_string(), param.to_string());
            }
        }
        if mutation == Some(field_id.as_str()) {
            let param = capability
                .metadata
                .get("param")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("value");
            return (field_id.clone(), param.to_string());
        }
    }
    (field_id.clone(), "value".to_string())
}

fn import_params_for_exportable(
    inspection: &PackageInspection,
    snapshot: &DeviceSnapshot,
    field: &ExportableField,
    value: &serde_json::Value,
) -> Option<(String, serde_json::Map<String, serde_json::Value>)> {
    let (mutation, param) = mutation_for_exportable(inspection, field);
    if !snapshot_supports_mutation(snapshot, &mutation) {
        return None;
    }
    let params = match value {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::from_iter([(param, value.clone())]),
    };
    Some((mutation, params))
}

#[tauri::command]
fn device_config_import(
    app: tauri::AppHandle,
    path: Option<String>,
) -> Result<DeviceSnapshot, String> {
    // #11 支持用户指定导入路径（文件选择器），未指定时用默认 app config 路径。
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => device_config_path(&app).ok_or_else(|| "config dir unavailable".to_string())?,
    };
    let config: serde_json::Value = fs::read_to_string(&path)
        .map_err(|error| format!("read device config: {error}"))
        .and_then(|text| {
            serde_json::from_str(&text).map_err(|error| format!("parse device config: {error}"))
        })?;
    if config
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err("unsupported device config schema".into());
    }
    let fields = config
        .get("fields")
        .and_then(serde_json::Value::as_object)
        .ok_or("device config has no fields object")?;
    let (inspection, snapshot) = current_plugin_for_selected_snapshot(&app)?;
    if config.get("pluginId").and_then(serde_json::Value::as_str)
        != Some(inspection.plugin_id.as_str())
    {
        return Err("device config plugin does not match the connected device".into());
    }
    let mut latest = snapshot;
    let mut applied: Vec<String> = Vec::new();
    for field in &inspection.exportable_fields {
        let Some(value) = fields.get(&field.export_key) else {
            continue;
        };
        let Some((mutation, params)) =
            import_params_for_exportable(&inspection, &latest, field, value)
        else {
            continue;
        };
        match device_mutate_blocking(&app, &mutation, &params) {
            Ok(snapshot) => {
                latest = snapshot;
                applied.push(field.export_key.clone());
            }
            Err(error) => {
                // 修复 #11：导入逐字段写入，第 N 字段失败时前 N-1 个已生效。
                // 返回包含已成功字段列表的错误，让用户知道设备已部分变更。
                if applied.is_empty() {
                    return Err(error);
                }
                return Err(format!(
                    "导入在字段 {export_key} 失败：{error}。已成功导入字段：{applied_list}。建议重新导出当前配置或手动校准未导入字段。",
                    export_key = field.export_key,
                    applied_list = applied.join(", ")
                ));
            }
        }
    }
    Ok(latest)
}

fn control_mode(reading: &DeviceReading) -> Option<u8> {
    reading
        .capabilities
        .get("controlMode")
        .and_then(serde_json::Value::as_object)
        .and_then(|mode| mode.get("mode"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|mode| u8::try_from(mode).ok())
}

fn software_profile_key(device: &hid::MatchedDevice, reading: &DeviceReading) -> String {
    let unit_id = reading
        .capabilities
        .get("deviceInfo")
        .and_then(serde_json::Value::as_object)
        .and_then(|info| info.get("unitId"))
        .and_then(serde_json::Value::as_array)
        .map(|bytes| {
            bytes
                .iter()
                .filter_map(serde_json::Value::as_u64)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{:04x}:{:04x}", device.vendor_id, device.product_id));
    format!("{}:{}:{unit_id}", device.plugin_id, device.family)
}

fn seed_standard_software_mutations(
    profile: &mut SoftwareProfile,
    reading: &DeviceReading,
    allowed: &[String],
    capabilities: &[Capability],
) {
    // 契约化：mutation 名由插件 manifest 声明，Host 不再硬编码 "set-dpi-value" /
    // "set-polling-rate"。DpiStages capability 通过 metadata.mutations.value 声明
    // 写入 mutation（支持 mutationDecl: string 或 string[] 候选数组）；
    // polling-rate capability（labelKey='capability.polling-rate'）通过
    // metadata.mutation 声明。这与前端 pluginAdapter.ts 的 pluginMutations 契约一致。
    if let Some(mutation_name) = capabilities
        .iter()
        .find(|capability| capability.control == Control::DpiStages && !capability.read_only)
        .and_then(|capability| capability.metadata.get("mutations"))
        .and_then(|mutations| mutations.get("value"))
        .and_then(|value| pick_mutation_decl(value, allowed))
    {
        if let Some(dpi) = reading.dpi {
            let stage = reading
                .dpi_stages
                .as_ref()
                .and_then(|stages| stages.iter().position(|stage| stage.active))
                .map(|index| index + 1)
                .unwrap_or(1);
            profile.mutations.insert(
                mutation_name,
                BTreeMap::from([
                    ("dpi".into(), serde_json::json!(dpi)),
                    ("stage".into(), serde_json::json!(stage)),
                ]),
            );
        }
    }
    if let Some(mutation_name) = capabilities
        .iter()
        .find(|capability| {
            capability.label_key == "capability.polling-rate" && !capability.read_only
        })
        .and_then(|capability| capability.metadata.get("mutation"))
        .and_then(|value| pick_mutation_decl(value, allowed))
    {
        if let Some(rate) = reading.polling_rate_hz {
            profile.mutations.insert(
                mutation_name,
                BTreeMap::from([("rate".into(), serde_json::json!(rate))]),
            );
        }
    }
}

/// 从 mutationDecl（string 或 string[]）中选取第一个被设备支持的 mutation。
/// 与前端 pluginAdapter.ts 的 pickMutation 行为一致：
/// - string → 直接返回
/// - string[] → 优先返回第一个在 `allowed` 中的候选，否则返回首个候选
fn pick_mutation_decl(value: &serde_json::Value, allowed: &[String]) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = value.as_array() {
        let candidates: Vec<String> = arr
            .iter()
            .filter_map(|item| item.as_str().map(String::from))
            .collect();
        return candidates
            .iter()
            .find(|candidate| {
                allowed
                    .iter()
                    .any(|allowed_mutation| allowed_mutation == *candidate)
            })
            .or_else(|| candidates.first())
            .cloned();
    }
    None
}

fn parse_connection(value: &str) -> mira_core::Connection {
    match value {
        "usb" => mira_core::Connection::Usb,
        "wireless" | "wireless-receiver" => mira_core::Connection::Wireless,
        "bluetooth" => mira_core::Connection::Bluetooth,
        _ => mira_core::Connection::Usb,
    }
}

/// Parse a connection string and return both the semantic connection type and
/// the runtime `ConnectionKind` used by the protocol engine.
fn connection_kind(value: &str) -> (mira_core::Connection, ConnectionKind) {
    let connection = parse_connection(value);
    let kind = match connection {
        mira_core::Connection::Usb => ConnectionKind::Usb,
        mira_core::Connection::Wireless => ConnectionKind::Wireless,
        mira_core::Connection::Bluetooth => ConnectionKind::Bluetooth,
        mira_core::Connection::Virtual => ConnectionKind::Usb,
    };
    (connection, kind)
}

fn runtime_connection(value: ConnectionKind) -> mira_core::Connection {
    match value {
        ConnectionKind::Usb => mira_core::Connection::Usb,
        ConnectionKind::Wireless => mira_core::Connection::Wireless,
        ConnectionKind::Bluetooth => mira_core::Connection::Bluetooth,
    }
}

fn capability_connection_label(value: mira_core::Connection) -> &'static str {
    match value {
        mira_core::Connection::Usb => "usb",
        mira_core::Connection::Wireless => "receiver",
        mira_core::Connection::Bluetooth => "bluetooth",
        mira_core::Connection::Virtual => "usb",
    }
}

fn device_evidence_allows_writes(evidence: &str) -> bool {
    matches!(evidence, "hardware-verified" | "protocol-verified")
}

fn snapshot_allows_writes(snapshot: &DeviceSnapshot) -> bool {
    !snapshot.readonly
        && device_evidence_allows_writes(&snapshot.evidence)
        && !snapshot.writable_mutations.is_empty()
}

fn snapshot_supports_mutation(snapshot: &DeviceSnapshot, mutation: &str) -> bool {
    snapshot_allows_writes(snapshot)
        && snapshot
            .writable_mutations
            .iter()
            .any(|candidate| candidate == mutation)
}

/// 从设备快照的 plugin_capabilities 中查询灯光 mutation 名。
/// 优先读 metadata.lightingRole（旧版强类型字段），回退到 metadata.zones
/// （声明式 zones 中 id 为 'mouse'/'receiver' 的 fields[].mutation）。
/// 与 Capability::lighting_role() 的回退逻辑一致。
/// 返回 (mouse_mutation, receiver_mutation)，未声明则为 None。
fn pick_supported_lighting_mutation(
    value: Option<&serde_json::Value>,
    snapshot: &DeviceSnapshot,
) -> Option<String> {
    let value = value?;
    if let Some(mutation) = value.as_str() {
        return Some(mutation.to_string());
    }
    value.as_array().and_then(|items| {
        let candidates = items.iter().filter_map(|item| item.as_str());
        let mut first = None;
        for candidate in candidates {
            if first.is_none() {
                first = Some(candidate.to_string());
            }
            if snapshot_supports_mutation(snapshot, candidate) {
                return Some(candidate.to_string());
            }
        }
        first
    })
}

/// 从 zones 声明式字段提取灯光 mutation 角色映射。
/// 遍历 zone id 为 'mouse'/'receiver' 的 fields[].mutation，
/// 收集候选 mutation 名并选取第一个被设备支持的。
fn zone_lighting_mutations(
    zones: &[serde_json::Value],
    snapshot: &DeviceSnapshot,
) -> (Option<String>, Option<String>) {
    let mut mouse = Vec::new();
    let mut receiver = Vec::new();
    for zone in zones {
        let Some(zone) = zone.as_object() else {
            continue;
        };
        let Some(id) = zone.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(fields) = zone.get("fields").and_then(|v| v.as_array()) else {
            continue;
        };
        let mutations: Vec<serde_json::Value> = fields
            .iter()
            .filter_map(|f| f.get("mutation").cloned())
            .collect();
        match id {
            "mouse" => mouse.extend(mutations),
            "receiver" => receiver.extend(mutations),
            _ => {}
        }
    }
    let mouse = if mouse.is_empty() {
        None
    } else {
        pick_supported_lighting_mutation(Some(&serde_json::Value::Array(mouse)), snapshot)
    };
    let receiver = if receiver.is_empty() {
        None
    } else {
        pick_supported_lighting_mutation(Some(&serde_json::Value::Array(receiver)), snapshot)
    };
    (mouse, receiver)
}

fn resolve_lighting_mutations(snapshot: &DeviceSnapshot) -> (Option<String>, Option<String>) {
    for cap in &snapshot.plugin_capabilities {
        if cap.control != "LightingZone" {
            continue;
        }
        // 优先读 lightingRole 强类型字段（旧版兼容）
        let role = cap
            .metadata
            .get("lightingRole")
            .and_then(|value| value.as_object());
        let role_mouse =
            pick_supported_lighting_mutation(role.and_then(|value| value.get("mouse")), snapshot);
        let role_receiver = pick_supported_lighting_mutation(
            role.and_then(|value| value.get("receiver")),
            snapshot,
        );
        if role_mouse.is_some() || role_receiver.is_some() {
            return (role_mouse, role_receiver);
        }
        // 回退到 zones 声明式字段：从 zone id 为 'mouse'/'receiver' 的
        // fields[].mutation 提取 mutation 名。与 Capability::lighting_role() 一致。
        if let Some(zones) = cap.metadata.get("zones").and_then(|v| v.as_array()) {
            let (mouse, receiver) = zone_lighting_mutations(zones, snapshot);
            if mouse.is_some() || receiver.is_some() {
                return (mouse, receiver);
            }
        }
    }
    (None, None)
}

fn display_name(
    plugin_id: &str,
    family: &str,
    verified_models: &[String],
    evidence: &str,
) -> String {
    if evidence == "hardware-verified" {
        if let Some(model) = verified_models.first() {
            return model.clone();
        }
    }
    format!(
        "{} {}",
        plugin_id.split('.').next_back().unwrap_or(plugin_id),
        family
    )
}

/// Build a `DeviceSnapshot` from a `DeviceReading` and the matched plugin/device
/// context. When `mutation_result` is provided, its `color` field is used as a
/// fallback for `confirmed_light_color` (write path); otherwise the reading's
/// `light_color` is used directly (read path).
struct SnapshotRuntimeContext<'a> {
    package_files: Option<&'a BTreeMap<String, Vec<u8>>>,
    fallback_connection: mira_core::Connection,
    writable_mutations: Vec<String>,
    mutation_result: Option<&'a serde_json::Value>,
}

/// Merge a mutation's verified parser result into the cached raw workflow
/// outputs. Matching is structural and plugin-neutral: the verify fields are
/// applied to output objects that already expose at least one of those fields.
/// This keeps protocol naming in the signed plugin while allowing the host to
/// publish an immediate normalized snapshot without another Full read.
fn merge_verified_mutation_output(
    outputs: &mut BTreeMap<String, serde_json::Value>,
    verified: &serde_json::Value,
) {
    let Some(verified_object) = verified.as_object() else {
        return;
    };
    for output in outputs.values_mut() {
        let Some(output_object) = output.as_object_mut() else {
            continue;
        };
        if !verified_object
            .keys()
            .any(|key| output_object.contains_key(key))
        {
            continue;
        }
        for (key, value) in verified_object {
            output_object.insert(key.clone(), value.clone());
        }
    }
}

fn build_device_snapshot(
    reading: DeviceReading,
    inspection: &PackageInspection,
    devices: &hid::DevicesFile,
    device: &hid::MatchedDevice,
    context: SnapshotRuntimeContext<'_>,
) -> DeviceSnapshot {
    let resolved_name = reading.display_name.unwrap_or_else(|| {
        display_name(
            &device.plugin_id,
            &device.family,
            &devices.hardware_verified_models,
            &device.evidence,
        )
    });
    let resolved_connection = reading
        .connection
        .map(runtime_connection)
        .unwrap_or(context.fallback_connection);
    let confirmed_light_color = reading.light_color.or_else(|| {
        context
            .mutation_result
            .and_then(|result| result.get("color"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });
    let readonly = !(inspection.signature_verified && inspection.writes_enabled);
    // 能力动态协商：用 workflow 输出计算每个能力的 available 标记。
    // 在 reading.capabilities 被移动前借用，传给 plugin_capabilities。
    // #3 连接类型能力分支：优先使用 workflow 实际读到的连接类型，
    // 避免 devices.json fallback 与真实连接不一致时误判能力可见性。
    let plugin_capabilities = plugin_capabilities(
        inspection,
        &reading.capabilities,
        capability_connection_label(resolved_connection),
        context.package_files,
        Some(&device.family),
        &context.writable_mutations,
    );
    DeviceSnapshot {
        display_name: resolved_name,
        connection: resolved_connection,
        battery_percent: reading.battery_percent,
        charging: reading.charging,
        batteries: reading.batteries,
        dpi: reading.dpi,
        dpi_stages: reading.dpi_stages,
        polling_rate_hz: reading.polling_rate_hz,
        supported_polling_rates_hz: reading.supported_polling_rates_hz,
        profile: reading.profile.map(|p| (p + 1).to_string()),
        confirmed_light_color,
        capabilities: reading.capabilities,
        plugin_capabilities,
        writable_mutations: context.writable_mutations,
        evidence: device.evidence.clone(),
        readonly,
        plugin_id: Some(inspection.plugin_id.clone()),
        history_identity: device.identity.as_ref().map(|identity| DeviceIdentity {
            group: identity.group.clone(),
            display_name: identity.display_name.clone(),
            aliases: identity.aliases.clone(),
        }),
    }
}

#[tauri::command]
fn device_snapshot(state: tauri::State<'_, SessionState>) -> Option<DeviceSnapshot> {
    selected_snapshot(&state)
}

fn device_snapshot_entries(state: &SessionState) -> Vec<DeviceSnapshotEntry> {
    let guard = match state.last_snapshot.lock() {
        Ok(guard) => guard,
        Err(_) => return Vec::new(),
    };
    let selected_path = selected_snapshot_entry(state, &guard).map(|(path, _)| path.clone());
    guard
        .iter()
        .map(|(device_key, snapshot)| DeviceSnapshotEntry {
            device_key: device_key.clone(),
            snapshot: snapshot.clone(),
            selected: selected_path.as_ref() == Some(device_key),
        })
        .collect()
}

/// 返回所有已连接设备的快照列表，供多设备 UI 使用。
#[tauri::command]
fn device_snapshots(state: tauri::State<'_, SessionState>) -> Vec<DeviceSnapshotEntry> {
    device_snapshot_entries(&state)
}

#[tauri::command]
fn device_select(
    state: tauri::State<'_, SessionState>,
    device_key: String,
) -> Result<DeviceSnapshot, String> {
    let snapshot = {
        let guard = state
            .last_snapshot
            .lock()
            .map_err(|_| "device snapshot state unavailable".to_string())?;
        guard
            .get(&device_key)
            .cloned()
            .ok_or_else(|| "selected device is no longer connected".to_string())?
    };
    let mut selected = state
        .selected_device_path
        .lock()
        .map_err(|_| "selected device state unavailable".to_string())?;
    *selected = Some(device_key);
    Ok(snapshot)
}

/// Trigger an immediate device read on the background thread.
/// The read result is delivered via the `device-updated` event, so this
/// command returns immediately. Used by the manual "刷新" button and any
/// other UI flow that needs a fresh reading without waiting for the fallback poll.
///
/// 用户手动刷新属于退避重置事件，清除所有设备退避状态。
#[tauri::command]
fn device_refresh(state: tauri::State<'_, SessionState>) -> Result<(), String> {
    reset_all_backoff(&state);
    request_refresh_with_plan(&state, ReadPlan::Full);
    Ok(())
}

/// Refresh only the dynamic fields needed by parameter editors.
/// UI 交互（tab 切换、编辑按钮点击）触发，不重置退避：
/// 避免高频交互反复重置休眠设备退避，导致"超时-重置-超时"循环。
#[tauri::command]
fn device_refresh_quick(state: tauri::State<'_, SessionState>) -> Result<(), String> {
    request_refresh_with_plan(&state, ReadPlan::Quick);
    Ok(())
}

/// Refresh battery fields without reading the rest of the device protocol.
/// UI 交互（查看电量）触发，不重置退避。
#[tauri::command]
fn device_refresh_battery(state: tauri::State<'_, SessionState>) -> Result<(), String> {
    request_refresh_with_plan(&state, ReadPlan::BatteryOnly);
    Ok(())
}

struct SoftwareProfileRuntime<'a> {
    api: &'a HidApi,
    connection: ConnectionKind,
    files: &'a BTreeMap<String, Vec<u8>>,
    package: &'a ProtocolPackage,
}

/// 在读取路径中恢复软件配置（控制模式 + 已保存的 mutation）。
///
/// 注意：此函数在 `read_device_once` 持有 `device_io` 锁期间调用，**不经过
/// `begin_device_write` 写入队列**。这是有意为之——`device_mutate_blocking` 的
/// 锁顺序是 `write_in_progress` → `device_io`，若此处再获取 `write_in_progress`
/// 会形成 `device_io` → `write_in_progress` 的反向锁顺序，导致死锁。
/// 此处的写入由 `device_io` 锁序列化（与 `device_mutate_blocking` 互斥），
/// 且仅对已验证插件的 `allowed` mutation 执行，安全性等价。
fn reapply_software_profile(
    app: &AppHandle,
    state: &SessionState,
    device: &hid::MatchedDevice,
    reading: &DeviceReading,
    allowed: &[String],
    runtime: &SoftwareProfileRuntime<'_>,
) -> Option<DeviceReading> {
    let key = software_profile_key(device, reading);
    let profiles = cached_software_profiles(app);
    let profile = profiles.devices.get(&key)?;
    let already_applied = state.applied_software_profiles.lock().ok()?.contains(&key);
    if already_applied && control_mode(reading) == Some(2) {
        return None;
    }
    let context = ProtocolContext {
        api: runtime.api,
        path: &device.path,
        family: &device.family,
        connection: runtime.connection,
        files: runtime.files,
        outputs: reading.capabilities.clone(),
        feature_index_cache: Some(&state.feature_index_cache),
        onboard_memory_cache: Some(&state.onboard_memory_cache),
        cached_handles: Some(&state.cached_handles),
        hid_io_stats: hid_io_stats_ref(state),
    };
    let mut failed = false;
    if allowed
        .iter()
        .any(|mutation| mutation == "set-control-mode")
        && control_mode(reading) != Some(2)
    {
        let params = serde_json::Map::from_iter([("mode".into(), serde_json::json!(2))]);
        if let Err(error) =
            mutate_device_with_package(runtime.package, &context, "set-control-mode", &params)
        {
            failed = true;
            eprintln!("[mira] unable to restore software control mode: {error}");
        }
    }
    if !failed {
        for (mutation, params) in &profile.mutations {
            if mutation == "set-control-mode"
                || !allowed.iter().any(|candidate| candidate == mutation)
            {
                continue;
            }
            let params = serde_json::Map::from_iter(params.clone());
            if let Err(error) =
                mutate_device_with_package(runtime.package, &context, mutation, &params)
            {
                failed = true;
                eprintln!("[mira] unable to restore {mutation}: {error}");
            }
        }
    }
    if let Ok(mut applied) = state.applied_software_profiles.lock() {
        applied.insert(key);
    }
    if failed {
        None
    } else {
        read_device_with_package(
            runtime.package,
            &ProtocolContext {
                api: runtime.api,
                path: &device.path,
                family: &device.family,
                connection: runtime.connection,
                files: runtime.files,
                outputs: BTreeMap::new(),
                feature_index_cache: Some(&state.feature_index_cache),
                onboard_memory_cache: Some(&state.onboard_memory_cache),
                cached_handles: Some(&state.cached_handles),
                hid_io_stats: hid_io_stats_ref(state),
            },
        )
        .ok()
    }
}

/// 读取计划：决定对设备执行多重的读取操作。
///
/// 强度顺序：Full > Quick > BatteryOnly > PresenceOnly
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReadPlan {
    /// 只检测设备存在（HID 枚举 + 插件匹配），不执行协议工作流。
    /// 发布临时设备状态（readiness=Detected）。
    PresenceOnly,
    /// 只读取电量和充电状态。
    /// 使用工作流投影，目标 output 仅限 battery 相关。
    BatteryOnly,
    /// 读取当前交互界面需要的动态状态（电量、DPI、回报率等）。
    /// 使用工作流投影，只执行生成目标 output 所需的最小 step 子集。
    Quick,
    /// 完整执行工作流，读取所有状态和能力。
    /// 用于首次接入后的后台补全、用户手动完整刷新、插件更新等。
    Full,
}

const CONNECTED_PRESENCE_INTERVAL: Duration = Duration::from_secs(15);
const BATTERY_READ_INTERVAL: Duration = Duration::from_secs(300);

impl ReadPlan {
    /// 返回此计划对应的语义字段集合（用于工作流投影）。
    /// PresenceOnly 不执行工作流，返回空集合。
    fn semantic_fields(&self) -> BTreeSet<SemanticField> {
        match self {
            ReadPlan::PresenceOnly => BTreeSet::new(),
            ReadPlan::Quick => {
                let mut fields = BTreeSet::new();
                fields.insert(SemanticField::BatteryPercent);
                fields.insert(SemanticField::Charging);
                fields.insert(SemanticField::ReceiverBatteryPercent);
                fields.insert(SemanticField::CurrentDpi);
                fields.insert(SemanticField::PollingRate);
                fields.insert(SemanticField::ActiveProfile);
                fields.insert(SemanticField::LightingState);
                fields
            }
            ReadPlan::BatteryOnly => {
                let mut fields = BTreeSet::new();
                fields.insert(SemanticField::BatteryPercent);
                fields.insert(SemanticField::Charging);
                fields.insert(SemanticField::ReceiverBatteryPercent);
                fields
            }
            ReadPlan::Full => BTreeSet::new(), // Full 不使用投影，执行完整工作流
        }
    }

    /// 是否使用工作流投影（而非完整工作流）。
    fn uses_projection(&self) -> bool {
        matches!(self, ReadPlan::Quick | ReadPlan::BatteryOnly)
    }
}

fn select_read_plan(
    connected: bool,
    initial_plan: Option<ReadPlan>,
    forced_plan: Option<ReadPlan>,
    battery_due: bool,
) -> ReadPlan {
    let requested = match (initial_plan, forced_plan) {
        (Some(initial), Some(forced)) => Some(initial.max(forced)),
        (Some(initial), None) => Some(initial),
        (None, Some(forced)) => Some(forced),
        (None, None) => None,
    };
    if let Some(plan) = requested {
        plan
    } else if connected && battery_due {
        ReadPlan::BatteryOnly
    } else {
        ReadPlan::PresenceOnly
    }
}

/// 设备快照就绪程度：标识当前快照的完整性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotReadiness {
    /// 仅检测到设备存在，尚未读取任何协议状态。
    Detected,
    /// 已读取动态状态（Quick），静态能力可能尚未完整。
    Quick,
    /// 已读取完整状态和能力。
    Full,
}

/// 部分快照补丁：用于合并不同读取计划的结果。
///
/// Quick 和 BatteryOnly 不能用空字段覆盖 Full 快照。
/// 每种补丁只更新其负责的字段，保留其他字段不变。
#[allow(dead_code)] // Full variant 和诊断字段在后续阶段使用
#[allow(clippy::large_enum_variant)] // Full(DeviceSnapshot) 较大，但 Full 目前未被构造
#[derive(Debug, Clone)]
enum SnapshotPatch {
    /// Presence 补丁：更新设备存在信息，保留旧状态。
    Presence {
        plugin_id: Option<String>,
        connection: Option<Connection>,
    },
    /// Quick 补丁：只更新成功读取的动态字段。
    Quick {
        battery_percent: Option<u8>,
        charging: Option<bool>,
        batteries: Option<Vec<mira_core::DeviceBattery>>,
        dpi: Option<u16>,
        dpi_stages: Option<Vec<mira_core::DpiStage>>,
        polling_rate_hz: Option<u16>,
        profile: Option<String>,
        confirmed_light_color: Option<String>,
        /// 投影诊断信息
        projection_valid: bool,
        fallback_reason: Option<String>,
    },
    /// BatteryOnly 补丁：只更新电量和充电。
    Battery {
        battery_percent: Option<u8>,
        charging: Option<bool>,
        batteries: Option<Vec<mira_core::DeviceBattery>>,
    },
    /// Full 替换：完整替换快照。
    Full(DeviceSnapshot),
}

/// 将补丁应用到现有快照上，返回更新后的快照。
///
/// 如果 `existing` 为 None，Presence 和 Full 会创建新快照；
/// Quick 和 Battery 在没有现有快照时会回退为 Detected 状态。
fn apply_snapshot_patch(
    existing: Option<&DeviceSnapshot>,
    patch: SnapshotPatch,
    inspection: &PackageInspection,
    device: &hid::MatchedDevice,
    devices: &hid::DevicesFile,
    fallback_connection: Connection,
) -> DeviceSnapshot {
    match patch {
        SnapshotPatch::Full(snapshot) => snapshot,
        SnapshotPatch::Presence {
            plugin_id,
            connection,
        } => {
            // 保留旧状态，只更新存在信息
            if let Some(existing) = existing {
                let mut updated = existing.clone();
                if let Some(pid) = plugin_id {
                    updated.plugin_id = Some(pid);
                }
                if let Some(conn) = connection {
                    updated.connection = conn;
                }
                updated
            } else {
                // 短时重连无旧状态：创建 Detected 临时快照
                let readonly = !(inspection.signature_verified && inspection.writes_enabled);
                DeviceSnapshot {
                    display_name: display_name(
                        &device.plugin_id,
                        &device.family,
                        &devices.hardware_verified_models,
                        &device.evidence,
                    ),
                    connection: connection.unwrap_or(fallback_connection),
                    battery_percent: None,
                    charging: false,
                    batteries: Vec::new(),
                    dpi: None,
                    dpi_stages: None,
                    polling_rate_hz: None,
                    supported_polling_rates_hz: None,
                    profile: None,
                    confirmed_light_color: None,
                    capabilities: BTreeMap::new(),
                    plugin_capabilities: Vec::new(),
                    writable_mutations: Vec::new(),
                    evidence: device.evidence.clone(),
                    readonly,
                    plugin_id: plugin_id.or_else(|| Some(inspection.plugin_id.clone())),
                    history_identity: device.identity.as_ref().map(|identity| DeviceIdentity {
                        group: identity.group.clone(),
                        display_name: identity.display_name.clone(),
                        aliases: identity.aliases.clone(),
                    }),
                }
            }
        }
        SnapshotPatch::Quick {
            battery_percent,
            charging,
            batteries,
            dpi,
            dpi_stages,
            polling_rate_hz,
            profile,
            confirmed_light_color,
            projection_valid: _,
            fallback_reason: _,
        } => {
            if let Some(existing) = existing {
                let mut updated = existing.clone();
                if let Some(bp) = battery_percent {
                    updated.battery_percent = Some(bp);
                }
                if let Some(c) = charging {
                    updated.charging = c;
                }
                if let Some(bats) = batteries {
                    updated.batteries = bats;
                }
                if let Some(d) = dpi {
                    updated.dpi = Some(d);
                }
                if let Some(ds) = dpi_stages {
                    updated.dpi_stages = Some(ds);
                }
                if let Some(pr) = polling_rate_hz {
                    updated.polling_rate_hz = Some(pr);
                }
                if let Some(p) = profile {
                    updated.profile = Some(p);
                }
                if let Some(lc) = confirmed_light_color {
                    updated.confirmed_light_color = Some(lc);
                }
                updated
            } else {
                // 没有 existing 快照时，Quick 补丁无法应用
                // 回退为 Detected 临时快照
                apply_snapshot_patch(
                    existing,
                    SnapshotPatch::Presence {
                        plugin_id: Some(inspection.plugin_id.clone()),
                        connection: Some(fallback_connection),
                    },
                    inspection,
                    device,
                    devices,
                    fallback_connection,
                )
            }
        }
        SnapshotPatch::Battery {
            battery_percent,
            charging,
            batteries,
        } => {
            if let Some(existing) = existing {
                let mut updated = existing.clone();
                if let Some(bp) = battery_percent {
                    updated.battery_percent = Some(bp);
                }
                if let Some(c) = charging {
                    updated.charging = c;
                }
                if let Some(bats) = batteries {
                    updated.batteries = bats;
                }
                updated
            } else {
                apply_snapshot_patch(
                    existing,
                    SnapshotPatch::Presence {
                        plugin_id: Some(inspection.plugin_id.clone()),
                        connection: Some(fallback_connection),
                    },
                    inspection,
                    device,
                    devices,
                    fallback_connection,
                )
            }
        }
    }
}

/// Outcome of a background device read, carried out of the locked section so
/// disk I/O (load_settings) and UI work (update_tray, app.emit) can run without
/// holding the device_io / plugins locks.
enum DeviceReadOutcome {
    /// Nothing to do (no plugins, HidApi failed, etc.) — skip silently.
    Skip,
    /// Device disconnected or read failed — clear the cached state.
    Clear,
    /// 设备仍被枚举到但读取失败（如无线设备休眠导致的间歇性超时）。
    /// 保留上次有效快照，避免清除缓存后形成"识别→丢失→识别"的无限循环。
    /// 注意：不应清除 feature_index_cache 等缓存，否则下次读取更慢，加剧循环。
    PreserveLast,
    /// Read succeeded — publish snapshots for all matched devices.
    /// 修复 #10：携带所有 matched 设备的快照，实现多设备并行读取。
    Ready(Vec<(String, DeviceSnapshot)>),
}

/// Read the device once, update the cached snapshot, and emit `device-updated`.
/// Called by the background reader thread on every loop iteration (whether
/// triggered by a signal or the fallback timeout).
///
/// `plan` 决定读取强度：PresenceOnly 只枚举；Quick/BatteryOnly 使用工作流投影
/// 读取部分字段并通过 SnapshotPatch 合并；Full 执行完整工作流。
fn read_device_once(app: &AppHandle, plan: ReadPlan) {
    let state = app.state::<SessionState>();
    // 系统主题缓存由 ThemeChanged 事件和窗口聚焦事件更新，
    // 轮询期间只读缓存，避免每轮都 fork 进程检测主题。
    // Lock scope: only HID I/O and snapshot construction. Disk reads, tray
    // updates and event emission run after the locks are released so they
    // cannot block discover_devices or other commands.
    let outcome = (|| -> Option<DeviceReadOutcome> {
        let _io_guard = state.device_io.lock().ok()?;
        let plugins_guard = state.plugins.lock().ok()?;
        let plugins = plugins_guard.as_ref()?;
        if plugins.is_empty() {
            return None;
        }
        // Reuse the cached HidApi instance and refresh the device list to
        // detect newly plugged/unplugged devices. This avoids re-enumerating
        // all HID devices from scratch on every poll.
        let mut hidapi_guard = state.cached_hidapi.lock().ok()?;
        if hidapi_guard.is_none() {
            *hidapi_guard = Some(HidApi::new().ok()?);
        }
        let cached_api = hidapi_guard.as_mut().unwrap();
        let _ = cached_api.refresh_devices();
        let api: &HidApi = cached_api;
        let matched = hid::enumerate_matched_devices(api, plugins);
        #[cfg(debug_assertions)]
        {
            let count = matched.len();
            if let Ok(mut last) = state.last_matched_count.lock() {
                if *last != count {
                    eprintln!("[mira] background: matched {} device(s)", count);
                    *last = count;
                }
            }
        }

        // 修复 #10：遍历所有 matched 设备，逐个读取并收集快照。
        if matched.is_empty() {
            return Some(DeviceReadOutcome::Clear);
        }
        let mut entries: Vec<(String, DeviceSnapshot)> = Vec::new();
        for device in &matched {
            let (inspection, devices, plugin_files) = match plugins
                .iter()
                .find(|(inspection, _, _)| inspection.plugin_id == device.plugin_id)
            {
                Some(triple) => triple,
                None => continue,
            };

            let (connection, kind) = connection_kind(&device.connection);

            // PresenceOnly：不解析包、不执行工作流，只更新设备存在信息。
            // 保留旧快照的动态状态（电量、DPI 等），仅刷新 plugin_id 和 connection。
            if plan == ReadPlan::PresenceOnly {
                let existing = state
                    .last_snapshot
                    .lock()
                    .ok()
                    .and_then(|map| map.get(&device.path).cloned());
                let snapshot = apply_snapshot_patch(
                    existing.as_ref(),
                    SnapshotPatch::Presence {
                        plugin_id: Some(inspection.plugin_id.clone()),
                        connection: Some(connection),
                    },
                    inspection,
                    device,
                    devices,
                    connection,
                );
                entries.push((device.path.clone(), snapshot));
                if let Ok(mut readiness) = state.snapshot_readiness.lock() {
                    readiness
                        .entry(device.path.clone())
                        .or_insert(SnapshotReadiness::Detected);
                }
                continue;
            }

            // 缓存命中时直接复用 Arc<ProtocolPackage>，避免每轮轮询都解析 JSON。
            let package = match get_or_parse_package(
                &state,
                inspection,
                device.model.as_deref(),
                plugin_files,
                plugins,
            ) {
                Ok(pkg) => pkg,
                Err(error) => {
                    record_device_read_error(
                        &state,
                        device,
                        format!("parse protocol package: {error}"),
                    );
                    continue;
                }
            };

            // #9 防抖式 TTL 缓存：500ms 内复用快照跳过 HID 往返。
            let cache_hit = state.last_read_at.lock().ok().is_some_and(|cache| {
                cache
                    .get(&device.path)
                    .is_some_and(|t| t.elapsed() < READ_DEBOUNCE_TTL)
            });
            if cache_hit {
                if let Some(snapshot) = state
                    .last_snapshot
                    .lock()
                    .ok()
                    .and_then(|map| map.get(&device.path).cloned())
                {
                    entries.push((device.path.clone(), snapshot));
                    continue;
                }
            }

            // 指数退避：连续失败的设备跳过 HID 读取，使用现有快照。
            // PresenceOnly 不执行 HID I/O，无需退避。
            if plan != ReadPlan::PresenceOnly {
                let in_backoff = state.backoff_state.lock().ok().is_some_and(|backoff| {
                    backoff
                        .get(&device.path)
                        .is_some_and(|b| b.is_backing_off())
                });
                if in_backoff {
                    if let Some(snapshot) = state
                        .last_snapshot
                        .lock()
                        .ok()
                        .and_then(|map| map.get(&device.path).cloned())
                    {
                        entries.push((device.path.clone(), snapshot));
                    }
                    // 没有现有快照则跳过此设备（不添加到 entries）
                    continue;
                }
            }

            let ctx = ProtocolContext {
                api,
                path: &device.path,
                family: &device.family,
                connection: kind,
                files: plugin_files,
                outputs: BTreeMap::new(),
                feature_index_cache: Some(&state.feature_index_cache),
                onboard_memory_cache: Some(&state.onboard_memory_cache),
                cached_handles: Some(&state.cached_handles),
                hid_io_stats: hid_io_stats_ref(&state),
            };

            // Phase 1: 根据读取计划执行工作流。
            // projection_valid=true 表示投影成功，用 SnapshotPatch 合并；
            // projection_valid=false 表示 Full 或投影回退，用 build_device_snapshot 完整构建。
            let read_start = Instant::now();
            let read_result: Result<(DeviceReading, bool, Option<String>), String> =
                if plan.uses_projection() {
                    read_device_with_projection(&package, &ctx, &plan.semantic_fields())
                        .map(|p| (p.reading, p.projection_valid, p.fallback_reason))
                } else {
                    read_device_with_package(&package, &ctx).map(|r| (r, false, None))
                };

            match read_result {
                Ok((mut reading, projection_valid, projection_fallback_reason)) => {
                    clear_device_read_error(&state, &device.path);
                    // 读取成功：清除该设备的退避状态。
                    if let Ok(mut backoff) = state.backoff_state.lock() {
                        if let Some(b) = backoff.get_mut(&device.path) {
                            b.reset();
                        }
                    }
                    if projection_fallback_reason.is_some() {
                        // An unsupported projection must not silently become a
                        // periodic Full read. Keep the last complete snapshot;
                        // explicit initial/manual Full reads remain available.
                        if let Some(snapshot) = state
                            .last_snapshot
                            .lock()
                            .ok()
                            .and_then(|map| map.get(&device.path).cloned())
                        {
                            entries.push((device.path.clone(), snapshot));
                        }
                        let elapsed_ms = read_start.elapsed().as_millis() as u64;
                        record_device_metric(&state, device, |m| {
                            m.plan_counts.bump(plan);
                            m.plan_failures.bump(plan);
                            m.projection_fallback += 1;
                            m.last_read_ms = Some(elapsed_ms);
                        });
                        continue;
                    }
                    let snapshot = if projection_valid {
                        // Quick/BatteryOnly 投影成功：用 SnapshotPatch 合并，保留旧快照的静态字段。
                        let existing = state
                            .last_snapshot
                            .lock()
                            .ok()
                            .and_then(|map| map.get(&device.path).cloned());
                        let patch = if plan == ReadPlan::Quick {
                            SnapshotPatch::Quick {
                                battery_percent: reading.battery_percent,
                                charging: reading.battery_percent.map(|_| reading.charging),
                                batteries: if reading.batteries.is_empty() {
                                    None
                                } else {
                                    Some(reading.batteries)
                                },
                                dpi: reading.dpi,
                                dpi_stages: reading.dpi_stages,
                                polling_rate_hz: reading.polling_rate_hz,
                                profile: reading.profile.map(|p| (p + 1).to_string()),
                                confirmed_light_color: reading.light_color,
                                projection_valid: true,
                                fallback_reason: None,
                            }
                        } else {
                            // BatteryOnly
                            SnapshotPatch::Battery {
                                battery_percent: reading.battery_percent,
                                charging: reading.battery_percent.map(|_| reading.charging),
                                batteries: if reading.batteries.is_empty() {
                                    None
                                } else {
                                    Some(reading.batteries)
                                },
                            }
                        };
                        apply_snapshot_patch(
                            existing.as_ref(),
                            patch,
                            inspection,
                            device,
                            devices,
                            connection,
                        )
                    } else {
                        // Full 或投影回退：完整构建快照（含 writable_mutations、software profile）
                        let writable_mutations = if inspection.signature_verified
                            && inspection.writes_enabled
                            && device_evidence_allows_writes(&device.evidence)
                        {
                            writable_mutations_with_package(
                                &package,
                                &ProtocolContext {
                                    api,
                                    path: &device.path,
                                    family: &device.family,
                                    connection: kind,
                                    files: plugin_files,
                                    outputs: reading.capabilities.clone(),
                                    feature_index_cache: Some(&state.feature_index_cache),
                                    onboard_memory_cache: Some(&state.onboard_memory_cache),
                                    cached_handles: Some(&state.cached_handles),
                                    hid_io_stats: hid_io_stats_ref(&state),
                                },
                            )
                            .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        if let Some(updated) = reapply_software_profile(
                            app,
                            &state,
                            device,
                            &reading,
                            &writable_mutations,
                            &SoftwareProfileRuntime {
                                api,
                                connection: kind,
                                files: plugin_files,
                                package: &package,
                            },
                        ) {
                            reading = updated;
                        }
                        build_device_snapshot(
                            reading,
                            inspection,
                            devices,
                            device,
                            SnapshotRuntimeContext {
                                package_files: Some(plugin_files),
                                fallback_connection: connection,
                                writable_mutations,
                                mutation_result: None,
                            },
                        )
                    };
                    if let Ok(mut readiness) = state.snapshot_readiness.lock() {
                        let next = if projection_valid {
                            SnapshotReadiness::Quick
                        } else {
                            SnapshotReadiness::Full
                        };
                        readiness
                            .entry(device.path.clone())
                            .and_modify(|current| {
                                if *current != SnapshotReadiness::Full {
                                    *current = next;
                                }
                            })
                            .or_insert(next);
                    }
                    entries.push((device.path.clone(), snapshot));
                    // #9 记录读取时间戳，供下一轮 TTL 防抖判断。
                    if let Ok(mut cache) = state.last_read_at.lock() {
                        cache.insert(device.path.clone(), Instant::now());
                    }
                    // 诊断指标：记录计划执行、投影结果、延迟。
                    let elapsed_ms = read_start.elapsed().as_millis() as u64;
                    record_device_metric(&state, device, |m| {
                        m.plan_counts.bump(plan);
                        m.plan_success.bump(plan);
                        if plan.uses_projection() {
                            if projection_valid {
                                m.projection_success += 1;
                                m.full_avoided += 1;
                            } else {
                                m.projection_fallback += 1;
                            }
                        }
                        m.last_read_ms = Some(elapsed_ms);
                        m.backoff_failures = 0;
                        m.backing_off = false;
                    });
                }
                Err(error) => {
                    record_device_read_error(&state, device, error.clone());
                    // 读取失败：记录指数退避。
                    let fail_count = if let Ok(mut backoff) = state.backoff_state.lock() {
                        let b = backoff.entry(device.path.clone()).or_default();
                        b.record_failure();
                        b.consecutive_failures
                    } else {
                        0
                    };
                    // 诊断指标：记录计划失败和退避状态。
                    record_device_metric(&state, device, |m| {
                        m.plan_counts.bump(plan);
                        m.plan_failures.bump(plan);
                        m.backoff_failures = fail_count;
                        m.backing_off = fail_count > 0;
                    });
                    eprintln!(
                        "[mira] background read failed for {}: {error}",
                        device.family
                    );
                }
            }
        }

        if entries.is_empty() {
            // 设备仍被枚举到（matched 非空）但所有设备读取都失败。
            // 这种情况通常由无线设备休眠导致的间歇性超时引起。
            // 保留上次有效快照，避免清除缓存后形成"识别→丢失→识别"的无限循环：
            //   - 清除 last_snapshot 会让前端显示"未找到设备"
            //   - 清除 feature_index_cache 会让下次 HID++ 读取重新查询所有 feature index
            //     （罗技 42 步工作流最坏情况 14 秒），加剧超时
            //   - 立即重试会放大无线休眠期间的超时问题
            // 只有 matched 为空（设备真正断开）时才触发 Clear。
            Some(DeviceReadOutcome::PreserveLast)
        } else {
            Some(DeviceReadOutcome::Ready(entries))
        }
    })()
    .unwrap_or(DeviceReadOutcome::Skip);

    // Post-lock: disk I/O, tray updates and event emission run without holding
    // device_io or plugins locks so concurrent commands are not blocked.
    match outcome {
        DeviceReadOutcome::Skip => {}
        DeviceReadOutcome::PreserveLast => {
            // 设备仍被枚举到但读取失败：保留上次有效快照，不触发前端更新。
            // 这样前端不会看到"设备丢失"的闪烁，settle 也不会被重置，
            // 下次轮询会以正常间隔重试。如果设备真正断开，matched 会变空，
            // 下一轮会走到 Clear 分支。
        }
        DeviceReadOutcome::Clear => {
            if let Ok(mut applied) = state.applied_software_profiles.lock() {
                applied.clear();
            }
            clear_snapshots(&state);
            if let Ok(mut readiness) = state.snapshot_readiness.lock() {
                readiness.clear();
            }
            let _ = app.emit("device-updated", Option::<DeviceSnapshot>::None);
            let _ = app.emit(
                "device-snapshots-updated",
                Vec::<DeviceSnapshotEntry>::new(),
            );
            let _ = update_tray(app, None, &cached_settings(app));
        }
        DeviceReadOutcome::Ready(entries) => {
            // 多设备管理：用当前 matched 设备的快照整体替换 map，清除已断开的设备。
            // into_iter 移动所有权，避免逐条克隆 DeviceSnapshot。
            let new_map: BTreeMap<String, DeviceSnapshot> = entries.into_iter().collect();
            if let Ok(mut readiness) = state.snapshot_readiness.lock() {
                readiness.retain(|path, _| new_map.contains_key(path));
            }
            // 清除断开设备的缓存句柄，仅保留仍连接的路径。
            if let Ok(mut cache) = state.cached_handles.lock() {
                cache.retain(|path, _| new_map.contains_key(path));
            }
            // 在存储前从 new_map 直接构建 entries，避免再次锁定 last_snapshot 克隆快照。
            let selected_path =
                selected_snapshot_entry(&state, &new_map).map(|(path, _)| path.clone());
            let snapshot_entries: Vec<DeviceSnapshotEntry> = new_map
                .iter()
                .map(|(device_key, snapshot)| DeviceSnapshotEntry {
                    device_key: device_key.clone(),
                    snapshot: snapshot.clone(),
                    selected: selected_path.as_ref() == Some(device_key),
                })
                .collect();
            let snapshots_changed = store_snapshots(&state, &new_map);
            if snapshots_changed {
                let _ = app.emit("device-snapshots-updated", &snapshot_entries);
            }
            // 电量使用情况采样：在设备轮询成功后记录电量样本。
            // 记录失败不影响设备功能（record_samples 内部静默处理）。
            if plan != ReadPlan::PresenceOnly {
                let settings = cached_settings(app);
                battery_history::record_samples(
                    &state.battery_history,
                    app,
                    settings.battery_history_enabled,
                    settings.low_battery_threshold,
                    settings.battery_history_retention_days as i64,
                    &snapshot_entries,
                );
                // 异常耗电通知：仅在设置开启时检查，节流由 AbnormalDrainNotifyState 保证。
                if settings.unusual_drain_alerts {
                    let now = chrono::Utc::now();
                    let alerts = battery_history::check_abnormal_drain(
                        &state.battery_history,
                        &state.abnormal_drain_notify,
                        now,
                    );
                    if !alerts.is_empty() {
                        let lang = effective_language(&settings.language);
                        if let Ok(mut guard) = state.pending_notification_action.lock() {
                            *guard = Some("battery-usage".to_string());
                        }
                        for (_key, device_name) in alerts {
                            let _ = app
                                .notification()
                                .builder()
                                .title(tr_abnormal_drain_title(lang))
                                .body(tr_abnormal_drain_body(lang, &device_name))
                                .show();
                        }
                        // 持久化节流状态：重启后 24h 内不重复通知。
                        let _ = state.abnormal_drain_notify.save_to_disk(app);
                    }
                }
            }
            // 选择当前设备通知前端（向后兼容单设备 API）。
            if snapshots_changed {
                let Some(snapshot) = selected_snapshot(&state) else {
                    return;
                };
                // 通知前端有新数据，前端通过事件监听更新，无需轮询
                let _ = app.emit("device-updated", &snapshot);
                let settings = cached_settings(app);
                let _ = update_tray(app, Some(&snapshot), &settings);
                // 低电量跨阈值检测：充电时不触发，充电结束后若仍低于阈值才再次提醒
                let battery_value = tray::state::low_battery_notification_value(&snapshot);
                let threshold = settings.low_battery_threshold;
                let notify = state
                    .low_battery
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .update(battery_value, threshold);
                if notify {
                    if let Some(percent) = battery_value {
                        let lang = effective_language(&settings.language);
                        if let Ok(mut guard) = state.pending_notification_action.lock() {
                            *guard = Some("battery-usage".to_string());
                        }
                        let _ = app
                            .notification()
                            .builder()
                            .title(tr_low_battery_title(lang))
                            .body(tr_low_battery_body(lang, threshold, percent))
                            .show();
                    }
                }
            }
        }
    }
}

/// Background reader thread: on-demand protocol reads with a lightweight
/// presence fallback.
///
/// The thread sleeps on `mpsc::recv_timeout` instead of a fixed `sleep`:
/// - An explicit editor, mutation, manual refresh, or wake signal
///   wakes it immediately for an on-demand read.
/// - Connected, stable devices only receive a PresenceOnly enumeration every
///   15 seconds. No protocol workflow runs merely because the window is visible.
/// - BatteryOnly runs independently every 5 minutes.
/// - The fallback interval otherwise adapts to the situation:
///   * 500 ms only while the initial Presence → Quick → Full sequence completes.
///   * 1 s when no device is connected and the window is visible, so plug-in
///     is detected quickly without continuous high-frequency polling.
///   * 15 s for a connected stable device or while hidden without a device.
fn spawn_device_reader(app: AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    *app.state::<SessionState>()
        .refresh_tx
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(tx);

    let mut last_battery_read_at: Option<Instant> = None;
    std::thread::spawn(move || loop {
        // Determine state before reading to choose the appropriate ReadPlan.
        let state = app.state::<SessionState>();
        let forced_plan = state
            .forced_read_plan
            .lock()
            .ok()
            .and_then(|mut pending| pending.take());
        let visible = app
            .get_webview_window("main")
            .and_then(|window| window.is_visible().ok())
            .unwrap_or(false);
        let (connected_before, next_initial_plan) = {
            let snapshots = state
                .last_snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let connected = !snapshots.is_empty();
            let next_initial_plan = state.snapshot_readiness.lock().ok().and_then(|readiness| {
                if readiness
                    .values()
                    .any(|value| *value == SnapshotReadiness::Detected)
                {
                    Some(ReadPlan::Quick)
                } else if readiness
                    .values()
                    .any(|value| *value == SnapshotReadiness::Quick)
                {
                    Some(ReadPlan::Full)
                } else {
                    None
                }
            });
            (connected, next_initial_plan)
        };

        let battery_due = connected_before
            && last_battery_read_at.is_none_or(|last| last.elapsed() >= BATTERY_READ_INTERVAL);

        // Choose ReadPlan:
        // - Connected but no Full snapshot yet → Full (首次读取需要获取能力)
        // - Explicit editor/mutation requests → Quick
        // - Battery due → BatteryOnly
        // - Otherwise → PresenceOnly (enumeration only, no protocol reports)
        let plan = select_read_plan(
            connected_before,
            next_initial_plan,
            forced_plan,
            battery_due,
        );

        read_device_once(&app, plan);
        if matches!(
            plan,
            ReadPlan::BatteryOnly | ReadPlan::Quick | ReadPlan::Full
        ) {
            last_battery_read_at = Some(Instant::now());
        }

        // Initial Presence → Quick → Full progresses promptly; once initialized,
        // a connected device returns to lightweight presence checks.
        let (connected, initializing) = {
            let connected = !state
                .last_snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty();
            let initializing = state
                .snapshot_readiness
                .lock()
                .ok()
                .is_some_and(|readiness| {
                    readiness
                        .values()
                        .any(|value| *value != SnapshotReadiness::Full)
                });
            (connected, initializing)
        };
        let wait = if initializing {
            std::time::Duration::from_millis(500)
        } else if connected {
            // Connected devices are only enumerated; protocol reads are on-demand.
            CONNECTED_PRESENCE_INTERVAL
        } else if visible {
            // No device connected: poll faster while visible so plug-in is noticed quickly.
            std::time::Duration::from_secs(1)
        } else {
            std::time::Duration::from_secs(15)
        };

        // 将等待分成最多 10s 的块，检测系统睡眠/唤醒。
        // Instant（单调时钟）在系统睡眠期间暂停，recv_timeout 不会在唤醒后立即超时；
        // 用 SystemTime（墙上时钟）检测跳跃，唤醒后触发一次 Quick 重新枚举设备。
        // 分块保证最多 SLEEP_DETECT_CHUNK 延迟即可检测到唤醒。
        const SLEEP_DETECT_CHUNK: std::time::Duration = std::time::Duration::from_secs(10);
        let mut remaining = wait;
        let mut woke_from_sleep = false;
        let mut channel_disconnected = false;
        while !remaining.is_zero() {
            let chunk = remaining.min(SLEEP_DETECT_CHUNK);
            let chunk_start = std::time::SystemTime::now();
            match rx.recv_timeout(chunk) {
                Ok(()) => {
                    // Drain any additional pending signals so a burst of focus
                    // events doesn't trigger a burst of redundant reads.
                    while rx.try_recv().is_ok() {}
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // 正常情况下 elapsed ≈ chunk；系统睡眠后 elapsed 远大于 chunk。
                    if let Ok(elapsed) = chunk_start.elapsed() {
                        if elapsed > chunk + std::time::Duration::from_secs(2) {
                            woke_from_sleep = true;
                            break;
                        }
                    }
                    remaining = remaining.saturating_sub(chunk);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    channel_disconnected = true;
                    break;
                }
            }
        }
        if channel_disconnected {
            break;
        }
        if woke_from_sleep {
            if let Some(state) = app.try_state::<SessionState>() {
                // 系统恢复后清除退避，重新检测所有设备。
                reset_all_backoff(&state);
                request_refresh_with_plan(&state, ReadPlan::Quick);
                // 全局诊断：记录睡眠恢复次数。
                if let Ok(mut g) = state.global_metrics.lock() {
                    g.sleep_resume_count += 1;
                }
            }
        }
        continue;
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredDevice {
    plugin_id: String,
    family: String,
    connection: String,
    evidence: String,
    path: String,
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    last_error_kind: Option<String>,
    last_error: Option<String>,
}

#[tauri::command]
fn discover_devices(
    state: tauri::State<'_, SessionState>,
) -> Result<Vec<DiscoveredDevice>, String> {
    let plugins_guard = state.plugins.lock().map_err(|_| "state lock failed")?;
    let plugins = plugins_guard.as_ref().ok_or("plugins not loaded")?;
    if plugins.is_empty() {
        return Ok(Vec::new());
    }
    // Reuse the cached HidApi instance and refresh the device list.
    let mut hidapi_guard = state
        .cached_hidapi
        .lock()
        .map_err(|_| "HidApi cache unavailable")?;
    if hidapi_guard.is_none() {
        *hidapi_guard = Some(HidApi::new().map_err(|e| e.to_string())?);
    }
    let cached_api = hidapi_guard.as_mut().unwrap();
    let _ = cached_api.refresh_devices();
    let api: &HidApi = cached_api;
    let matched = hid::enumerate_matched_devices(api, plugins);
    let errors = state
        .last_read_errors
        .lock()
        .map(|errors| errors.clone())
        .unwrap_or_default();
    Ok(matched
        .into_iter()
        .map(|d| {
            let diagnostic = errors.get(&d.path);
            let path = sanitized_device_key(&d);
            let last_error_kind = diagnostic.map(|diagnostic| diagnostic.error_kind.clone());
            let last_error = diagnostic.map(|diagnostic| diagnostic.error.clone());
            DiscoveredDevice {
                plugin_id: d.plugin_id,
                family: d.family,
                connection: d.connection,
                evidence: d.evidence,
                // 不暴露原始 HID 路径（macOS 上可能包含序列号），
                // 用脱敏标识符替代，前端仅用作 React key。
                path,
                vendor_id: d.vendor_id,
                product_id: d.product_id,
                usage_page: d.usage_page,
                usage: d.usage,
                last_error_kind,
                last_error,
            }
        })
        .collect())
}

struct WriteFlagGuard<'a>(&'a Mutex<bool>, &'a Condvar);

impl Drop for WriteFlagGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = false;
        }
        self.1.notify_one();
    }
}

fn begin_device_write(state: &SessionState) -> Result<WriteFlagGuard<'_>, String> {
    let mut active = state
        .write_in_progress
        .lock()
        .map_err(|_| "transaction state unavailable")?;
    // 排队等待前一个写入完成。最多等 25 秒，留 5 秒给实际 HID 写入，
    // 配合 device_mutate 的 30 秒总超时。
    let (guard, wait_result) = state
        .write_cond
        .wait_timeout_while(active, Duration::from_secs(25), |a| *a)
        .map_err(|_| "transaction state unavailable")?;
    active = guard;
    if *active || wait_result.timed_out() {
        return Err("写入排队超时：前一个写入仍未完成，请稍后重试".into());
    }
    *active = true;
    drop(active);
    Ok(WriteFlagGuard(&state.write_in_progress, &state.write_cond))
}

#[allow(clippy::too_many_arguments)]
fn remember_software_profile(
    app: &AppHandle,
    state: &SessionState,
    device: &hid::MatchedDevice,
    reading: &DeviceReading,
    allowed: &[String],
    capabilities: &[Capability],
    mutation: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if control_mode(reading).is_none() {
        return Ok(());
    }
    let key = software_profile_key(device, reading);
    let mut profiles = cached_software_profiles(app);
    if control_mode(reading) == Some(1) {
        profiles.devices.remove(&key);
        if let Ok(mut applied) = state.applied_software_profiles.lock() {
            applied.remove(&key);
        }
        return save_software_profiles(app, &profiles);
    }
    if control_mode(reading) != Some(2) {
        return Ok(());
    }
    let profile = profiles.devices.entry(key.clone()).or_default();
    if mutation == "set-control-mode" {
        seed_standard_software_mutations(profile, reading, allowed, capabilities);
    } else {
        profile
            .mutations
            .insert(mutation.to_string(), params.clone().into_iter().collect());
    }
    profiles.schema_version = 1;
    save_software_profiles(app, &profiles)?;
    if let Ok(mut applied) = state.applied_software_profiles.lock() {
        applied.insert(key);
    }
    Ok(())
}

#[tauri::command]
async fn device_mutate(
    app: tauri::AppHandle,
    mutation: String,
    params: serde_json::Map<String, serde_json::Value>,
) -> Result<DeviceSnapshot, String> {
    // HID 写入/回读是阻塞式调用，必须放在独立线程中执行，否则主线程会被卡死。
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_app = app.clone();
    std::thread::spawn(move || {
        let result = device_mutate_blocking(&worker_app, &mutation, &params);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("设备写入超时（30 秒）。鼠标可能处于休眠状态，请移动鼠标唤醒后重试。写入可能仍在后台执行，设备状态将在完成后自动刷新。".into())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("设备写入线程异常退出".into()),
    }
}

fn device_mutate_blocking(
    app: &tauri::AppHandle,
    mutation: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<DeviceSnapshot, String> {
    let state = app.state::<SessionState>();
    // Lock scope: only HID I/O and snapshot construction.
    // Disk I/O (profile save, load_settings), tray updates and event emission
    // run after the locks are released so they cannot block discover_devices or reads.
    let (device_path, snapshot, profile_remember) = {
        let _write_guard = begin_device_write(&state)?;
        let _io_guard = state
            .device_io
            .lock()
            .map_err(|_| "device I/O state unavailable")?;
        let plugins_guard = state.plugins.lock().map_err(|_| "state lock failed")?;
        let plugins = plugins_guard.as_ref().ok_or("plugins not loaded")?;
        // Reuse the cached HidApi instance and refresh the device list.
        let mut hidapi_guard = state
            .cached_hidapi
            .lock()
            .map_err(|_| "HidApi cache unavailable")?;
        if hidapi_guard.is_none() {
            *hidapi_guard = Some(HidApi::new().map_err(|e| e.to_string())?);
        }
        let cached_api = hidapi_guard.as_mut().unwrap();
        let _ = cached_api.refresh_devices();
        let api: &HidApi = cached_api;
        let matched = hid::enumerate_matched_devices(api, plugins);
        // 多设备并存时，写入必须跟随首页当前选中的设备。
        // 当前设备已断开时回退到第一个可写设备，再回退到首个匹配设备。
        let current_path = selected_device_path(&state);
        let device = matched
            .iter()
            .find(|device| Some(&device.path) == current_path.as_ref())
            .or_else(|| {
                matched
                    .iter()
                    .find(|device| device_evidence_allows_writes(&device.evidence))
            })
            .or_else(|| matched.first())
            .ok_or("supported device is not connected")?;
        let (inspection, devices, files) = plugins
            .iter()
            .find(|(inspection, _, _)| inspection.plugin_id == device.plugin_id)
            .ok_or("matched plugin is unavailable")?;
        if !inspection.signature_verified || !inspection.writes_enabled {
            return Err("the matched plugin is not trusted for device writes".into());
        }
        if !device_evidence_allows_writes(&device.evidence) {
            return Err("device writes require verified protocol evidence".into());
        }

        let (connection, kind) = connection_kind(&device.connection);
        // Parse the protocol package once and reuse it for writable_mutations,
        // read_device, and mutate_device to avoid re-parsing the JSON files 4 times.
        // 缓存命中时复用 Arc<ProtocolPackage>，写入路径同样受益。
        let package =
            get_or_parse_package(&state, inspection, device.model.as_deref(), files, plugins)?;
        let cached_snapshot = state
            .last_snapshot
            .lock()
            .ok()
            .and_then(|snapshots| snapshots.get(&device.path).cloned());
        // A completed Full snapshot already contains the raw plugin outputs
        // required by mutation guards. Reuse it instead of forcing a complete
        // workflow before every write. Only a cold/missing cache performs the
        // safety Full read once.
        let mut base_outputs = cached_snapshot
            .as_ref()
            .map(|snapshot| snapshot.capabilities.clone())
            .unwrap_or_default();
        if base_outputs.is_empty() {
            let context = ProtocolContext {
                api,
                path: &device.path,
                family: &device.family,
                connection: kind,
                files,
                outputs: BTreeMap::new(),
                feature_index_cache: Some(&state.feature_index_cache),
                onboard_memory_cache: Some(&state.onboard_memory_cache),
                cached_handles: Some(&state.cached_handles),
                hid_io_stats: hid_io_stats_ref(&state),
            };
            base_outputs = read_device_with_package(&package, &context)?.capabilities;
        }
        let mutate_context = ProtocolContext {
            api,
            path: &device.path,
            family: &device.family,
            connection: kind,
            files,
            outputs: base_outputs.clone(),
            feature_index_cache: Some(&state.feature_index_cache),
            onboard_memory_cache: Some(&state.onboard_memory_cache),
            cached_handles: Some(&state.cached_handles),
            hid_io_stats: hid_io_stats_ref(&state),
        };
        // 用与 mutate 一致的真实 outputs 计算 allowed 列表，避免空 outputs
        // 导致所有 mutation 都被视为可写，而实际 mutate 时被 skipIf 守门拒绝。
        let allowed = writable_mutations_with_package(&package, &mutate_context)?;
        if !allowed.iter().any(|candidate| candidate == mutation) {
            return Err(format!("unsupported device mutation {mutation}"));
        }
        let mutation_result =
            mutate_device_with_package(&package, &mutate_context, mutation, params)?;
        // The mutation engine already ran its declared verify parser and
        // assertions. Merge that verified object into the cached raw outputs,
        // normalize locally, and publish immediately; the on-demand scheduler
        // performs one follow-up Quick read for unrelated dynamic state.
        merge_verified_mutation_output(&mut base_outputs, &mutation_result);
        let mut reading = normalize_device_outputs_with_package(&package, base_outputs);
        reading.connection = Some(kind);
        if let Some(cached) = cached_snapshot.as_ref() {
            reading.display_name = Some(cached.display_name.clone());
            reading.battery_percent = reading.battery_percent.or(cached.battery_percent);
            if reading.batteries.is_empty() {
                reading.batteries = cached.batteries.clone();
                reading.charging = cached.charging;
            }
            reading.dpi = reading.dpi.or(cached.dpi);
            reading.dpi_stages = reading.dpi_stages.or_else(|| cached.dpi_stages.clone());
            reading.polling_rate_hz = reading.polling_rate_hz.or(cached.polling_rate_hz);
            reading.supported_polling_rates_hz = reading
                .supported_polling_rates_hz
                .or_else(|| cached.supported_polling_rates_hz.clone());
            reading.profile = reading.profile.or_else(|| {
                cached
                    .profile
                    .as_deref()
                    .and_then(|profile| profile.parse::<u8>().ok())
                    .and_then(|profile| profile.checked_sub(1))
            });
            reading.light_color = reading
                .light_color
                .or_else(|| cached.confirmed_light_color.clone());
        }
        // 修复 P-2：remember_software_profile 涉及磁盘 I/O（save_software_profiles），
        // 原代码在锁作用域内调用，违反 L2720-2722 注释承诺，会阻塞并发
        // discover_devices / read。改为仅在锁内 clone 所需数据，锁外再执行磁盘写入。
        let profile_remember = if control_mode(&reading).is_some() {
            Some((
                device.clone(),
                reading.clone(),
                allowed.clone(),
                inspection.capabilities.clone(),
            ))
        } else {
            None
        };
        let snapshot = build_device_snapshot(
            reading,
            inspection,
            devices,
            device,
            SnapshotRuntimeContext {
                package_files: Some(files),
                fallback_connection: connection,
                writable_mutations: allowed,
                mutation_result: Some(&mutation_result),
            },
        );
        (device.path.clone(), snapshot, profile_remember)
    };

    // Post-lock: disk I/O, tray updates and event emission run without holding
    // device_io or plugins locks so concurrent reads are not blocked.
    // 修复 P-2：remember_software_profile 在锁外执行磁盘写入，保持与注释承诺一致。
    if let Some((device_clone, reading_clone, allowed_clone, capabilities_clone)) = profile_remember
    {
        remember_software_profile(
            app,
            &state,
            &device_clone,
            &reading_clone,
            &allowed_clone,
            &capabilities_clone,
            mutation,
            params,
        )?;
    }
    // 修复 #10：用单条 insert 替代 clone→insert→store，避免覆盖其他设备的快照。
    store_snapshot(&state, device_path.clone(), snapshot.clone());
    // #9 写入后清除 TTL 缓存，确保下一轮读取强制刷新 HID。
    if let Ok(mut cache) = state.last_read_at.lock() {
        cache.remove(&device_path);
    }
    // mutation 成功后清除该设备的退避状态：写入已改变设备状态，旧退避计数不再适用。
    if let Ok(mut backoff) = state.backoff_state.lock() {
        if let Some(b) = backoff.get_mut(&device_path) {
            b.reset();
        }
    }
    let _ = app.emit("device-updated", &snapshot);
    let _ = app.emit("device-snapshots-updated", device_snapshot_entries(&state));
    let _ = update_tray(app, Some(&snapshot), &cached_settings(app));
    request_refresh(&state);
    Ok(snapshot)
}

fn autostart_entry(
    app: &AppHandle,
    settings: &AppSettings,
) -> Result<auto_launch::AutoLaunch, String> {
    let mut builder = AutoLaunchBuilder::new();
    let package = app.package_info();
    builder.set_app_name(package.name.as_ref());
    builder.set_args(&autostart_args(settings));

    let current_exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve current executable: {err}"))?;

    #[cfg(target_os = "windows")]
    builder.set_app_path(&current_exe.display().to_string());

    #[cfg(target_os = "macos")]
    {
        builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::AppleScript);
        let exe_path = current_exe
            .canonicalize()
            .map_err(|err| format!("failed to resolve app bundle path: {err}"))?
            .display()
            .to_string();
        let parts: Vec<&str> = exe_path.split(".app/").collect();
        let app_path = if parts.len() == 2 {
            format!("{}.app", parts.first().unwrap())
        } else {
            exe_path
        };
        builder.set_app_path(&app_path);
    }

    #[cfg(target_os = "linux")]
    if let Some(appimage) = app
        .env()
        .appimage
        .and_then(|path| path.to_str().map(|path| path.to_string()))
    {
        builder.set_app_path(&appimage);
    } else {
        builder.set_app_path(&current_exe.display().to_string());
    }

    builder
        .build()
        .map_err(|err| format!("failed to prepare autostart entry: {err}"))
}

#[tauri::command]
fn autostart_state(app: tauri::AppHandle) -> Result<bool, String> {
    let settings = cached_settings(&app);
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut enabled = autostart_entry(&app, &settings)?
        .is_enabled()
        .map_err(|err| format!("failed to read autostart state: {err}"))?;
    #[cfg(target_os = "macos")]
    {
        enabled = migrate_legacy_launch_agent(&app, enabled)?;
    }
    Ok(enabled)
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let settings = cached_settings(&app);
    let autolaunch = autostart_entry(&app, &settings)?;
    if enabled {
        #[cfg(target_os = "macos")]
        if running_from_disk_image() {
            remove_legacy_launch_agent(&app)?;
            return Err(
                "Mira is running from a mounted DMG. Drag Mira to Applications before enabling launch at login."
                    .to_string(),
            );
        }
        if autolaunch.is_enabled().unwrap_or(false) {
            autolaunch
                .disable()
                .map_err(|err| format!("failed to replace autostart entry: {err}"))?;
        }
        autolaunch
            .enable()
            .map_err(|err| format!("failed to enable autostart: {err}"))?;
        #[cfg(target_os = "macos")]
        remove_legacy_launch_agent(&app)?;
        Ok(())
    } else {
        autolaunch
            .disable()
            .map_err(|err| format!("failed to disable autostart: {err}"))?;
        #[cfg(target_os = "macos")]
        remove_legacy_launch_agent(&app)?;
        Ok(())
    }
}

fn refresh_autostart_entry_if_enabled(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    if running_from_disk_image() {
        return;
    }

    let settings = cached_settings(app);
    let Ok(autolaunch) = autostart_entry(app, &settings) else {
        return;
    };
    let Ok(true) = autolaunch.is_enabled() else {
        return;
    };
    if let Err(err) = autolaunch.disable() {
        eprintln!("[mira] refresh autostart entry disable failed: {err}");
        return;
    }
    if let Err(err) = autolaunch.enable() {
        eprintln!("[mira] refresh autostart entry enable failed: {err}");
    }
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    if !is_allowed_external_url(&url) {
        return Err("unsupported external URL".to_string());
    }

    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(&url).status();

    #[cfg(target_os = "windows")]
    let status = {
        let mut command = windows_hidden_command("rundll32");
        command
            .args(["url.dll,FileProtocolHandler", url.as_str()])
            .status()
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(&url).status();

    status
        .map_err(|err| format!("failed to open external URL: {err}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!("failed to open external URL: {status}"))
            }
        })
}

fn is_allowed_external_url(url: &str) -> bool {
    let is_web_url = url.starts_with("https://") || url.starts_with("http://");
    is_web_url && !url.chars().any(|ch| ch.is_control() || ch.is_whitespace())
}

#[cfg(target_os = "macos")]
fn legacy_launch_agent_path(app: &AppHandle) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{}.plist", app.package_info().name)),
    )
}

#[cfg(target_os = "macos")]
fn migrate_legacy_launch_agent(app: &AppHandle, enabled: bool) -> Result<bool, String> {
    let legacy_enabled = legacy_launch_agent_path(app).is_some_and(|path| path.exists());
    if !legacy_enabled {
        return Ok(enabled);
    }

    if !enabled {
        if running_from_disk_image() {
            remove_legacy_launch_agent(app)?;
            return Ok(false);
        }
        let settings = cached_settings(app);
        autostart_entry(app, &settings)?
            .enable()
            .map_err(|err| format!("failed to migrate autostart item: {err}"))?;
    }
    remove_legacy_launch_agent(app)?;
    Ok(true)
}

#[cfg(target_os = "macos")]
fn remove_legacy_launch_agent(app: &AppHandle) -> Result<(), String> {
    let Some(plist) = legacy_launch_agent_path(app) else {
        return Ok(());
    };
    if !plist.exists() {
        return Ok(());
    }

    let _ = Command::new("launchctl")
        .args(["remove", app.package_info().name.as_ref()])
        .status();
    fs::remove_file(&plist)
        .map_err(|err| format!("failed to remove legacy autostart launch agent: {err}"))
}

#[cfg(target_os = "macos")]
fn running_from_disk_image() -> bool {
    current_app_bundle_path().is_some_and(|path| path.starts_with("/Volumes"))
}

#[cfg(target_os = "macos")]
fn current_app_bundle_path() -> Option<PathBuf> {
    let mut path = std::env::current_exe().ok()?.canonicalize().ok()?;
    loop {
        if path.extension().is_some_and(|ext| ext == "app") {
            return Some(path);
        }
        if !path.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod external_url_tests {
    use super::is_allowed_external_url;

    #[test]
    fn allows_web_urls_only() {
        assert!(is_allowed_external_url("https://hey.run/donate/"));
        assert!(is_allowed_external_url("http://example.test/path"));
        assert!(!is_allowed_external_url("file:///Applications/Mira.app"));
        assert!(!is_allowed_external_url("javascript:alert(1)"));
        assert!(!is_allowed_external_url("https://hey.run/donate/\nopen"));
    }
}

fn active_plugin_versions(app: &AppHandle) -> Result<BTreeMap<String, String>, String> {
    let state = app.state::<SessionState>();
    let plugins = state
        .plugins
        .lock()
        .map_err(|_| "plugin state lock failed".to_string())?;
    Ok(plugins
        .as_ref()
        .into_iter()
        .flatten()
        .map(|plugin| (plugin.0.plugin_id.clone(), plugin.0.version.clone()))
        .collect())
}

#[tauri::command]
async fn plugin_updates_check(app: tauri::AppHandle) -> Result<Vec<PluginUpdateInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let current = active_plugin_versions(&app)?;
        plugin_updates_for_versions(&current, fetch_plugin_registry()?)
    })
    .await
    .map_err(|error| format!("plugin update task failed: {error}"))?
}

fn find_installed_plugin_path(
    directory: &std::path::Path,
    plugin_id: &str,
    trust: &TrustStore,
) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("mira-plugin")
                && !path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.contains(".rollback."))
        })
        .filter_map(|path| {
            let inspection = fs::read(&path)
                .ok()
                .and_then(|bytes| inspect_package(Cursor::new(bytes), trust, true).ok())?;
            (inspection.plugin_id == plugin_id)
                .then(|| {
                    semver::Version::parse(&inspection.version)
                        .ok()
                        .map(|version| (version, path))
                })
                .flatten()
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, path)| path)
}

fn install_plugin_update(app: &AppHandle, plugin_id: &str) -> Result<PluginInstallResult, String> {
    let current = active_plugin_versions(app)?;
    let previous_version = current
        .get(plugin_id)
        .cloned()
        .ok_or_else(|| format!("plugin {plugin_id} is not installed"))?;
    let registry = fetch_plugin_registry()?;
    let entry = registry
        .plugins
        .into_iter()
        .find(|entry| entry.plugin_id == plugin_id)
        .ok_or_else(|| format!("plugin {plugin_id} is not in the update registry"))?;
    let current_semver = semver::Version::parse(&previous_version)
        .map_err(|error| format!("invalid installed version: {error}"))?;
    let next_semver = semver::Version::parse(&entry.version)
        .map_err(|error| format!("invalid registry version: {error}"))?;
    if next_semver <= current_semver {
        return Err(format!("plugin {plugin_id} is already up to date"));
    }
    let allowed_prefix = "https://github.com/hello-yunshu/mira-mouse-plugins/releases/download/";
    if !entry.url.starts_with(allowed_prefix) {
        return Err("plugin asset URL is outside the trusted release origin".into());
    }
    let bytes = fetch_bounded(&entry.url, MAX_PLUGIN_BYTES)?;
    let actual_sha = hex::encode(Sha256::digest(&bytes));
    if actual_sha != entry.sha256 {
        return Err(format!(
            "plugin SHA-256 mismatch: expected {}, got {actual_sha}",
            entry.sha256
        ));
    }
    let trust = production_trust_store();
    let (inspection, files) = extract_package(Cursor::new(&bytes), &trust, true)
        .map_err(|error| format!("plugin signature or package validation failed: {error}"))?;
    if inspection.plugin_id != entry.plugin_id || inspection.version != entry.version {
        return Err("plugin registry identity does not match signed package".into());
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        files
            .get("plugin.json")
            .ok_or("signed package has no plugin.json")?,
    )
    .map_err(|error| format!("parse signed plugin manifest: {error}"))?;
    if manifest
        .get("publisherKeyId")
        .and_then(serde_json::Value::as_str)
        != Some(entry.publisher_key_id.as_str())
    {
        return Err("plugin registry publisher does not match signed package".into());
    }
    let devices = hid::parse_devices_json(
        files
            .get("devices.json")
            .ok_or("signed package has no devices.json")?,
    )?;

    let directory = installed_plugins_dir(app)?;
    fs::create_dir_all(&directory).map_err(|error| format!("create plugin directory: {error}"))?;
    let previous_path = find_installed_plugin_path(&directory, plugin_id, &trust);
    let backup_path = directory.join(format!("{plugin_id}.rollback.mira-plugin"));
    if backup_path.exists() {
        fs::remove_file(&backup_path).map_err(|error| format!("remove stale rollback: {error}"))?;
    }
    if let Some(path) = &previous_path {
        fs::rename(path, &backup_path)
            .map_err(|error| format!("prepare plugin rollback: {error}"))?;
    }
    let final_path = directory.join(format!("{plugin_id}-{}.mira-plugin", entry.version));
    let write_result = (|| -> Result<(), String> {
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|error| format!("create plugin temporary file: {error}"))?;
        std::io::Write::write_all(&mut temporary, &bytes)
            .map_err(|error| format!("write plugin temporary file: {error}"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| format!("sync plugin temporary file: {error}"))?;
        temporary
            .persist(&final_path)
            .map_err(|error| format!("install plugin atomically: {}", error.error))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        if backup_path.exists() {
            if let Some(path) = &previous_path {
                let _ = fs::rename(&backup_path, path);
            }
        }
        return Err(error);
    }

    let runtime_result = {
        let state = app.state::<SessionState>();
        let result = state
            .plugins
            .lock()
            .map_err(|_| "plugin state lock failed".to_string())
            .map(|mut plugins| {
                let active = plugins.get_or_insert_with(Vec::new);
                active.retain(|plugin| plugin.0.plugin_id != plugin_id);
                active.push((inspection, devices, files));
            });
        // 插件集合变化后清空 ProtocolPackage 缓存，避免使用旧版本解析结果。
        invalidate_package_cache(&state);
        result
    };
    if let Err(error) = runtime_result {
        let _ = fs::remove_file(&final_path);
        if backup_path.exists() {
            if let Some(path) = &previous_path {
                let _ = fs::rename(&backup_path, path);
            }
        }
        return Err(error);
    }
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }
    // 前端需重新装载当前活动插件的 locale；否则设备运行时已切到新包，
    // 标签仍停留在旧包的 namespace 中。
    let _ = app.emit("plugin-locales-updated", ());
    Ok(PluginInstallResult {
        plugin_id: plugin_id.to_string(),
        version: entry.version,
        previous_version,
        restarted_runtime: true,
    })
}

#[tauri::command]
async fn plugin_update_install(
    app: tauri::AppHandle,
    plugin_id: String,
) -> Result<PluginInstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || install_plugin_update(&app, &plugin_id))
        .await
        .map_err(|error| format!("plugin install task failed: {error}"))?
}

#[tauri::command]
fn about_info(app: tauri::AppHandle) -> Result<AboutInfo, String> {
    let package = app.package_info();
    let bundled = active_plugins_info(&app);
    Ok(AboutInfo {
        name: package.name.to_string(),
        version: package.version.to_string(),
        identifier: app.config().identifier.to_string(),
        platform: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        build_date: env!("BUILD_DATE").to_string(),
        git_commit: env!("GIT_COMMIT").to_string(),
        bundled_plugins: bundled,
        contact: load_contact_links(),
        updater_active: read_lock_file().is_some_and(|lock| lock.release_ready),
    })
}

#[tauri::command]
fn settings_get(app: tauri::AppHandle) -> Result<AppSettings, String> {
    Ok(cached_settings(&app))
}

#[tauri::command]
fn settings_set(app: tauri::AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    let settings = settings.normalized();
    let previous_settings = cached_settings(&app);
    let theme_changed = previous_settings.theme != settings.theme;
    let tray_icon_color_changed = previous_settings.tray_icon_color != settings.tray_icon_color;
    let start_hidden_changed = previous_settings.start_hidden != settings.start_hidden;
    let low_battery_threshold_changed =
        previous_settings.low_battery_threshold != settings.low_battery_threshold;
    // 安静灯光设置变更时唤醒调度器立即重新评估阶段。
    let night_mode_changed = previous_settings.night_mode_enabled != settings.night_mode_enabled
        || previous_settings.night_mode_start != settings.night_mode_start
        || previous_settings.night_mode_end != settings.night_mode_end
        || previous_settings.night_mode_trigger_time != settings.night_mode_trigger_time
        || previous_settings.night_mode_trigger_theme != settings.night_mode_trigger_theme
        || previous_settings.night_mode_theme_dark != settings.night_mode_theme_dark
        || previous_settings.night_mode_trigger_charging != settings.night_mode_trigger_charging
        || previous_settings.night_mode_trigger_low_battery
            != settings.night_mode_trigger_low_battery
        || previous_settings.night_mode_target_mouse != settings.night_mode_target_mouse
        || previous_settings.night_mode_target_receiver != settings.night_mode_target_receiver
        || (settings.night_mode_trigger_low_battery
            && previous_settings.low_battery_threshold != settings.low_battery_threshold);
    save_settings(&app, &settings)?;
    update_cached_settings(&app, &settings);
    if start_hidden_changed {
        refresh_autostart_entry_if_enabled(&app);
    }
    if theme_changed {
        apply_saved_app_theme(&app, &settings);
        #[cfg(target_os = "windows")]
        if let Some(window) = app.get_webview_window("main") {
            apply_windows_backdrop(&window, &settings);
        }
    }
    if tray_icon_color_changed {
        // tray_icon_color 变化会改变 TrayVisualStyle → TrayIconCacheKey，
        // 控制器的 needs_update() 自然返回 true，无需手动失效缓存。
    }
    let state = app.state::<SessionState>();
    let snapshot = selected_snapshot(&state);
    update_tray(&app, snapshot.as_ref(), &settings)
        .map_err(|error| format!("update tray: {error}"))?;
    if low_battery_threshold_changed {
        let battery_value = snapshot
            .as_ref()
            .and_then(tray::state::low_battery_notification_value);
        app.state::<SessionState>()
            .low_battery
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sync(battery_value, settings.low_battery_threshold);
    }
    if night_mode_changed {
        request_night_mode_eval(&app.state::<SessionState>());
    }
    Ok(settings)
}

#[tauri::command]
fn export_diagnostics(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let package = app.package_info();
    let bundled = active_plugins_info(&app);
    let state = app.state::<SessionState>();
    let read_errors: Vec<DeviceReadDiagnostic> = state
        .last_read_errors
        .lock()
        .map(|errors| errors.values().cloned().collect())
        .unwrap_or_default();
    // 宿主诊断指标：投影、计划、命令数、延迟、退避。
    // 本地、脱敏、有界：不含原始 HID path、序列号或用户路径。
    let device_metrics: Vec<DeviceMetrics> = state
        .device_metrics
        .lock()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();
    let global_metrics = state
        .global_metrics
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let hid_io = state
        .hid_io_stats
        .lock()
        .map(|stats| HidIoDiagnostics::from(*stats))
        .unwrap_or_default();
    // Diagnostics are intentionally minimal and sanitized: no serial numbers,
    // no HID report payloads, no raw HID paths. Device errors use only
    // VID/PID/usage and the plugin-declared family.
    Ok(serde_json::json!({
        "app": {
            "name": package.name,
            "version": package.version.to_string(),
            "identifier": app.config().identifier,
        },
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "rust_version": env!("CARGO_PKG_RUST_VERSION"),
        "autostart_enabled": autostart_entry(&app, &cached_settings(&app))
            .and_then(|autostart| {
                autostart
                    .is_enabled()
                    .map_err(|err| format!("failed to read autostart state: {err}"))
            })
            .unwrap_or(false),
        "bundled_plugins": bundled,
        "device_read_errors": read_errors,
        "host_diagnostics": HostDiagnostics {
            global: global_metrics,
            devices: device_metrics,
            hid_io,
        },
        "updater_active": read_lock_file().is_some_and(|lock| lock.release_ready),
        "note": "Diagnostics contain no device serial numbers, HID payloads, or user-identifying data.",
    }))
}

fn focus_main(window: Option<WebviewWindow>) {
    if let Some(window) = window {
        // macOS: 先恢复 Regular 激活策略，再 show/set_focus。
        // 顺序很重要：Accessory 状态下 show 的窗口无法获取焦点。
        #[cfg(target_os = "macos")]
        {
            use tauri::ActivationPolicy;
            let _ = window
                .app_handle()
                .set_activation_policy(ActivationPolicy::Regular);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main_window_to_tray(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
        #[cfg(target_os = "macos")]
        {
            use tauri::ActivationPolicy;
            let _ = app.set_activation_policy(ActivationPolicy::Accessory);
        }
    }
}

fn app_theme_from_settings(settings: &AppSettings) -> Option<Theme> {
    match settings.theme.as_str() {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    }
}

fn apply_saved_app_theme(app: &AppHandle, settings: &AppSettings) {
    app.set_theme(app_theme_from_settings(settings));
}

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(test, allow(dead_code))]
fn settings_prefer_dark(app: &AppHandle, settings: &AppSettings) -> bool {
    match settings.theme.as_str() {
        "dark" => true,
        "light" => false,
        _ => detect_system_dark(app),
    }
}

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(test, allow(dead_code))]
fn windows_acrylic_tint(app: &AppHandle, settings: &AppSettings) -> (u8, u8, u8, u8) {
    if settings_prefer_dark(app, settings) {
        (24, 24, 28, 168)
    } else {
        (247, 245, 248, 184)
    }
}

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(test, allow(dead_code))]
fn apply_windows_backdrop(window: &WebviewWindow, settings: &AppSettings) {
    use window_vibrancy::{apply_acrylic, apply_mica};

    // 优先使用 Acrylic（半透明毛玻璃），匹配用户对「毛玻璃半透明」的预期；
    // Mica 为不透明壁纸着色材质，仅作为 Acrylic 不可用时的兜底。
    if let Err(acrylic_error) = apply_acrylic(
        window,
        Some(windows_acrylic_tint(window.app_handle(), settings)),
    ) {
        if let Err(mica_error) = apply_mica(window, None) {
            eprintln!(
                "[mira] Windows backdrop unavailable: Acrylic: {acrylic_error}; Mica: {mica_error}"
            );
        }
    }
}

/// macOS Vibrancy 背景：Sidebar 材质 + Active 状态。
///
/// `NSVisualEffectState::Active` 期望始终保持活跃外观，但 macOS 在窗口失焦后
/// 仍可能对 Sidebar 材质应用系统级 dimming（暗色模式下表现为「一段时间后变黑」）。
/// 因此在 `Focused(true)` 事件中重新调用本函数，强制刷新 vibrancy 状态。
#[cfg(target_os = "macos")]
fn apply_macos_vibrancy(window: &WebviewWindow) {
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
    let _ = apply_vibrancy(
        window,
        NSVisualEffectMaterial::Sidebar,
        Some(NSVisualEffectState::Active),
        None,
    );
}

#[tauri::command]
fn hide_to_tray(app: tauri::AppHandle) {
    hide_main_window_to_tray(&app);
}

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
fn navigate_about_update(app: &AppHandle) {
    focus_main(app.get_webview_window("main"));
    let _ = app.emit("navigate-about-update", ());
}

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
fn navigate_plugin_update(app: &AppHandle) {
    focus_main(app.get_webview_window("main"));
    let _ = app.emit("navigate-plugin-update", ());
}

/// 发送原生系统通知，可选地携带点击跳转动作。
/// `action` 目前支持 `"about-update"`、`"settings-plugin-update"` 与 `"battery-usage"`。
/// - macOS：`tauri-plugin-notification` 不暴露点击回调，改将 action 写入
///   `pending_notification_action`，由前端窗口 focus 时通过
///   `take_pending_notification_action` 取走并执行跳转。
/// - Windows/Linux：`notify-rust` 的 `wait_for_action` 直接处理点击并 emit 事件。
#[tauri::command]
fn show_update_notification(
    app: tauri::AppHandle,
    state: tauri::State<SessionState>,
    title: String,
    body: String,
    action: Option<String>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if let Some(a) = &action {
            if let Ok(mut guard) = state.pending_notification_action.lock() {
                *guard = Some(a.clone());
            }
        }
        let _ = state;
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|err| format!("failed to show update notification: {err}"))?;
        Ok(())
    }

    #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
    {
        let _ = &state;
        let identifier = app.config().identifier.clone();
        let navigate = action.clone();
        std::thread::spawn(move || {
            let mut notification = notify_rust::Notification::new();
            notification
                .summary(&title)
                .body(&body)
                .timeout(notify_rust::Timeout::Never);
            #[cfg(target_os = "windows")]
            notification.app_id(&identifier);
            #[cfg(all(unix, not(target_os = "macos")))]
            notification.appname(&identifier);
            match notification.show() {
                Ok(handle) => {
                    if let Some(action) = navigate {
                        handle.wait_for_action(|action_kind| {
                            if action_kind != "__closed" {
                                match action.as_str() {
                                    "about-update" => navigate_about_update(&app),
                                    "settings-plugin-update" => navigate_plugin_update(&app),
                                    _ => {}
                                }
                            }
                        });
                    }
                }
                Err(error) => eprintln!("[mira] update notification failed: {error}"),
            }
        });
        Ok(())
    }
}

/// 取走并返回 macOS 原生通知点击后待执行的跳转动作（供前端窗口 focus 时调用）。
/// Windows/Linux 不使用此机制（直接由 `wait_for_action` 处理），始终返回 `None`。
#[tauri::command]
fn take_pending_notification_action(state: tauri::State<SessionState>) -> Option<String> {
    state
        .pending_notification_action
        .lock()
        .map(|mut guard| guard.take())
        .unwrap_or(None)
}

// ─── 电量使用情况 Tauri 命令 ─────────────────────────────────────────────────

/// 获取电量历史（24 小时或 10 天聚合 + 洞察分析）。
#[tauri::command]
fn battery_history_get(
    state: tauri::State<SessionState>,
    range: String,
) -> Result<BatteryHistoryResponse, String> {
    let settings = cached_settings_for_state(&state);
    let range_str = if range == "24h" { "24h" } else { "10d" };
    Ok(battery_history::build_response(
        &state.battery_history,
        settings.low_battery_threshold,
        range_str,
    ))
}

/// 清除电量历史。不影响低电量通知设置。
#[tauri::command]
fn battery_history_clear(
    state: tauri::State<SessionState>,
    app: tauri::AppHandle,
    device_key: Option<String>,
) -> Result<(), String> {
    battery_history::clear_history(&state.battery_history, &app, device_key.as_deref())
}

/// 导出电量历史。默认 JSON，可选 CSV。仅包含脱敏后的电量历史数据。
/// 如果提供 path，直接写入文件；否则返回内容字符串。
#[tauri::command]
fn battery_history_export(
    state: tauri::State<SessionState>,
    format: Option<String>,
    path: Option<String>,
) -> Result<String, String> {
    let fmt = format.as_deref().unwrap_or("json");
    let content = battery_history::export_history(&state.battery_history, fmt)?;
    if let Some(p) = path {
        std::fs::write(&p, &content).map_err(|e| format!("write failed: {e}"))?;
        Ok(String::new())
    } else {
        Ok(content)
    }
}

/// 从 SessionState 读取缓存的设置（避免在 command 中需要 AppHandle）。
fn cached_settings_for_state(state: &SessionState) -> AppSettings {
    state
        .cached_settings
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default()
}

pub(crate) fn focus_main_from_tray(app: &AppHandle) {
    focus_main(app.get_webview_window("main"));
    request_presence_refresh(&app.state::<SessionState>());
}

pub(crate) fn open_battery_usage_from_tray(app: &AppHandle) {
    let state = app.state::<SessionState>();
    if let Ok(mut guard) = state.pending_notification_action.lock() {
        *guard = Some("battery-usage".to_string());
    }
    focus_main_from_tray(app);
    let _ = app.emit("open-battery-usage", ());
}

const TRAY_ID: &str = "mira-status";

/// Resolve a settings language value ("auto"|"zh-CN"|"en") to a concrete language code.
/// "auto" follows the system locale via `sys_locale`; defaults to Chinese when undetectable.
fn effective_language(settings_language: &str) -> &'static str {
    match settings_language {
        "zh-CN" => "zh-CN",
        "en" => "en",
        _ => {
            let locale = get_locale().unwrap_or_default();
            if locale.starts_with("en") {
                "en"
            } else {
                "zh-CN"
            }
        }
    }
}

fn tr_connection(connection: mira_core::Connection, lang: &str) -> &'static str {
    match (connection, lang) {
        (mira_core::Connection::Usb, _) => "USB",
        (mira_core::Connection::Wireless, "en") => "Wireless",
        (mira_core::Connection::Wireless, _) => "无线",
        (mira_core::Connection::Bluetooth, "en") => "Bluetooth",
        (mira_core::Connection::Bluetooth, _) => "蓝牙",
        (mira_core::Connection::Virtual, "en") => "Virtual",
        (mira_core::Connection::Virtual, _) => "虚拟",
    }
}

pub(crate) fn tr_open(lang: &str) -> &'static str {
    if lang == "en" {
        "Open Mira"
    } else {
        "打开 Mira"
    }
}

pub(crate) fn tr_quit(lang: &str) -> &'static str {
    if lang == "en" {
        "Quit Mira"
    } else {
        "退出 Mira"
    }
}

fn tr_disconnected(lang: &str) -> &'static str {
    if lang == "en" {
        "No supported mouse connected"
    } else {
        "未连接受支持的鼠标"
    }
}

pub(crate) fn tr_charging_suffix(lang: &str) -> &'static str {
    if lang == "en" {
        " · Charging"
    } else {
        " · 充电中"
    }
}

fn tr_mouse_label(lang: &str) -> &'static str {
    if lang == "en" {
        "M"
    } else {
        "鼠"
    }
}

fn tr_receiver_label(lang: &str) -> &'static str {
    if lang == "en" {
        "R"
    } else {
        "接"
    }
}

fn tr_battery_fallback_label(lang: &str) -> &'static str {
    if lang == "en" {
        "Mouse"
    } else {
        "鼠标"
    }
}

fn tr_receiver_battery_label(lang: &str) -> &'static str {
    if lang == "en" {
        "Receiver"
    } else {
        "接收器"
    }
}

fn tr_battery_label(lang: &str, id: &str, label: &str) -> String {
    match label {
        "mock.mouseLabel" => tr_battery_fallback_label(lang).to_string(),
        "mock.receiverLabel" => tr_receiver_battery_label(lang).to_string(),
        _ if label.starts_with("mock.") => match id {
            "receiver" => tr_receiver_battery_label(lang).to_string(),
            "mouse" => tr_battery_fallback_label(lang).to_string(),
            _ => tr_battery_fallback_label(lang).to_string(),
        },
        _ => label.to_string(),
    }
}

fn tr_low_battery_title(lang: &str) -> &'static str {
    if lang == "en" {
        "Low battery alert"
    } else {
        "低电量提醒"
    }
}

fn tr_low_battery_body(lang: &str, threshold: u8, percent: u8) -> String {
    if lang == "en" {
        format!(
            "Mouse battery is below {}% (currently {}%)",
            threshold, percent
        )
    } else {
        format!("鼠标电量已低于 {}%（当前 {}%）", threshold, percent)
    }
}

fn tr_abnormal_drain_title(lang: &str) -> &'static str {
    if lang == "en" {
        "Unusual battery drain"
    } else {
        "异常耗电"
    }
}

fn tr_abnormal_drain_body(lang: &str, device_name: &str) -> String {
    if lang == "en" {
        format!(
            "Recent drain rate is noticeably higher than usual for {}.",
            device_name
        )
    } else {
        format!("{} 最近掉电速度明显高于平时。", device_name)
    }
}

fn tr_night_mode_title(lang: &str) -> &'static str {
    if lang == "en" {
        "Quiet lighting"
    } else {
        "安静灯光"
    }
}

/// 将设备写入/读取产生的原始协议错误转换为用户可读的提示。
///
/// 安静灯光调度器每分钟重试一次，原始错误（如
/// `workflow ... condition at offset 5 timed out`、`proxy target is offline`）
/// 对普通用户没有意义。这类超时通常意味着 2.4G 接收器无法与鼠标建立链路
/// （鼠标休眠或离线），移动鼠标后会自动恢复，因此翻译为统一提示。
/// 其它（配置/协议校验类）错误按原样返回，便于排查。
fn friendly_device_error(error: &str, lang: &str) -> String {
    let lower = error.to_ascii_lowercase();
    let unreachable = lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("proxy target is offline")
        || lower.contains("offline");
    if unreachable {
        if lang == "en" {
            "Mouse not responding; it may be sleeping. Mira will retry automatically once you move the mouse.".to_string()
        } else {
            "鼠标未响应，可能处于休眠状态。移动鼠标后 Mira 将自动重试。".to_string()
        }
    } else {
        error.to_string()
    }
}

fn tr_night_mode_off_failed(lang: &str, error: &str) -> String {
    let detail = friendly_device_error(error, lang);
    if lang == "en" {
        format!("Failed to turn off mouse lighting: {detail}")
    } else {
        format!("关闭鼠标灯光失败：{detail}")
    }
}

fn tr_night_mode_restore_failed(lang: &str, error: &str) -> String {
    let detail = friendly_device_error(error, lang);
    if lang == "en" {
        format!("Failed to restore mouse lighting: {detail}")
    } else {
        format!("恢复鼠标灯光失败：{detail}")
    }
}

fn tr_night_mode_off_failed_receiver(lang: &str, error: &str) -> String {
    let detail = friendly_device_error(error, lang);
    if lang == "en" {
        format!("Failed to turn off receiver lighting: {detail}")
    } else {
        format!("关闭接收器灯光失败：{detail}")
    }
}

fn tr_night_mode_restore_failed_receiver(lang: &str, error: &str) -> String {
    let detail = friendly_device_error(error, lang);
    if lang == "en" {
        format!("Failed to restore receiver lighting: {detail}")
    } else {
        format!("恢复接收器灯光失败：{detail}")
    }
}

fn tr_tooltip_connected(_lang: &str, connection: &str, name: &str) -> String {
    format!("Mira · {} · {}", connection, name)
}

pub(crate) fn tr_tooltip_disconnected(lang: &str) -> String {
    if lang == "en" {
        "Mira · No supported mouse connected".to_string()
    } else {
        "Mira · 未连接受支持的鼠标".to_string()
    }
}

fn tr_connection_status(lang: &str, connection: &str, name: &str) -> String {
    if lang == "en" {
        format!("Connection: {} · {}", connection, name)
    } else {
        format!("连接：{} · {}", connection, name)
    }
}

pub(crate) fn tr_battery_item(lang: &str, label: &str, percentage: u8, charging: bool) -> String {
    let charging_suffix = if charging {
        tr_charging_suffix(lang)
    } else {
        ""
    };
    if lang == "en" {
        format!("{} battery: {}%{}", label, percentage, charging_suffix)
    } else {
        format!("{}电量：{}%{}", label, percentage, charging_suffix)
    }
}

pub(crate) fn connection_label(connection: mira_core::Connection, lang: &str) -> &'static str {
    tr_connection(connection, lang)
}

fn battery_title(snapshot: &DeviceSnapshot, settings: &AppSettings) -> Option<String> {
    if !settings.tray_show_battery_title {
        return None;
    }
    let lang = effective_language(&settings.language);
    let mouse_percentage = tray::state::mouse_battery_percentage(snapshot)?;
    let mut title = format!("{mouse_percentage}%");
    if settings.tray_include_receiver_battery {
        if let Some(receiver) = snapshot
            .batteries
            .iter()
            .find(|battery| battery.id == "receiver")
        {
            title = format!(
                "{} {mouse_percentage}% · {} {}%",
                tr_mouse_label(lang),
                tr_receiver_label(lang),
                receiver.percentage
            );
        }
    }
    Some(title)
}

// Battery helpers and static PNG icon arrays have been migrated to:
//   crate::tray::state::{mouse_battery_percentage, mouse_battery_charging, low_battery_notification_value}
//   crate::tray::static_icon::{static_tray_icon_bytes, static_tray_app_icon_bytes, static_tray_app_icon_bytes_for_theme}

#[cfg(any(not(target_os = "macos"), test))]
fn app_icon_bytes_for_theme(dark: bool) -> &'static [u8] {
    if dark {
        include_bytes!("../icons/icon-dark.png")
    } else {
        include_bytes!("../icons/icon.png")
    }
}

#[cfg(target_os = "macos")]
fn app_icon_icns_bytes_for_theme(dark: bool) -> &'static [u8] {
    if dark {
        include_bytes!("../icons/icon-dark.icns")
    } else {
        include_bytes!("../icons/icon.icns")
    }
}

#[cfg(target_os = "macos")]
fn set_macos_dock_icon(icon_bytes: &'static [u8]) {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let data = NSData::with_bytes(icon_bytes);
    if let Some(app_icon) = NSImage::initWithData(NSImage::alloc(), &data) {
        unsafe { app.setApplicationIconImage(Some(&app_icon)) };
    }
}

fn update_runtime_app_icon(app: &AppHandle, dark: bool) {
    #[cfg(target_os = "macos")]
    {
        let icon_bytes = app_icon_icns_bytes_for_theme(dark);
        let _ = app.run_on_main_thread(move || set_macos_dock_icon(icon_bytes));
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(app_icon) = tauri::image::Image::from_bytes(app_icon_bytes_for_theme(dark)) {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_icon(app_icon);
            }
        }
    }
}

/// 直接查询系统外观，不依赖窗口状态。
/// macOS: `defaults read -g AppleInterfaceStyle`（Light 模式返回非零退出码，Dark 返回 "Dark"）
/// Windows: 直接读注册表 `AppsUseLightTheme`（0=Dark, 1=Light）
/// Linux: 回退到 window.theme()
fn detect_system_dark(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        {
            return output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .eq_ignore_ascii_case("dark");
        }
    }
    #[cfg(target_os = "windows")]
    {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};

        if let Ok(personalize) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        {
            if let Ok(apps_use_light_theme) = personalize.get_value::<u32, _>("AppsUseLightTheme") {
                return apps_use_light_theme == 0;
            }
        }
    }
    // Linux 或命令不可用时，回退到 window.theme()
    app.get_webview_window("main")
        .and_then(|window| window.theme().ok())
        .map(|theme| theme == tauri::Theme::Dark)
        .unwrap_or(false)
}

/// 系统主题变化时的统一处理：检测主题、更新缓存、刷新 app 图标、托盘图标、
/// Windows 背景效果，并唤醒安静灯光调度器。
/// 由 `ThemeChanged` 窗口事件和 `spawn_theme_watcher` 后台监听器调用。
fn handle_system_theme_changed(app: &AppHandle) {
    let dark = detect_system_dark(app);
    let state = app.state::<SessionState>();
    let theme_changed = {
        let cache = state.system_dark.lock().unwrap_or_else(|e| e.into_inner());
        *cache != Some(dark)
    };
    if !theme_changed {
        return;
    }
    if let Ok(mut cache) = state.system_dark.lock() {
        *cache = Some(dark);
    }
    update_runtime_app_icon(app, dark);
    let snapshot = state.last_snapshot.lock().ok().and_then(|guard| {
        selected_snapshot_entry(&state, &guard).map(|(_, snapshot)| snapshot.clone())
    });
    let settings = cached_settings(app);
    let _ = update_tray(app, snapshot.as_ref(), &settings);
    #[cfg(target_os = "windows")]
    if let Some(window) = app.get_webview_window("main") {
        apply_windows_backdrop(&window, &settings);
    }
    request_night_mode_eval(&state);
}

/// 启动系统主题变化监听器（事件驱动）。
/// - macOS: `NSDistributedNotificationCenter` 监听 `AppleInterfaceThemeChangedNotification`
/// - Windows: `RegNotifyChangeKeyValue` 监听注册表变化
/// - Linux: `gsettings monitor` 监听 GNOME 颜色方案变化
///
/// 窗口隐藏到托盘后仍可接收事件，无需轮询。
fn spawn_theme_watcher(app: AppHandle) {
    #[cfg(target_os = "macos")]
    install_macos_theme_watcher(app);

    #[cfg(target_os = "windows")]
    spawn_windows_theme_watcher(app);

    #[cfg(target_os = "linux")]
    spawn_linux_theme_watcher(app);
}

#[cfg(target_os = "macos")]
fn install_macos_theme_watcher(app: AppHandle) {
    use std::ffi::c_void;
    use std::ptr;

    type CFNotificationCenterRef = *mut c_void;
    type CFStringRef = *const c_void;
    type CFNotificationCallback = extern "C" fn(
        center: CFNotificationCenterRef,
        observer: *mut c_void,
        name: CFStringRef,
        object: *const c_void,
        user_info: *const c_void,
    );

    extern "C" {
        fn CFNotificationCenterGetDistributedCenter() -> CFNotificationCenterRef;
        fn CFNotificationCenterAddObserver(
            center: CFNotificationCenterRef,
            observer: *const c_void,
            call_back: CFNotificationCallback,
            name: CFStringRef,
            object: *const c_void,
            suspension_behavior: u32,
        );
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const u8,
            encoding: u32,
        ) -> CFStringRef;
    }

    // kCFStringEncodingUTF8 = 0x08000100
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;
    // CFNotificationSuspensionBehaviorDeliverImmediately = 2
    const DELIVER_IMMEDIATELY: u32 = 2;

    extern "C" fn theme_changed_callback(
        _center: CFNotificationCenterRef,
        observer: *mut c_void,
        _name: CFStringRef,
        _object: *const c_void,
        _user_info: *const c_void,
    ) {
        // observer 指向 Box::leak 的 AppHandle（生命周期与进程相同）
        let app: &AppHandle = unsafe { &*(observer as *const AppHandle) };
        handle_system_theme_changed(app);
    }

    // AppHandle 泄漏到静态内存：observer 生命周期需与进程一样长。
    // 应用退出时进程终止，内存自动回收。
    let app_box: &'static AppHandle = Box::leak(Box::new(app));
    let observer = app_box as *const AppHandle as *const c_void;

    let name = unsafe {
        CFStringCreateWithCString(
            ptr::null(),
            c"AppleInterfaceThemeChangedNotification".as_ptr() as *const u8,
            K_CF_STRING_ENCODING_UTF8,
        )
    };

    let center = unsafe { CFNotificationCenterGetDistributedCenter() };
    unsafe {
        CFNotificationCenterAddObserver(
            center,
            observer,
            theme_changed_callback,
            name,
            ptr::null(),
            DELIVER_IMMEDIATELY,
        );
    }
}

#[cfg(target_os = "windows")]
fn spawn_windows_theme_watcher(app: AppHandle) {
    use std::ffi::c_void;
    use std::ptr;

    type HKEY = *mut c_void;
    type HANDLE = *mut c_void;

    extern "system" {
        fn RegOpenKeyExW(
            h_key: HKEY,
            lp_sub_key: *const u16,
            ul_options: u32,
            sam_desired: u32,
            phk_result: *mut HKEY,
        ) -> i32;
        fn RegNotifyChangeKeyValue(
            h_key: HKEY,
            b_watch_subtree: i32,
            dw_notify_filter: u32,
            h_event: HANDLE,
            f_asynchronous: i32,
        ) -> i32;
        fn RegCloseKey(h_key: HKEY) -> i32;
    }

    const HKEY_CURRENT_USER: usize = 0x80000001;
    const KEY_NOTIFY: u32 = 0x0010;
    const REG_NOTIFY_CHANGE_LAST_SET: u32 = 0x00000004;

    std::thread::spawn(move || {
        let sub_key: Vec<u16> =
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
                .encode_utf16()
                .collect();

        loop {
            let mut h_key: HKEY = ptr::null_mut();
            let result = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER as HKEY,
                    sub_key.as_ptr(),
                    0,
                    KEY_NOTIFY,
                    &mut h_key,
                )
            };

            if result != 0 {
                // 打开失败（如键不存在），等待后重试
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }

            // 同步等待注册表值变化（阻塞直到变化）
            let result = unsafe {
                RegNotifyChangeKeyValue(
                    h_key,
                    0, // 不监听子树
                    REG_NOTIFY_CHANGE_LAST_SET,
                    ptr::null_mut(),
                    0, // 同步模式
                )
            };

            unsafe {
                RegCloseKey(h_key);
            }

            if result == 0 {
                handle_system_theme_changed(&app);
            } else {
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    });
}

#[cfg(target_os = "linux")]
fn spawn_linux_theme_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        use std::process::{Command, Stdio};

        loop {
            // `gsettings monitor` 持续输出颜色方案变化事件，每行一次。
            // 比 `gsettings get` 轮询更高效，且是真正的事件驱动。
            let result = Command::new("gsettings")
                .args(["monitor", "org.gnome.desktop.interface", "color-scheme"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();

            let Ok(mut child) = result else {
                // gsettings 不可用（非 GNOME 桌面），等待后重试
                std::thread::sleep(std::time::Duration::from_secs(10));
                continue;
            };

            if let Some(stdout) = child.stdout.take() {
                let reader = std::io::BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(_) => handle_system_theme_changed(&app),
                        Err(_) => break,
                    }
                }
            }

            let _ = child.wait();
            // gsettings 进程意外退出，等待后重启
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    });
}

/// 读取缓存的系统主题。缓存由 `spawn_theme_watcher`（事件驱动监听器）和
/// `ThemeChanged` 事件更新，避免每次 `update_tray` 都 fork 进程。
fn tray_theme_is_dark(app: &AppHandle) -> bool {
    let state = app.state::<SessionState>();
    if let Ok(cache) = state.system_dark.lock() {
        if let Some(dark) = *cache {
            return dark;
        }
    }
    // 缓存为空（首次调用），直接检测一次并填充缓存
    let dark = detect_system_dark(app);
    if let Ok(mut cache) = state.system_dark.lock() {
        *cache = Some(dark);
    }
    dark
}

fn update_tray(
    app: &AppHandle,
    snapshot: Option<&DeviceSnapshot>,
    settings: &AppSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    if objc2::MainThreadMarker::new().is_none() {
        let app = app.clone();
        let snapshot = snapshot.cloned();
        let settings = settings.clone();
        let app_for_update = app.clone();
        app.run_on_main_thread(move || {
            if let Err(error) = update_tray(&app_for_update, snapshot.as_ref(), &settings) {
                eprintln!("[mira] tray update on main thread failed: {error}");
            }
        })?;
        return Ok(());
    }

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };

    // 构建 tray 模块所需的设置子集和状态
    let tray_settings = tray::state::TraySettings {
        show_receiver: settings.tray_include_receiver_battery,
        show_connection: settings.tray_show_connection,
        show_battery_title: settings.tray_show_battery_title,
        low_battery_threshold: settings.low_battery_threshold,
        tray_icon_color: &settings.tray_icon_color,
        tray_render_mode: &settings.tray_render_mode,
    };
    let tray_state = tray::state::TrayStatusState::from_snapshot(snapshot, &tray_settings);
    #[cfg(target_os = "macos")]
    tray::macos::set_app_lang(effective_language(&settings.language));
    let system_dark = tray_theme_is_dark(app);
    let theme = tray::style::resolve_tray_theme(&tray_settings, system_dark);
    let style = tray::style::TrayVisualStyle::from_settings(&tray_settings, theme);

    // 图标更新委托给平台控制器（内部做 diff，未变化时跳过 set_icon）
    let session = app.state::<SessionState>();
    if let Ok(mut controller) = session.tray_renderer.lock() {
        if let Err(err) = tray::renderer::TrayController::update_icon(
            &mut *controller,
            &tray,
            &tray_state,
            &style,
        ) {
            eprintln!("[mira] tray icon update failed, falling back: {err}");
            // fallback 到静态图标：不 panic，不影响菜单和 tooltip 更新
            let icon_bytes = if tray_state.connected {
                let percentage = tray_state.mouse_battery.unwrap_or(0);
                let level = ((percentage.saturating_add(5)) / 10).min(10) * 10;
                tray::static_icon::static_tray_icon_bytes(
                    level,
                    style.theme.is_dark(),
                    tray_state.mouse_charging,
                )
            } else {
                tray::static_icon::static_tray_app_icon_bytes_for_theme(style.theme.is_dark())
            };
            if let Ok(image) = tauri::image::Image::from_bytes(icon_bytes) {
                let _ = tray.set_icon(Some(image));
                let _ = tray.set_icon_as_template(false);
            }
        }
    }

    let state = app.state::<SessionState>();
    let menu = Menu::new(app)?;

    let lang = effective_language(&settings.language);

    // 计算当前菜单签名，用于判断是否需要重建菜单
    // Clone batteries once and share between signature calculation and menu rebuild.
    let prepared_batteries = snapshot.map(|snapshot| {
        let mut batteries = snapshot.batteries.clone();
        if batteries.is_empty() {
            if let Some(percentage) = snapshot.battery_percent {
                batteries.push(mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: tr_battery_fallback_label(lang).into(),
                    percentage,
                    charging: snapshot.charging,
                });
            }
        }
        batteries
    });
    let current_signature = if let Some(snapshot) = snapshot {
        let batteries = prepared_batteries.as_ref().unwrap();
        TrayMenuSignature {
            connected: true,
            batteries: batteries
                .iter()
                .map(|b| {
                    (
                        tr_battery_label(lang, &b.id, &b.label),
                        b.percentage,
                        b.charging,
                    )
                })
                .collect(),
            show_connection: settings.tray_show_connection,
            connection_label: connection_label(snapshot.connection, lang).to_string(),
            display_name: snapshot.display_name.clone(),
        }
    } else {
        TrayMenuSignature {
            connected: false,
            batteries: Vec::new(),
            show_connection: false,
            connection_label: String::new(),
            display_name: String::new(),
        }
    };

    // 比较签名：相同则跳过菜单重建，仅更新 title/tooltip（轻量文本操作）
    let menu_changed = state
        .tray_menu_signature
        .lock()
        .map(|cached| *cached != Some(current_signature.clone()))
        .unwrap_or(true);

    if menu_changed {
        if let Some(snapshot) = snapshot {
            // Reuse the batteries prepared for the signature instead of cloning again.
            let batteries = prepared_batteries.as_ref().unwrap();
            for (index, battery) in batteries.iter().enumerate() {
                let label = tr_battery_label(lang, &battery.id, &battery.label);
                let item = MenuItem::with_id(
                    app,
                    format!("battery-{index}"),
                    tr_battery_item(lang, &label, battery.percentage, battery.charging),
                    true,
                    None::<&str>,
                )?;
                menu.append(&item)?;
            }
            if settings.tray_show_connection {
                let connection = MenuItem::with_id(
                    app,
                    "connection-status",
                    tr_connection_status(
                        lang,
                        connection_label(snapshot.connection, lang),
                        &snapshot.display_name,
                    ),
                    true,
                    None::<&str>,
                )?;
                menu.append(&connection)?;
            }
        } else {
            let disconnected = MenuItem::with_id(
                app,
                "disconnected",
                tr_disconnected(lang),
                true,
                None::<&str>,
            )?;
            menu.append(&disconnected)?;
        }

        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(
            app,
            "open",
            tr_open(lang),
            true,
            None::<&str>,
        )?)?;
        menu.append(&MenuItem::with_id(
            app,
            "quit",
            tr_quit(lang),
            true,
            None::<&str>,
        )?)?;
        tray.set_menu(Some(menu))?;
        if let Ok(mut cached) = state.tray_menu_signature.lock() {
            *cached = Some(current_signature);
        }
    }

    // title/tooltip 是轻量文本操作，每次都更新
    if let Some(snapshot) = snapshot {
        // On macOS, `None` means "leave the existing title unchanged".
        // An empty string is required to actually hide a previously shown percentage.
        tray.set_title(Some(battery_title(snapshot, settings).unwrap_or_default()))?;
        tray.set_tooltip(Some(tr_tooltip_connected(
            lang,
            connection_label(snapshot.connection, lang),
            &snapshot.display_name,
        )))?;
    } else {
        tray.set_title(Some(""))?;
        tray.set_tooltip(Some(tr_tooltip_disconnected(lang)))?;
    }
    Ok(())
}

fn build_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    tray::macos::set_app_handle(app.handle().clone());

    let lang = effective_language("auto");
    let open_i = MenuItem::with_id(app, "open", tr_open(lang), true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", tr_quit(lang), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_i, &quit_i])?;
    let initial_icon =
        tauri::image::Image::from_bytes(tray::static_icon::static_tray_app_icon_bytes())?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(initial_icon)
        .icon_as_template(false)
        .show_menu_on_left_click(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            id if id.starts_with("battery-") => {
                open_battery_usage_from_tray(app);
            }
            _ => {
                focus_main_from_tray(app);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                focus_main_from_tray(app);
            }
        })
        .tooltip("Mira · 未连接受支持的鼠标")
        .build(app)?;
    let settings = cached_settings(app.handle());
    let state = app.state::<SessionState>();
    let snapshot = selected_snapshot(&state);
    update_tray(app.handle(), snapshot.as_ref(), &settings)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .manage(SessionState::default())
        .manage(production_trust_store())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            focus_main(app.get_webview_window("main"))
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let startup_settings = cached_settings(app.handle());
            let launch_hidden = should_start_hidden(&startup_settings);
            refresh_autostart_entry_if_enabled(app.handle());

            // macOS native Vibrancy backdrop.
            // Sidebar 材质提供类似 Finder/Mail 侧边栏的明显毛玻璃效果；
            // UnderWindowBackground 过于微妙，在纯色壁纸下几乎不可见。
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                apply_macos_vibrancy(&window);
                // Show after applying Vibrancy to avoid a black startup flash.
                if !launch_hidden {
                    let _ = window.show();
                }
            }

            // Windows 11 uses Mica; Windows 10 v1809+ falls back to Acrylic.
            // 移除系统标题栏（最小化/最大化/关闭按钮），改用前端 data-tauri-drag-region 拖拽。
            #[cfg(target_os = "windows")]
            if let Some(window) = app.get_webview_window("main") {
                let settings = startup_settings.clone();
                apply_saved_app_theme(app.handle(), &settings);
                let _ = window.set_decorations(false);
                let _ = window.set_maximizable(false);
                apply_windows_backdrop(&window, &settings);
            }

            {
                let dark = detect_system_dark(app.handle());
                if let Ok(mut cache) = app.state::<SessionState>().system_dark.lock() {
                    *cache = Some(dark);
                }
                update_runtime_app_icon(app.handle(), dark);
            }

            // Load bundled plugins once and cache them for the app lifetime.
            let plugins = load_bundled_plugin_devices(app.handle());
            #[cfg(debug_assertions)]
            eprintln!("[mira] loaded {} bundled plugin(s)", plugins.len());
            {
                let state = app.state::<SessionState>();
                *state.plugins.lock().unwrap_or_else(|e| e.into_inner()) = Some(plugins);
                // 启动加载后清空缓存，确保首次读取使用最新插件文件。
                invalidate_package_cache(&state);
            }

            // Retry tray setup a few times: on some platforms the tray is not
            // ready immediately at startup and a single attempt fails silently.
            let mut tray_ok = false;
            for attempt in 1..=3 {
                match build_tray(app) {
                    Ok(()) => {
                        tray_ok = true;
                        break;
                    }
                    Err(err) => {
                        eprintln!("[mira] tray setup attempt {attempt}/3 failed: {err}");
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            }
            if !tray_ok {
                eprintln!(
                    "[mira] tray setup failed after 3 attempts; tray icon will be unavailable"
                );
            }

            // Listen for window events: focus triggers an immediate device read,
            // and theme changes refresh the tray icon so the mouse battery outline
            // stays readable on both light and dark menu bars/taskbars.
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            // 点击官方关闭按钮时不退出应用，改为隐藏到托盘。
                            // macOS 同时从 Dock 中隐藏，仅保留在状态栏。
                            api.prevent_close();
                            hide_main_window_to_tray(&app_handle);
                        }
                        tauri::WindowEvent::Focused(true) => {
                            // Window focus only rechecks presence; protocol fields are read
                            // when the user opens a related control.
                            // 同时刷新系统主题缓存：窗口隐藏期间系统主题可能已变化，
                            // 但 ThemeChanged 事件可能未触发（窗口不可见时）。
                            let state = app_handle.state::<SessionState>();
                            let dark = detect_system_dark(&app_handle);
                            if let Ok(mut cache) = state.system_dark.lock() {
                                *cache = Some(dark);
                            }
                            update_runtime_app_icon(&app_handle, dark);
                            // macOS：窗口失焦期间 macOS 可能对 Sidebar 材质应用系统级
                            // dimming（暗色模式下表现为「一段时间后变黑」），重新应用
                            // vibrancy 强制刷新回 Active 状态。
                            #[cfg(target_os = "macos")]
                            if let Some(window) = app_handle.get_webview_window("main") {
                                apply_macos_vibrancy(&window);
                            }
                            request_presence_refresh(&app_handle.state::<SessionState>());
                        }
                        tauri::WindowEvent::ThemeChanged(_) => {
                            // 窗口可见时系统主题变化会触发此事件。
                            // 窗口隐藏时由 spawn_theme_watcher 的事件驱动监听器处理。
                            handle_system_theme_changed(&app_handle);
                        }
                        _ => {}
                    }
                });
            }

            // 电量历史 + 异常耗电节流状态：在后台线程加载，避免阻塞启动。
            // load_from_disk 采用合并策略：保留内存中比磁盘最新样本更晚的记录，
            // 即使轮询线程在此期间已写入新样本也不会丢失（见 merge_samples）。
            let bg_handle = app.handle().clone();
            std::thread::spawn(move || {
                bg_handle
                    .state::<SessionState>()
                    .battery_history
                    .load_from_disk(&bg_handle);
                bg_handle
                    .state::<SessionState>()
                    .abnormal_drain_notify
                    .load_from_disk(&bg_handle);
            });

            // Spawn background thread that reads the device periodically.
            // This keeps `device_snapshot` instant — the UI never blocks on HID I/O.
            spawn_device_reader(app.handle().clone());

            // 启动系统主题变化监听器（事件驱动，跨平台）。
            // 窗口隐藏到托盘后仍可接收主题变化事件，无需轮询。
            spawn_theme_watcher(app.handle().clone());

            // 安静灯光：从磁盘加载持久化状态（saved_mouse_light），然后启动调度器线程。
            //
            // 初始 phase 的选择策略（关键）：
            // - saved_mouse_light 存在 → phase = Night：说明上次 Mira 退出/崩溃时
            //   灯光已被关闭且未恢复。调度器首次循环会根据当前时间决定：
            //     · 日间时段 → Night→Day 转换，恢复灯光。
            //     · 夜间时段 → 相等返回，灯光保持关闭。
            // - saved_mouse_light 不存在 → phase = Day：说明上次退出时灯光状态已正常。
            //   调度器首次循环会根据当前时间决定：
            //     · 日间时段 → 相等返回。
            //     · 夜间时段 → Day→Night 转换，关闭灯光。
            // 这个策略确保 Mira 重启后能正确恢复/关闭灯光，不依赖持久化的 phase
            // （phase 在启动时根据 saved_mouse_light 和当前时间推导）。
            {
                let store = load_night_mode_state(app.handle());
                let runtime = NightModeRuntime {
                    phase: if store.saved_mouse_light.is_some()
                        || store.saved_receiver_light.is_some()
                    {
                        NightPhase::Night
                    } else {
                        NightPhase::Day
                    },
                    saved_mouse_light: store.saved_mouse_light,
                    saved_receiver_light: store.saved_receiver_light,
                    notified: false,
                };
                let state = app.state::<SessionState>();
                let mut guard = state.night_mode.lock().unwrap_or_else(|e| e.into_inner());
                *guard = runtime;
            }
            spawn_night_mode_scheduler(app.handle().clone());

            if let Some(window) = app.get_webview_window("main") {
                if launch_hidden {
                    if let Err(err) = window.hide() {
                        eprintln!("[mira] hide main window failed: {err}");
                    }
                    // macOS: 启动即隐藏到托盘时，也从 Dock 中隐藏。
                    #[cfg(target_os = "macos")]
                    {
                        use tauri::ActivationPolicy;
                        app.set_activation_policy(ActivationPolicy::Accessory);
                    }
                } else {
                    if let Err(err) = window.show() {
                        eprintln!("[mira] show main window failed: {err}");
                    }
                    let _ = window.set_focus();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            device_snapshot,
            device_snapshots,
            device_select,
            device_refresh,
            device_refresh_quick,
            device_refresh_battery,
            device_mutate,
            discover_devices,
            autostart_state,
            set_autostart,
            open_external_url,
            plugin_updates_check,
            plugin_update_install,
            device_config_export,
            device_config_import,
            about_info,
            settings_get,
            settings_set,
            hide_to_tray,
            show_update_notification,
            take_pending_notification_action,
            export_diagnostics,
            plugin_locales,
            battery_history_get,
            battery_history_clear,
            battery_history_export
        ])
        .build(tauri::generate_context!())
        .expect("Mira application runtime failed")
        .run(|app_handle, event| {
            // macOS: 用户点击 Dock 图标时恢复窗口（窗口被隐藏到托盘后，
            // Dock 图标在 Accessory 策略下不可见，但 Regular 状态下仍可点击）。
            // RunEvent::Reopen 是 macOS 专有变体，必须用 cfg 门控，
            // 否则 Linux/Windows CI 会因找不到该变体而编译失败。
            #[cfg(target_os = "macos")]
            {
                if let tauri::RunEvent::Reopen { .. } = event {
                    focus_main(app_handle.get_webview_window("main"));
                    request_presence_refresh(&app_handle.state::<SessionState>());
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (app_handle, event);
            }
        });
}

#[cfg(test)]
mod backoff_tests {
    use super::DeviceBackoff;
    use std::time::Duration;

    #[test]
    fn backoff_schedule_matches_spec() {
        assert_eq!(DeviceBackoff::backoff_duration(0), Duration::ZERO);
        assert_eq!(DeviceBackoff::backoff_duration(1), Duration::from_secs(60));
        assert_eq!(DeviceBackoff::backoff_duration(2), Duration::from_secs(300));
        assert_eq!(DeviceBackoff::backoff_duration(3), Duration::from_secs(900));
        assert_eq!(
            DeviceBackoff::backoff_duration(4),
            Duration::from_secs(1800)
        );
        assert_eq!(
            DeviceBackoff::backoff_duration(100),
            Duration::from_secs(1800)
        );
    }

    #[test]
    fn fresh_backoff_is_not_active() {
        let b = DeviceBackoff::default();
        assert!(!b.is_backing_off());
    }

    #[test]
    fn record_failure_activates_backoff() {
        let mut b = DeviceBackoff::default();
        b.record_failure();
        assert!(b.is_backing_off());
        assert_eq!(b.consecutive_failures, 1);
    }

    #[test]
    fn reset_clears_backoff() {
        let mut b = DeviceBackoff::default();
        b.record_failure();
        b.record_failure();
        assert!(b.is_backing_off());
        b.reset();
        assert!(!b.is_backing_off());
        assert_eq!(b.consecutive_failures, 0);
    }
}

#[cfg(test)]
mod read_plan_tests {
    use super::*;

    #[test]
    fn presence_only_never_uses_projection() {
        assert!(!ReadPlan::PresenceOnly.uses_projection());
        assert!(ReadPlan::PresenceOnly.semantic_fields().is_empty());
    }

    #[test]
    fn quick_uses_projection_with_dynamic_fields() {
        assert!(ReadPlan::Quick.uses_projection());
        let fields = ReadPlan::Quick.semantic_fields();
        assert!(fields.contains(&SemanticField::BatteryPercent));
        assert!(fields.contains(&SemanticField::Charging));
        assert!(fields.contains(&SemanticField::CurrentDpi));
        assert!(fields.contains(&SemanticField::PollingRate));
        assert!(fields.contains(&SemanticField::ActiveProfile));
        assert!(fields.contains(&SemanticField::LightingState));
    }

    #[test]
    fn battery_only_uses_projection_with_battery_fields_only() {
        assert!(ReadPlan::BatteryOnly.uses_projection());
        let fields = ReadPlan::BatteryOnly.semantic_fields();
        assert!(fields.contains(&SemanticField::BatteryPercent));
        assert!(fields.contains(&SemanticField::Charging));
        assert!(fields.contains(&SemanticField::ReceiverBatteryPercent));
        // BatteryOnly 不应包含 DPI、灯光等非电量字段
        assert!(!fields.contains(&SemanticField::CurrentDpi));
        assert!(!fields.contains(&SemanticField::PollingRate));
        assert!(!fields.contains(&SemanticField::LightingState));
    }

    #[test]
    fn full_does_not_use_projection() {
        assert!(!ReadPlan::Full.uses_projection());
        assert!(ReadPlan::Full.semantic_fields().is_empty());
    }

    #[test]
    fn plan_strength_ordering() {
        assert!(ReadPlan::Full > ReadPlan::Quick);
        assert!(ReadPlan::Quick > ReadPlan::BatteryOnly);
        assert!(ReadPlan::BatteryOnly > ReadPlan::PresenceOnly);
    }

    #[test]
    fn initial_sequence_is_presence_then_quick_then_full() {
        assert_eq!(
            select_read_plan(false, None, None, false),
            ReadPlan::PresenceOnly
        );
        assert_eq!(
            select_read_plan(true, Some(ReadPlan::Quick), None, false),
            ReadPlan::Quick
        );
        assert_eq!(
            select_read_plan(true, Some(ReadPlan::Full), None, false),
            ReadPlan::Full
        );
    }

    #[test]
    fn stable_connected_device_uses_presence_only() {
        assert_eq!(
            select_read_plan(true, None, None, false),
            ReadPlan::PresenceOnly
        );
    }

    #[test]
    fn connected_device_uses_battery_only_when_due() {
        assert_eq!(
            select_read_plan(true, None, None, true),
            ReadPlan::BatteryOnly
        );
    }

    #[test]
    fn manual_full_overrides_pending_quick() {
        assert_eq!(
            select_read_plan(true, Some(ReadPlan::Quick), Some(ReadPlan::Full), false),
            ReadPlan::Full
        );
    }
}

#[cfg(test)]
mod mutation_verify_patch_tests {
    use super::*;

    #[test]
    fn verified_fields_patch_matching_raw_output_only() {
        let mut outputs = BTreeMap::from([
            (
                "settings".to_string(),
                serde_json::json!({"pollingRate": 500, "profile": 1}),
            ),
            ("battery".to_string(), serde_json::json!({"percentage": 80})),
        ]);
        merge_verified_mutation_output(&mut outputs, &serde_json::json!({"pollingRate": 1000}));
        assert_eq!(outputs["settings"]["pollingRate"], 1000);
        assert_eq!(outputs["settings"]["profile"], 1);
        assert_eq!(outputs["battery"]["percentage"], 80);
    }
}
