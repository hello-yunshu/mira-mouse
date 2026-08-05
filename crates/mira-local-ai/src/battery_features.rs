// SPDX-License-Identifier: AGPL-3.0-or-later
//! 电量模型特征的唯一权威定义。
//!
//! 本模块是 host、handler 与测试共同引用特征顺序与归一化约定的唯一来源，
//! 禁止在其他路径中再手写一遍 9 个特征的顺序。
//!
//! 特征身份（顺序、名称、归一化公式、单位、范围）由 rill-ml 1.1.0 的
//! `FeatureSchema` / `ModelDescriptor` 描述并生成确定性 hash；模型包中携带
//! 这些 hash 后，加载时不再只依赖 `feature_count == 9` 判断兼容性。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};
use mira_protocol::{BatteryModelConfig, DeviceContextSnapshot};
use rill_ml::descriptor::{
    AlgorithmDescriptor, FeatureConstraint, FeatureDescriptor, FeatureSchema, FeatureSchemaHash,
    ModelDescriptor,
};
use rill_ml::RillError;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// 当前特征 schema 的身份标识。归一化公式或特征顺序发生变化时，必须创建
/// schema v2（新 id + 新 hash），并同步更新模型包。
pub const BATTERY_SCHEMA_ID: &str = "mira-battery-feature-schema-v1";
pub const BATTERY_SCHEMA_VERSION: u32 = 1;

/// DPI 归一化上界。当前主流最高 DPI 约 30000（Razer DeathAdder V3 Pro），
/// 设为 60000（2x）为未来高分辨率传感器预留空间。
pub const MAX_DPI: f64 = 60000.0;
/// 回报率归一化上界。当前主流最高回报率 8000 Hz（Razer Viper 8KHz），
/// 设为 16000 Hz（2x）为未来更高刷新率设备预留空间。
pub const MAX_POLLING_RATE_HZ: f64 = 16000.0;

/// 特征顺序是身份的一部分：改动顺序会改变 schema hash，并导致旧模型包被拒绝。
pub const BATTERY_FEATURE_NAMES: [&str; 9] = [
    "battery_percentage",
    "local_hour_sin",
    "local_hour_cos",
    "weekday_sin",
    "weekday_cos",
    "recent_drain_rate",
    "dpi_normalized",
    "polling_rate_normalized",
    "lighting_intensity",
];

/// 每个特征的语义契约。实际归一化公式以代码实现为准（见 `build_battery_features`），
/// 这里集中记录单位 / 范围 / 缺失策略，防止约定散落。
///
/// battery_percentage     f64；raw 0..=100；normalized 0.0..=1.0（/100）；缺失：拒绝样本
/// local_hour_sin/cos     f64；raw 0..=24h 本地小时角；normalized -1..=1；缺失：拒绝样本
/// weekday_sin/cos        f64；raw 周一=0..=6；normalized -1..=1；缺失：拒绝样本
/// recent_drain_rate      f64；raw 0..=max_drain_per_hour（%h）；normalized /10（不 clamp）；缺失：取 1.0/10=0.1
/// dpi_normalized         f64；raw 100..=100000；normalized clamp(dpi/60000,0,1)；缺失：0.0 + 降低 context quality
/// polling_rate_normalized f64；raw 1..=16000；normalized clamp(polling/16000,0,1)；缺失：0.0 + 降低 context quality
/// lighting_intensity     f64；raw mode∈[0,1] × brightness∈[0,100]；normalized mode×(brightness/100)；缺失：0.0 + 降低 context quality
///
/// 归一化公式的修改必须创建新的 schema 版本，不得原地改动 v1 语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureContextQuality {
    /// DPI、回报率、灯光三项上下文全部可用。
    FullContext,
    /// 至少一项上下文缺失（部分字段为 None）。
    PartialContext,
    /// 完全无上下文（旧 schema 样本或未读取设备参数）。
    NoContext,
}

impl FeatureContextQuality {
    /// 上下文完整度对样本权重的修正系数（Phase 3 使用）。
    pub const fn weight_factor(self) -> f64 {
        match self {
            FeatureContextQuality::FullContext => 1.0,
            FeatureContextQuality::PartialContext => 0.8,
            FeatureContextQuality::NoContext => 0.6,
        }
    }
}

#[derive(Debug, Error)]
pub enum BatteryFeatureError {
    #[error("battery feature schema identity is invalid: {0}")]
    InvalidSchema(#[from] RillError),
    #[error("invalid prediction timezone offset")]
    InvalidTimezone,
}

/// 构造带顺序与归一化描述的 9 特征 schema。顺序、名称、unit、transform 与
/// constraint 全部参与 hash。
pub fn battery_feature_schema() -> Result<FeatureSchema, BatteryFeatureError> {
    let descriptor = |name: &str,
                      unit: &str,
                      transform: &str,
                      min: f64,
                      max: f64|
     -> Result<FeatureDescriptor, RillError> {
        let mut metadata = BTreeMap::new();
        metadata.insert("type".to_owned(), "f64".to_owned());
        metadata.insert("unit".to_owned(), unit.to_owned());
        metadata.insert("formula".to_owned(), transform.to_owned());
        let mut feature = FeatureDescriptor::new(name)?;
        feature.unit = Some(unit.to_owned());
        feature.transform = Some(transform.to_owned());
        feature.constraint = Some(FeatureConstraint {
            min: Some(min),
            max: Some(max),
        });
        feature.metadata = metadata;
        Ok(feature)
    };
    let features = vec![
        descriptor(
            "battery_percentage",
            "percent",
            "percentage / 100",
            0.0,
            100.0,
        )?,
        descriptor(
            "local_hour_sin",
            "radian",
            "sin(hour / 24 * 2pi)",
            -1.0,
            1.0,
        )?,
        descriptor(
            "local_hour_cos",
            "radian",
            "cos(hour / 24 * 2pi)",
            -1.0,
            1.0,
        )?,
        descriptor("weekday_sin", "radian", "sin(weekday / 7 * 2pi)", -1.0, 1.0)?,
        descriptor("weekday_cos", "radian", "cos(weekday / 7 * 2pi)", -1.0, 1.0)?,
        descriptor(
            "recent_drain_rate",
            "percent-per-hour",
            "recent_drain_per_hour / 10",
            0.0,
            f64::MAX,
        )?,
        descriptor(
            "dpi_normalized",
            "ratio",
            "clamp(dpi / 60000, 0, 1)",
            0.0,
            1.0,
        )?,
        descriptor(
            "polling_rate_normalized",
            "ratio",
            "clamp(polling_rate_hz / 16000, 0, 1)",
            0.0,
            1.0,
        )?,
        descriptor(
            "lighting_intensity",
            "ratio",
            "mode_intensity * brightness / 100",
            0.0,
            1.0,
        )?,
    ];
    Ok(FeatureSchema::new(BATTERY_SCHEMA_VERSION, features)?)
}

/// 进程内缓存：schema 身份在运行期不可变，每次观测都重算 hash 会显著拖慢
/// 大批量训练路径（每样本一次 SHA-256），这里只在首次调用时计算一次。
static SCHEMA_HASH_CACHE: OnceLock<FeatureSchemaHash> = OnceLock::new();

/// 当前 schema 的确定性 hash（跨进程稳定）。
pub fn battery_schema_hash() -> Result<FeatureSchemaHash, BatteryFeatureError> {
    if let Some(hash) = SCHEMA_HASH_CACHE.get() {
        return Ok(*hash);
    }
    let hash = battery_feature_schema()?.hash()?;
    let _ = SCHEMA_HASH_CACHE.set(hash);
    Ok(hash)
}

pub fn battery_schema_hash_hex() -> Result<String, BatteryFeatureError> {
    Ok(battery_schema_hash()?.to_hex())
}

/// 模型身份描述：算法 + 特征 schema hash + 配置摘要。
///
/// 配置摘要覆盖影响训练行为的数值参数（不含 schema 身份字段自身），
/// 保证超参数变化会改变 descriptor hash，从而拒绝按旧身份训练的模型状态。
pub fn battery_model_descriptor(
    config: &BatteryModelConfig,
) -> Result<ModelDescriptor, BatteryFeatureError> {
    let algorithm = AlgorithmDescriptor::new("linear-regression-sgd", "1", "1")?;
    let schema_hash = battery_schema_hash()?;
    let configuration_digest = config_digest(config);
    Ok(ModelDescriptor::new(
        algorithm,
        schema_hash,
        Some(configuration_digest),
    )?)
}

/// model descriptor 的确定性字符串身份：对 rill-ml 的 `ModelDescriptor`
/// 做规范 JSON 序列化后取 SHA-256。字段顺序由 serde derive 固定，跨进程稳定。
pub fn battery_model_descriptor_hash_hex(
    config: &BatteryModelConfig,
) -> Result<String, BatteryFeatureError> {
    let descriptor = battery_model_descriptor(config)?;
    let canonical = serde_json::to_string(&descriptor).map_err(|error| {
        BatteryFeatureError::InvalidSchema(RillError::InvalidState(error.to_string()))
    })?;
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(hex_digest(&digest))
}

/// 检查模型包声明的 schema / descriptor 身份与当前实现是否一致。
///
/// 返回：
/// - `Ok(true)`：声明身份与当前实现一致。
/// - `Ok(false)`：声明了身份但全部缺失（legacy 模型包），由调用方决定是否
///   走 legacy 兼容路径。
/// - `Err(SchemaIdentityMismatch)`：声明了身份但与当前实现不一致，必须拒绝该模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaIdentityMismatch {
    pub kind: SchemaMismatchKind,
    pub expected: String,
    pub actual: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaMismatchKind {
    SchemaId,
    SchemaHash,
    ModelDescriptorHash,
}

impl std::fmt::Display for SchemaMismatchKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaMismatchKind::SchemaId => formatter.write_str("schema id"),
            SchemaMismatchKind::SchemaHash => formatter.write_str("schema hash"),
            SchemaMismatchKind::ModelDescriptorHash => formatter.write_str("model descriptor hash"),
        }
    }
}

#[derive(Debug)]
pub enum SchemaIdentity {
    Matched,
    LegacyModelPack,
    Mismatch(SchemaIdentityMismatch),
}

pub fn check_schema_identity(
    config: &BatteryModelConfig,
) -> Result<SchemaIdentity, BatteryFeatureError> {
    let declared_schema_id = config.schema_id.as_deref();
    let declared_schema_hash = config.schema_hash.as_deref();
    let declared_descriptor_hash = config.model_descriptor_hash.as_deref();
    if declared_schema_id.is_none()
        && declared_schema_hash.is_none()
        && declared_descriptor_hash.is_none()
    {
        return Ok(SchemaIdentity::LegacyModelPack);
    }
    if let Some(expected) = declared_schema_id {
        if expected != BATTERY_SCHEMA_ID {
            return Ok(SchemaIdentity::Mismatch(SchemaIdentityMismatch {
                kind: SchemaMismatchKind::SchemaId,
                expected: expected.to_owned(),
                actual: Some(BATTERY_SCHEMA_ID.to_owned()),
            }));
        }
    }
    if let Some(expected) = declared_schema_hash {
        let actual = battery_schema_hash_hex()?;
        if expected != actual {
            return Ok(SchemaIdentity::Mismatch(SchemaIdentityMismatch {
                kind: SchemaMismatchKind::SchemaHash,
                expected: expected.to_owned(),
                actual: Some(actual),
            }));
        }
    }
    if let Some(expected) = declared_descriptor_hash {
        let actual = battery_model_descriptor_hash_hex(config)?;
        if expected != actual {
            return Ok(SchemaIdentity::Mismatch(SchemaIdentityMismatch {
                kind: SchemaMismatchKind::ModelDescriptorHash,
                expected: expected.to_owned(),
                actual: Some(actual),
            }));
        }
    }
    Ok(SchemaIdentity::Matched)
}

/// 将灯光模式名映射为功耗强度评分 \[0, 1\]。
///
/// 不同灯光模式的功耗差异显著：关闭最省电，彩虹/星光等全彩动态模式最耗电。
/// 未知模式默认取中位强度 0.5，避免引入偏差。
fn light_mode_intensity(mode: &str) -> f64 {
    match mode.to_lowercase().as_str() {
        "off" | "disabled" | "none" => 0.0,
        "static" | "fixed" | "solid" => 0.3,
        "breathing" | "breath" => 0.5,
        "reactive" => 0.6,
        "ripple" => 0.7,
        "wave" => 0.8,
        "starlight" => 0.85,
        "rainbow" | "cycle" | "spectrum" => 0.9,
        "custom" => 1.0,
        _ => 0.5,
    }
}

/// 上下文缺失度评估：DPI / 回报率 / 灯光三项中缺失几项。
fn context_quality(context: Option<&DeviceContextSnapshot>) -> FeatureContextQuality {
    match context {
        None => FeatureContextQuality::NoContext,
        Some(ctx) => {
            let present = [
                ctx.dpi.is_some(),
                ctx.polling_rate_hz.is_some(),
                ctx.light_mode.is_some() || ctx.light_brightness.is_some(),
            ]
            .into_iter()
            .filter(|present| *present)
            .count();
            match present {
                3 => FeatureContextQuality::FullContext,
                0 => FeatureContextQuality::NoContext,
                _ => FeatureContextQuality::PartialContext,
            }
        }
    }
}

/// 从 `DeviceContextSnapshot` 提取 3 个归一化特征：DPI、回报率、灯光综合强度。
///
/// 归一化策略：
/// - DPI: `dpi / MAX_DPI`，clamp 到 \[0, 1\]
/// - 回报率: `polling_rate_hz / MAX_POLLING_RATE_HZ`，clamp 到 \[0, 1\]
/// - 灯光强度: `mode_intensity * (brightness / 100.0)`，无亮度时仅用 mode_intensity
///
/// 上下文缺失（旧 schema 样本）时返回 `[0.0, 0.0, 0.0]`，
/// 线性模型中对应权重贡献为 0，等价于不使用该特征，保证向后兼容。
fn context_features(context: Option<&DeviceContextSnapshot>) -> [f64; 3] {
    match context {
        Some(ctx) => {
            let dpi = ctx
                .dpi
                .map(|d| (d as f64 / MAX_DPI).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let polling_rate = ctx
                .polling_rate_hz
                .map(|p| (p as f64 / MAX_POLLING_RATE_HZ).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let light_intensity = ctx
                .light_mode
                .as_ref()
                .map(|mode| {
                    let base = light_mode_intensity(mode);
                    match ctx.light_brightness {
                        Some(b) => base * (b as f64 / 100.0),
                        None => base,
                    }
                })
                .unwrap_or(0.0);
            [dpi, polling_rate, light_intensity]
        }
        None => [0.0, 0.0, 0.0],
    }
}

/// 特征向量：顺序严格对应 `BATTERY_FEATURE_NAMES`，并携带生成它的 schema hash
/// 与上下文完整度。
#[derive(Debug, Clone, PartialEq)]
pub struct BatteryFeatureVector {
    pub values: [f64; 9],
    pub schema_hash: FeatureSchemaHash,
    pub context_quality: FeatureContextQuality,
}

/// 构造 9 维特征向量：6 个基础特征 + 3 个上下文特征。
///
/// 基础特征：电量百分比、时间（sin/cos）、星期（sin/cos）、近期放电率
/// 上下文特征：DPI、回报率、灯光综合强度
pub fn build_battery_features(
    percentage: u8,
    at: DateTime<Utc>,
    timezone_offset_minutes: i32,
    recent_rate: Option<f64>,
    context: Option<&DeviceContextSnapshot>,
) -> Result<BatteryFeatureVector, BatteryFeatureError> {
    let timezone = FixedOffset::east_opt(
        timezone_offset_minutes
            .checked_mul(60)
            .ok_or(BatteryFeatureError::InvalidTimezone)?,
    )
    .ok_or(BatteryFeatureError::InvalidTimezone)?;
    let local = at.with_timezone(&timezone);
    let hour_angle = local.hour() as f64 / 24.0 * std::f64::consts::TAU;
    let weekday_angle = local.weekday().num_days_from_monday() as f64 / 7.0 * std::f64::consts::TAU;
    let [dpi, polling_rate, light_intensity] = context_features(context);
    let values = [
        percentage as f64 / 100.0,
        hour_angle.sin(),
        hour_angle.cos(),
        weekday_angle.sin(),
        weekday_angle.cos(),
        recent_rate.unwrap_or(1.0) / 10.0,
        dpi,
        polling_rate,
        light_intensity,
    ];
    Ok(BatteryFeatureVector {
        values,
        schema_hash: battery_schema_hash()?,
        context_quality: context_quality(context),
    })
}

/// 训练配置摘要：对影响模型行为的数值参数做确定性 sha256。
/// schema 身份字段（schema_id / schema_hash / model_descriptor_hash）本身不参与。
fn config_digest(config: &BatteryModelConfig) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"mira-battery-config-v1\0");
    for value in [
        config.feature_count as f64,
        config.learning_rate,
        config.l2,
        config.huber_delta,
        config.min_training_samples as f64,
        config.min_validation_samples as f64,
        config.quality_window as f64,
        config.required_error_ratio,
        config.max_drain_per_hour,
        config.max_remaining_hours,
        config.session_gap_minutes as f64,
        config.replacement_rise_percent as f64,
        config.min_drop_percent,
        config.baseline_decay_tau_hours,
        config.weighted_learning_enabled as u8 as f64,
        config.learning_recency_tau_hours.unwrap_or(0.0),
    ] {
        hasher.update(value.to_bits().to_be_bytes());
    }
    hasher.finalize().into()
}

fn hex_digest(digest: &[u8]) -> String {
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn config() -> BatteryModelConfig {
        BatteryModelConfig::default()
    }

    #[test]
    fn feature_names_and_count_are_frozen() {
        assert_eq!(BATTERY_FEATURE_NAMES.len(), 9);
        assert_eq!(BATTERY_FEATURE_NAMES[0], "battery_percentage");
        assert_eq!(BATTERY_FEATURE_NAMES[8], "lighting_intensity");
        let schema = battery_feature_schema().unwrap();
        assert_eq!(schema.features.len(), 9);
        for (index, feature) in schema.features.iter().enumerate() {
            assert_eq!(feature.name, BATTERY_FEATURE_NAMES[index]);
        }
    }

    #[test]
    fn feature_order_change_changes_schema_hash() {
        let original = battery_feature_schema().unwrap();
        let mut reordered = original.clone();
        reordered.features.swap(0, 8);
        let original_hash = original.hash().unwrap();
        let reordered_hash = reordered.hash().unwrap();
        assert_ne!(original_hash, reordered_hash);
    }

    #[test]
    fn normalization_description_change_changes_descriptor() {
        let original = battery_feature_schema().unwrap();
        let mut changed = original.clone();
        changed.features[6].transform = Some("clamp(dpi / 80000, 0, 1)".into());
        assert_ne!(original.hash().unwrap(), changed.hash().unwrap());
    }

    #[test]
    fn hash_is_identical_across_processes_and_rebuilds() {
        let first = battery_schema_hash_hex().unwrap();
        let second = battery_schema_hash_hex().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        let golden = include_str!("../tests/fixtures/battery_schema_v1.json");
        let golden_hash = golden.trim();
        assert_eq!(
            first, golden_hash,
            "golden fixture battery_schema_v1.json must match the current schema hash; \
             regenerate it after intentional schema changes"
        );
    }

    #[test]
    fn legacy_config_without_identity_enters_legacy_path() {
        let config = config();
        assert!(config.schema_id.is_none());
        match check_schema_identity(&config).unwrap() {
            SchemaIdentity::LegacyModelPack => {}
            other => panic!("expected LegacyModelPack, got {other:?}"),
        }
    }

    #[test]
    fn matching_identity_is_accepted() {
        let mut config = config();
        config.schema_id = Some(BATTERY_SCHEMA_ID.to_owned());
        config.schema_hash = Some(battery_schema_hash_hex().unwrap());
        config.model_descriptor_hash = Some(battery_model_descriptor_hash_hex(&config).unwrap());
        match check_schema_identity(&config).unwrap() {
            SchemaIdentity::Matched => {}
            other => panic!("expected Matched, got {other:?}"),
        }
    }

    #[test]
    fn wrong_schema_hash_is_rejected() {
        let mut config = config();
        config.schema_hash = Some("0".repeat(64));
        match check_schema_identity(&config).unwrap() {
            SchemaIdentity::Mismatch(mismatch) => {
                assert_eq!(mismatch.kind, SchemaMismatchKind::SchemaHash);
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    #[test]
    fn wrong_schema_id_is_rejected() {
        let mut config = config();
        config.schema_id = Some("mira-battery-feature-schema-v2".to_owned());
        match check_schema_identity(&config).unwrap() {
            SchemaIdentity::Mismatch(mismatch) => {
                assert_eq!(mismatch.kind, SchemaMismatchKind::SchemaId);
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    #[test]
    fn same_feature_count_but_different_order_is_rejected() {
        // feature_count 仍为 9，但 schema hash 不匹配 → 拒绝。
        let mut config = config();
        config.schema_hash = Some("0".repeat(64));
        assert!(matches!(
            check_schema_identity(&config).unwrap(),
            SchemaIdentity::Mismatch(_)
        ));
    }

    #[test]
    fn model_descriptor_changes_with_config_digest() {
        let a = battery_model_descriptor_hash_hex(&config()).unwrap();
        let mut changed = config();
        changed.learning_rate = 0.07;
        let b = battery_model_descriptor_hash_hex(&changed).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn descriptor_json_roundtrips_stably() {
        let descriptor = battery_model_descriptor(&config()).unwrap();
        let json = serde_json::to_string(&descriptor).unwrap();
        let restored: ModelDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, descriptor);
    }

    #[test]
    fn context_quality_detects_missing_fields() {
        let full = DeviceContextSnapshot {
            dpi: Some(800),
            polling_rate_hz: Some(125),
            light_mode: Some("off".into()),
            light_brightness: None,
            profile: None,
        };
        assert_eq!(
            context_quality(Some(&full)),
            FeatureContextQuality::FullContext
        );
        let partial = DeviceContextSnapshot {
            dpi: Some(800),
            polling_rate_hz: None,
            light_mode: None,
            light_brightness: None,
            profile: None,
        };
        assert_eq!(
            context_quality(Some(&partial)),
            FeatureContextQuality::PartialContext
        );
        assert_eq!(context_quality(None), FeatureContextQuality::NoContext);
        assert_eq!(FeatureContextQuality::FullContext.weight_factor(), 1.0);
        assert_eq!(FeatureContextQuality::NoContext.weight_factor(), 0.6);
    }

    #[test]
    fn feature_vector_order_matches_names() {
        let at = Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        let ctx = DeviceContextSnapshot {
            dpi: Some(16000),
            polling_rate_hz: Some(8000),
            light_mode: Some("static".into()),
            light_brightness: Some(50),
            profile: None,
        };
        let vector = build_battery_features(80, at, 0, Some(5.0), Some(&ctx)).unwrap();
        assert_eq!(vector.values.len(), 9);
        assert!((vector.values[0] - 0.8).abs() < 1e-9);
        assert!((vector.values[5] - 0.5).abs() < 1e-9);
        assert!((vector.values[6] - (16000.0 / MAX_DPI)).abs() < 1e-9);
        assert!((vector.values[7] - (8000.0 / MAX_POLLING_RATE_HZ)).abs() < 1e-9);
        assert!((vector.values[8] - 0.15).abs() < 1e-9);
        assert_eq!(vector.schema_hash, battery_schema_hash().unwrap());
        assert_eq!(vector.context_quality, FeatureContextQuality::FullContext);
    }

    #[test]
    fn config_digest_ignores_identity_fields() {
        let mut declared = config();
        declared.schema_id = Some(BATTERY_SCHEMA_ID.to_owned());
        declared.schema_hash = Some("0".repeat(64));
        let with_identity = battery_model_descriptor_hash_hex(&declared).unwrap();
        let mut plain = config();
        plain.schema_id = None;
        plain.schema_hash = None;
        let without_identity = battery_model_descriptor_hash_hex(&plain).unwrap();
        assert_eq!(with_identity, without_identity);
    }
}
