// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use chrono::{DateTime, Duration, Local, Timelike, Utc};
use mira_core::{Connection, PluginCapability};
use mira_protocol::DeviceContextSnapshot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::DeviceSnapshotEntry;

const DEDUP_INTERVAL_MINUTES: i64 = 5;
#[allow(dead_code)]
const DEFAULT_RETENTION_DAYS: i64 = 30;
const RETENTION_BUFFER_DAYS: i64 = 1;
/// schema v3：`BatterySample` 新增 `context: Option<DeviceContextSnapshot>` 字段，
/// 携带采样时刻的 DPI/回报率/灯光模式等低频变动参数。
/// v2 样本经 `migrate_schema` 填充 `context: None`，不丢历史。
const SCHEMA_VERSION: u32 = 3;
const SESSION_GAP_THRESHOLD_MINUTES: i64 = 10;
const MAX_SAMPLES: usize = 20000;
const BOUNCE_THRESHOLD_PERCENT: f64 = 1.0;
/// A level change this large may be an off-device charge, battery swap, or
/// reconnect recalibration rather than ordinary discharge.
const LEVEL_DISCONTINUITY_THRESHOLD_PERCENT: u8 = 5;
/// Short integer-level jumps are too coarse to extrapolate as an hourly rate.
const SHORT_LEVEL_DISCONTINUITY_MINUTES: i64 = 15;
/// A rate observation must span enough time to smooth 1% battery quantization.
const MIN_RATE_OBSERVATION_MINUTES: i64 = 30;
/// One short session with a single 1% step is still too weak to support a
/// remaining-active-use estimate. Longer observations may use a 1% drop.
const MIN_SHORT_ACTIVE_OBSERVATION_HOURS: f64 = 2.0;
const MIN_SHORT_ACTIVE_TOTAL_DROP_PERCENT: f64 = 2.0;
/// Above this bound, prefer treating the transition as a discontinuity instead
/// of teaching prediction/alert paths an implausibly high mouse drain rate.
const MAX_PLAUSIBLE_DRAIN_PER_HOUR: f64 = 20.0;
const POST_CHARGE_SKIP_MINUTES: i64 = 10;
const VERY_SLOW_DRAIN_HOURS: f64 = 9999.0;
/// 日均耗电至少覆盖半天自然时间，避免用短时波动外推整天。
const MIN_DAILY_DRAIN_OBSERVATION_HOURS: f64 = 12.0;
const PERSIST_INTERVAL_SECS: u64 = 300;
/// EWMA 时间衰减常数：段结束时间距今每 48h，权重衰减到 e^-1 ≈ 37%。
/// 让剩余时间预估更关注近期使用模式，而非 10 天平均。
const RATE_DECAY_TAU_HOURS: f64 = 48.0;
/// Remaining-time predictions should be stable when the chart switches between
/// 24h and 10d. Use the same natural-time evidence window for both views.
const REMAINING_PREDICTION_WINDOW_DAYS: i64 = 10;
/// Even a model that passes its own quality gate must stay broadly consistent
/// with the robust active-session baseline before Mira displays it.
const AI_ACTIVE_REMAINING_RATIO_MIN: f64 = 0.5;
const AI_ACTIVE_REMAINING_RATIO_MAX: f64 = 2.0;
/// z-score 异常检测的阈值：超过均值 +2σ（95% 置信）视为异常。
/// 历史段不足 5 段时回退到固定 2.0 倍阈值。
const ABNORMAL_ZSCORE_THRESHOLD: f64 = 2.0;
const ABNORMAL_MIN_HIST_SEGMENTS: usize = 5;

/// 每个插件自行声明哪些连接方式的电量读数可用于历史分析。
/// 未声明时 fail-closed，避免宿主猜测某个协议或厂商的电量语义。
fn battery_history_allowed(capabilities: &[PluginCapability], connection: &Connection) -> bool {
    let connection = connection_str(connection);
    capabilities
        .iter()
        .find(|capability| capability.id == "battery" && capability.available)
        .and_then(|capability| capability.metadata.get("batteryHistory"))
        .and_then(|policy| policy.get("validConnections"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|connections| {
            connections
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|allowed| allowed == connection)
        })
}

/// 只有经插件策略确认的样本能参与曲线、预测和耗电告警。
fn is_usable_history_sample(sample: &BatterySample) -> bool {
    sample.eligible_for_prediction
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatterySample {
    pub at: DateTime<Utc>,
    pub device_id: String,
    pub device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_aliases: Vec<String>,
    pub connection: String,
    pub component_id: String,
    pub component_label: String,
    pub percentage: u8,
    pub charging: bool,
    pub low_power: bool,
    #[serde(default)]
    pub eligible_for_prediction: bool,
    /// 采样时刻的设备上下文（DPI/回报率/灯光等）。
    /// 复用宿主 `DeviceSnapshot` 缓存，不触发额外 HID 读取。
    /// schema v2 升级到 v3 时填充为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<DeviceContextSnapshot>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHistoryFile {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub samples: Vec<BatterySample>,
}

pub struct BatteryHistoryState {
    samples: Mutex<Vec<BatterySample>>,
    last_record: Mutex<BTreeMap<String, (BatterySample, Instant)>>,
    last_persist: Mutex<Instant>,
}

impl BatteryHistoryState {
    pub fn new() -> Self {
        let first_persist_due = Instant::now()
            .checked_sub(std::time::Duration::from_secs(PERSIST_INTERVAL_SECS))
            .unwrap_or_else(Instant::now);
        Self {
            samples: Mutex::new(Vec::new()),
            last_record: Mutex::new(BTreeMap::new()),
            last_persist: Mutex::new(first_persist_due),
        }
    }

    pub fn load_from_disk(&self, app: &AppHandle) {
        let Some(path) = history_path(app) else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        match serde_json::from_slice::<BatteryHistoryFile>(&bytes) {
            Ok(file) => {
                let mut file = file;
                if file.schema_version < SCHEMA_VERSION {
                    file = migrate_schema(file);
                    let _ = save_history(app, &file);
                }
                if let Ok(mut guard) = self.samples.lock() {
                    *guard = merge_samples(file.samples, &guard);
                    if let Ok(mut last) = self.last_record.lock() {
                        last.clear();
                        for sample in guard.iter() {
                            let key = sample_device_key(sample);
                            last.insert(key, (sample.clone(), Instant::now()));
                        }
                    }
                }
            }
            Err(_) => {
                if let Ok(mut guard) = self.samples.lock() {
                    guard.clear();
                }
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

impl Default for BatteryHistoryState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHistoryResponse {
    pub range: String,
    pub devices: Vec<BatteryHistoryDevice>,
    pub series: Vec<BatteryHistorySeries>,
    pub insights: Vec<BatteryInsight>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHistoryDevice {
    pub key: String,
    pub device_id: String,
    pub device_name: String,
    pub connection: String,
    pub component_id: String,
    pub component_label: String,
    pub latest_percentage: Option<u8>,
    pub latest_charging: Option<bool>,
    pub latest_at: Option<String>,
    pub low_battery: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHistorySeries {
    pub key: String,
    pub points: Vec<BatteryHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHistoryPoint {
    pub bucket_start: String,
    pub bucket_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_elapsed_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_percentage: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_percentage: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_battery: Option<bool>,
    pub sample_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryInsight {
    #[serde(rename = "type")]
    pub insight_type: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_key: Option<String>,
}

fn history_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("battery_history.json"))
}

/// 写入时使用临时文件 + rename，保证原子性。
fn save_history(app: &AppHandle, file: &BatteryHistoryFile) -> Result<(), String> {
    let Some(path) = history_path(app) else {
        return Err("config dir unavailable".into());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    let bytes = serde_json::to_vec(file).map_err(|e| format!("serialize failed: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("write tmp failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename failed: {e}"))?;
    Ok(())
}

fn migrate_schema(mut file: BatteryHistoryFile) -> BatteryHistoryFile {
    if file.schema_version < 2 {
        // v1 未记录样本是否符合插件的电量遥测策略。先保留为待确认样本，
        // 等当前插件明确批准同一设备、部件和连接后再纳入统计，避免升级丢历史。
        for sample in &mut file.samples {
            sample.eligible_for_prediction = false;
        }
    }
    if file.schema_version < 3 {
        // v2 样本无 context 字段。反序列化时 `#[serde(default)]` 已填充为 `None`，
        // 这里显式标记以表达升级意图，并防御未来字段默认值变更。
        for sample in &mut file.samples {
            if sample.context.is_none() {
                sample.context = None;
            }
        }
    }
    file.schema_version = SCHEMA_VERSION;
    file
}

fn merge_samples(disk: Vec<BatterySample>, memory: &[BatterySample]) -> Vec<BatterySample> {
    let disk_latest = disk.iter().map(|s| s.at).max();
    let mut merged = disk;
    for s in memory {
        if disk_latest.is_none_or(|latest| s.at > latest) {
            merged.push(s.clone());
        }
    }
    merged.sort_by_key(|s| s.at);
    merged
}

fn should_persist(last_persist: Instant) -> bool {
    last_persist.elapsed() >= std::time::Duration::from_secs(PERSIST_INTERVAL_SECS)
}

/// 将 HID path 脱敏为稳定的 16 字符 hex key。
/// 不存储原始 path，仅存哈希，保证同一设备同一端口的稳定识别。
pub fn anonymize_device_key(device_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(device_key.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..8])
}

fn normalize_history_device_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalized_history_identity_group(value: &str) -> Option<String> {
    let normalized = normalize_history_device_name(value);
    (!normalized.is_empty()).then_some(normalized)
}

fn stable_device_id_for_entry(entry: &DeviceSnapshotEntry) -> String {
    let snapshot = &entry.snapshot;
    let plugin = snapshot.plugin_id.as_deref().unwrap_or("unknown-plugin");
    let identity_group = snapshot
        .history_identity
        .as_ref()
        .and_then(|identity| normalized_history_identity_group(&identity.group));
    let normalized_name = normalize_history_device_name(&snapshot.display_name);
    let source = identity_group
        .map(|group| format!("plugin:{plugin}|identity:{group}"))
        .or_else(|| {
            (!normalized_name.is_empty()).then(|| format!("plugin:{plugin}|name:{normalized_name}"))
        })
        .unwrap_or_else(|| format!("path:{}", entry.device_key));
    anonymize_device_key(&source)
}

fn history_device_group_id(device_id: &str, device_name: &str) -> String {
    let normalized_name = normalize_history_device_name(device_name);
    if normalized_name.is_empty() {
        device_id.to_string()
    } else {
        anonymize_device_key(&format!("device-name:{normalized_name}"))
    }
}

fn sample_history_device_group_id(sample: &BatterySample) -> String {
    if let Some(identity_group) = sample
        .identity_group
        .as_deref()
        .and_then(normalized_history_identity_group)
    {
        anonymize_device_key(&format!("device-identity:{identity_group}"))
    } else {
        history_device_group_id(&sample.device_id, &sample.device_name)
    }
}

fn history_key(device_id: &str, device_name: &str, component_id: &str) -> String {
    format!(
        "{}:{}",
        history_device_group_id(device_id, device_name),
        component_id
    )
}

fn sample_device_key(sample: &BatterySample) -> String {
    format!(
        "{}:{}",
        sample_history_device_group_id(sample),
        sample.component_id
    )
}

fn sample_legacy_key(sample: &BatterySample) -> String {
    format!("{}:{}", sample.device_id, sample.component_id)
}

#[derive(Debug, Clone)]
struct SampleGroup {
    key: String,
    component_id: String,
    device_ids: BTreeSet<String>,
    identity_groups: BTreeSet<String>,
    names: BTreeSet<String>,
    aliases: BTreeSet<String>,
}

impl SampleGroup {
    fn new(sample: &BatterySample) -> Self {
        let mut group = SampleGroup {
            key: sample_device_key(sample),
            component_id: sample.component_id.clone(),
            device_ids: BTreeSet::new(),
            identity_groups: BTreeSet::new(),
            names: BTreeSet::new(),
            aliases: BTreeSet::new(),
        };
        group.add_sample(sample);
        group
    }

    fn add_sample(&mut self, sample: &BatterySample) {
        self.device_ids.insert(sample.device_id.clone());
        if let Some(identity_group) = sample
            .identity_group
            .as_deref()
            .and_then(normalized_history_identity_group)
        {
            self.identity_groups.insert(identity_group);
        }
        let name = normalize_history_device_name(&sample.device_name);
        if !name.is_empty() {
            self.names.insert(name);
        }
        for alias in &sample.identity_aliases {
            let alias = normalize_history_device_name(alias);
            if !alias.is_empty() {
                self.names.insert(alias);
            }
        }
        self.aliases.insert(sample_device_key(sample));
        self.aliases.insert(sample_legacy_key(sample));
    }

    fn absorb(&mut self, other: SampleGroup) {
        self.device_ids.extend(other.device_ids);
        self.identity_groups.extend(other.identity_groups);
        self.names.extend(other.names);
        self.aliases.extend(other.aliases);
    }

    fn matches_sample(&self, sample: &BatterySample) -> bool {
        if self.component_id != sample.component_id {
            return false;
        }
        if self.device_ids.contains(&sample.device_id) {
            return true;
        }
        if sample
            .identity_group
            .as_deref()
            .and_then(normalized_history_identity_group)
            .is_some_and(|identity_group| self.identity_groups.contains(&identity_group))
        {
            return true;
        }
        let name = normalize_history_device_name(&sample.device_name);
        if !name.is_empty() && self.names.contains(&name) {
            return true;
        }
        sample.identity_aliases.iter().any(|alias| {
            let alias = normalize_history_device_name(alias);
            !alias.is_empty() && self.names.contains(&alias)
        })
    }

    fn matches_key(&self, key: &str) -> bool {
        self.key == key || self.aliases.contains(key)
    }
}

fn build_sample_groups(samples: &[BatterySample]) -> Vec<SampleGroup> {
    let mut groups: Vec<SampleGroup> = Vec::new();
    for sample in samples {
        let matches: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter_map(|(idx, group)| group.matches_sample(sample).then_some(idx))
            .collect();
        let Some(first) = matches.first().copied() else {
            groups.push(SampleGroup::new(sample));
            continue;
        };

        groups[first].add_sample(sample);
        for idx in matches.into_iter().skip(1).rev() {
            let other = groups.remove(idx);
            groups[first].absorb(other);
        }
    }
    groups
}

fn samples_for_group<'a>(
    samples: &'a [BatterySample],
    groups: &'a [SampleGroup],
    key: &str,
) -> Vec<&'a BatterySample> {
    if let Some(group) = groups.iter().find(|group| group.matches_key(key)) {
        samples
            .iter()
            .filter(|sample| group.matches_sample(sample))
            .collect()
    } else {
        samples
            .iter()
            .filter(|sample| sample_device_key(sample) == key || sample_legacy_key(sample) == key)
            .collect()
    }
}

/// 当前插件批准某一连接后，恢复同一设备/部件在旧 schema 中留下的样本。
/// 这让升级前后的曲线连续，同时不由宿主猜测协议或厂商的电量语义。
fn promote_policy_approved_legacy_samples(
    samples: &mut [BatterySample],
    approved_sample: &BatterySample,
) -> usize {
    if !approved_sample.eligible_for_prediction {
        return 0;
    }

    let approved_group = SampleGroup::new(approved_sample);
    let mut promoted = 0;
    for sample in samples {
        if !sample.eligible_for_prediction
            && sample.connection == approved_sample.connection
            && sample.component_id == approved_sample.component_id
            && approved_group.matches_sample(sample)
        {
            sample.eligible_for_prediction = true;
            promoted += 1;
        }
    }
    promoted
}

fn normalize_percentage(raw: u8) -> Option<u8> {
    if raw == 0xFF {
        return None;
    }
    Some(raw.min(100))
}

fn should_record_candidate(
    previous: &BatterySample,
    candidate: &BatterySample,
    elapsed: std::time::Duration,
) -> bool {
    previous.percentage != candidate.percentage
        || previous.charging != candidate.charging
        || previous.context != candidate.context
        || elapsed >= std::time::Duration::from_secs((DEDUP_INTERVAL_MINUTES * 60) as u64)
}

/// 从 `DeviceSnapshot` 缓存中提取预测模型所需的低频变动参数上下文。
///
/// **不触发任何 HID 读取**：所有字段直接从宿主已缓存的快照中投影。
/// 缓存更新（Quick 读取或 mutation 验证读）时，下一次 `record_samples` 调用
/// 自动携带新上下文，rill 端随之看到新参数。
///
/// 灯光信息位于 `capabilities["mouseLighting"]`，由
/// `normalized_mouse_lighting` 规范化为包含 `enabled`/`mode`/`effect`/
/// `modeName`/`effectName`/`brightness` 等字段的对象。
fn extract_context_from_snapshot(snapshot: &mira_core::DeviceSnapshot) -> DeviceContextSnapshot {
    let light_mode = snapshot
        .capabilities
        .get("mouseLighting")
        .and_then(|value| value.as_object())
        .and_then(|lighting| {
            if lighting.get("enabled").and_then(serde_json::Value::as_bool) == Some(false) {
                return Some("off".to_string());
            }
            // 优先取人类可读的名称字段，回退到原始 mode/effect 值。
            lighting
                .get("modeName")
                .or_else(|| lighting.get("effectName"))
                .or_else(|| lighting.get("mode"))
                .or_else(|| lighting.get("effect"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    let light_brightness = snapshot
        .capabilities
        .get("mouseLighting")
        .and_then(|value| value.as_object())
        .and_then(|lighting| lighting.get("brightness"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u8::try_from(value.min(100)).ok());
    DeviceContextSnapshot {
        dpi: snapshot.dpi,
        polling_rate_hz: snapshot.polling_rate_hz,
        light_mode,
        light_brightness,
        profile: snapshot.profile.clone(),
    }
}

pub fn record_samples(
    state: &BatteryHistoryState,
    app: &AppHandle,
    history_enabled: bool,
    low_battery_threshold: u8,
    retention_days: i64,
    entries: &[DeviceSnapshotEntry],
) {
    if !history_enabled {
        return;
    }

    let now = Utc::now();

    let retention_with_buffer = retention_days.max(1) + RETENTION_BUFFER_DAYS;
    let cutoff = now - Duration::days(retention_with_buffer);

    let to_persist: Option<Vec<BatterySample>> = {
        let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        let mut last_record = state.last_record.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed = false;

        for entry in entries {
            let snapshot = &entry.snapshot;
            if !battery_history_allowed(&snapshot.plugin_capabilities, &snapshot.connection) {
                continue;
            }
            let device_id = stable_device_id_for_entry(entry);
            let connection = connection_str(&snapshot.connection);
            let history_identity = snapshot.history_identity.as_ref();
            let identity_group = history_identity
                .and_then(|identity| normalized_history_identity_group(&identity.group));
            let mut identity_aliases = history_identity
                .map(|identity| identity.aliases.clone())
                .unwrap_or_default();
            let device_name = history_identity
                .and_then(|identity| identity.display_name.as_deref())
                .filter(|name| !normalize_history_device_name(name).is_empty())
                .unwrap_or(&snapshot.display_name)
                .to_string();
            if !snapshot.display_name.is_empty() {
                identity_aliases.push(snapshot.display_name.clone());
            }
            if let Some(identity_name) =
                history_identity.and_then(|identity| identity.display_name.clone())
            {
                identity_aliases.push(identity_name);
            }
            identity_aliases.sort_by_key(|alias| normalize_history_device_name(alias));
            identity_aliases.dedup_by(|a, b| {
                normalize_history_device_name(a) == normalize_history_device_name(b)
            });

            let batteries: Vec<(String, String, u8, bool)> = if !snapshot.batteries.is_empty() {
                snapshot
                    .batteries
                    .iter()
                    .filter_map(|b| {
                        // 过滤未知值（0xFF=255），clamp 到 0-100 防止越界。
                        let pct = normalize_percentage(b.percentage)?;
                        Some((b.id.clone(), b.label.clone(), pct, b.charging))
                    })
                    .collect()
            } else if let Some(percent) = snapshot.battery_percent {
                // 旧字段兼容：作为 componentId="mouse" 记录。
                match normalize_percentage(percent) {
                    Some(pct) => vec![(
                        "mouse".into(),
                        "mock.mouseLabel".into(),
                        pct,
                        snapshot.charging,
                    )],
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };

            for (component_id, component_label, percentage, charging) in batteries {
                let key = if let Some(identity_group) = &identity_group {
                    format!(
                        "{}:{}",
                        anonymize_device_key(&format!("device-identity:{identity_group}")),
                        component_id
                    )
                } else {
                    history_key(&device_id, &device_name, &component_id)
                };
                let low_power = !charging && percentage <= low_battery_threshold;
                // 从宿主缓存的 DeviceSnapshot 投影低频变动参数上下文。
                // 不触发任何 HID 读取：缓存由 Quick 读取/mutation 验证读维护。
                let context = extract_context_from_snapshot(snapshot);
                let context = (!context.is_empty()).then_some(context);
                let candidate = BatterySample {
                    at: now,
                    device_id: device_id.clone(),
                    device_name: device_name.clone(),
                    identity_group: identity_group.clone(),
                    identity_aliases: identity_aliases.clone(),
                    connection: connection.clone(),
                    component_id: component_id.clone(),
                    component_label: component_label.clone(),
                    percentage,
                    charging,
                    low_power,
                    eligible_for_prediction: true,
                    context,
                };
                if promote_policy_approved_legacy_samples(&mut samples, &candidate) > 0 {
                    changed = true;
                }
                let should_record = match last_record.get(&key) {
                    Some((prev, last_instant)) => {
                        // Context changes are prediction-relevant events. Record them
                        // immediately even when the coarse battery percentage has not
                        // moved, so a DPI/polling/lighting change is visible to the next
                        // prediction instead of waiting for the five-minute dedup window.
                        should_record_candidate(prev, &candidate, last_instant.elapsed())
                    }
                    None => true,
                };

                if should_record {
                    last_record.insert(key, (candidate.clone(), Instant::now()));
                    samples.push(candidate);
                    changed = true;
                }
            }
        }

        let before_retain = samples.len();
        samples.retain(|s| s.at >= cutoff);
        if samples.len() != before_retain {
            changed = true;
        }

        if samples.len() > MAX_SAMPLES {
            samples.sort_by_key(|a| a.at);
            let drop_count = samples.len() - MAX_SAMPLES;
            samples.drain(0..drop_count);
            changed = true;
        }

        let mut last_persist = state.last_persist.lock().unwrap_or_else(|e| e.into_inner());
        if changed && should_persist(*last_persist) {
            *last_persist = Instant::now();
            Some(samples.clone())
        } else {
            None
        }
    };

    if let Some(to_persist) = to_persist {
        let file = BatteryHistoryFile {
            schema_version: SCHEMA_VERSION,
            samples: to_persist,
        };
        let _ = save_history(app, &file);
    }
}

fn connection_str(conn: &Connection) -> String {
    match conn {
        Connection::Usb => "usb".into(),
        Connection::Wireless => "wireless".into(),
        Connection::Bluetooth => "bluetooth".into(),
        Connection::Virtual => "virtual".into(),
    }
}

#[cfg(test)]
pub fn build_response(
    state: &BatteryHistoryState,
    low_battery_threshold: u8,
    range: &str,
) -> BatteryHistoryResponse {
    build_response_with_analysis(state, low_battery_threshold, range, false, None)
}

pub fn build_response_with_analysis(
    state: &BatteryHistoryState,
    low_battery_threshold: u8,
    range: &str,
    local_ai_analysis_enabled: bool,
    app: Option<&AppHandle>,
) -> BatteryHistoryResponse {
    let samples: Vec<BatterySample> = {
        let guard = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .filter(|sample| is_usable_history_sample(sample))
            .cloned()
            .collect()
    };
    let samples_ref: &[BatterySample] = &samples;
    let now = Utc::now();

    struct DeviceAccum {
        key: String,
        device_id: String,
        device_name: String,
        connection: String,
        component_id: String,
        component_label: String,
        latest_percentage: u8,
        latest_charging: bool,
        latest_at: DateTime<Utc>,
        low_battery: bool,
    }
    let groups = build_sample_groups(samples_ref);
    let mut device_keys: BTreeMap<String, DeviceAccum> = BTreeMap::new();
    for group in &groups {
        for s in samples_ref
            .iter()
            .filter(|sample| group.matches_sample(sample))
        {
            device_keys
                .entry(group.key.clone())
                .and_modify(|d| {
                    if s.at > d.latest_at {
                        d.device_id = s.device_id.clone();
                        d.latest_percentage = s.percentage;
                        d.latest_charging = s.charging;
                        d.latest_at = s.at;
                        d.low_battery = !s.charging && s.percentage <= low_battery_threshold;
                        d.device_name = s.device_name.clone();
                        d.connection = s.connection.clone();
                        d.component_label = s.component_label.clone();
                    }
                })
                .or_insert(DeviceAccum {
                    key: group.key.clone(),
                    device_id: s.device_id.clone(),
                    device_name: s.device_name.clone(),
                    connection: s.connection.clone(),
                    component_id: s.component_id.clone(),
                    component_label: s.component_label.clone(),
                    latest_percentage: s.percentage,
                    latest_charging: s.charging,
                    latest_at: s.at,
                    low_battery: !s.charging && s.percentage <= low_battery_threshold,
                });
        }
    }
    let devices: Vec<BatteryHistoryDevice> = device_keys
        .values()
        .map(|d| BatteryHistoryDevice {
            key: d.key.clone(),
            device_id: d.device_id.clone(),
            device_name: d.device_name.clone(),
            connection: d.connection.clone(),
            component_id: d.component_id.clone(),
            component_label: d.component_label.clone(),
            latest_percentage: Some(d.latest_percentage),
            latest_charging: Some(d.latest_charging),
            latest_at: Some(d.latest_at.to_rfc3339()),
            low_battery: Some(d.low_battery),
        })
        .collect();

    let series: Vec<BatteryHistorySeries> = devices
        .iter()
        .map(|d| {
            let key = &d.key;
            let device_samples = samples_for_group(samples_ref, &groups, key);
            let points = if range == "24h" {
                aggregate_active_usage(&device_samples, now, low_battery_threshold)
            } else {
                aggregate_ten_day_history(&device_samples, now, low_battery_threshold)
            };
            BatteryHistorySeries {
                key: key.clone(),
                points,
            }
        })
        .collect();

    let insights = build_insights(
        samples_ref,
        &devices,
        low_battery_threshold,
        range,
        now,
        local_ai_analysis_enabled,
        app,
    );

    BatteryHistoryResponse {
        range: range.into(),
        devices,
        series,
        insights,
        generated_at: now.to_rfc3339(),
    }
}

/// 过去 24 小时按累计使用时长聚合图表点。
///
/// 鼠标会断续连接和使用，真实时钟 X 轴会把大量空闲/断连时间画成空洞。
/// 横坐标只累计相邻采样间隔较短的在线时间，将长间隔压缩掉；每个返回点仍然
/// 来自真实样本，不补值、不插值。`usage_elapsed_minutes` 让前端能用匹配的
/// 累计使用时长刻度，而不是把压缩后的柱子错误标成真实钟点。
#[derive(Clone, Copy)]
struct ActiveSample<'a> {
    sample: &'a BatterySample,
    active_minutes: i64,
}

fn aggregate_active_usage(
    samples: &[&BatterySample],
    now: DateTime<Utc>,
    low_battery_threshold: u8,
) -> Vec<BatteryHistoryPoint> {
    let cutoff = now - Duration::hours(24);

    let mut sorted: Vec<&BatterySample> = samples
        .iter()
        .copied()
        .filter(|sample| sample.at >= cutoff && sample.at <= now)
        .collect();
    sorted.sort_by_key(|sample| sample.at);
    if sorted.is_empty() {
        return Vec::new();
    }

    let mut active_samples = Vec::with_capacity(sorted.len());
    let mut active_minutes = 0i64;
    let mut prev: Option<&BatterySample> = None;
    for sample in sorted {
        if let Some(prev_sample) = prev {
            let gap = (sample.at - prev_sample.at).num_minutes();
            if gap > 0 && gap <= SESSION_GAP_THRESHOLD_MINUTES {
                active_minutes += gap;
            }
        }
        active_samples.push(ActiveSample {
            sample,
            active_minutes,
        });
        prev = Some(sample);
    }

    // 原始样本继续完整保留给分析；图表只在超过 48 点时按相邻样本压缩。
    // 不足 48 点时一条真实记录对应一根柱，休眠/断连时段不会生成空柱或假数据。
    let max_points = 48usize;
    let chunk_size = active_samples.len().div_ceil(max_points).max(1);
    active_samples
        .chunks(chunk_size)
        .map(|chunk| build_active_usage_point(chunk, low_battery_threshold))
        .collect()
}

fn ten_day_first_day(now: DateTime<Utc>) -> chrono::NaiveDate {
    now.with_timezone(&Local).date_naive() - Duration::days(9)
}

/// 与 10 天图共享同一个范围起点：包含今天在内的十个本地自然日。
/// 这样图表与随范围变化的传统分析不会各自使用不同的“10 天”口径。
/// 本地 AI 仍使用完整的有效保留历史，避免因切换视图而损失预测上下文。
fn range_window_start(range: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    if range == "24h" {
        return now - Duration::hours(24);
    }

    ten_day_first_day(now)
        .and_hms_opt(0, 0, 0)
        .and_then(|start| start.and_local_timezone(Local).earliest())
        .map(|start| start.with_timezone(&Utc))
        // 极少数时区若本地午夜不存在，宁可回退到滚动窗口，也不让分析失败。
        .unwrap_or(now - Duration::days(10))
}

/// 过去 10 天按本地自然日的 8 小时时段聚合，共 30 个固定槽位。
/// 日期轴保留真实日历间隔；三段/天兼顾趋势细节和图表密度。
fn aggregate_ten_day_history(
    samples: &[&BatterySample],
    now: DateTime<Utc>,
    low_battery_threshold: u8,
) -> Vec<BatteryHistoryPoint> {
    let today = now.with_timezone(&Local).date_naive();
    let first_day = ten_day_first_day(now);
    let mut by_slot: BTreeMap<_, Vec<&BatterySample>> = BTreeMap::new();

    for sample in samples {
        let local = sample.at.with_timezone(&Local);
        let day = local.date_naive();
        if day >= first_day && day <= today {
            let slot = local.hour() / 8;
            by_slot.entry((day, slot)).or_default().push(*sample);
        }
    }

    let mut points = Vec::with_capacity(30);
    for offset in 0..10 {
        let day = first_day + Duration::days(offset);
        for slot in 0..3 {
            let mut slot_samples = by_slot.remove(&(day, slot)).unwrap_or_default();
            slot_samples.sort_by_key(|sample| sample.at);
            let last = slot_samples.last().copied();
            let min = slot_samples.iter().map(|sample| sample.percentage).min();
            let max = slot_samples.iter().map(|sample| sample.percentage).max();
            let charging = slot_samples.iter().any(|sample| sample.charging);
            let low_battery = slot_samples
                .iter()
                .any(|sample| !sample.charging && sample.percentage <= low_battery_threshold);
            let start_hour = slot * 8;
            let end_hour = start_hour + 8;

            points.push(BatteryHistoryPoint {
                bucket_start: format!("{day}T{start_hour:02}:00"),
                bucket_label: format!(
                    "{} {start_hour:02}:00–{end_hour:02}:00",
                    day.format("%m-%d")
                ),
                usage_elapsed_minutes: None,
                percentage: last.map(|sample| sample.percentage),
                min_percentage: min,
                max_percentage: max,
                charging: last.map(|_| charging),
                low_battery: last.map(|_| low_battery),
                sample_count: slot_samples.len() as u32,
            });
        }
    }
    points
}

fn build_active_usage_point(
    chunk: &[ActiveSample<'_>],
    low_battery_threshold: u8,
) -> BatteryHistoryPoint {
    let first = chunk.first().unwrap().sample;
    let last_active = chunk.last().unwrap();
    let last = last_active.sample;
    let min = chunk.iter().map(|s| s.sample.percentage).min();
    let max = chunk.iter().map(|s| s.sample.percentage).max();
    let charging = chunk.iter().any(|s| s.sample.charging);
    let low_battery = chunk
        .iter()
        .any(|s| !s.sample.charging && s.sample.percentage <= low_battery_threshold);

    BatteryHistoryPoint {
        bucket_start: first.at.to_rfc3339(),
        bucket_label: format_active_duration(last_active.active_minutes),
        usage_elapsed_minutes: Some(last_active.active_minutes),
        percentage: Some(last.percentage),
        min_percentage: min,
        max_percentage: max,
        charging: Some(charging),
        low_battery: Some(low_battery),
        sample_count: chunk.len() as u32,
    }
}

fn format_active_duration(minutes: i64) -> String {
    if minutes < 60 {
        return format!("{}m", minutes.max(0));
    }
    let hours = minutes / 60;
    let mins = minutes % 60;
    if mins == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {mins}m")
    }
}

fn build_insights(
    samples: &[BatterySample],
    devices: &[BatteryHistoryDevice],
    _threshold: u8,
    range: &str,
    now: DateTime<Utc>,
    local_ai_analysis_enabled: bool,
    app: Option<&AppHandle>,
) -> Vec<BatteryInsight> {
    let mut insights = Vec::new();
    let groups = build_sample_groups(samples);
    let ai_batches = if local_ai_analysis_enabled {
        devices
            .iter()
            .map(|device| {
                (
                    device.key.clone(),
                    samples_for_group(samples, &groups, &device.key),
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let ai_estimates = app
        .filter(|_| local_ai_analysis_enabled)
        .map(|app| crate::local_ai_runtime::predict_batteries(app, &ai_batches, now))
        .unwrap_or_default();
    let window_start = range_window_start(range, now);

    for device in devices {
        let key = &device.key;
        let device_samples = samples_for_group(samples, &groups, key);
        if device_samples.is_empty() {
            continue;
        }

        let current = device.latest_percentage;
        let charging = device.latest_charging.unwrap_or(false);

        // Natural remaining time drives the summary and runout date. It includes
        // sleep/disconnect time and therefore matches the wall-clock wording.
        let prediction_daily_drain = calendar_daily_drain(
            &device_samples,
            now - Duration::days(REMAINING_PREDICTION_WINDOW_DAYS),
            now,
        );
        let calendar_remaining =
            estimate_calendar_remaining(current, charging, prediction_daily_drain);

        if let Some(remaining_hours) = calendar_remaining {
            if !charging && current.is_some() {
                if remaining_hours >= VERY_SLOW_DRAIN_HOURS {
                    insights.push(BatteryInsight {
                        insight_type: "estimatedRemaining".into(),
                        severity: "info".into(),
                        title: "estimatedRemaining".into(),
                        message: "veryLowDrain".into(),
                        device_key: Some(key.clone()),
                    });
                } else {
                    insights.push(BatteryInsight {
                        insight_type: "estimatedRemaining".into(),
                        severity: "info".into(),
                        title: "estimatedRemaining".into(),
                        message: remaining_message(remaining_hours),
                        device_key: Some(key.clone()),
                    });

                    let runout = now + Duration::minutes((remaining_hours * 60.0).round() as i64);
                    insights.push(BatteryInsight {
                        insight_type: "estimatedRunout".into(),
                        severity: "info".into(),
                        title: "estimatedRunout".into(),
                        message: runout
                            .with_timezone(&Local)
                            .format("%m-%d %H:%M")
                            .to_string(),
                        device_key: Some(key.clone()),
                    });
                }
            }
        } else if !charging {
            insights.push(BatteryInsight {
                insight_type: "estimatedRemaining".into(),
                severity: "info".into(),
                title: "estimatedRemaining".into(),
                message: "notEnoughData".into(),
                device_key: Some(key.clone()),
            });
        }

        // Active-use time is a separate quantity. AI and the deterministic
        // fallback both learn from connected usage sessions, so never turn this
        // value into a wall-clock runout date.
        let baseline_active_remaining = estimate_remaining(&device_samples, "10d", now);
        let active_remaining = if local_ai_analysis_enabled {
            select_active_remaining(ai_estimates.get(key).copied(), baseline_active_remaining)
        } else {
            baseline_active_remaining
        };
        if !charging {
            if let Some(remaining_hours) = active_remaining {
                insights.push(BatteryInsight {
                    insight_type: "estimatedActiveRemaining".into(),
                    severity: "info".into(),
                    title: "estimatedActiveRemaining".into(),
                    message: if remaining_hours >= VERY_SLOW_DRAIN_HOURS {
                        "veryLowDrain".into()
                    } else {
                        remaining_message(remaining_hours)
                    },
                    device_key: Some(key.clone()),
                });
            }
        }

        if let Some(mut habit) = analyze_charging_habit(&device_samples, now) {
            habit.device_key = Some(key.clone());
            insights.push(habit);
        }

        if let Some(drain) = detect_abnormal_drain(&device_samples, now) {
            insights.push(BatteryInsight {
                insight_type: "abnormalDrain".into(),
                severity: "warning".into(),
                title: "abnormalDrain".into(),
                message: format!("abnormalDrain2h|{:.0}", drain),
                device_key: Some(key.clone()),
            });
        }

        if let Some(mut consistency) = compute_consistency(&device_samples, range, now) {
            consistency.device_key = Some(key.clone());
            insights.push(consistency);
        }

        // 日均耗电：按放电周期的自然时间跨度估算，离线间隔仍计入一天。
        if let Some(daily_drain) = calendar_daily_drain(&device_samples, window_start, now) {
            insights.push(BatteryInsight {
                insight_type: "averageDailyDrain".into(),
                severity: "info".into(),
                title: "averageDailyDrain".into(),
                message: format!("averageDailyDrain|{daily_drain:.1}"),
                device_key: Some(key.clone()),
            });
        }

        // 充电次数：窗口内充电状态转换 + session_gap 期间推断的充电
        if let Some(count) = count_charges_in_window(&device_samples, window_start) {
            insights.push(BatteryInsight {
                insight_type: "chargingCount".into(),
                severity: "info".into(),
                title: "chargingCount".into(),
                message: format!("chargingCount|{}", count),
                device_key: Some(key.clone()),
            });
        }
    }

    if devices.len() > 1 {
        if let Some(mut comparison) = compare_devices(samples, devices, range, now) {
            comparison.device_key = None;
            insights.push(comparison);
        }
    }

    for device in devices {
        if device.low_battery.unwrap_or(false) {
            insights.push(BatteryInsight {
                insight_type: "powerSavingTip".into(),
                severity: "info".into(),
                title: "powerSavingTip".into(),
                message: format!("powerSavingTipLow|{}", device.component_label),
                device_key: Some(device.key.clone()),
            });
        }
    }

    insights
}

fn remaining_message(remaining_hours: f64) -> String {
    if remaining_hours < 1.0 {
        format!("remainingMinutes|{:.0}", remaining_hours * 60.0)
    } else if remaining_hours < 24.0 {
        format!("remainingHours|{:.0}", remaining_hours)
    } else {
        let rounded_hours = remaining_hours.round() as i64;
        let days = rounded_hours / 24;
        let hours = rounded_hours % 24;
        format!("remainingDaysHours|{}|{}", days, hours)
    }
}

fn estimate_calendar_remaining(
    current_percentage: Option<u8>,
    charging: bool,
    daily_drain: Option<f64>,
) -> Option<f64> {
    if charging {
        return None;
    }
    let current = current_percentage?;
    if current == 0 {
        return Some(0.0);
    }
    let daily_drain = daily_drain?;
    if !daily_drain.is_finite() || daily_drain < 0.0 {
        return None;
    }
    if daily_drain <= f64::EPSILON {
        return Some(VERY_SLOW_DRAIN_HOURS);
    }
    let remaining = current as f64 / daily_drain * 24.0;
    if !remaining.is_finite() || remaining < 0.0 {
        None
    } else {
        if remaining >= VERY_SLOW_DRAIN_HOURS * 0.99 {
            Some(VERY_SLOW_DRAIN_HOURS)
        } else {
            Some(remaining)
        }
    }
}

fn select_active_remaining(ai: Option<f64>, baseline: Option<f64>) -> Option<f64> {
    match (ai, baseline) {
        (Some(ai), Some(baseline)) if baseline > 0.0 => {
            let ratio = ai / baseline;
            if ratio.is_finite()
                && (AI_ACTIVE_REMAINING_RATIO_MIN..=AI_ACTIVE_REMAINING_RATIO_MAX).contains(&ratio)
            {
                Some(ai)
            } else {
                Some(baseline)
            }
        }
        // Without a robust baseline there is no host-side scale check for AI.
        (Some(_), None) => None,
        (None, baseline) => baseline,
        (Some(_), Some(baseline)) => Some(baseline),
    }
}

fn is_session_gap(prev_at: DateTime<Utc>, curr_at: DateTime<Utc>) -> bool {
    (curr_at - prev_at).num_minutes() > SESSION_GAP_THRESHOLD_MINUTES
}

/// Detects a swap or off-device charge while the device itself is not charging.
fn is_energy_replenishment(prev: &BatterySample, curr: &BatterySample) -> bool {
    !prev.charging
        && !curr.charging
        && curr.percentage.saturating_sub(prev.percentage) >= LEVEL_DISCONTINUITY_THRESHOLD_PERCENT
}

/// Detect a boundary where the battery level cannot safely be interpreted as
/// ordinary drain. Upward jumps cover off-device charging and higher-charge
/// battery swaps. A large downward jump is only treated as a boundary when it
/// happens quickly (or implies an implausible rate); long disconnect gaps remain
/// valid wall-clock drain evidence for the calendar estimate.
fn is_battery_level_discontinuity(prev: &BatterySample, curr: &BatterySample) -> bool {
    if prev.charging || curr.charging {
        return true;
    }
    if is_energy_replenishment(prev, curr) {
        return true;
    }
    let drop = prev.percentage.saturating_sub(curr.percentage);
    if drop < LEVEL_DISCONTINUITY_THRESHOLD_PERCENT {
        return false;
    }
    let elapsed_minutes = (curr.at - prev.at).num_minutes();
    if elapsed_minutes <= 0 {
        return true;
    }
    if elapsed_minutes <= SHORT_LEVEL_DISCONTINUITY_MINUTES {
        return true;
    }
    let implied_rate = drop as f64 / (elapsed_minutes as f64 / 60.0);
    implied_rate > MAX_PLAUSIBLE_DRAIN_PER_HOUR
}

fn segment_drain_rate(first: &BatterySample, last: &BatterySample) -> Option<(f64, f64)> {
    let elapsed_minutes = (last.at - first.at).num_minutes();
    if elapsed_minutes < MIN_RATE_OBSERVATION_MINUTES {
        return None;
    }
    let drop = first.percentage as f64 - last.percentage as f64;
    if drop < BOUNCE_THRESHOLD_PERCENT {
        return None;
    }
    let hours = elapsed_minutes as f64 / 60.0;
    let rate = drop / hours;
    (rate.is_finite() && rate > 0.0 && rate <= MAX_PLAUSIBLE_DRAIN_PER_HOUR)
        .then_some((rate, hours))
}

fn estimate_remaining(samples: &[&BatterySample], range: &str, now: DateTime<Utc>) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }

    let cutoff = range_window_start(range, now);
    let recent: Vec<&&BatterySample> = samples.iter().filter(|s| s.at >= cutoff).collect();
    if recent.len() < 2 {
        return None;
    }

    let mut sorted = recent.clone();
    sorted.sort_by_key(|s| s.at);

    let mut segments: Vec<Vec<&&BatterySample>> = Vec::new();
    let mut current_seg: Vec<&&BatterySample> = Vec::new();
    let mut prev: Option<&&BatterySample> = None;
    for s in &sorted {
        let split = match prev {
            None => false,
            Some(p) => is_session_gap(p.at, s.at) || is_battery_level_discontinuity(p, s),
        };
        if split && !current_seg.is_empty() {
            segments.push(std::mem::take(&mut current_seg));
        }
        if !s.charging {
            current_seg.push(s);
        }
        prev = Some(s);
    }
    if !current_seg.is_empty() {
        segments.push(current_seg);
    }

    let mut weighted_rate: f64 = 0.0;
    let mut total_weight: f64 = 0.0;
    let mut total_hours: f64 = 0.0;
    let mut total_drop: f64 = 0.0;
    for seg in &segments {
        if seg.len() < 2 {
            continue;
        }
        let first = seg[0];
        let last = seg[seg.len() - 1];
        let Some((rate, hours)) = segment_drain_rate(first, last) else {
            continue;
        };
        // 时间衰减加权：段结束时间越近，权重越高
        let hours_ago = (now - last.at).num_minutes() as f64 / 60.0;
        let weight = (-hours_ago / RATE_DECAY_TAU_HOURS).exp();
        weighted_rate += rate * weight;
        total_weight += weight;
        total_hours += hours;
        total_drop += rate * hours;
    }

    if total_hours < 0.5
        || total_weight <= 0.0
        || (total_hours < MIN_SHORT_ACTIVE_OBSERVATION_HOURS
            && total_drop < MIN_SHORT_ACTIVE_TOTAL_DROP_PERCENT)
    {
        return None;
    }

    let drain_per_hour = weighted_rate / total_weight;
    if drain_per_hour <= 0.0 {
        return Some(VERY_SLOW_DRAIN_HOURS);
    }

    let current = sorted.iter().rev().find(|s| !s.charging)?.percentage;
    if current == 0 {
        return Some(0.0);
    }
    Some(current as f64 / drain_per_hour)
}

fn analyze_charging_habit(
    samples: &[&BatterySample],
    now: DateTime<Utc>,
) -> Option<BatteryInsight> {
    if samples.len() < 2 {
        return None;
    }

    let cutoff = now - Duration::days(10);
    let recent: Vec<&&BatterySample> = samples.iter().filter(|s| s.at >= cutoff).collect();
    if recent.len() < 2 {
        return None;
    }

    let mut sorted = recent.clone();
    sorted.sort_by_key(|s| s.at);

    let events = collect_replenishment_events(&sorted);
    if events.is_empty() {
        return None;
    }

    let avg_start =
        events.iter().map(|(start, _)| *start as f64).sum::<f64>() / events.len() as f64;
    let completed_ends = events
        .iter()
        .filter_map(|(_, end)| *end)
        .collect::<Vec<_>>();
    let avg_end = if completed_ends.is_empty() {
        None
    } else {
        Some(completed_ends.iter().map(|&p| p as f64).sum::<f64>() / completed_ends.len() as f64)
    };

    let count = events.len();
    let message = if let Some(end) = avg_end {
        format!(
            "chargingHabitStartEnd|{:.0}|{:.0}|{}",
            avg_start, end, count
        )
    } else {
        format!("chargingHabitStartOnly|{:.0}|{}", avg_start, count)
    };

    Some(BatteryInsight {
        insight_type: "chargingHabit".into(),
        severity: "info".into(),
        title: "chargingHabit".into(),
        message,
        device_key: None,
    })
}

/// Returns meaningful replenishment events. Explicit charging toggles at an
/// unchanged 100% are ignored: they commonly come from receiver/USB status
/// changes and do not demonstrate that energy was added.
fn collect_replenishment_events(samples: &[&&BatterySample]) -> Vec<(u8, Option<u8>)> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mut events = Vec::new();
    let mut open_charge_start = samples[0].charging.then_some(samples[0].percentage);
    for window in samples.windows(2) {
        let prev = window[0];
        let curr = window[1];
        if !prev.charging && curr.charging {
            open_charge_start = Some(prev.percentage);
        }
        if prev.charging && !curr.charging {
            if let Some(start) = open_charge_start.take() {
                if curr.percentage.saturating_sub(start) >= LEVEL_DISCONTINUITY_THRESHOLD_PERCENT {
                    events.push((start, Some(curr.percentage)));
                }
            }
        }
        if is_energy_replenishment(prev, curr) {
            events.push((prev.percentage, Some(curr.percentage)));
        }
    }

    if let Some(start) = open_charge_start {
        let latest = samples[samples.len() - 1];
        if latest.percentage.saturating_sub(start) >= LEVEL_DISCONTINUITY_THRESHOLD_PERCENT {
            events.push((start, Some(latest.percentage)));
        } else if start <= 95 {
            // An unfinished low-level charging session is still useful start
            // evidence, even before a meaningful rise has been sampled.
            events.push((start, None));
        }
    }
    events
}

/// 统计指定时间窗口内明确的充电或补能次数。非充电状态下只有达到阈值的
/// 电量上升才算补能，避免把 1% 读数回弹误报成一次充电。
fn count_charges_in_window(samples: &[&BatterySample], window_start: DateTime<Utc>) -> Option<u32> {
    let recent: Vec<&&BatterySample> = samples.iter().filter(|s| s.at >= window_start).collect();
    if recent.len() < 2 {
        return None;
    }
    let mut sorted = recent.clone();
    sorted.sort_by_key(|s| s.at);

    Some(collect_replenishment_events(&sorted).len() as u32)
}

fn capitalize_first(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn detect_abnormal_drain(samples: &[&BatterySample], now: DateTime<Utc>) -> Option<f64> {
    if samples.len() < 4 {
        return None;
    }

    let two_hours_ago = now - Duration::hours(2);
    let ten_days_ago = now - Duration::days(10);

    let recent_all: Vec<&&BatterySample> =
        samples.iter().filter(|s| s.at >= two_hours_ago).collect();
    let last_charge_time = recent_all.iter().filter(|s| s.charging).map(|s| s.at).max();
    if let Some(t) = last_charge_time {
        if now - t < Duration::minutes(POST_CHARGE_SKIP_MINUTES) {
            return None;
        }
    }
    let effective_start = last_charge_time
        .map(|t| t + Duration::minutes(POST_CHARGE_SKIP_MINUTES))
        .unwrap_or(two_hours_ago);

    let recent: Vec<&&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= effective_start && s.at >= two_hours_ago && !s.charging)
        .collect();
    if recent.len() < 2 {
        return None;
    }
    let mut recent_sorted = recent.clone();
    recent_sorted.sort_by_key(|s| s.at);

    let mut segments: Vec<Vec<&&BatterySample>> = vec![vec![recent_sorted[0]]];
    for s in &recent_sorted[1..] {
        let last_seg = segments.last().unwrap();
        let last_sample = last_seg.last().unwrap();
        if is_session_gap(last_sample.at, s.at) || is_battery_level_discontinuity(last_sample, s) {
            segments.push(vec![*s]);
        } else {
            segments.last_mut().unwrap().push(*s);
        }
    }
    let last_segment: Vec<&&BatterySample> =
        segments.iter().rev().find(|seg| seg.len() >= 2).cloned()?;
    if last_segment.len() < 2 {
        return None;
    }
    let seg_first = last_segment.first()?;
    let seg_last = last_segment.last()?;
    let recent_drop = seg_first.percentage as f64 - seg_last.percentage as f64;
    let (recent_rate, _) = segment_drain_rate(seg_first, seg_last)?;

    let historical: Vec<&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= ten_days_ago && s.at < two_hours_ago)
        .cloned()
        .collect();
    let hist_rates = collect_segment_rates(&historical, ten_days_ago, two_hours_ago);

    let is_abnormal = if hist_rates.len() >= ABNORMAL_MIN_HIST_SEGMENTS {
        // z-score 检测：recent_rate 超过历史均值 +2σ 视为异常
        let mean = hist_rates.iter().sum::<f64>() / hist_rates.len() as f64;
        let variance: f64 =
            hist_rates.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / hist_rates.len() as f64;
        let std = variance.sqrt();
        if std > 1e-9 {
            (recent_rate - mean) / std > ABNORMAL_ZSCORE_THRESHOLD
        } else {
            recent_rate > mean * 2.0
        }
    } else {
        // 历史段不足，回退到固定 2.0 倍阈值
        let hist_rate = drain_rate(&historical, ten_days_ago, two_hours_ago)?;
        if hist_rate <= 0.0 {
            return None;
        }
        recent_rate > hist_rate * 2.0
    };

    if is_abnormal && recent_drop > 5.0 {
        Some(recent_drop)
    } else {
        None
    }
}

pub fn check_abnormal_drain(
    state: &BatteryHistoryState,
    notify_state: &AbnormalDrainNotifyState,
    now: DateTime<Utc>,
) -> Vec<(String, String)> {
    let samples: Vec<BatterySample> = {
        let guard = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .filter(|sample| is_usable_history_sample(sample))
            .cloned()
            .collect()
    };
    let sample_groups = build_sample_groups(&samples);
    let mut result = Vec::new();
    for group in &sample_groups {
        let group_samples = samples_for_group(&samples, &sample_groups, &group.key);
        let device_name = group_samples
            .iter()
            .max_by_key(|sample| sample.at)
            .map(|sample| sample.device_name.clone())
            .unwrap_or_default();
        if detect_abnormal_drain(&group_samples, now).is_some()
            && notify_state.should_notify(&group.key, now)
        {
            result.push((group.key.clone(), device_name));
        }
    }
    result
}

fn compute_consistency(
    samples: &[&BatterySample],
    range: &str,
    now: DateTime<Utc>,
) -> Option<BatteryInsight> {
    if samples.len() < 4 {
        return None;
    }

    let (recent_cutoff, hist_cutoff) = match range {
        "24h" => (now - Duration::hours(6), range_window_start(range, now)),
        _ => (now - Duration::days(3), range_window_start(range, now)),
    };

    let recent_rate = drain_rate(samples, recent_cutoff, now)?;
    let hist_rate = drain_rate(samples, hist_cutoff, recent_cutoff)?;

    let ratio = if hist_rate > 0.0 {
        recent_rate / hist_rate
    } else {
        1.0
    };

    let message = if ratio > 1.5 {
        "faster".to_string()
    } else if ratio < 0.67 {
        "slower".to_string()
    } else {
        "stable".to_string()
    };

    Some(BatteryInsight {
        insight_type: "batteryConsistency".into(),
        severity: if ratio > 1.5 { "warning" } else { "info" }.into(),
        title: "batteryConsistency".into(),
        message: format!("consistency{}", capitalize_first(&message)),
        device_key: None,
    })
}

/// 收集指定时间范围内所有有效放电段的速率列表（%/h），用于 z-score 异常检测。
fn collect_segment_rates(
    samples: &[&BatterySample],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<f64> {
    let filtered: Vec<&&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= start && s.at < end)
        .collect();
    if filtered.len() < 2 {
        return Vec::new();
    }
    let mut sorted = filtered.clone();
    sorted.sort_by_key(|s| s.at);

    let mut segments: Vec<Vec<&&BatterySample>> = Vec::new();
    let mut current_seg: Vec<&&BatterySample> = Vec::new();
    let mut prev: Option<&&BatterySample> = None;
    for s in &sorted {
        let split = match prev {
            None => false,
            Some(p) => is_session_gap(p.at, s.at) || is_battery_level_discontinuity(p, s),
        };
        if split && !current_seg.is_empty() {
            segments.push(std::mem::take(&mut current_seg));
        }
        if !s.charging {
            current_seg.push(s);
        }
        prev = Some(s);
    }
    if !current_seg.is_empty() {
        segments.push(current_seg);
    }

    let mut rates = Vec::new();
    for seg in &segments {
        if seg.len() < 2 {
            continue;
        }
        let first = seg[0];
        let last = seg[seg.len() - 1];
        if let Some((rate, _)) = segment_drain_rate(first, last) {
            rates.push(rate);
        }
    }
    rates
}

fn drain_rate(samples: &[&BatterySample], start: DateTime<Utc>, end: DateTime<Utc>) -> Option<f64> {
    let filtered: Vec<&&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= start && s.at < end)
        .collect();
    if filtered.len() < 2 {
        return None;
    }
    let mut sorted = filtered.clone();
    sorted.sort_by_key(|s| s.at);

    let mut segments: Vec<Vec<&&BatterySample>> = Vec::new();
    let mut current_seg: Vec<&&BatterySample> = Vec::new();
    let mut prev: Option<&&BatterySample> = None;
    for s in &sorted {
        let split = match prev {
            None => false,
            Some(p) => is_session_gap(p.at, s.at) || is_battery_level_discontinuity(p, s),
        };
        if split && !current_seg.is_empty() {
            segments.push(std::mem::take(&mut current_seg));
        }
        if !s.charging {
            current_seg.push(s);
        }
        prev = Some(s);
    }
    if !current_seg.is_empty() {
        segments.push(current_seg);
    }

    let mut total_drop: f64 = 0.0;
    let mut total_hours: f64 = 0.0;
    for seg in &segments {
        if seg.len() < 2 {
            continue;
        }
        let first = seg[0];
        let last = seg[seg.len() - 1];
        let Some((rate, hours)) = segment_drain_rate(first, last) else {
            continue;
        };
        total_drop += rate * hours;
        total_hours += hours;
    }
    if total_hours < 0.5 {
        return None;
    }
    Some(total_drop / total_hours)
}

/// Estimates average drain per natural day. Session gaps remain part of elapsed
/// time. Charging, replenishment, and short level discontinuities start a new
/// episode so battery swaps/recalibration are not mistaken for drain.
fn calendar_daily_drain(
    samples: &[&BatterySample],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Option<f64> {
    let mut sorted: Vec<&&BatterySample> = samples
        .iter()
        .filter(|sample| sample.at >= start && sample.at < end)
        .collect();
    if sorted.len() < 2 {
        return None;
    }
    sorted.sort_by_key(|sample| sample.at);

    let mut episodes: Vec<Vec<&&BatterySample>> = Vec::new();
    let mut current_episode: Vec<&&BatterySample> = Vec::new();
    let mut prev: Option<&&BatterySample> = None;
    for sample in &sorted {
        let split = prev.is_some_and(|previous| is_battery_level_discontinuity(previous, sample));
        if split && !current_episode.is_empty() {
            episodes.push(std::mem::take(&mut current_episode));
        }
        if !sample.charging {
            current_episode.push(sample);
        }
        prev = Some(sample);
    }
    if !current_episode.is_empty() {
        episodes.push(current_episode);
    }

    let mut total_drop = 0.0;
    let mut total_hours = 0.0;
    for episode in episodes {
        if episode.len() < 2 {
            continue;
        }
        let first = episode[0];
        let last = episode[episode.len() - 1];
        let elapsed_minutes = (last.at - first.at).num_minutes();
        if elapsed_minutes < MIN_RATE_OBSERVATION_MINUTES {
            continue;
        }
        let hours = elapsed_minutes as f64 / 60.0;
        total_drop += (first.percentage as f64 - last.percentage as f64).max(0.0);
        total_hours += hours;
    }

    if total_hours < MIN_DAILY_DRAIN_OBSERVATION_HOURS {
        return None;
    }
    Some(total_drop / total_hours * 24.0)
}

fn compare_devices(
    samples: &[BatterySample],
    devices: &[BatteryHistoryDevice],
    range: &str,
    now: DateTime<Utc>,
) -> Option<BatteryInsight> {
    let cutoff = range_window_start(range, now);
    let groups = build_sample_groups(samples);

    let mut rates: Vec<(String, f64)> = Vec::new();
    for device in devices {
        let key = &device.key;
        let device_samples: Vec<&BatterySample> = samples_for_group(samples, &groups, key)
            .into_iter()
            .filter(|s| s.at >= cutoff)
            .collect();
        if let Some(rate) = drain_rate(&device_samples, cutoff, now) {
            rates.push((device.component_label.clone(), rate));
        }
    }

    if rates.len() < 2 {
        return None;
    }
    rates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let fastest = &rates[0];
    let slowest = &rates[rates.len() - 1];
    if fastest.1 <= slowest.1 || slowest.1 <= 0.0 {
        return None;
    }

    Some(BatteryInsight {
        insight_type: "deviceComparison".into(),
        severity: "info".into(),
        title: "deviceComparison".into(),
        message: format!(
            "deviceComparisonDrain|{}|{:.2}|{}|{:.2}",
            fastest.0, fastest.1, slowest.0, slowest.1
        ),
        device_key: None,
    })
}

pub fn export_history(state: &BatteryHistoryState, format: &str) -> Result<String, String> {
    let samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
    let file = BatteryHistoryFile {
        schema_version: SCHEMA_VERSION,
        samples: samples.clone(),
    };

    match format {
        "csv" => Ok(samples_to_csv(&samples)),
        _ => serde_json::to_string_pretty(&file).map_err(|e| format!("serialize failed: {e}")),
    }
}

fn samples_to_csv(samples: &[BatterySample]) -> String {
    let mut buf = String::from("at,deviceId,deviceName,connection,componentId,componentLabel,percentage,charging,lowPower\n");
    for s in samples {
        buf.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&s.at.to_rfc3339()),
            csv_escape(&s.device_id),
            csv_escape(&s.device_name),
            csv_escape(&s.connection),
            csv_escape(&s.component_id),
            csv_escape(&s.component_label),
            s.percentage,
            s.charging,
            s.low_power,
        ));
    }
    buf
}

/// CSV 字段转义：RFC 4180 + 公式注入防护。
/// - 包含逗号/引号/换行：用双引号包裹，内部引号翻倍；
/// - 以 `=`/`+`/`-`/`@` 开头：前缀单引号 `'`，防止 Excel/Sheets 公式注入。
fn csv_escape(field: &str) -> String {
    let needs_quote =
        field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r');
    let needs_formula_guard = field.starts_with('=')
        || field.starts_with('+')
        || field.starts_with('-')
        || field.starts_with('@');
    let mut s = String::new();
    if needs_formula_guard {
        s.push('\'');
    }
    if needs_quote {
        s.push('"');
        for c in field.chars() {
            if c == '"' {
                s.push('"');
                s.push('"');
            } else {
                s.push(c);
            }
        }
        s.push('"');
    } else {
        s.push_str(field);
    }
    s
}

pub fn clear_history(
    state: &BatteryHistoryState,
    app: &AppHandle,
    device_key: Option<&str>,
) -> Result<(), String> {
    let persisted_samples = {
        let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        let mut last = state.last_record.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(device_key) = device_key {
            let groups = build_sample_groups(&samples);
            let target_group = groups
                .iter()
                .find(|group| group.matches_key(device_key))
                .cloned();
            samples.retain(|sample| {
                target_group
                    .as_ref()
                    .map(|group| !group.matches_sample(sample))
                    .unwrap_or_else(|| {
                        sample_device_key(sample) != device_key
                            && sample_legacy_key(sample) != device_key
                    })
            });
            last.retain(|key, (sample, _)| {
                key != device_key
                    && target_group
                        .as_ref()
                        .map(|group| !group.matches_sample(sample))
                        .unwrap_or_else(|| {
                            sample_device_key(sample) != device_key
                                && sample_legacy_key(sample) != device_key
                        })
            });
        } else {
            samples.clear();
            last.clear();
        }
        samples.clone()
    };

    let file = BatteryHistoryFile {
        schema_version: SCHEMA_VERSION,
        samples: persisted_samples,
    };
    save_history(app, &file)
}

#[allow(dead_code)]
fn clear_device_history(
    state: &BatteryHistoryState,
    app: &AppHandle,
    device_key: &str,
) -> Result<(), String> {
    clear_history(state, app, Some(device_key))
}

#[cfg(test)]
fn clear_history_in_memory(state: &BatteryHistoryState, device_key: Option<&str>) {
    {
        let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        let mut last = state.last_record.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(device_key) = device_key {
            let groups = build_sample_groups(&samples);
            let target_group = groups
                .iter()
                .find(|group| group.matches_key(device_key))
                .cloned();
            samples.retain(|sample| {
                target_group
                    .as_ref()
                    .map(|group| !group.matches_sample(sample))
                    .unwrap_or_else(|| {
                        sample_device_key(sample) != device_key
                            && sample_legacy_key(sample) != device_key
                    })
            });
            last.retain(|key, (sample, _)| {
                key != device_key
                    && target_group
                        .as_ref()
                        .map(|group| !group.matches_sample(sample))
                        .unwrap_or_else(|| {
                            sample_device_key(sample) != device_key
                                && sample_legacy_key(sample) != device_key
                        })
            });
        } else {
            samples.clear();
            last.clear();
        }
    }
}

pub struct AbnormalDrainNotifyState {
    last_notify: Mutex<BTreeMap<String, DateTime<Utc>>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbnormalDrainNotifyFile {
    #[serde(default)]
    last_notify: BTreeMap<String, DateTime<Utc>>,
}

fn notify_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("battery_drain_notify.json"))
}

/// 原子写入节流状态文件。
fn save_notify(app: &AppHandle, file: &AbnormalDrainNotifyFile) -> Result<(), String> {
    let Some(path) = notify_path(app) else {
        return Err("config dir unavailable".into());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    let bytes = serde_json::to_vec(file).map_err(|e| format!("serialize failed: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("write tmp failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename failed: {e}"))?;
    Ok(())
}

impl AbnormalDrainNotifyState {
    pub fn new() -> Self {
        Self {
            last_notify: Mutex::new(BTreeMap::new()),
        }
    }

    /// 从磁盘加载节流状态。文件不存在或损坏时返回空状态，不崩溃。
    pub fn load_from_disk(&self, app: &AppHandle) {
        let Some(path) = notify_path(app) else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        match serde_json::from_slice::<AbnormalDrainNotifyFile>(&bytes) {
            Ok(file) => {
                if let Ok(mut guard) = self.last_notify.lock() {
                    let now = Utc::now();
                    *guard = file
                        .last_notify
                        .into_iter()
                        .filter(|(_, t)| now - t < Duration::hours(24))
                        .collect();
                }
            }
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// 持久化节流状态到磁盘。
    pub fn save_to_disk(&self, app: &AppHandle) -> Result<(), String> {
        let guard = self.last_notify.lock().unwrap_or_else(|e| e.into_inner());
        let file = AbnormalDrainNotifyFile {
            last_notify: guard.clone(),
        };
        save_notify(app, &file)
    }

    /// 检查是否应该通知。同一设备同一部件 24 小时内最多通知一次。
    /// 注意：此方法只更新内存状态，调用方需要在通知发送后调用 `save_to_disk` 持久化。
    pub fn should_notify(&self, key: &str, now: DateTime<Utc>) -> bool {
        let mut guard = self.last_notify.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&last) = guard.get(key) {
            if now - last < Duration::hours(24) {
                return false;
            }
        }
        guard.insert(key.to_string(), now);
        true
    }
}

impl Default for AbnormalDrainNotifyState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 单元测试 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample(at: DateTime<Utc>, pct: u8, charging: bool) -> BatterySample {
        BatterySample {
            at,
            device_id: "abc123".into(),
            device_name: "Test Mouse".into(),
            identity_group: None,
            identity_aliases: Vec::new(),
            connection: "wireless".into(),
            component_id: "mouse".into(),
            component_label: "mock.mouseLabel".into(),
            percentage: pct,
            charging,
            low_power: !charging && pct < 20,
            eligible_for_prediction: true,
            context: None,
        }
    }

    #[test]
    fn anonymize_device_key_is_stable_and_no_raw_path() {
        let key1 = anonymize_device_key("/dev/hidraw0");
        let key2 = anonymize_device_key("/dev/hidraw0");
        let key3 = anonymize_device_key("/dev/hidraw1");
        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_eq!(key1.len(), 16);
        assert!(!key1.contains("hidraw"));
        assert!(!key1.contains("/dev"));
    }

    #[test]
    fn record_samples_dedup_same_percentage_and_charging() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        state.last_record.lock().unwrap().insert(
            "abc123:mouse".into(),
            (make_sample(now, 80, false), Instant::now()),
        );
        assert!(state
            .last_record
            .lock()
            .unwrap()
            .contains_key("abc123:mouse"));
    }

    #[test]
    fn context_change_bypasses_battery_sample_dedup_window() {
        let previous = make_sample(Utc::now(), 80, false);
        let mut changed = previous.clone();
        changed.context = Some(DeviceContextSnapshot {
            dpi: Some(3200),
            ..DeviceContextSnapshot::default()
        });
        assert!(should_record_candidate(
            &previous,
            &changed,
            std::time::Duration::from_secs(1),
        ));
        assert!(!should_record_candidate(
            &previous,
            &previous,
            std::time::Duration::from_secs(1),
        ));
    }

    #[test]
    fn aggregate_active_usage_tracks_only_connected_usage_time() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(10), 90, false),
            make_sample(now - Duration::minutes(5), 85, false),
            make_sample(now, 80, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let points = aggregate_active_usage(&refs, now, 20);

        assert_eq!(points.len(), 3);
        assert_eq!(points[0].usage_elapsed_minutes, Some(0));
        assert_eq!(points[1].usage_elapsed_minutes, Some(5));
        assert_eq!(points[2].usage_elapsed_minutes, Some(10));
        assert_eq!(points[2].bucket_label, "10m");
    }

    #[test]
    fn aggregate_active_usage_compresses_sleep_and_disconnect_gaps() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::hours(6), 90, false),
            make_sample(now - Duration::minutes(5), 80, false),
            make_sample(now, 70, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let points = aggregate_active_usage(&refs, now, 20);

        assert_eq!(points.len(), 3);
        assert_eq!(points[0].usage_elapsed_minutes, Some(0));
        assert_eq!(points[1].usage_elapsed_minutes, Some(0));
        assert_eq!(points[2].usage_elapsed_minutes, Some(5));
        assert_eq!(points[1].bucket_label, "0m");
    }

    #[test]
    fn aggregate_active_usage_caps_display_at_48_real_sample_groups() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = (0..192)
            .map(|i| make_sample(now - Duration::minutes((191 - i) * 5), 90, false))
            .collect();
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let points = aggregate_active_usage(&refs, now, 20);

        assert_eq!(points.len(), 48);
        assert!(points.iter().all(|point| point.sample_count == 4));
        assert_eq!(points.last().unwrap().usage_elapsed_minutes, Some(955));
    }

    #[test]
    fn aggregate_ten_day_history_uses_thirty_calendar_slots_and_keeps_gaps() {
        let now = Utc::now();
        let samples = [
            make_sample(now - Duration::days(2), 84, false),
            make_sample(now - Duration::days(1), 81, false),
            make_sample(now, 79, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();

        let points = aggregate_ten_day_history(&refs, now, 20);

        assert_eq!(points.len(), 30);
        assert_eq!(points[0].sample_count, 0);
        assert_eq!(
            points.iter().map(|point| point.sample_count).sum::<u32>(),
            3
        );
        let current_slot = 27 + (now.with_timezone(&Local).hour() / 8) as usize;
        assert_eq!(points[current_slot].percentage, Some(79));
        assert!(points[current_slot]
            .bucket_label
            .starts_with(&now.with_timezone(&Local).format("%m-%d").to_string()));
    }

    #[test]
    fn ten_day_window_starts_at_first_displayed_local_midnight() {
        let now = Utc::now();
        let start = range_window_start("10d", now).with_timezone(&Local);

        assert_eq!(start.date_naive(), ten_day_first_day(now));
        assert_eq!(start.hour(), 0);
        assert_eq!(start.minute(), 0);
        assert_eq!(start.second(), 0);
        assert_eq!(range_window_start("24h", now), now - Duration::hours(24));
    }

    #[test]
    fn ten_day_baseline_prediction_excludes_samples_before_the_chart() {
        let now = Utc::now();
        let start = range_window_start("10d", now);
        let samples = [
            make_sample(start - Duration::minutes(1), 90, false),
            make_sample(start + Duration::minutes(1), 80, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();

        assert!(estimate_remaining(&refs, "10d", now).is_none());
    }

    #[test]
    fn ten_day_slot_reports_charging_occurrence_without_changing_last_level() {
        let now = Utc::now();
        let local_day = now.with_timezone(&Local).date_naive() - Duration::days(1);
        let slot_start = local_day
            .and_hms_opt(8, 0, 0)
            .and_then(|start| start.and_local_timezone(Local).earliest())
            .unwrap()
            .with_timezone(&Utc);
        let samples = [
            make_sample(slot_start + Duration::minutes(5), 60, true),
            make_sample(slot_start + Duration::minutes(10), 82, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();

        let points = aggregate_ten_day_history(&refs, now, 20);
        let point = &points[25];
        assert_eq!(point.percentage, Some(82));
        assert_eq!(point.charging, Some(true));
        assert_eq!(point.sample_count, 2);
    }

    #[test]
    fn new_state_persists_first_sample_immediately() {
        let state = BatteryHistoryState::new();
        let last_persist = *state.last_persist.lock().unwrap();
        assert!(should_persist(last_persist));
    }

    #[test]
    fn build_response_recomputes_low_battery_with_current_threshold() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        // Equality is low battery too, matching tray fill and notifications.
        let mut sample = make_sample(now, 30, false);
        sample.low_power = false;
        state.samples.lock().unwrap().push(sample);

        let response = build_response(&state, 30, "24h");
        assert_eq!(response.devices.len(), 1);
        assert_eq!(response.devices[0].low_battery, Some(true));
        let low_points = response.series[0]
            .points
            .iter()
            .filter(|point| point.sample_count > 0)
            .filter(|point| point.low_battery == Some(true))
            .count();
        assert_eq!(low_points, 1);
    }

    #[test]
    fn plugin_identity_keeps_policy_approved_history_when_direct_alias_is_present() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        let mut usb = make_sample(now - Duration::minutes(5), 82, false);
        usb.device_id = "usb-device".into();
        usb.device_name = "amaster protocol-a-direct".into();
        usb.connection = "usb".into();
        usb.eligible_for_prediction = false;

        let mut wireless = make_sample(now, 81, false);
        wireless.device_id = "wireless-device".into();
        wireless.device_name = "AM INFINITY 8K MOUSE".into();
        wireless.identity_group = Some("am-infinity-8k-mouse".into());
        wireless.identity_aliases = vec![
            "amaster protocol-a-direct".into(),
            "amaster protocol-a-receiver".into(),
            "AM INFINITY 8K MOUSE".into(),
        ];
        wireless.connection = "wireless".into();

        state.samples.lock().unwrap().extend([usb, wireless]);
        let response = build_response(&state, 20, "24h");

        assert_eq!(response.devices.len(), 1);
        assert_eq!(response.devices[0].device_name, "AM INFINITY 8K MOUSE");
        assert_eq!(response.devices[0].connection, "wireless");
        assert_eq!(response.devices[0].latest_percentage, Some(81));
        assert_eq!(response.series.len(), 1);
        assert_eq!(
            response.series[0]
                .points
                .iter()
                .map(|point| point.sample_count)
                .sum::<u32>(),
            1
        );
    }

    #[test]
    fn usb_powered_samples_are_excluded_from_battery_analysis() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        let wireless_start = make_sample(now - Duration::minutes(55), 72, false);
        let wireless_latest = make_sample(now - Duration::minutes(5), 70, false);
        let mut usb_placeholder = make_sample(now, 100, false);
        usb_placeholder.connection = "usb".into();
        usb_placeholder.eligible_for_prediction = false;
        state
            .samples
            .lock()
            .unwrap()
            .extend([wireless_start, wireless_latest, usb_placeholder]);

        let response = build_response(&state, 20, "24h");

        assert_eq!(response.devices.len(), 1);
        assert_eq!(response.devices[0].connection, "wireless");
        assert_eq!(response.devices[0].latest_percentage, Some(70));
        assert_eq!(
            response.series[0]
                .points
                .iter()
                .map(|point| point.sample_count)
                .sum::<u32>(),
            2
        );
        assert!(response
            .insights
            .iter()
            .any(|insight| insight.insight_type == "estimatedRemaining"));
    }

    #[test]
    fn battery_history_follows_plugin_declared_connections() {
        let battery = PluginCapability {
            id: "battery".into(),
            control: "ReadOnlyValue".into(),
            label_key: "capability.battery".into(),
            read_only: true,
            placements: Vec::new(),
            metadata: BTreeMap::from([(
                "batteryHistory".into(),
                serde_json::json!({ "validConnections": ["wireless", "bluetooth", "usb"] }),
            )]),
            available: true,
            connections: None,
            min_firmware: None,
        };

        assert!(battery_history_allowed(
            std::slice::from_ref(&battery),
            &Connection::Usb
        ));
        assert!(battery_history_allowed(
            std::slice::from_ref(&battery),
            &Connection::Wireless
        ));
        assert!(battery_history_allowed(&[battery], &Connection::Bluetooth));
        assert!(!battery_history_allowed(&[], &Connection::Wireless));
    }

    #[test]
    fn estimate_remaining_ignores_charging() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(100), 100, false),
            make_sample(now - Duration::minutes(90), 99, false),
            make_sample(now - Duration::minutes(80), 98, false),
            make_sample(now - Duration::minutes(70), 97, false),
            make_sample(now - Duration::minutes(60), 97, true),
            make_sample(now - Duration::minutes(50), 100, true),
            make_sample(now - Duration::minutes(30), 100, false),
            make_sample(now - Duration::minutes(20), 99, false),
            make_sample(now - Duration::minutes(10), 98, false),
            make_sample(now, 97, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_some());
        let hours = remaining.unwrap();
        assert!(hours > 0.0);
    }

    #[test]
    fn estimate_remaining_ignores_session_gap() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        for i in 0..16 {
            samples.push(make_sample(
                now - Duration::hours(9) + Duration::minutes(i * 5),
                100 - i as u8,
                false,
            ));
        }
        samples.push(make_sample(now - Duration::minutes(5), 78, false));
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_some());
        if let Some(h) = remaining {
            assert!(
                h < 20.0,
                "remaining should be based on continuous segment only, got {}h",
                h
            );
        }
    }

    #[test]
    fn estimate_remaining_splits_on_quick_battery_replacement() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(75), 22, false),
            make_sample(now - Duration::minutes(65), 21, false),
            make_sample(now - Duration::minutes(55), 20, false),
            make_sample(now - Duration::minutes(45), 19, false),
            make_sample(now - Duration::minutes(40), 92, false),
            make_sample(now - Duration::minutes(30), 91, false),
            make_sample(now - Duration::minutes(20), 90, false),
            make_sample(now - Duration::minutes(10), 89, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_some());
        if let Some(h) = remaining {
            assert!(
                h > 0.0 && h < 20.0,
                "replacement boundary should not hide drain, got {h}h"
            );
        }
    }

    #[test]
    fn estimate_remaining_not_enough_data() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![make_sample(now, 80, false)];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_none());
    }

    #[test]
    fn detect_abnormal_drain_triggers_on_high_drain() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - (i / 6) as u8,
                false,
            ));
        }
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(1) + Duration::minutes(i * 5),
                80 - ((i * 6) / 11) as u8,
                false,
            ));
        }
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let drain = detect_abnormal_drain(&refs, now);
        assert!(drain.is_some());
        assert!(drain.unwrap() > 5.0);
    }

    #[test]
    fn detect_abnormal_drain_no_trigger_on_normal() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - i as u8,
                false,
            ));
        }
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(1) + Duration::minutes(i * 5),
                78 - i as u8,
                false,
            ));
        }
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let drain = detect_abnormal_drain(&refs, now);
        assert!(drain.is_none());
    }

    #[test]
    fn detect_abnormal_drain_no_false_positive_on_reconnect() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - i as u8,
                false,
            ));
        }
        for i in 0..6 {
            samples.push(make_sample(
                now - Duration::minutes(30) + Duration::minutes(i * 5),
                80 - i as u8,
                false,
            ));
        }
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let drain = detect_abnormal_drain(&refs, now);
        assert!(drain.is_none());
    }

    #[test]
    fn analyze_charging_habit_detects_charging_during_disconnect() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        for i in 0..6 {
            samples.push(make_sample(
                now - Duration::hours(5) + Duration::minutes(i * 5),
                20,
                false,
            ));
        }
        for i in 0..6 {
            samples.push(make_sample(
                now - Duration::minutes(30) + Duration::minutes(i * 5),
                85,
                false,
            ));
        }
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let habit = analyze_charging_habit(&refs, now);
        assert!(habit.is_some());
        let message = habit.unwrap().message;
        assert!(
            message.contains("chargingHabitStartEnd|20|85"),
            "should detect charge during disconnect: {}",
            message
        );
    }

    #[test]
    fn analyze_charging_habit_detects_quick_battery_replacement() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(30), 19, false),
            make_sample(now - Duration::minutes(20), 18, false),
            make_sample(now - Duration::minutes(10), 91, false),
            make_sample(now, 90, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let habit = analyze_charging_habit(&refs, now);
        assert!(habit.is_some());
        let message = habit.unwrap().message;
        assert!(
            message.contains("chargingHabitStartEnd|18|91"),
            "should detect quick replacement replenishment: {}",
            message
        );
    }

    #[test]
    fn full_level_charging_status_toggle_is_not_a_replenishment_event() {
        let now = Utc::now();
        let samples = vec![
            make_sample(now - Duration::minutes(30), 100, false),
            make_sample(now - Duration::minutes(20), 100, true),
            make_sample(now - Duration::minutes(10), 100, false),
            make_sample(now, 100, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();

        assert!(analyze_charging_habit(&refs, now).is_none());
        assert_eq!(
            count_charges_in_window(&refs, now - Duration::hours(1)),
            Some(0)
        );
    }

    #[test]
    fn explicit_charging_with_level_gain_is_a_replenishment_event() {
        let now = Utc::now();
        let samples = vec![
            make_sample(now - Duration::minutes(30), 50, false),
            make_sample(now - Duration::minutes(20), 50, true),
            make_sample(now - Duration::minutes(10), 70, true),
            make_sample(now, 70, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();

        let habit = analyze_charging_habit(&refs, now).expect("meaningful charge should be shown");
        assert_eq!(habit.message, "chargingHabitStartEnd|50|70|1");
        assert_eq!(
            count_charges_in_window(&refs, now - Duration::hours(1)),
            Some(1)
        );
    }

    #[test]
    fn drain_rate_splits_on_quick_battery_replacement() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(95), 24, false),
            make_sample(now - Duration::minutes(85), 23, false),
            make_sample(now - Duration::minutes(75), 22, false),
            make_sample(now - Duration::minutes(65), 21, false),
            make_sample(now - Duration::minutes(60), 95, false),
            make_sample(now - Duration::minutes(50), 94, false),
            make_sample(now - Duration::minutes(40), 93, false),
            make_sample(now - Duration::minutes(30), 92, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let rate = drain_rate(&refs, now - Duration::hours(1), now);
        assert!(rate.is_some());
        let rate = rate.unwrap();
        assert!(
            (rate - 6.0).abs() < 0.01,
            "replacement should preserve per-segment drain rate, got {rate}"
        );
    }

    #[test]
    fn clear_history_empties_state() {
        let state = BatteryHistoryState::new();
        state
            .samples
            .lock()
            .unwrap()
            .push(make_sample(Utc::now(), 80, false));
        state.last_record.lock().unwrap().insert(
            "key".into(),
            (make_sample(Utc::now(), 80, false), Instant::now()),
        );
        let mut samples = state.samples.lock().unwrap();
        samples.clear();
        assert!(samples.is_empty());
    }

    #[test]
    fn damaged_json_does_not_crash() {
        let bad = b"{ not valid json";
        let result: Result<BatteryHistoryFile, _> = serde_json::from_slice(bad);
        assert!(result.is_err());
    }

    #[test]
    fn samples_to_csv_includes_header() {
        let samples = vec![make_sample(Utc::now(), 80, false)];
        let csv = samples_to_csv(&samples);
        assert!(csv.starts_with("at,deviceId"));
        assert!(csv.contains("80"));
        assert!(csv.contains("Test Mouse"));
    }

    #[test]
    fn export_history_json_is_valid() {
        let state = BatteryHistoryState::new();
        state
            .samples
            .lock()
            .unwrap()
            .push(make_sample(Utc::now(), 80, false));
        let json = export_history(&state, "json").unwrap();
        assert!(json.contains("schemaVersion"));
        assert!(json.contains("samples"));
        assert!(!json.contains("hidraw"));
        assert!(!json.contains("/dev/"));
    }

    #[test]
    fn abnormal_drain_notify_throttle_24h() {
        let state = AbnormalDrainNotifyState::new();
        let now = Utc::now();
        assert!(state.should_notify("mouse:abc", now));
        assert!(!state.should_notify("mouse:abc", now + Duration::hours(1)));
        assert!(!state.should_notify("mouse:abc", now + Duration::hours(23)));
        assert!(state.should_notify("mouse:abc", now + Duration::hours(25)));
    }

    #[test]
    fn record_samples_prunes_old_data() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        let old = make_sample(now - Duration::days(32), 90, false);
        let recent = make_sample(now, 80, false);
        {
            let mut samples = state.samples.lock().unwrap();
            samples.push(old);
            samples.push(recent);
            let cutoff = now - Duration::days(31);
            samples.retain(|s| s.at >= cutoff);
        }
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].percentage, 80);
    }

    #[test]
    fn csv_escape_handles_special_characters() {
        assert_eq!(csv_escape("Test Mouse"), "Test Mouse");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape("a\nb"), "\"a\nb\"");
        assert_eq!(csv_escape("=cmd"), "'=cmd");
        assert_eq!(csv_escape("+1+1"), "'+1+1");
        assert_eq!(csv_escape("-1-1"), "'-1-1");
        assert_eq!(csv_escape("@sum"), "'@sum");
        assert_eq!(csv_escape("=a,b"), "'\"=a,b\"");
    }

    #[test]
    fn csv_export_escapes_device_name_with_comma() {
        let mut sample = make_sample(Utc::now(), 80, false);
        sample.device_name = "Mouse, Pro".into();
        let csv = samples_to_csv(&[sample]);
        assert!(csv.contains("\"Mouse, Pro\""));
    }

    #[test]
    fn normalize_percentage_filters_unknown_and_clamps() {
        assert_eq!(normalize_percentage(50), Some(50));
        assert_eq!(normalize_percentage(0), Some(0));
        assert_eq!(normalize_percentage(100), Some(100));
        assert_eq!(normalize_percentage(0xFF), None);
        assert_eq!(normalize_percentage(101), Some(100));
        assert_eq!(normalize_percentage(200), Some(100));
    }

    #[test]
    fn estimate_remaining_uses_last_non_charging_sample() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(60), 80, false),
            make_sample(now - Duration::minutes(50), 79, false),
            make_sample(now - Duration::minutes(40), 78, false),
            make_sample(now - Duration::minutes(30), 77, false),
            make_sample(now - Duration::minutes(20), 95, true),
            make_sample(now - Duration::minutes(10), 95, true),
            make_sample(now, 95, true),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_some());
        if let Some(h) = remaining {
            assert!(
                h < 15.0,
                "should use last non-charging sample (77), got {}h",
                h
            );
        }
    }

    #[test]
    fn estimate_remaining_does_not_predict_from_one_percent_bounce() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(50), 80, false),
            make_sample(now - Duration::minutes(40), 79, false),
            make_sample(now - Duration::minutes(30), 80, false),
            make_sample(now - Duration::minutes(20), 79, false),
            make_sample(now - Duration::minutes(10), 80, false),
            make_sample(now, 79, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_none());
    }

    #[test]
    fn short_one_percent_step_is_not_extrapolated_as_a_rate() {
        let now = Utc::now();
        let first = make_sample(now - Duration::minutes(5), 80, false);
        let last = make_sample(now, 79, false);

        assert!(segment_drain_rate(&first, &last).is_none());
    }

    #[test]
    fn active_ai_estimate_must_stay_close_to_robust_baseline() {
        assert_eq!(select_active_remaining(Some(60.0), Some(50.0)), Some(60.0));
        assert_eq!(select_active_remaining(Some(24.0), Some(56.0)), Some(56.0));
        assert_eq!(select_active_remaining(Some(150.0), Some(50.0)), Some(50.0));
        assert_eq!(select_active_remaining(Some(60.0), None), None);
    }

    #[test]
    fn remaining_message_normalizes_rounded_day_boundary() {
        assert_eq!(remaining_message(47.6), "remainingDaysHours|2|0");
    }

    #[test]
    fn estimate_remaining_very_slow_drain_exceeds_sentinel() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        let intervals = 606;
        for i in 0..=intervals {
            let at = now - Duration::hours(101) + Duration::minutes(i * 10);
            let pct = if i <= intervals / 2 { 100 } else { 99 };
            samples.push(make_sample(at, pct, false));
        }
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "10d", now);
        assert!(remaining.is_some());
        let h = remaining.unwrap();
        assert!(
            h >= VERY_SLOW_DRAIN_HOURS,
            "very slow drain should return >= sentinel ({}), got {}h",
            VERY_SLOW_DRAIN_HOURS,
            h
        );
    }

    #[test]
    fn build_insights_very_slow_drain_shows_i18n_key() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        let intervals = 606;
        for i in 0..=intervals {
            let at = now - Duration::hours(101) + Duration::minutes(i * 10);
            let pct = if i <= intervals / 2 { 100 } else { 99 };
            samples.push(make_sample(at, pct, false));
        }
        let device = BatteryHistoryDevice {
            key: "abc123:mouse".into(),
            device_id: "abc123".into(),
            device_name: "Test Mouse".into(),
            connection: "wireless".into(),
            component_id: "mouse".into(),
            component_label: "mock.mouseLabel".into(),
            latest_percentage: Some(99),
            latest_charging: Some(false),
            latest_at: Some(now.to_rfc3339()),
            low_battery: Some(false),
        };
        let insights = build_insights(&samples, &[device], 20, "10d", now, false, None);
        let remaining = insights
            .iter()
            .find(|i| i.insight_type == "estimatedRemaining");
        assert!(
            remaining.is_some(),
            "should have estimatedRemaining insight"
        );
        assert_eq!(
            remaining.unwrap().message,
            "veryLowDrain",
            "very slow drain should show i18n key"
        );
        let runout = insights
            .iter()
            .find(|i| i.insight_type == "estimatedRunout");
        assert!(
            runout.is_none(),
            "should not show runout time for very slow drain"
        );
    }

    #[test]
    fn detect_abnormal_drain_skips_post_charge_window() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - i as u8,
                false,
            ));
        }
        samples.push(make_sample(now - Duration::minutes(30), 60, true));
        samples.push(make_sample(now - Duration::minutes(25), 70, true));
        samples.push(make_sample(now - Duration::minutes(5), 80, true));
        samples.push(make_sample(now, 70, false));
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let drain = detect_abnormal_drain(&refs, now);
        assert!(
            drain.is_none(),
            "should skip post-charge voltage correction window"
        );
    }

    #[test]
    fn migrate_schema_upgrades_old_version() {
        let mut old_sample = make_sample(Utc::now(), 80, false);
        old_sample.eligible_for_prediction = true;
        let file = BatteryHistoryFile {
            schema_version: 0,
            samples: vec![old_sample],
        };
        let migrated = migrate_schema(file);
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert_eq!(migrated.samples.len(), 1);
        assert!(!migrated.samples[0].eligible_for_prediction);
    }

    #[test]
    fn current_plugin_policy_restores_matching_legacy_history_only() {
        let now = Utc::now();
        let mut matching = make_sample(now - Duration::hours(1), 82, false);
        matching.eligible_for_prediction = false;
        let mut wrong_connection = matching.clone();
        wrong_connection.connection = "usb".into();
        let mut wrong_device = matching.clone();
        wrong_device.device_name = "Another Mouse".into();
        wrong_device.device_id = "another-device".into();
        let mut samples = vec![matching, wrong_connection, wrong_device];
        let approved = make_sample(now, 80, false);

        let promoted = promote_policy_approved_legacy_samples(&mut samples, &approved);

        assert_eq!(promoted, 1);
        assert!(samples[0].eligible_for_prediction);
        assert!(!samples[1].eligible_for_prediction);
        assert!(!samples[2].eligible_for_prediction);
    }

    #[test]
    fn migrate_schema_keeps_current_version() {
        let file = BatteryHistoryFile {
            schema_version: SCHEMA_VERSION,
            samples: vec![make_sample(Utc::now(), 80, false)],
        };
        let migrated = migrate_schema(file);
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn max_samples_cap_drops_oldest() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        {
            let mut samples = state.samples.lock().unwrap();
            for i in 0..(MAX_SAMPLES + 10) {
                let at = now - Duration::minutes((MAX_SAMPLES + 10 - i) as i64);
                samples.push(make_sample(at, (i % 100) as u8, false));
            }
            if samples.len() > MAX_SAMPLES {
                samples.sort_by_key(|a| a.at);
                let drop_count = samples.len() - MAX_SAMPLES;
                samples.drain(0..drop_count);
            }
        }
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), MAX_SAMPLES);
        let earliest = samples.first().unwrap();
        assert!(earliest.at >= now - Duration::minutes(MAX_SAMPLES as i64));
    }

    #[test]
    fn drain_rate_splits_by_charging_transition() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(120), 80, false),
            make_sample(now - Duration::minutes(110), 78, false),
            make_sample(now - Duration::minutes(100), 77, false),
            make_sample(now - Duration::minutes(90), 75, false),
            make_sample(now - Duration::minutes(70), 100, true),
            make_sample(now - Duration::minutes(60), 100, true),
            make_sample(now - Duration::minutes(30), 95, false),
            make_sample(now - Duration::minutes(20), 92, false),
            make_sample(now - Duration::minutes(10), 88, false),
            make_sample(now, 85, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let rate = drain_rate(&refs, now - Duration::hours(2), now + Duration::seconds(1));
        assert!(rate.is_some());
        if let Some(r) = rate {
            assert!(
                (r - 15.0).abs() < 0.01,
                "should split by charging, got {} %/h",
                r
            );
        }
    }

    #[test]
    fn calendar_daily_drain_uses_wall_clock_time_across_session_gaps() {
        let now = Utc::now();
        let samples = [
            make_sample(now - Duration::hours(30), 91, false),
            make_sample(now - Duration::hours(29), 87, false),
            make_sample(now - Duration::hours(12), 85, false),
            make_sample(now, 62, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let daily =
            calendar_daily_drain(&refs, now - Duration::hours(31), now + Duration::seconds(1))
                .expect("30 hours of history should be enough");

        assert!(
            (daily - 23.2).abs() < 0.01,
            "unexpected daily drain: {daily}"
        );
    }

    #[test]
    fn calendar_daily_drain_requires_half_day_observation() {
        let now = Utc::now();
        let samples = [
            make_sample(now - Duration::minutes(30), 80, false),
            make_sample(now, 79, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();

        assert!(
            calendar_daily_drain(&refs, now - Duration::hours(1), now + Duration::seconds(1),)
                .is_none()
        );
    }

    #[test]
    fn calendar_daily_drain_splits_charging_cycles() {
        let now = Utc::now();
        let samples = [
            make_sample(now - Duration::hours(48), 80, false),
            make_sample(now - Duration::hours(36), 70, false),
            make_sample(now - Duration::hours(35), 100, true),
            make_sample(now - Duration::hours(24), 95, false),
            make_sample(now, 85, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let daily =
            calendar_daily_drain(&refs, now - Duration::hours(49), now + Duration::seconds(1))
                .expect("two discharge episodes should provide enough history");

        assert!(
            (daily - 13.333_333).abs() < 0.01,
            "unexpected daily drain: {daily}"
        );
    }

    #[test]
    fn calendar_daily_drain_counts_stable_time() {
        let now = Utc::now();
        let samples = [
            make_sample(now - Duration::hours(24), 80, false),
            make_sample(now - Duration::hours(12), 80, false),
            make_sample(now, 79, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let daily =
            calendar_daily_drain(&refs, now - Duration::hours(25), now + Duration::seconds(1))
                .expect("a full day of stable history should be enough");

        assert!(
            (daily - 1.0).abs() < 0.01,
            "unexpected daily drain: {daily}"
        );
    }

    #[test]
    fn calendar_daily_drain_excludes_short_downward_swap_or_recalibration() {
        let now = Utc::now();
        let samples = [
            make_sample(now - Duration::hours(36), 90, false),
            make_sample(now - Duration::hours(24), 87, false),
            make_sample(now - Duration::hours(24) + Duration::minutes(5), 50, false),
            make_sample(now, 45, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let daily =
            calendar_daily_drain(&refs, now - Duration::hours(37), now + Duration::seconds(1))
                .expect("both sides of the discontinuity provide natural-time evidence");

        assert!(
            (5.0..5.7).contains(&daily),
            "battery swap/recalibration jump must not be counted as drain: {daily}"
        );
    }

    #[test]
    fn calendar_daily_drain_keeps_plausible_drop_across_long_disconnect() {
        let now = Utc::now();
        let samples = [
            make_sample(now - Duration::hours(48), 80, false),
            make_sample(now, 60, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let daily =
            calendar_daily_drain(&refs, now - Duration::hours(49), now + Duration::seconds(1))
                .expect("a long disconnect is still natural-time evidence");

        assert!(
            (daily - 10.0).abs() < 0.01,
            "unexpected daily drain: {daily}"
        );
    }

    #[test]
    fn build_response_updates_device_name_to_latest() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        {
            let mut samples = state.samples.lock().unwrap();
            let mut s1 = make_sample(now - Duration::hours(1), 80, false);
            s1.device_name = "Old Name".into();
            let mut s2 = make_sample(now, 75, false);
            s2.device_name = "New Name".into();
            samples.push(s1);
            samples.push(s2);
        }
        let resp = build_response(&state, 20, "24h");
        assert_eq!(resp.devices.len(), 1);
        assert_eq!(resp.devices[0].device_name, "New Name");
    }

    #[test]
    fn build_response_merges_same_named_device_with_different_ids() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        {
            let mut samples = state.samples.lock().unwrap();
            let mut s1 = make_sample(now - Duration::hours(1), 80, false);
            s1.device_id = "old-path-hash".into();
            s1.device_name = "Mira Mouse".into();
            let mut s2 = make_sample(now, 75, false);
            s2.device_id = "new-path-hash".into();
            s2.device_name = "Mira Mouse".into();
            samples.push(s1);
            samples.push(s2);
        }

        let resp = build_response(&state, 20, "24h");

        assert_eq!(resp.devices.len(), 1);
        assert_eq!(resp.devices[0].latest_percentage, Some(75));
        let total_samples: u32 = resp.series[0].points.iter().map(|p| p.sample_count).sum();
        assert_eq!(total_samples, 2);
    }

    #[test]
    fn clear_history_in_memory_only_removes_requested_device() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        let mut mouse = make_sample(now, 80, false);
        mouse.device_name = "Mira Mouse".into();
        let mut receiver = make_sample(now, 100, false);
        receiver.device_name = "Mira Receiver".into();
        receiver.component_id = "receiver".into();
        receiver.component_label = "mock.receiverLabel".into();
        let mouse_key = sample_device_key(&mouse);
        let receiver_key = sample_device_key(&receiver);
        {
            let mut samples = state.samples.lock().unwrap();
            samples.push(mouse.clone());
            samples.push(receiver.clone());
        }
        {
            let mut last = state.last_record.lock().unwrap();
            last.insert(mouse_key.clone(), (mouse, Instant::now()));
            last.insert(receiver_key.clone(), (receiver, Instant::now()));
        }

        clear_history_in_memory(&state, Some(&mouse_key));

        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(sample_device_key(&samples[0]), receiver_key);
        let last = state.last_record.lock().unwrap();
        assert!(!last.contains_key(&mouse_key));
        assert!(last.contains_key(&receiver_key));
    }

    #[test]
    fn merge_samples_preserves_newer_memory_samples() {
        let now = Utc::now();
        let disk = vec![
            make_sample(now - Duration::hours(2), 80, false),
            make_sample(now - Duration::hours(1), 78, false),
        ];
        let memory = vec![make_sample(now, 75, false)];
        let merged = merge_samples(disk, &memory);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged.last().unwrap().percentage, 75);
        assert_eq!(merged[0].percentage, 80);
    }

    #[test]
    fn merge_samples_keeps_all_memory_when_disk_empty() {
        let now = Utc::now();
        let disk: Vec<BatterySample> = Vec::new();
        let memory = vec![
            make_sample(now - Duration::hours(1), 80, false),
            make_sample(now, 75, false),
        ];
        let merged = merge_samples(disk, &memory);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_samples_drops_memory_older_than_disk() {
        let now = Utc::now();
        let disk = vec![make_sample(now - Duration::hours(1), 78, false)];
        let memory = vec![
            make_sample(now - Duration::hours(3), 85, false),
            make_sample(now, 75, false),
        ];
        let merged = merge_samples(disk, &memory);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].percentage, 78);
        assert_eq!(merged[1].percentage, 75);
    }

    #[test]
    fn should_persist_respects_interval() {
        use std::time::Duration;
        assert!(!should_persist(Instant::now()));
        assert!(should_persist(
            Instant::now() - Duration::from_secs(PERSIST_INTERVAL_SECS)
        ));
        assert!(should_persist(
            Instant::now() - Duration::from_secs(PERSIST_INTERVAL_SECS + 1)
        ));
        assert!(!should_persist(
            Instant::now() - Duration::from_secs(PERSIST_INTERVAL_SECS - 60)
        ));
    }
}
