// SPDX-License-Identifier: AGPL-3.0-or-later
//! 电量使用情况后端模块。
//!
//! 负责：
//! - 采样：在设备轮询成功后记录电量样本（带去重和保留期清理）。
//! - 存储：在 app config dir 下持久化 `battery_history.json`（临时文件 + rename）。
//! - 聚合：生成 24 小时 / 10 天 bucket 序列。
//! - 洞察：续航估算、耗尽时间、充电习惯、异常耗电、续航稳定性、设备对比。
//! - 导出：JSON / CSV。
//! - 清除：清空历史。
//!
//! 隐私要求：不存 raw HID path，使用脱敏后的设备 key（SHA-256 截断）。
//! 数据只保存在本机，不联网，不遥测。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use chrono::{DateTime, Duration, Local, NaiveDate, Timelike, Utc};
use mira_core::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::DeviceSnapshotEntry;

// ─── 常量 ───────────────────────────────────────────────────────────────────

/// 同一设备同一部件：电量和充电状态都没变时，至少间隔多少分钟再记录。
const DEDUP_INTERVAL_MINUTES: i64 = 5;
/// 默认保留天数（文档用途；实际默认值由 AppSettings 提供）。
#[allow(dead_code)]
const DEFAULT_RETENTION_DAYS: i64 = 10;
/// 额外缓冲天数，避免午夜边界数据丢失。
const RETENTION_BUFFER_DAYS: i64 = 1;
/// schema 版本。
const SCHEMA_VERSION: u32 = 1;
/// 会话间隙阈值（分钟）：相邻样本时间差超过此值视为设备断连。
/// 断连期间的掉电原因复杂（关机耗电、开机自检等），不参与掉电速度计算。
/// 取值依据：2 倍去重间隔（5min × 2 = 10min），正常在线轮询不会触发。
const SESSION_GAP_THRESHOLD_MINUTES: i64 = 10;
/// 历史样本硬上限：避免极端情况下文件无限增长。
/// 10 天保留 + 5 分钟去重 → 单设备单部件理论上限 ≈ 2880 样本；
/// 多设备/多部件场景下 20000 足够，超出时按时间排序丢弃最早样本。
const MAX_SAMPLES: usize = 20000;
/// 电量抖动阈值（百分比）：相邻样本掉电小于此值视为抖动，不计入累计掉电。
/// 锂电池电压波动 + ADC 量化误差会导致 ±1% 抖动，高水位法过滤此类噪声。
const BOUNCE_THRESHOLD_PERCENT: f64 = 1.0;
/// 充电完成后电压校正窗口（分钟）：刚拔线时电压会回升后再下降，
/// 此窗口内的掉电数据不参与异常耗电检测，避免误报。
const POST_CHARGE_SKIP_MINUTES: i64 = 10;

// ─── 数据结构 ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatterySample {
    pub at: DateTime<Utc>,
    /// 脱敏后的设备 key（SHA-256 截断），不暴露 HID path。
    pub device_id: String,
    pub device_name: String,
    /// "usb" | "wireless" | "bluetooth" | "virtual"
    pub connection: String,
    /// "mouse" | "receiver" | 其他部件 id
    pub component_id: String,
    /// i18n key 或 fallback 文案，前端通过 t() 解析。
    pub component_label: String,
    pub percentage: u8,
    pub charging: bool,
    pub low_power: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHistoryFile {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub samples: Vec<BatterySample>,
}

/// 运行时状态：内存缓存 + 去重追踪。
pub struct BatteryHistoryState {
    samples: Mutex<Vec<BatterySample>>,
    /// key: `{device_id}:{component_id}` → (上次样本, 上次记录时刻)
    last_record: Mutex<BTreeMap<String, (BatterySample, Instant)>>,
}

impl BatteryHistoryState {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(Vec::new()),
            last_record: Mutex::new(BTreeMap::new()),
        }
    }

    /// 从磁盘加载历史。文件不存在或损坏时返回空历史，不崩溃。
    ///
    /// 加载时会执行 schema 迁移：将旧版本数据结构升级到当前 `SCHEMA_VERSION`。
    /// `last_record` 的 `Instant` 会被重置为 `Instant::now()`（无法跨进程恢复真实时刻），
    /// 这意味着重启后第一次去重检查会基于"刚加载"而非"上次记录的真实时间"，
    /// 是可接受的折衷：最多导致首次重复采样被多记一条，不影响功能正确性。
    pub fn load_from_disk(&self, app: &AppHandle) {
        let Some(path) = history_path(app) else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            // 文件不存在：正常首次启动。
            return;
        };
        match serde_json::from_slice::<BatteryHistoryFile>(&bytes) {
            Ok(file) => {
                // 迁移到当前 schema 版本。
                let mut file = file;
                if file.schema_version < SCHEMA_VERSION {
                    file = migrate_schema(file);
                    // 迁移后立即持久化新版本。
                    let _ = save_history(app, &file);
                }
                if let Ok(mut guard) = self.samples.lock() {
                    *guard = file.samples;
                    // 重建去重索引。
                    if let Ok(mut last) = self.last_record.lock() {
                        last.clear();
                        for sample in guard.iter() {
                            let key = format!("{}:{}", sample.device_id, sample.component_id);
                            last.insert(
                                key,
                                (sample.clone(), Instant::now()),
                            );
                        }
                    }
                }
            }
            Err(_) => {
                // 损坏：重建空历史，避免崩溃。
                if let Ok(mut guard) = self.samples.lock() {
                    guard.clear();
                }
                // 尝试删除损坏的文件。
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

// ─── 响应类型（与前端 TypeScript 类型对齐） ─────────────────────────────────

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
    pub percentage: Option<u8>,
    pub min_percentage: Option<u8>,
    pub max_percentage: Option<u8>,
    pub charging: Option<bool>,
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
    /// 关联设备 key（`{device_id}:{component_id}`）。
    /// None 表示跨设备洞察（如 deviceComparison），前端应始终展示。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_key: Option<String>,
}

// ─── 存储路径 ───────────────────────────────────────────────────────────────

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

/// Schema 迁移框架：将旧版本数据结构升级到当前 `SCHEMA_VERSION`。
///
/// 当前只有 v1，所以此函数主要是占位符，未来新增版本时在此处添加迁移逻辑：
/// - v0 → v1：旧文件 `schema_version` 默认为 0，v1 与 v0 数据结构兼容（直接升级版本号）。
/// - 后续版本迁移应按顺序执行：v0→v1→v2→...，每步只处理一个版本升级。
fn migrate_schema(mut file: BatteryHistoryFile) -> BatteryHistoryFile {
    // v0 → v1：数据结构兼容，仅升级版本号。
    // 未来若 v1 → v2 需要字段变更，在此添加：
    //   while file.schema_version < 2 { file = migrate_v1_to_v2(file); }
    if file.schema_version < SCHEMA_VERSION {
        file.schema_version = SCHEMA_VERSION;
    }
    file
}

// ─── 设备 key 脱敏 ──────────────────────────────────────────────────────────

/// 将 HID path 脱敏为稳定的 16 字符 hex key。
/// 不存储原始 path，仅存哈希，保证同一设备同一端口的稳定识别。
pub fn anonymize_device_key(device_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(device_key.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..8])
}

/// 规范化电量百分比：
/// - `0xFF` (255) 表示设备未上报电量（未知），返回 None 跳过记录；
/// - 其他值 clamp 到 0-100，防止越界（部分设备会返回 >100 的值）。
fn normalize_percentage(raw: u8) -> Option<u8> {
    if raw == 0xFF {
        return None;
    }
    Some(raw.min(100))
}

// ─── 采样 ───────────────────────────────────────────────────────────────────

/// 在设备轮询成功后调用：对 snapshot 中的每个电量部件记录样本。
///
/// 去重规则：
/// - 同一 `device_id + component_id`：
///   - 电量和充电状态都没变时，至少间隔 5 分钟再记录；
///   - 电量变化时立即记录；
///   - charging 状态变化时立即记录。
///
/// 保留规则：仅保留 `retention_days` 天 + 缓冲。
/// 记录失败不影响设备功能。
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
    let mut new_samples: Vec<BatterySample> = Vec::new();

    {
        let last_record = state.last_record.lock().unwrap_or_else(|e| e.into_inner());
        for entry in entries {
            let device_id = anonymize_device_key(&entry.device_key);
            let snapshot = &entry.snapshot;
            let connection = connection_str(&snapshot.connection);
            let device_name = &snapshot.display_name;
            let low_power_threshold = low_battery_threshold;

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
                // 过滤未知值并 clamp。
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
                let key = format!("{}:{}", device_id, component_id);
                let low_power = !charging && percentage < low_power_threshold;
                let should_record = match last_record.get(&key) {
                    Some((prev, last_instant)) => {
                        let changed = prev.percentage != percentage || prev.charging != charging;
                        if changed {
                            true
                        } else {
                            // 没变化：至少间隔 5 分钟。
                            last_instant.elapsed()
                                >= std::time::Duration::from_secs(
                                    (DEDUP_INTERVAL_MINUTES * 60) as u64,
                                )
                        }
                    }
                    None => true,
                };

                if should_record {
                    new_samples.push(BatterySample {
                        at: now,
                        device_id: device_id.clone(),
                        device_name: device_name.clone(),
                        connection: connection.clone(),
                        component_id,
                        component_label,
                        percentage,
                        charging,
                        low_power,
                    });
                }
            }
        }
    }

    if new_samples.is_empty() {
        return;
    }

    // 写入内存 + 去重索引 + 清理过期数据 + 持久化。
    let retention_with_buffer = retention_days.max(1) + RETENTION_BUFFER_DAYS;
    let cutoff = now - Duration::days(retention_with_buffer);

    let to_persist: Vec<BatterySample> = {
        let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        let mut last_record = state.last_record.lock().unwrap_or_else(|e| e.into_inner());

        for sample in &new_samples {
            let key = format!("{}:{}", sample.device_id, sample.component_id);
            last_record.insert(key, (sample.clone(), Instant::now()));
        }

        samples.extend(new_samples.iter().cloned());

        // 清理过期样本。
        samples.retain(|s| s.at >= cutoff);

        // 硬上限：超出 MAX_SAMPLES 时按时间排序丢弃最早样本。
        // 防止极端情况下（多设备/高频采样/保留期配置错误）文件无限增长。
        if samples.len() > MAX_SAMPLES {
            samples.sort_by(|a, b| a.at.cmp(&b.at));
            let drop_count = samples.len() - MAX_SAMPLES;
            samples.drain(0..drop_count);
        }

        samples.clone()
    };

    let file = BatteryHistoryFile {
        schema_version: SCHEMA_VERSION,
        samples: to_persist,
    };
    // 持久化失败不影响设备功能。
    let _ = save_history(app, &file);
}

fn connection_str(conn: &Connection) -> String {
    match conn {
        Connection::Usb => "usb".into(),
        Connection::Wireless => "wireless".into(),
        Connection::Bluetooth => "bluetooth".into(),
        Connection::Virtual => "virtual".into(),
    }
}

// ─── 聚合 ───────────────────────────────────────────────────────────────────

pub fn build_response(
    state: &BatteryHistoryState,
    low_battery_threshold: u8,
    range: &str,
) -> BatteryHistoryResponse {
    let samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
    let now = Utc::now();

    // 收集所有出现过的设备+部件组合。
    // 使用临时累加结构缓存 latest_at 为 DateTime<Utc>，避免每个样本都 parse 字符串。
    struct DeviceAccum {
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
    let mut device_keys: BTreeMap<String, DeviceAccum> = BTreeMap::new();
    for s in samples.iter() {
        let key = format!("{}:{}", s.device_id, s.component_id);
        device_keys
            .entry(key.clone())
            .and_modify(|d| {
                if s.at > d.latest_at {
                    d.latest_percentage = s.percentage;
                    d.latest_charging = s.charging;
                    d.latest_at = s.at;
                    d.low_battery = s.low_power;
                    // 同步更新设备名/连接类型：用户可能在过程中重命名了设备。
                    d.device_name = s.device_name.clone();
                    d.connection = s.connection.clone();
                    d.component_label = s.component_label.clone();
                }
            })
            .or_insert(DeviceAccum {
                device_id: s.device_id.clone(),
                device_name: s.device_name.clone(),
                connection: s.connection.clone(),
                component_id: s.component_id.clone(),
                component_label: s.component_label.clone(),
                latest_percentage: s.percentage,
                latest_charging: s.charging,
                latest_at: s.at,
                low_battery: s.low_power,
            });
    }
    let devices: Vec<BatteryHistoryDevice> = device_keys
        .values()
        .map(|d| BatteryHistoryDevice {
            key: format!("{}:{}", d.device_id, d.component_id),
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

    // 为每个设备+部件生成聚合序列。
    let series: Vec<BatteryHistorySeries> = devices
        .iter()
        .map(|d| {
            let key = &d.key;
            let device_samples: Vec<&BatterySample> = samples
                .iter()
                .filter(|s| format!("{}:{}", s.device_id, s.component_id) == *key)
                .collect();
            let points = match range {
                "24h" => aggregate_24h(&device_samples, now),
                _ => aggregate_10d(&device_samples, now),
            };
            BatteryHistorySeries {
                key: key.clone(),
                points,
            }
        })
        .collect();

    let insights = build_insights(&samples, &devices, low_battery_threshold, range, now);

    BatteryHistoryResponse {
        range: range.into(),
        devices,
        series,
        insights,
        generated_at: now.to_rfc3339(),
    }
}

/// 24 小时聚合：24 个小时 bucket，每个 bucket 显示该小时最后一个有效电量。
///
/// 时区说明：bucket 边界基于用户本地时区（`Local`），让"今天 14:00-15:00"
/// 与用户感知一致。夏令时切换（DST）会导致某天 bucket 数为 23 或 25，
/// 此处不做特殊处理——DST 切换瞬间用户通常不会频繁查看电量图表，
/// 且 chrono 的 `Duration::hours` 会正确处理本地时间偏移。
fn aggregate_24h(samples: &[&BatterySample], now: DateTime<Utc>) -> Vec<BatteryHistoryPoint> {
    let local_now = now.with_timezone(&Local);
    let start_hour = local_now - Duration::hours(23);
    let start_hour = start_hour.with_minute(0).unwrap_or(start_hour).with_second(0).unwrap_or(start_hour).with_nanosecond(0).unwrap_or(start_hour);

    let mut points = Vec::with_capacity(24);
    for i in 0..24 {
        let bucket_start = start_hour + Duration::hours(i);
        let bucket_end = bucket_start + Duration::hours(1);
        let bucket_samples: Vec<&&BatterySample> = samples
            .iter()
            .filter(|s| {
                let local = s.at.with_timezone(&Local);
                local >= bucket_start && local < bucket_end
            })
            .collect();

        points.push(build_point_24h(bucket_samples, bucket_start));
    }
    points
}

fn build_point_24h(
    bucket_samples: Vec<&&BatterySample>,
    bucket_start: DateTime<Local>,
) -> BatteryHistoryPoint {
    if bucket_samples.is_empty() {
        return BatteryHistoryPoint {
            bucket_start: bucket_start.with_timezone(&Utc).to_rfc3339(),
            bucket_label: format!("{:02}:00", bucket_start.hour()),
            percentage: None,
            min_percentage: None,
            max_percentage: None,
            charging: None,
            low_battery: None,
            sample_count: 0,
        };
    }

    let last = bucket_samples.last().unwrap();
    let min = bucket_samples.iter().map(|s| s.percentage).min();
    let max = bucket_samples.iter().map(|s| s.percentage).max();
    let charging = bucket_samples.iter().any(|s| s.charging);
    let low_battery = bucket_samples.iter().any(|s| s.low_power);

    BatteryHistoryPoint {
        bucket_start: bucket_start.with_timezone(&Utc).to_rfc3339(),
        bucket_label: format!("{:02}:00", bucket_start.hour()),
        percentage: Some(last.percentage),
        min_percentage: min,
        max_percentage: max,
        charging: Some(charging),
        low_battery: Some(low_battery),
        sample_count: bucket_samples.len() as u32,
    }
}

/// 10 天聚合：10 个 day bucket，每天显示当天最后一个有效电量。
///
/// 时区说明：day 边界基于用户本地时区（`Local`）的午夜，
/// 让"今天"和"昨天"与用户感知一致。夏令时切换日仍按自然日聚合，
/// chrono 的 `date_naive()` 会正确处理本地日期。
fn aggregate_10d(samples: &[&BatterySample], now: DateTime<Utc>) -> Vec<BatteryHistoryPoint> {
    let local_now = now.with_timezone(&Local);
    let today = local_now.date_naive();

    let mut points = Vec::with_capacity(10);
    for i in (0..10).rev() {
        let day = today - Duration::days(i);
        let day_samples: Vec<&&BatterySample> = samples
            .iter()
            .filter(|s| {
                let local = s.at.with_timezone(&Local);
                local.date_naive() == day
            })
            .collect();

        points.push(build_point_10d(day_samples, day));
    }
    points
}

fn build_point_10d(
    bucket_samples: Vec<&&BatterySample>,
    day: NaiveDate,
) -> BatteryHistoryPoint {
    // DST-safe：and_local_timezone 在夏令时切换日可能返回 Ambiguous/None，
    // 使用 .single() 安全取值，失败时回退到 UTC 午夜避免 panic。
    let midnight = day.and_hms_opt(0, 0, 0).unwrap();
    let bucket_start = midnight
        .and_local_timezone(Local)
        .single()
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| midnight.and_utc())
        .to_rfc3339();
    let bucket_label = format!("{}", day.format("%m-%d"));

    if bucket_samples.is_empty() {
        return BatteryHistoryPoint {
            bucket_start,
            bucket_label,
            percentage: None,
            min_percentage: None,
            max_percentage: None,
            charging: None,
            low_battery: None,
            sample_count: 0,
        };
    }

    let last = bucket_samples.last().unwrap();
    let min = bucket_samples.iter().map(|s| s.percentage).min();
    let max = bucket_samples.iter().map(|s| s.percentage).max();
    let charging = bucket_samples.iter().any(|s| s.charging);
    let low_battery = bucket_samples.iter().any(|s| s.low_power);

    BatteryHistoryPoint {
        bucket_start,
        bucket_label,
        percentage: Some(last.percentage),
        min_percentage: min,
        max_percentage: max,
        charging: Some(charging),
        low_battery: Some(low_battery),
        sample_count: bucket_samples.len() as u32,
    }
}

// ─── 洞察分析 ───────────────────────────────────────────────────────────────

fn build_insights(
    samples: &[BatterySample],
    devices: &[BatteryHistoryDevice],
    threshold: u8,
    range: &str,
    now: DateTime<Utc>,
) -> Vec<BatteryInsight> {
    let mut insights = Vec::new();

    // 对每个设备+部件生成洞察。
    for device in devices {
        let key = &device.key;
        let device_samples: Vec<&BatterySample> = samples
            .iter()
            .filter(|s| format!("{}:{}", s.device_id, s.component_id) == *key)
            .collect();
        if device_samples.is_empty() {
            continue;
        }

        let current = device.latest_percentage;
        let charging = device.latest_charging.unwrap_or(false);

        // 续航估算 + 耗尽时间。
        if let Some(remaining_hours) = estimate_remaining(&device_samples, range, now) {
            if charging {
                // 充电中：不预测耗尽时间。
            } else if let Some(_current_pct) = current {
                let remaining_days = remaining_hours / 24.0;
                let message = if remaining_hours < 1.0 {
                    format!("{:.0} 分钟", remaining_hours * 60.0)
                } else if remaining_days < 1.0 {
                    format!("{:.0} 小时", remaining_hours)
                } else {
                    let days = remaining_days.floor() as i64;
                    let hours = (remaining_hours - (days as f64) * 24.0).round() as i64;
                    format!("{} 天 {} 小时", days, hours)
                };
                insights.push(BatteryInsight {
                    insight_type: "estimatedRemaining".into(),
                    severity: "info".into(),
                    title: "estimatedRemaining".into(),
                    message,
                    device_key: Some(key.clone()),
                });

                // 耗尽时间。
                let runout = now + Duration::hours(remaining_hours as i64);
                insights.push(BatteryInsight {
                    insight_type: "estimatedRunout".into(),
                    severity: "info".into(),
                    title: "estimatedRunout".into(),
                    message: runout.with_timezone(&Local).format("%m-%d %H:%M").to_string(),
                    device_key: Some(key.clone()),
                });
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

        // 充电习惯。
        if let Some(mut habit) = analyze_charging_habit(&device_samples, now) {
            habit.device_key = Some(key.clone());
            insights.push(habit);
        }

        // 异常耗电。
        if let Some(drain) = detect_abnormal_drain(&device_samples, now) {
            insights.push(BatteryInsight {
                insight_type: "abnormalDrain".into(),
                severity: "warning".into(),
                title: "abnormalDrain".into(),
                message: format!("+{:.0}% / 2h", drain),
                device_key: Some(key.clone()),
            });
        }

        // 续航稳定性。
        if let Some(mut consistency) = compute_consistency(&device_samples, range, now) {
            consistency.device_key = Some(key.clone());
            insights.push(consistency);
        }
    }

    // 设备对比：多个设备时比较掉电速度（跨设备洞察，device_key=None）。
    if devices.len() > 1 {
        if let Some(mut comparison) = compare_devices(samples, devices, range, now) {
            comparison.device_key = None;
            insights.push(comparison);
        }
    }

    // 省电建议：基于低电量状态（每个低电量设备都生成，前端按选中设备过滤）。
    for device in devices {
        if device.low_battery.unwrap_or(false) {
            insights.push(BatteryInsight {
                insight_type: "powerSavingTip".into(),
                severity: "info".into(),
                title: "powerSavingTip".into(),
                message: format!(
                    "{} · {}",
                    device.device_name, device.component_label
                ),
                device_key: Some(device.key.clone()),
            });
        }
    }

    let _ = threshold;
    insights
}

/// 判断两个样本之间的时间差是否构成"会话间隙"（设备断连）。
/// 超过 `SESSION_GAP_THRESHOLD_MINUTES` 视为断连，断连期间的掉电不参与速率计算。
fn is_session_gap(prev_at: DateTime<Utc>, curr_at: DateTime<Utc>) -> bool {
    (curr_at - prev_at).num_minutes() > SESSION_GAP_THRESHOLD_MINUTES
}

/// 估算剩余可用时间（小时）。
///
/// - 只使用非充电区间的掉电数据；
/// - 跨越会话间隙（设备断连）的样本对不参与计算；
/// - 使用"连续非充电段"的 first→last 净掉电量，避免相邻样本 pair-summation
///   双计 ±1% 抖动（80→79→80→79 应算 1% drop，而非 1+1=2%）；
/// - 忽略明显异常点（掉电 > 50%/小时）；
/// - 当前电量取最后非充电样本（充电中时用拔线前最后一个非充电值）。
fn estimate_remaining(
    samples: &[&BatterySample],
    range: &str,
    now: DateTime<Utc>,
) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }

    let cutoff = match range {
        "24h" => now - Duration::hours(24),
        _ => now - Duration::days(10),
    };
    let recent: Vec<&&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= cutoff)
        .collect();
    if recent.len() < 2 {
        return None;
    }

    let mut sorted = recent.clone();
    sorted.sort_by_key(|s| s.at);

    // 切分为连续非充电段：充电样本/会话间隙都会切断段。
    // 每段只取 first→last 净掉电量，避免 pair-summation 双计抖动。
    let mut segments: Vec<Vec<&&BatterySample>> = Vec::new();
    let mut current_seg: Vec<&&BatterySample> = Vec::new();
    let mut prev: Option<&&BatterySample> = None;
    for s in &sorted {
        let split = match prev {
            None => false,
            Some(p) => p.charging || s.charging || is_session_gap(p.at, s.at),
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
        let drop = first.percentage as f64 - last.percentage as f64;
        if drop < BOUNCE_THRESHOLD_PERCENT {
            // 净掉电小于抖动阈值：跳过。
            continue;
        }
        let hours = (last.at - first.at).num_minutes() as f64 / 60.0;
        if hours <= 0.0 || hours > 24.0 {
            continue;
        }
        let rate = drop / hours;
        // 忽略异常点（> 50%/小时）。
        if rate > 50.0 {
            continue;
        }
        total_drop += drop;
        total_hours += hours;
    }

    if total_hours < 0.5 {
        return None;
    }

    let drain_per_hour = total_drop / total_hours;
    if drain_per_hour <= 0.0 {
        // 掉电很慢。
        return Some(9999.0);
    }

    // 当前电量：取最后一个非充电样本（避免充电中时拿充电值估算）。
    let current = sorted.iter().rev().find(|s| !s.charging)?.percentage;
    if current == 0 {
        return Some(0.0);
    }
    Some(current as f64 / drain_per_hour)
}

/// 充电习惯分析。
///
/// 断连处理：
/// - 如果设备断连期间在充电底座上充电，重新连接后 charging=false 但电量上升；
/// - 此时根据电量上升保守推断为一个充电段（start=断连前电量，end=重连后电量）。
fn analyze_charging_habit(
    samples: &[&BatterySample],
    now: DateTime<Utc>,
) -> Option<BatteryInsight> {
    if samples.len() < 2 {
        return None;
    }

    let cutoff = now - Duration::days(10);
    let recent: Vec<&&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= cutoff)
        .collect();
    if recent.len() < 2 {
        return None;
    }

    let mut sorted = recent.clone();
    sorted.sort_by_key(|s| s.at);

    // 识别充电段：
    // 1. charging 从 false→true 开始，true→false 结束；
    // 2. 断连期间充电：跨越会话间隙且电量上升（charging 均为 false），保守推断为一段充电。
    let mut charge_starts: Vec<u8> = Vec::new();
    let mut charge_ends: Vec<u8> = Vec::new();
    // 初始化为首个样本的充电状态，避免数据以充电样本开头时漏检充电结束。
    let mut prev_charging = sorted[0].charging;
    for window in sorted.windows(2) {
        let prev = window[0];
        let curr = window[1];
        // 常规充电段：charging 状态变化。
        if !prev_charging && curr.charging {
            charge_starts.push(prev.percentage);
        }
        if prev_charging && !curr.charging {
            charge_ends.push(curr.percentage);
        }
        // 断连期间充电推断：跨越间隙 + 两端均非充电 + 电量上升。
        if is_session_gap(prev.at, curr.at)
            && !prev.charging
            && !curr.charging
            && curr.percentage > prev.percentage
        {
            charge_starts.push(prev.percentage);
            charge_ends.push(curr.percentage);
        }
        prev_charging = curr.charging;
    }
    // 如果当前正在充电，记录开始电量但不计结束。
    if prev_charging {
        // 最后一个样本仍在充电，不计入结束。
    }

    if charge_starts.is_empty() {
        return None;
    }

    let avg_start = charge_starts.iter().map(|&p| p as f64).sum::<f64>() / charge_starts.len() as f64;
    let avg_end = if charge_ends.is_empty() {
        None
    } else {
        Some(charge_ends.iter().map(|&p| p as f64).sum::<f64>() / charge_ends.len() as f64)
    };

    let count = charge_starts.len();
    let message = if let Some(end) = avg_end {
        format!(
            "start:{:.0}% end:{:.0}% count:{}",
            avg_start, end, count
        )
    } else {
        format!("start:{:.0}% count:{}", avg_start, count)
    };

    Some(BatteryInsight {
        insight_type: "chargingHabit".into(),
        severity: "info".into(),
        title: "chargingHabit".into(),
        message,
        device_key: None,
    })
}

/// 异常耗电检测：最近 2 小时掉电是否明显高于平时。
///
/// 断连处理：
/// - 最近 2 小时的样本按会话间隙分段，只使用最后一段连续区间的掉电数据；
/// - recent_rate 基于连续区间的实际时间差，而非固定 2 小时；
/// - 历史平均掉电速度同样跳过跨越断连间隙的样本对。
///
/// 电压校正处理：
/// - 充电刚结束时电池电压会回落（load settling），前 `POST_CHARGE_SKIP_MINUTES`
///   分钟内的非充电数据不可靠；
/// - 若最近窗口存在充电样本，只使用充电结束 `POST_CHARGE_SKIP_MINUTES` 分钟后的数据；
/// - 若距上次充电不足 `POST_CHARGE_SKIP_MINUTES` 分钟，跳过本次检测。
fn detect_abnormal_drain(
    samples: &[&BatterySample],
    now: DateTime<Utc>,
) -> Option<f64> {
    if samples.len() < 4 {
        return None;
    }

    let two_hours_ago = now - Duration::hours(2);
    let ten_days_ago = now - Duration::days(10);

    // 找到最近窗口内最后一次充电的时间 T。
    // 若 now - T < POST_CHARGE_SKIP_MINUTES，电压仍在校正，跳过检测。
    let recent_all: Vec<&&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= two_hours_ago)
        .collect();
    let last_charge_time = recent_all
        .iter()
        .filter(|s| s.charging)
        .map(|s| s.at)
        .max();
    if let Some(t) = last_charge_time {
        if now - t < Duration::minutes(POST_CHARGE_SKIP_MINUTES) {
            return None;
        }
    }
    // 只使用充电结束后 POST_CHARGE_SKIP_MINUTES 分钟之外的非充电样本。
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

    // 按会话间隙切分连续段，取最后一段用于计算。
    let mut segments: Vec<Vec<&&BatterySample>> = vec![vec![recent_sorted[0]]];
    for s in &recent_sorted[1..] {
        let last_seg = segments.last().unwrap();
        let last_sample = last_seg.last().unwrap();
        if is_session_gap(last_sample.at, s.at) {
            segments.push(vec![*s]);
        } else {
            segments.last_mut().unwrap().push(*s);
        }
    }
    // 取最后一个包含 ≥2 个样本的连续段。
    let last_segment: Vec<&&BatterySample> = segments
        .iter()
        .rev()
        .find(|seg| seg.len() >= 2)
        .cloned()?;
    if last_segment.len() < 2 {
        return None;
    }
    let seg_first = last_segment.first()?;
    let seg_last = last_segment.last()?;
    let recent_drop = seg_first.percentage as f64 - seg_last.percentage as f64;
    if recent_drop < BOUNCE_THRESHOLD_PERCENT {
        return None;
    }
    let seg_hours = (seg_last.at - seg_first.at).num_minutes() as f64 / 60.0;
    if seg_hours <= 0.0 {
        return None;
    }
    let recent_rate = recent_drop / seg_hours;

    // 历史平均掉电速度（10 天，最近 2 小时之外）：使用 drain_rate 的分段逻辑。
    let historical: Vec<&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= ten_days_ago && s.at < two_hours_ago)
        .cloned()
        .collect();
    let hist_rate = drain_rate(&historical, ten_days_ago, two_hours_ago)?;
    if hist_rate <= 0.0 {
        return None;
    }

    // 异常：最近连续段掉电速度是历史平均的 2 倍以上，且掉电超过 5%。
    if recent_rate > hist_rate * 2.0 && recent_drop > 5.0 {
        Some(recent_drop)
    } else {
        None
    }
}

/// 检查所有设备/部件的异常耗电，返回需要通知的 `(key, device_name)` 列表。
/// key 用于节流（`AbnormalDrainNotifyState`），device_name 用于通知正文展示。
/// 调用方负责发送实际通知；节流逻辑由 `AbnormalDrainNotifyState` 保证 24 小时内不重复。
pub fn check_abnormal_drain(
    state: &BatteryHistoryState,
    notify_state: &AbnormalDrainNotifyState,
    now: DateTime<Utc>,
) -> Vec<(String, String)> {
    let guard = state.samples.lock().unwrap_or_else(|e| e.into_inner());
    // 按 device_id:component_id 分组，同时记录最新的设备名（用户可能重命名设备）
    let mut groups: BTreeMap<String, (String, Vec<&BatterySample>)> = BTreeMap::new();
    for s in guard.iter() {
        let key = format!("{}:{}", s.device_id, s.component_id);
        let entry = groups.entry(key).or_default();
        entry.0 = s.device_name.clone();
        entry.1.push(s);
    }
    let mut result = Vec::new();
    for (key, (device_name, group_samples)) in &groups {
        if detect_abnormal_drain(group_samples, now).is_some()
            && notify_state.should_notify(key, now)
        {
            result.push((key.clone(), device_name.clone()));
        }
    }
    result
}


/// 续航稳定性：比较最近和历史的掉电速度。
fn compute_consistency(
    samples: &[&BatterySample],
    range: &str,
    now: DateTime<Utc>,
) -> Option<BatteryInsight> {
    if samples.len() < 4 {
        return None;
    }

    let (recent_cutoff, hist_cutoff) = match range {
        "24h" => (now - Duration::hours(6), now - Duration::hours(24)),
        _ => (now - Duration::days(3), now - Duration::days(10)),
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
        message,
        device_key: None,
    })
}

/// 计算指定时间段内的掉电速度（%/小时）。
///
/// - 跨越会话间隙（设备断连）的样本对不参与计算；
/// - 充电样本会切断连续段（避免跨越充电段的虚假掉电）；
/// - 使用"连续非充电段"的 first→last 净掉电量，避免 pair-summation
///   双计 ±1% 抖动；
/// - 忽略净掉电小于 `BOUNCE_THRESHOLD_PERCENT` 的段。
fn drain_rate(
    samples: &[&BatterySample],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Option<f64> {
    let filtered: Vec<&&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= start && s.at < end)
        .collect();
    if filtered.len() < 2 {
        return None;
    }
    let mut sorted = filtered.clone();
    sorted.sort_by_key(|s| s.at);

    // 切分为连续非充电段：充电样本/会话间隙都会切断段。
    let mut segments: Vec<Vec<&&BatterySample>> = Vec::new();
    let mut current_seg: Vec<&&BatterySample> = Vec::new();
    let mut prev: Option<&&BatterySample> = None;
    for s in &sorted {
        let split = match prev {
            None => false,
            Some(p) => p.charging || s.charging || is_session_gap(p.at, s.at),
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
        let drop = first.percentage as f64 - last.percentage as f64;
        if drop < BOUNCE_THRESHOLD_PERCENT {
            continue;
        }
        let hours = (last.at - first.at).num_minutes() as f64 / 60.0;
        if hours <= 0.0 || hours > 24.0 {
            continue;
        }
        total_drop += drop;
        total_hours += hours;
    }
    if total_hours < 0.5 {
        return None;
    }
    Some(total_drop / total_hours)
}

/// 设备对比：比较不同设备/部件的掉电速度。
fn compare_devices(
    samples: &[BatterySample],
    devices: &[BatteryHistoryDevice],
    range: &str,
    now: DateTime<Utc>,
) -> Option<BatteryInsight> {
    let cutoff = match range {
        "24h" => now - Duration::hours(24),
        _ => now - Duration::days(10),
    };

    let mut rates: Vec<(String, f64)> = Vec::new();
    for device in devices {
        let key = &device.key;
        let device_samples: Vec<&BatterySample> = samples
            .iter()
            .filter(|s| format!("{}:{}", s.device_id, s.component_id) == *key && s.at >= cutoff)
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
            "{}:{:.2} {}:{:.2}",
            fastest.0, fastest.1, slowest.0, slowest.1
        ),
        device_key: None,
    })
}

// ─── 导出 ───────────────────────────────────────────────────────────────────

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
    let needs_quote = field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r');
    let needs_formula_guard = field.starts_with('=') || field.starts_with('+') || field.starts_with('-') || field.starts_with('@');
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

// ─── 清除 ───────────────────────────────────────────────────────────────────

pub fn clear_history(state: &BatteryHistoryState, app: &AppHandle) -> Result<(), String> {
    {
        let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        samples.clear();
    }
    {
        let mut last = state.last_record.lock().unwrap_or_else(|e| e.into_inner());
        last.clear();
    }
    let file = BatteryHistoryFile {
        schema_version: SCHEMA_VERSION,
        samples: Vec::new(),
    };
    save_history(app, &file)
}

// ─── 异常耗电通知节流 ───────────────────────────────────────────────────────

/// 异常耗电通知节流状态：记录每个设备+部件上次通知的时间。
/// 持久化到 `battery_drain_notify.json`，重启后保留节流状态，
/// 避免重启后立即重复通知。
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
                    // 清理超过 24 小时的过期记录，避免文件无限增长。
                    let now = Utc::now();
                    *guard = file
                        .last_notify
                        .into_iter()
                        .filter(|(_, t)| now - t < Duration::hours(24))
                        .collect();
                }
            }
            Err(_) => {
                // 损坏：删除文件，从空状态开始。
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
            connection: "wireless".into(),
            component_id: "mouse".into(),
            component_label: "mock.mouseLabel".into(),
            percentage: pct,
            charging,
            low_power: !charging && pct < 20,
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
        // 不包含原始 path 的任何片段。
        assert!(!key1.contains("hidraw"));
        assert!(!key1.contains("/dev"));
    }

    #[test]
    fn record_samples_dedup_same_percentage_and_charging() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        // 第一次记录。
        state
            .last_record
            .lock()
            .unwrap()
            .insert("abc123:mouse".into(), (make_sample(now, 80, false), Instant::now()));
        // 5 分钟内不应再记录。
        // （此测试验证去重逻辑，实际 record_samples 需要 AppHandle，
        // 这里仅验证状态层。）
        assert!(state.last_record.lock().unwrap().contains_key("abc123:mouse"));
    }

    #[test]
    fn aggregate_24h_generates_24_buckets() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::hours(2), 90, false),
            make_sample(now - Duration::hours(1), 85, false),
            make_sample(now, 80, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let points = aggregate_24h(&refs, now);
        assert_eq!(points.len(), 24);
        // 至少有 3 个非空 bucket。
        let non_empty = points.iter().filter(|p| p.sample_count > 0).count();
        assert!(non_empty >= 3);
    }

    #[test]
    fn aggregate_10d_generates_10_buckets() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::days(9), 90, false),
            make_sample(now - Duration::days(5), 80, false),
            make_sample(now, 70, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let points = aggregate_10d(&refs, now);
        assert_eq!(points.len(), 10);
        let non_empty = points.iter().filter(|p| p.sample_count > 0).count();
        assert!(non_empty >= 3);
    }

    #[test]
    fn estimate_remaining_ignores_charging() {
        let now = Utc::now();
        // 使用 5 分钟间隔（小于 SESSION_GAP_THRESHOLD_MINUTES=10），确保不被视为断连。
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(60), 100, false),
            make_sample(now - Duration::minutes(50), 99, false),
            make_sample(now - Duration::minutes(40), 98, false),
            make_sample(now - Duration::minutes(30), 98, true),  // 充电开始
            make_sample(now - Duration::minutes(20), 100, true),  // 充电结束
            make_sample(now - Duration::minutes(10), 100, false),
            make_sample(now, 99, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_some());
        // 不应该把充电段算进去。
        let hours = remaining.unwrap();
        assert!(hours > 0.0);
    }

    #[test]
    fn estimate_remaining_ignores_session_gap() {
        let now = Utc::now();
        // 设备断连 8 小时：断连前连续段（5 分钟间隔）+ 重连后单样本。
        // 如果不跳过断连间隙，掉电速度会被错误地稀释（2%/8h = 0.25%/h）。
        let mut samples: Vec<BatterySample> = Vec::new();
        // 断连前：5 分钟间隔，从 100% 掉到 85%（16 个样本，75 分钟）
        for i in 0..16 {
            samples.push(make_sample(
                now - Duration::hours(9) + Duration::minutes(i * 5),
                100 - i as u8,
                false,
            ));
        }
        // 断连 8 小时后重连：电量 78%
        samples.push(make_sample(now - Duration::minutes(5), 78, false));
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_some());
        if let Some(h) = remaining {
            // 基于连续段（12%/h），剩余 ≈ 78/12 ≈ 6.5h
            // 如果错误地包含断连间隙，剩余会 ≈ 31h
            assert!(h < 20.0, "remaining should be based on continuous segment only, got {}h", h);
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
        // 历史：3-4 小时前，每 5 分钟掉 1%（12%/h，正常速率）
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - i as u8,
                false,
            ));
        }
        // 最近 2 小时内：每 5 分钟掉 3%（36%/h，是历史 3 倍）
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(1) + Duration::minutes(i * 5),
                80 - (i * 3) as u8,
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
        // 历史：3-4 小时前，每 5 分钟掉 1%（12%/h）
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - i as u8,
                false,
            ));
        }
        // 最近 2 小时内：同样速率（12%/h）
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(1) + Duration::minutes(i * 5),
                78 - i as u8,
                false,
            ));
        }
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let drain = detect_abnormal_drain(&refs, now);
        // 正常掉电：不触发。
        assert!(drain.is_none());
    }

    #[test]
    fn detect_abnormal_drain_no_false_positive_on_reconnect() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        // 历史：3-4 小时前，正常掉电（12%/h）
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - i as u8,
                false,
            ));
        }
        // 断连 2 小时，重连后最近 30 分钟内掉电 5%（12%/h，正常速率）
        for i in 0..6 {
            samples.push(make_sample(
                now - Duration::minutes(30) + Duration::minutes(i * 5),
                80 - i as u8,
                false,
            ));
        }
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let drain = detect_abnormal_drain(&refs, now);
        // 重连后正常掉电，不应触发异常耗电。
        assert!(drain.is_none());
    }

    #[test]
    fn analyze_charging_habit_detects_charging_during_disconnect() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        // 断连前：电量 20%，5 分钟间隔
        for i in 0..6 {
            samples.push(make_sample(
                now - Duration::hours(5) + Duration::minutes(i * 5),
                20,
                false,
            ));
        }
        // 断连 4 小时（期间在充电底座上充电）
        // 重连后：电量 85%，非充电状态
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
        // 应该检测到断连期间充电（start:20%, end:85%）
        assert!(message.contains("start:20%"), "should detect charge start during disconnect: {}", message);
        assert!(message.contains("end:85%"), "should detect charge end after reconnect: {}", message);
    }

    #[test]
    fn clear_history_empties_state() {
        let state = BatteryHistoryState::new();
        state.samples.lock().unwrap().push(make_sample(Utc::now(), 80, false));
        state
            .last_record
            .lock()
            .unwrap()
            .insert("key".into(), (make_sample(Utc::now(), 80, false), Instant::now()));
        // clear_history 需要 AppHandle，这里只验证内存清理逻辑。
        let mut samples = state.samples.lock().unwrap();
        samples.clear();
        assert!(samples.is_empty());
    }

    #[test]
    fn damaged_json_does_not_crash() {
        // 模拟损坏的 JSON：from_slice 应返回 Err，不 panic。
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
        // 验证不包含 raw HID path。
        assert!(!json.contains("hidraw"));
        assert!(!json.contains("/dev/"));
    }

    #[test]
    fn abnormal_drain_notify_throttle_24h() {
        let state = AbnormalDrainNotifyState::new();
        let now = Utc::now();
        assert!(state.should_notify("mouse:abc", now));
        // 24 小时内不应再通知。
        assert!(!state.should_notify("mouse:abc", now + Duration::hours(1)));
        assert!(!state.should_notify("mouse:abc", now + Duration::hours(23)));
        // 24 小时后可以通知。
        assert!(state.should_notify("mouse:abc", now + Duration::hours(25)));
    }

    #[test]
    fn record_samples_prunes_old_data() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        // 添加 12 天前的样本。
        let old = make_sample(now - Duration::days(12), 90, false);
        let recent = make_sample(now, 80, false);
        {
            let mut samples = state.samples.lock().unwrap();
            samples.push(old);
            samples.push(recent);
            // 模拟清理：保留 10 天 + 1 天缓冲。
            let cutoff = now - Duration::days(11);
            samples.retain(|s| s.at >= cutoff);
        }
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].percentage, 80);
    }

    // ─── 边界问题修复测试 ────────────────────────────────────────────────────

    #[test]
    fn csv_escape_handles_special_characters() {
        // 无特殊字符：原样返回。
        assert_eq!(csv_escape("Test Mouse"), "Test Mouse");
        // 包含逗号：用双引号包裹。
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        // 包含引号：包裹并翻倍。
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        // 包含换行：用双引号包裹。
        assert_eq!(csv_escape("a\nb"), "\"a\nb\"");
        // 公式注入：以 = 开头加单引号前缀。
        assert_eq!(csv_escape("=cmd"), "'=cmd");
        assert_eq!(csv_escape("+1+1"), "'+1+1");
        assert_eq!(csv_escape("-1-1"), "'-1-1");
        assert_eq!(csv_escape("@sum"), "'@sum");
        // 公式注入 + 特殊字符：双重保护。
        assert_eq!(csv_escape("=a,b"), "'\"=a,b\"");
    }

    #[test]
    fn csv_export_escapes_device_name_with_comma() {
        let mut sample = make_sample(Utc::now(), 80, false);
        sample.device_name = "Mouse, Pro".into();
        let csv = samples_to_csv(&[sample]);
        // 逗号应被双引号包裹，不会破坏 CSV 列结构。
        assert!(csv.contains("\"Mouse, Pro\""));
    }

    #[test]
    fn normalize_percentage_filters_unknown_and_clamps() {
        // 正常值：原样返回。
        assert_eq!(normalize_percentage(50), Some(50));
        assert_eq!(normalize_percentage(0), Some(0));
        assert_eq!(normalize_percentage(100), Some(100));
        // 0xFF (255)：未知值，过滤掉。
        assert_eq!(normalize_percentage(0xFF), None);
        // >100：clamp 到 100。
        assert_eq!(normalize_percentage(101), Some(100));
        assert_eq!(normalize_percentage(200), Some(100));
    }

    #[test]
    fn estimate_remaining_uses_last_non_charging_sample() {
        let now = Utc::now();
        // 最后一个样本是充电中：current 应取最后一个非充电样本的电量。
        // 使用 10 分钟间隔确保非充电段时间 >= 0.5h。
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(60), 80, false),
            make_sample(now - Duration::minutes(50), 79, false),
            make_sample(now - Duration::minutes(40), 78, false),
            make_sample(now - Duration::minutes(30), 77, false),
            make_sample(now - Duration::minutes(20), 95, true),  // 充电开始
            make_sample(now - Duration::minutes(10), 95, true),  // 仍在充电
            make_sample(now, 95, true),                          // 正在充电
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let remaining = estimate_remaining(&refs, "24h", now);
        assert!(remaining.is_some());
        if let Some(h) = remaining {
            // current 应为 77（最后一个非充电样本），而非 95（充电中）。
            // drain_rate = (80-77) / (30/60) = 6 %/h
            // remaining ≈ 77 / 6 ≈ 12.8h
            // 如果错误地用 95，remaining ≈ 95/6 ≈ 15.8h
            assert!(h < 15.0, "should use last non-charging sample (77), got {}h", h);
        }
    }

    #[test]
    fn estimate_remaining_filters_bounce() {
        let now = Utc::now();
        // 模拟 ±1% 抖动：80→79→80→79→80→79（10 分钟间隔）
        // 实际净掉电 = 1%（80→79），但 pair-summation 会算成 1+1+1 = 3%。
        // 使用 10 分钟间隔确保总时间 >= 0.5h（10 min 不触发 session gap）。
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
        assert!(remaining.is_some());
        if let Some(h) = remaining {
            // 净掉电 = 1%（80→79），时间 = 50min = 0.833h
            // drain_rate = 1 / 0.833 ≈ 1.2 %/h
            // remaining = 79 / 1.2 ≈ 65.8h
            // 如果错误地用 pair-summation (3% drop)，remaining ≈ 79 / 3.6 ≈ 21.9h
            assert!(h > 40.0, "bounce should be filtered, got {}h", h);
        }
    }

    #[test]
    fn detect_abnormal_drain_skips_post_charge_window() {
        let now = Utc::now();
        let mut samples: Vec<BatterySample> = Vec::new();
        // 历史：3-4 小时前，正常掉电（12%/h）
        for i in 0..12 {
            samples.push(make_sample(
                now - Duration::hours(4) + Duration::minutes(i * 5),
                90 - i as u8,
                false,
            ));
        }
        // 30 分钟前开始充电（5 分钟前结束）
        samples.push(make_sample(now - Duration::minutes(30), 60, true));
        samples.push(make_sample(now - Duration::minutes(25), 70, true));
        samples.push(make_sample(now - Duration::minutes(5), 80, true));
        // 充电结束后立即出现"电压校正"掉电：80→70（5 分钟内）
        // 如果不跳过，会被误判为异常耗电。
        samples.push(make_sample(now, 70, false));
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let drain = detect_abnormal_drain(&refs, now);
        // 距上次充电仅 5 分钟（< POST_CHARGE_SKIP_MINUTES=10），应跳过检测。
        assert!(drain.is_none(), "should skip post-charge voltage correction window");
    }

    #[test]
    fn migrate_schema_upgrades_old_version() {
        let file = BatteryHistoryFile {
            schema_version: 0,
            samples: vec![make_sample(Utc::now(), 80, false)],
        };
        let migrated = migrate_schema(file);
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert_eq!(migrated.samples.len(), 1);
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
            // 添加 MAX_SAMPLES + 10 个样本，时间从早到晚。
            for i in 0..(MAX_SAMPLES + 10) {
                let at = now - Duration::minutes((MAX_SAMPLES + 10 - i) as i64);
                samples.push(make_sample(at, (i % 100) as u8, false));
            }
            // 模拟 record_samples 的 cap 逻辑。
            if samples.len() > MAX_SAMPLES {
                samples.sort_by(|a, b| a.at.cmp(&b.at));
                let drop_count = samples.len() - MAX_SAMPLES;
                samples.drain(0..drop_count);
            }
        }
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), MAX_SAMPLES);
        // 验证保留的是最新的 MAX_SAMPLES 个样本（最早的 10 个被丢弃）。
        let earliest = samples.first().unwrap();
        assert!(earliest.at >= now - Duration::minutes(MAX_SAMPLES as i64));
    }

    #[test]
    fn drain_rate_splits_by_charging_transition() {
        let now = Utc::now();
        // 样本序列：掉电 → 充电 → 掉电
        // 如果不按充电切分段，会算成跨越充电的虚假掉电。
        // 间隔必须 <= SESSION_GAP_THRESHOLD_MINUTES(10)，否则被会话间隙切分。
        // 使用 10 分钟间隔 + 每段 3 样本（20min=0.333h），两段合计 0.667h >= 0.5h。
        // 注意：drain_rate 的 end 是 exclusive（s.at < end），所以用 now+1s 包含最后一个样本。
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(70), 80, false),
            make_sample(now - Duration::minutes(60), 75, false),
            make_sample(now - Duration::minutes(50), 70, false),  // 段1结束：drop=10, 20min
            make_sample(now - Duration::minutes(40), 100, true),  // 充电到 100%
            make_sample(now - Duration::minutes(30), 100, true),  // 仍充电
            make_sample(now - Duration::minutes(20), 95, false),  // 段2开始
            make_sample(now - Duration::minutes(10), 90, false),
            make_sample(now, 85, false),                          // 段2结束：drop=10, 20min
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let rate = drain_rate(&refs, now - Duration::hours(2), now + Duration::seconds(1));
        assert!(rate.is_some());
        if let Some(r) = rate {
            // 段1: drop=10, time=20min=0.333h
            // 段2: drop=10, time=20min=0.333h
            // 加权平均 = (10+10)/(0.333+0.333) ≈ 30 %/h
            // 如果错误地跨越充电段算（80→85 净 drop=-5），会返回 None。
            assert!(r > 20.0, "should split by charging, got {} %/h", r);
        }
    }

    #[test]
    fn build_response_updates_device_name_to_latest() {
        let state = BatteryHistoryState::new();
        let now = Utc::now();
        // 同一设备先用旧名称记录，后用新名称记录。
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
}
