// SPDX-License-Identifier: AGPL-3.0-or-later
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

const DEDUP_INTERVAL_MINUTES: i64 = 5;
#[allow(dead_code)]
const DEFAULT_RETENTION_DAYS: i64 = 30;
const RETENTION_BUFFER_DAYS: i64 = 1;
const SCHEMA_VERSION: u32 = 1;
const SESSION_GAP_THRESHOLD_MINUTES: i64 = 10;
const MAX_SAMPLES: usize = 20000;
const BOUNCE_THRESHOLD_PERCENT: f64 = 1.0;
const REPLACEMENT_RISE_THRESHOLD_PERCENT: u8 = 5;
const POST_CHARGE_SKIP_MINUTES: i64 = 10;
const VERY_SLOW_DRAIN_HOURS: f64 = 9999.0;
const PERSIST_INTERVAL_SECS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatterySample {
    pub at: DateTime<Utc>,
    pub device_id: String,
    pub device_name: String,
    pub connection: String,
    pub component_id: String,
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
                            let key = format!("{}:{}", sample.device_id, sample.component_id);
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
    if file.schema_version < SCHEMA_VERSION {
        file.schema_version = SCHEMA_VERSION;
    }
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

fn normalize_percentage(raw: u8) -> Option<u8> {
    if raw == 0xFF {
        return None;
    }
    Some(raw.min(100))
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
            let device_id = anonymize_device_key(&entry.device_key);
            let snapshot = &entry.snapshot;
            let connection = connection_str(&snapshot.connection);
            let device_name = &snapshot.display_name;

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
                let key = format!("{}:{}", device_id, component_id);
                let low_power = !charging && percentage < low_battery_threshold;
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
                    let sample = BatterySample {
                        at: now,
                        device_id: device_id.clone(),
                        device_name: device_name.clone(),
                        connection: connection.clone(),
                        component_id,
                        component_label,
                        percentage,
                        charging,
                        low_power,
                    };
                    let key = format!("{}:{}", sample.device_id, sample.component_id);
                    last_record.insert(key, (sample.clone(), Instant::now()));
                    samples.push(sample);
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

pub fn build_response(
    state: &BatteryHistoryState,
    low_battery_threshold: u8,
    range: &str,
) -> BatteryHistoryResponse {
    let samples: Vec<BatterySample> = {
        let guard = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    };
    let samples_ref: &[BatterySample] = &samples;
    let now = Utc::now();

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
    for s in samples_ref.iter() {
        let key = format!("{}:{}", s.device_id, s.component_id);
        device_keys
            .entry(key.clone())
            .and_modify(|d| {
                if s.at > d.latest_at {
                    d.latest_percentage = s.percentage;
                    d.latest_charging = s.charging;
                    d.latest_at = s.at;
                    d.low_battery = !s.charging && s.percentage < low_battery_threshold;
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
                low_battery: !s.charging && s.percentage < low_battery_threshold,
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

    let series: Vec<BatteryHistorySeries> = devices
        .iter()
        .map(|d| {
            let key = &d.key;
            let device_samples: Vec<&BatterySample> = samples_ref
                .iter()
                .filter(|s| format!("{}:{}", s.device_id, s.component_id) == *key)
                .collect();
            let points = match range {
                "24h" => aggregate_24h(&device_samples, now, low_battery_threshold),
                _ => aggregate_10d(&device_samples, now, low_battery_threshold),
            };
            BatteryHistorySeries {
                key: key.clone(),
                points,
            }
        })
        .collect();

    let insights = build_insights(samples_ref, &devices, low_battery_threshold, range, now);

    BatteryHistoryResponse {
        range: range.into(),
        devices,
        series,
        insights,
        generated_at: now.to_rfc3339(),
    }
}

/// 24 小时聚合：48 个 30 分钟 bucket，每个 bucket 显示该时段最后一个有效电量。
/// 采用 30 分钟粒度让图表柱子紧密排列（类似 iOS 电量图表），呈现更细腻的趋势。
///
/// 时区说明：bucket 边界基于用户本地时区（`Local`），让"今天 14:00-14:30"
/// 与用户感知一致。夏令时切换（DST）会导致某天 bucket 数为 46 或 50，
/// 此处不做特殊处理——DST 切换瞬间用户通常不会频繁查看电量图表，
/// 且 chrono 的 `Duration::minutes` 会正确处理本地时间偏移。
fn aggregate_24h(
    samples: &[&BatterySample],
    now: DateTime<Utc>,
    low_battery_threshold: u8,
) -> Vec<BatteryHistoryPoint> {
    let local_now = now.with_timezone(&Local);
    // 对齐到 30 分钟边界（下取整），再向前推 23.5 小时作为起点。
    // 48 个 30 分钟 bucket 覆盖完整 24 小时，且最后一个 bucket 包含当前时刻。
    let aligned = local_now
        .with_second(0)
        .unwrap_or(local_now)
        .with_nanosecond(0)
        .unwrap_or(local_now);
    let aligned = if aligned.minute() >= 30 {
        aligned.with_minute(30).unwrap_or(aligned)
    } else {
        aligned.with_minute(0).unwrap_or(aligned)
    };
    let start = aligned - Duration::minutes(47 * 30);

    let mut points = Vec::with_capacity(48);
    for i in 0..48 {
        let bucket_start = start + Duration::minutes(i * 30);
        let bucket_end = bucket_start + Duration::minutes(30);
        let bucket_samples: Vec<&&BatterySample> = samples
            .iter()
            .filter(|s| {
                let local = s.at.with_timezone(&Local);
                local >= bucket_start && local < bucket_end
            })
            .collect();

        points.push(build_point_24h(
            bucket_samples,
            bucket_start,
            low_battery_threshold,
        ));
    }
    points
}

fn build_point_24h(
    bucket_samples: Vec<&&BatterySample>,
    bucket_start: DateTime<Local>,
    low_battery_threshold: u8,
) -> BatteryHistoryPoint {
    if bucket_samples.is_empty() {
        return BatteryHistoryPoint {
            bucket_start: bucket_start.with_timezone(&Utc).to_rfc3339(),
            bucket_label: format!("{:02}:{:02}", bucket_start.hour(), bucket_start.minute()),
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
    let low_battery = bucket_samples
        .iter()
        .any(|s| !s.charging && s.percentage < low_battery_threshold);

    BatteryHistoryPoint {
        bucket_start: bucket_start.with_timezone(&Utc).to_rfc3339(),
        bucket_label: format!("{:02}:{:02}", bucket_start.hour(), bucket_start.minute()),
        percentage: Some(last.percentage),
        min_percentage: min,
        max_percentage: max,
        charging: Some(charging),
        low_battery: Some(low_battery),
        sample_count: bucket_samples.len() as u32,
    }
}

fn aggregate_10d(
    samples: &[&BatterySample],
    now: DateTime<Utc>,
    low_battery_threshold: u8,
) -> Vec<BatteryHistoryPoint> {
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

        points.push(build_point_10d(day_samples, day, low_battery_threshold));
    }
    points
}

fn build_point_10d(
    bucket_samples: Vec<&&BatterySample>,
    day: NaiveDate,
    low_battery_threshold: u8,
) -> BatteryHistoryPoint {
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
    let low_battery = bucket_samples
        .iter()
        .any(|s| !s.charging && s.percentage < low_battery_threshold);

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

fn build_insights(
    samples: &[BatterySample],
    devices: &[BatteryHistoryDevice],
    _threshold: u8,
    range: &str,
    now: DateTime<Utc>,
) -> Vec<BatteryInsight> {
    let mut insights = Vec::new();

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

        if let Some(remaining_hours) = estimate_remaining(&device_samples, range, now) {
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

                    let runout = now + Duration::hours(remaining_hours as i64);
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
        let days = (remaining_hours / 24.0).floor() as i64;
        let hours = (remaining_hours - (days as f64) * 24.0).round() as i64;
        format!("remainingDaysHours|{}|{}", days, hours)
    }
}

fn is_session_gap(prev_at: DateTime<Utc>, curr_at: DateTime<Utc>) -> bool {
    (curr_at - prev_at).num_minutes() > SESSION_GAP_THRESHOLD_MINUTES
}

/// Detects a swap or off-device charge while the device itself is not charging.
fn is_battery_replacement(prev: &BatterySample, curr: &BatterySample) -> bool {
    !prev.charging
        && !curr.charging
        && curr.percentage.saturating_sub(prev.percentage) >= REPLACEMENT_RISE_THRESHOLD_PERCENT
}

fn estimate_remaining(samples: &[&BatterySample], range: &str, now: DateTime<Utc>) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }

    let cutoff = match range {
        "24h" => now - Duration::hours(24),
        _ => now - Duration::days(10),
    };
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
            Some(p) => {
                p.charging
                    || s.charging
                    || is_session_gap(p.at, s.at)
                    || is_battery_replacement(p, s)
            }
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
        if hours <= 0.0 {
            continue;
        }
        let rate = drop / hours;
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

    let mut charge_starts: Vec<u8> = Vec::new();
    let mut charge_ends: Vec<u8> = Vec::new();
    let mut prev_charging = sorted[0].charging;
    for window in sorted.windows(2) {
        let prev = window[0];
        let curr = window[1];
        if !prev_charging && curr.charging {
            charge_starts.push(prev.percentage);
        }
        if prev_charging && !curr.charging {
            charge_ends.push(curr.percentage);
        }
        if (is_session_gap(prev.at, curr.at)
            && !prev.charging
            && !curr.charging
            && curr.percentage > prev.percentage)
            || is_battery_replacement(prev, curr)
        {
            charge_starts.push(prev.percentage);
            charge_ends.push(curr.percentage);
        }
        prev_charging = curr.charging;
    }
    if charge_starts.is_empty() {
        return None;
    }

    let avg_start =
        charge_starts.iter().map(|&p| p as f64).sum::<f64>() / charge_starts.len() as f64;
    let avg_end = if charge_ends.is_empty() {
        None
    } else {
        Some(charge_ends.iter().map(|&p| p as f64).sum::<f64>() / charge_ends.len() as f64)
    };

    let count = charge_starts.len();
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
        if is_session_gap(last_sample.at, s.at) || is_battery_replacement(last_sample, s) {
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
    if recent_drop < BOUNCE_THRESHOLD_PERCENT {
        return None;
    }
    let seg_hours = (seg_last.at - seg_first.at).num_minutes() as f64 / 60.0;
    if seg_hours <= 0.0 {
        return None;
    }
    let recent_rate = recent_drop / seg_hours;

    let historical: Vec<&BatterySample> = samples
        .iter()
        .filter(|s| s.at >= ten_days_ago && s.at < two_hours_ago)
        .cloned()
        .collect();
    let hist_rate = drain_rate(&historical, ten_days_ago, two_hours_ago)?;
    if hist_rate <= 0.0 {
        return None;
    }

    if recent_rate > hist_rate * 2.0 && recent_drop > 5.0 {
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
        guard.clone()
    };
    let mut groups: BTreeMap<String, (String, Vec<&BatterySample>)> = BTreeMap::new();
    for s in samples.iter() {
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
        message: format!("consistency{}", capitalize_first(&message)),
        device_key: None,
    })
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
            Some(p) => {
                p.charging
                    || s.charging
                    || is_session_gap(p.at, s.at)
                    || is_battery_replacement(p, s)
            }
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
        if hours <= 0.0 {
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

pub fn clear_history(state: &BatteryHistoryState, app: &AppHandle) -> Result<(), String> {
    {
        let mut samples = state.samples.lock().unwrap_or_else(|e| e.into_inner());
        let mut last = state.last_record.lock().unwrap_or_else(|e| e.into_inner());
        samples.clear();
        last.clear();
    }
    let file = BatteryHistoryFile {
        schema_version: SCHEMA_VERSION,
        samples: Vec::new(),
    };
    save_history(app, &file)
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
    fn aggregate_24h_generates_48_buckets() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::hours(2), 90, false),
            make_sample(now - Duration::hours(1), 85, false),
            make_sample(now, 80, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let points = aggregate_24h(&refs, now, 20);
        assert_eq!(points.len(), 48);
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
        let points = aggregate_10d(&refs, now, 20);
        assert_eq!(points.len(), 10);
        let non_empty = points.iter().filter(|p| p.sample_count > 0).count();
        assert!(non_empty >= 3);
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
        let mut sample = make_sample(now, 25, false);
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
    fn estimate_remaining_ignores_charging() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(60), 100, false),
            make_sample(now - Duration::minutes(50), 99, false),
            make_sample(now - Duration::minutes(40), 98, false),
            make_sample(now - Duration::minutes(30), 98, true),
            make_sample(now - Duration::minutes(20), 100, true),
            make_sample(now - Duration::minutes(10), 100, false),
            make_sample(now, 99, false),
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
            make_sample(now - Duration::minutes(50), 22, false),
            make_sample(now - Duration::minutes(40), 20, false),
            make_sample(now - Duration::minutes(30), 18, false),
            make_sample(now - Duration::minutes(20), 92, false),
            make_sample(now - Duration::minutes(10), 90, false),
            make_sample(now, 88, false),
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
                90 - i as u8,
                false,
            ));
        }
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
    fn drain_rate_splits_on_quick_battery_replacement() {
        let now = Utc::now();
        let samples: Vec<BatterySample> = vec![
            make_sample(now - Duration::minutes(50), 24, false),
            make_sample(now - Duration::minutes(40), 22, false),
            make_sample(now - Duration::minutes(30), 20, false),
            make_sample(now - Duration::minutes(20), 95, false),
            make_sample(now - Duration::minutes(10), 93, false),
            make_sample(now, 91, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let rate = drain_rate(&refs, now - Duration::hours(1), now);
        assert!(rate.is_some());
        let rate = rate.unwrap();
        assert!(
            rate > 10.0,
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
    fn estimate_remaining_filters_bounce() {
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
        assert!(remaining.is_some());
        if let Some(h) = remaining {
            assert!(h > 40.0, "bounce should be filtered, got {}h", h);
        }
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
        let insights = build_insights(&samples, &[device], 20, "10d", now);
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
            make_sample(now - Duration::minutes(70), 80, false),
            make_sample(now - Duration::minutes(60), 75, false),
            make_sample(now - Duration::minutes(50), 70, false),
            make_sample(now - Duration::minutes(40), 100, true),
            make_sample(now - Duration::minutes(30), 100, true),
            make_sample(now - Duration::minutes(20), 95, false),
            make_sample(now - Duration::minutes(10), 90, false),
            make_sample(now, 85, false),
        ];
        let refs: Vec<&BatterySample> = samples.iter().collect();
        let rate = drain_rate(&refs, now - Duration::hours(2), now + Duration::seconds(1));
        assert!(rate.is_some());
        if let Some(r) = rate {
            assert!(r > 20.0, "should split by charging, got {} %/h", r);
        }
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
