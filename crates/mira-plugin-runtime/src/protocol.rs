// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::engine::ProtocolPackage;
use hidapi::{HidApi, HidDevice};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

/// Feature index 缓存：按设备路径索引，存储 featureId → 完整 parsed output 映射。
/// feature index 在设备连接期间不变，缓存可避免每轮轮询重复查询。
/// 存储 complete Value（而非仅 featureIndex: u8）以保留 deviceIndex、connection 等
/// derived 字段，防止后续 step 引用 `{fromOutput: "device", field: "deviceIndex"}` 时
/// 因缓存命中丢失字段而报 "missing output reference"。
pub type FeatureIndexCache = HashMap<String, HashMap<u16, Value>>;

/// Onboard memory 缓存：按设备路径索引，存储最近一次 onboard read 的 (outputs, bytes)。
/// 写入 mutation 的预读阶段检查缓存，命中则跳过 16 chunk HID 往返。
/// 写入后的验证读更新缓存。设备断开时由调用方清空。
pub type OnboardMemoryCache = HashMap<String, (BTreeMap<String, Value>, Vec<u8>)>;

/// 已打开的 HID 设备句柄缓存，按设备路径索引。
/// `HidDevice` 不可 Clone，采用取用-归还策略：执行前从缓存取出（未命中则 open_path），
/// 执行成功后归还；执行出错时句柄随 session 析构关闭，不归还（设备可能处于异常状态）。
/// `device_io` 锁已序列化 HID 访问，缓存读写仅持有极短时段，无死锁风险。
pub type HidHandleCache = HashMap<String, HidDevice>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HidIoStats {
    pub handle_cache_hits: u64,
    pub handle_cache_misses: u64,
    pub open_path_attempts: u64,
    pub open_path_failures: u64,
    pub handles_returned: u64,
    pub handle_cache_lock_failures: u64,
    pub reports_executed: u64,
}

impl HidIoStats {
    pub fn record_cache_hit(&mut self) {
        self.handle_cache_hits += 1;
    }

    pub fn record_cache_miss(&mut self) {
        self.handle_cache_misses += 1;
        self.open_path_attempts += 1;
    }

    pub fn record_open_failure(&mut self) {
        self.open_path_failures += 1;
    }

    pub fn record_returned(&mut self) {
        self.handles_returned += 1;
    }

    pub fn record_lock_failure(&mut self) {
        self.handle_cache_lock_failures += 1;
    }

    /// 累加一次工作流会话中实际发送的 HID report 数量。
    pub fn record_reports_executed(&mut self, count: usize) {
        self.reports_executed += count as u64;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    Usb,
    Wireless,
    Bluetooth,
}

/// Per-output read status reported by workflow execution.
///
/// Brand-neutral: a step that errors with `on_failure: continue` records
/// `Failed(reason)`; a step skipped via `skip_if_zero` records `Skipped`;
/// a successful step records `Ok`. `NotSupported` is reserved for future
/// probe-driven gating. The host forwards these to `DeviceSnapshot.read_statuses`
/// (as `serde_json::Value`) so the UI can distinguish "missing because
/// unsupported" from "missing because the read failed".
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadStatus {
    /// Step executed and produced a value.
    Ok,
    /// Step skipped via `skip_if_zero` (a referenced output was zero).
    Skipped,
    /// Step targets an output the device does not support.
    NotSupported,
    /// Step errored; `on_failure: continue` kept the workflow running.
    Failed(String),
}

#[derive(Debug, Default, Clone)]
pub struct DeviceReading {
    pub display_name: Option<String>,
    pub connection: Option<ConnectionKind>,
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub batteries: Vec<mira_core::DeviceBattery>,
    pub dpi: Option<u16>,
    pub dpi_stages: Option<Vec<mira_core::DpiStage>>,
    pub polling_rate_hz: Option<u16>,
    pub supported_polling_rates_hz: Option<Vec<u16>>,
    pub profile: Option<u8>,
    pub light_color: Option<String>,
    pub capabilities: BTreeMap<String, Value>,
    /// Per-output read statuses, keyed by workflow output name. Populated from
    /// workflow execution results; serialized into the host snapshot so the UI
    /// can distinguish skipped/failed/unsupported outputs from absent ones.
    pub read_statuses: BTreeMap<String, ReadStatus>,
    /// 接收器场景下鼠标是否就位（基于 receiverIdle.mouseOnline）。
    /// 仅在接收器工作流产出 receiverIdle 时填充；其他场景保持 None。
    pub mouse_ready: Option<bool>,
}

/// HID 交换事件回调：由宿主实现，用于记录协议诊断事件。
///
/// `mira-plugin-runtime` crate 不直接依赖日志系统，通过此 trait 将
/// HID 交换、忙碌重试、响应不匹配、校验失败等事件通知宿主，
/// 由宿主决定是否记录及如何脱敏。
pub trait HidEventSink: Send + Sync {
    /// 一次 HID feature report / output-input / RACE 交换完成。
    ///
    /// - `transport`: 传输名称（如 "razer-hid-feature"）
    /// - `command`: 命令标识（如 "get-dpi"）
    /// - `request_hex`: 请求 payload 的十六进制（不含 report ID）
    /// - `response_hex`: 响应 payload 的十六进制（不含 report ID），空切片表示无响应
    /// - `duration_ms`: 本次交换耗时
    /// - `busy_reads`: 忙碌重试次数（poll_until 场景）
    /// - `checksum_valid`: 校验是否通过（None 表示无校验）
    #[allow(clippy::too_many_arguments)]
    fn on_hid_exchange(
        &self,
        transport: &str,
        command: &str,
        request_hex: &str,
        response_hex: &str,
        duration_ms: u64,
        busy_reads: usize,
        checksum_valid: Option<bool>,
    );

    /// HID 交换遇到忙碌状态并重试。
    fn on_hid_busy_retry(&self, transport: &str, command: &str, attempt: usize);

    /// HID 响应不匹配（transaction ID / command class / command ID 错误）。
    fn on_hid_response_mismatch(
        &self,
        transport: &str,
        command: &str,
        expected: &str,
        actual: &str,
    );

    /// HID 响应校验和失败。
    fn on_hid_checksum_failed(&self, transport: &str, command: &str, expected: &str, actual: &str);
}

/// 空实现：不记录任何事件。供不需要协议诊断的场景使用。
pub struct NullHidEventSink;

impl HidEventSink for NullHidEventSink {
    fn on_hid_exchange(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: u64,
        _: usize,
        _: Option<bool>,
    ) {
    }
    fn on_hid_busy_retry(&self, _: &str, _: &str, _: usize) {}
    fn on_hid_response_mismatch(&self, _: &str, _: &str, _: &str, _: &str) {}
    fn on_hid_checksum_failed(&self, _: &str, _: &str, _: &str, _: &str) {}
}

pub struct ProtocolContext<'a> {
    pub api: &'a HidApi,
    pub path: &'a str,
    pub family: &'a str,
    pub connection: ConnectionKind,
    pub files: &'a BTreeMap<String, Vec<u8>>,
    pub outputs: BTreeMap<String, Value>,
    /// Feature index 缓存（按设备路径索引）。设备连接期间 feature index 不变，
    /// 缓存命中时跳过 root-get-feature 的 HID 往返。设备断开时由调用方清空。
    pub feature_index_cache: Option<&'a Mutex<FeatureIndexCache>>,
    /// Onboard memory 缓存（按设备路径索引）。写入 mutation 预读时命中缓存则跳过
    /// 16 chunk HID 往返；验证读后更新缓存。设备断开时由调用方清空。
    pub onboard_memory_cache: Option<&'a Mutex<OnboardMemoryCache>>,
    /// 已打开的 HID 设备句柄缓存（按设备路径索引）。命中时复用句柄，跳过 open_path
    /// 系统调用；未命中时 open_path 并在执行成功后归还。设备断开时由调用方清空。
    pub cached_handles: Option<&'a Mutex<HidHandleCache>>,
    /// 可选 HID I/O 计数器，用于 debug/诊断：统计句柄缓存命中、open_path 次数、
    /// 归还次数和锁失败。未提供时不产生额外可见行为。
    pub hid_io_stats: Option<&'a Mutex<HidIoStats>>,
    /// 可选 HID 交换事件回调。宿主通过实现 `HidEventSink` trait 接收
    /// HID 交换、忙碌重试、响应不匹配、校验失败等事件，用于协议诊断。
    /// 未提供时使用 `NullHidEventSink`（不记录任何事件）。
    pub hid_event_sink: Option<&'a dyn HidEventSink>,
}

/// 宿主语义字段：品牌无关的设备状态语义。
///
/// UI 通过 `DeviceViewRequirement` 声明当前视图需要的语义字段，
/// 宿主将其映射为目标 output 名称后计算工作流投影。
/// 这层间接让 UI 不与插件原始 output 名称耦合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub enum SemanticField {
    /// 鼠标电量百分比
    BatteryPercent,
    /// 接收器电量百分比
    ReceiverBatteryPercent,
    /// 充电状态
    Charging,
    /// 当前 DPI
    CurrentDpi,
    /// 当前回报率
    PollingRate,
    /// 当前活动配置文件
    ActiveProfile,
    /// 灯光状态
    LightingState,
    /// 清单（inventory）读数：插件声明的 inventory workflow 产出的完整读数。
    /// 仅在用户打开"全部读数"详情视图或显式请求时通过投影读取，不进入后台轮询。
    Inventory,
}

impl SemanticField {
    /// 返回此语义字段对应的标准 output 名称候选列表。
    ///
    /// 这些名称是同一语义的可选规范化来源。映射时会收集当前
    /// workflow 中实际存在的所有候选项：例如 HID++ 标准/扩展回报率
    /// 二选一，只能在运行时知道哪一步真正有效。
    pub fn standard_output_names(&self) -> &'static [&'static str] {
        match self {
            // battery output 包含 percentage 和 charging 字段
            SemanticField::BatteryPercent => &["battery"],
            SemanticField::Charging => &["battery"],
            SemanticField::ReceiverBatteryPercent => {
                &["receiverBattery", "receiver", "receiverIdle"]
            }
            SemanticField::CurrentDpi => &["dpi", "dpiExtended", "dpiStages"],
            SemanticField::PollingRate => &["settings", "settingsExtended", "pollingRate"],
            SemanticField::ActiveProfile => &[
                "profileMgmtCurrent",
                "profile",
                "settings",
                "dpi",
                "onboardCurrentProfile",
            ],
            SemanticField::LightingState => &[
                "mouseLighting",
                "receiverLighting",
                "lighting",
                "settings",
                "rgbControl",
            ],
            // inventory output 由插件通过 PluginRuntime.inventory.workflows 声明，
            // 标准化为 "inventory" 名称。实际 workflow output 通过既有 output 机制映射。
            SemanticField::Inventory => &["inventory"],
        }
    }
}

/// 将语义字段集合映射为目标 output 名称集合。
///
/// 只选择工作流中实际存在的 output，避免请求不存在的 output 导致投影失败。
/// 返回 (有效目标, 缺失目标) 元组。
pub fn map_semantic_to_outputs(
    package: &ProtocolPackage,
    workflow_id: &str,
    fields: &BTreeSet<SemanticField>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let available = package.available_outputs(workflow_id);
    let preferred = package
        .semantic_output_cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(workflow_id).cloned())
        .unwrap_or_default();
    map_semantic_fields_to_outputs(&available, fields, &preferred)
}

fn map_semantic_fields_to_outputs(
    available: &BTreeSet<String>,
    fields: &BTreeSet<SemanticField>,
    preferred: &BTreeMap<String, BTreeSet<String>>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut targets = BTreeSet::new();
    let mut missing = BTreeSet::new();

    for field in fields {
        let field_key = format!("{field:?}");
        if let Some(outputs) = preferred
            .get(&field_key)
            .filter(|outputs| outputs.iter().all(|output| available.contains(output)))
        {
            targets.extend(outputs.iter().cloned());
            continue;
        }
        let mut matched = 0_u8;
        for name in field.standard_output_names() {
            if available.contains(*name) {
                targets.insert(name.to_string());
                matched = matched.saturating_add(1);
            }
        }
        if matched == 0 {
            missing.insert(format!("{field:?}"));
        }
    }

    (targets, missing)
}

fn remember_successful_semantic_outputs(
    package: &ProtocolPackage,
    workflow_id: &str,
    outputs: &BTreeMap<String, Value>,
) {
    let mut preferred = BTreeMap::new();
    for field in [
        SemanticField::BatteryPercent,
        SemanticField::ReceiverBatteryPercent,
        SemanticField::Charging,
        SemanticField::CurrentDpi,
        SemanticField::PollingRate,
        SemanticField::ActiveProfile,
        SemanticField::LightingState,
        SemanticField::Inventory,
    ] {
        let successful = field
            .standard_output_names()
            .iter()
            .filter(|name| {
                outputs
                    .get(**name)
                    .is_some_and(|value| semantic_output_is_useful(field, name, value))
            })
            .map(|name| (*name).to_string())
            .collect::<BTreeSet<_>>();
        if !successful.is_empty() {
            preferred.insert(format!("{field:?}"), successful);
        }
    }
    if let Ok(mut cache) = package.semantic_output_cache.lock() {
        cache.insert(workflow_id.to_string(), preferred);
    }
}

fn semantic_output_is_useful(field: SemanticField, name: &str, value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return !value.is_null();
    };
    if object.is_empty() {
        return false;
    }
    let has_any = |keys: &[&str]| keys.iter().any(|key| object.contains_key(*key));
    match field {
        SemanticField::BatteryPercent => {
            has_any(&["percentage", "batteryPercent", "level", "percent"])
        }
        SemanticField::ReceiverBatteryPercent => has_any(&[
            "percentage",
            "batteryPercent",
            "receiverBatteryPercent",
            "level",
        ]),
        SemanticField::Charging => has_any(&["charging", "chargeStatus", "status"]),
        SemanticField::CurrentDpi => {
            has_any(&["dpiValue", "dpiX", "currentDpi", "currentStage", "value"])
        }
        SemanticField::PollingRate => has_any(&["pollingRate", "pollingRateHz", "rate"]),
        SemanticField::ActiveProfile => {
            has_any(&["profile", "currentProfile", "activeProfile", "profileIndex"])
        }
        SemanticField::LightingState => {
            if name == "settings" || name == "settingsExtended" {
                has_any(&[
                    "mouseLightEnabled",
                    "mouseLightStartColor",
                    "mouseLightEndColor",
                ])
            } else {
                has_any(&["enabled", "effect", "mode", "color", "brightness", "speed"])
            }
        }
        // inventory output 是插件声明性的完整读数：非空对象即视为有用。
        SemanticField::Inventory => true,
    }
}

pub fn read_device(ctx: &ProtocolContext) -> Result<DeviceReading, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    read_device_with_package(&package, ctx)
}

/// 读取设备并接收 HID 交换事件。
pub fn read_device_with_sink(
    ctx: &ProtocolContext,
    sink: &dyn HidEventSink,
) -> Result<DeviceReading, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    read_device_with_package_and_sink(&package, ctx, sink)
}

/// 将已有的工作流 outputs 规范化为 `DeviceReading`，不重新执行工作流。
/// 用于 mutation 验证后合并字段到缓存 outputs、再就地规范化以发布即时快照。
pub fn normalize_device_outputs_with_package(
    package: &ProtocolPackage,
    outputs: BTreeMap<String, Value>,
) -> DeviceReading {
    let capabilities = package.capabilities().cloned();
    standard_reading(outputs, capabilities, BTreeMap::new())
}

/// Classify a structured runtime failure into the stable fault contract exposed
/// to fixture tests and host-facing diagnostics.
///
/// Keeping this as a pure runtime function lets fixtures execute the same
/// classification rules without needing HID hardware.
pub fn classify_contract_fault(input: &Value) -> Result<&'static str, String> {
    let object = input
        .as_object()
        .ok_or_else(|| "fault input must be an object".to_string())?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "fault input missing kind".to_string())?;
    match kind {
        "transport" => {
            let error = object
                .get("error")
                .and_then(Value::as_str)
                .ok_or_else(|| "transport fault missing error".to_string())?
                .to_ascii_lowercase();
            if error.contains("disconnect") || error.contains("unplug") {
                Ok("transaction-cancelled")
            } else if error.contains("timed out") || error.contains("timeout") {
                Ok("transport-timeout")
            } else if error.contains("unreachable") || error.contains("offline") {
                Ok("device-unreachable")
            } else {
                Err(format!("unclassified transport error: {error}"))
            }
        }
        "parser" => {
            let error = object
                .get("error")
                .and_then(Value::as_str)
                .ok_or_else(|| "parser fault missing error".to_string())?
                .to_ascii_lowercase();
            if error.contains("checksum") {
                Ok("checksum-mismatch")
            } else {
                Ok("parser-rejected")
            }
        }
        "battery-threshold" => {
            let number = |key: &str| {
                object
                    .get(key)
                    .and_then(Value::as_u64)
                    .ok_or_else(|| format!("battery-threshold fault missing {key}"))
            };
            let previous = number("previous")?;
            let current = number("current")?;
            let threshold = number("threshold")?;
            if previous > 100 || current > 100 || threshold > 100 {
                return Err("battery-threshold values must be in [0, 100]".into());
            }
            if previous > threshold && current <= threshold {
                Ok("threshold-crossed-once")
            } else {
                Err(format!(
                    "battery threshold was not crossed: previous={previous}, current={current}, threshold={threshold}"
                ))
            }
        }
        "readback" => {
            let expected = object
                .get("expected")
                .ok_or_else(|| "readback fault missing expected".to_string())?;
            let actual = object
                .get("actual")
                .ok_or_else(|| "readback fault missing actual".to_string())?;
            if expected != actual {
                Ok("actual-state-shown-not-success")
            } else {
                Err("readback values match; this is not a failure".into())
            }
        }
        other => Err(format!("unknown fault kind: {other}")),
    }
}

/// Like `read_device` but reuses a pre-parsed `ProtocolPackage` to avoid
/// re-parsing the JSON files on every call.
pub fn read_device_with_package(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
) -> Result<DeviceReading, String> {
    let workflow_id = format!("{}-read", ctx.family);
    let (mut outputs, read_statuses) = match ctx.hid_event_sink {
        Some(sink) => package.execute_with_cache_and_sink(
            ctx.api,
            ctx.path,
            &workflow_id,
            ctx.feature_index_cache,
            ctx.cached_handles,
            ctx.hid_io_stats,
            sink,
        )?,
        None => package.execute_with_cache(
            ctx.api,
            ctx.path,
            &workflow_id,
            ctx.feature_index_cache,
            ctx.cached_handles,
            ctx.hid_io_stats,
        )?,
    };
    remember_successful_semantic_outputs(package, &workflow_id, &outputs);
    let capabilities = package.capabilities().cloned();
    maybe_merge_onboard_lighting(package, ctx, capabilities.as_ref(), &mut outputs)?;
    #[cfg(debug_assertions)]
    eprintln!(
        "[mira] plugin workflow {workflow_id}: {} outputs: [{}]",
        outputs.len(),
        outputs
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(standard_reading(outputs, capabilities, read_statuses))
}

/// 与 `read_device_with_package` 相同，但接收 HID 交换事件回调。
pub fn read_device_with_package_and_sink(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
    sink: &dyn HidEventSink,
) -> Result<DeviceReading, String> {
    let workflow_id = format!("{}-read", ctx.family);
    let (mut outputs, read_statuses) = package.execute_with_cache_and_sink(
        ctx.api,
        ctx.path,
        &workflow_id,
        ctx.feature_index_cache,
        ctx.cached_handles,
        ctx.hid_io_stats,
        sink,
    )?;
    remember_successful_semantic_outputs(package, &workflow_id, &outputs);
    let capabilities = package.capabilities().cloned();
    maybe_merge_onboard_lighting(package, ctx, capabilities.as_ref(), &mut outputs)?;
    Ok(standard_reading(outputs, capabilities, read_statuses))
}

/// 投影读取结果：包含读取的 DeviceReading 和投影诊断信息。
pub struct ProjectedReading {
    pub reading: DeviceReading,
    /// 投影是否成功。失败时回退到完整读取。
    pub projection_valid: bool,
    /// 投影回退原因（如果有）。
    pub fallback_reason: Option<String>,
    /// 投影选中的 step 数量。
    pub projected_step_count: usize,
}

/// 使用工作流投影读取设备状态。
///
/// 根据语义字段集合计算工作流投影，只执行生成目标 output 所需的最小 step 子集。
/// 投影失败时自动回退到完整读取（`read_device_with_package`）。
///
/// UI 不直接调用此函数，而是由宿主调度器根据 ReadPlan 和视图需求调用。
pub fn read_device_with_projection(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
    fields: &BTreeSet<SemanticField>,
) -> Result<ProjectedReading, String> {
    let workflow_id = format!("{}-read", ctx.family);

    // 1. 将语义字段映射为目标 output 名称
    let (target_outputs, missing_fields) = map_semantic_to_outputs(package, &workflow_id, fields);

    // 2. 计算工作流投影
    let projection = package.compute_projection(&workflow_id, &target_outputs);

    // 3. 如果投影有效，执行投影读取
    if projection.is_valid() {
        let (outputs, read_statuses) = match ctx.hid_event_sink {
            Some(sink) => package.execute_projection_with_cache_and_sink(
                ctx.api,
                ctx.path,
                &workflow_id,
                &projection,
                ctx.feature_index_cache,
                ctx.cached_handles,
                ctx.hid_io_stats,
                sink,
            )?,
            None => package.execute_projection_with_cache(
                ctx.api,
                ctx.path,
                &workflow_id,
                &projection,
                ctx.feature_index_cache,
                ctx.cached_handles,
                ctx.hid_io_stats,
            )?,
        };
        let capabilities = package.capabilities().cloned();
        // 投影读取也需要合并 onboard lighting（如果 lighting 是目标 output 的话）
        let mut outputs = outputs;
        if target_outputs.iter().any(|o| {
            matches!(
                o.as_str(),
                "mouseLighting" | "lighting" | "mouseLightingOnboard"
            )
        }) {
            maybe_merge_onboard_lighting(package, ctx, capabilities.as_ref(), &mut outputs)?;
        }
        #[cfg(debug_assertions)]
        eprintln!(
            "[mira] projected workflow {workflow_id}: {}/{} steps, outputs: [{}]",
            projection.selected_step_count(),
            package.available_outputs(&workflow_id).len(),
            outputs
                .keys()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(ProjectedReading {
            reading: standard_reading(outputs, capabilities, read_statuses),
            projection_valid: true,
            fallback_reason: None,
            projected_step_count: projection.selected_step_count(),
        })
    } else {
        let reason = projection
            .fallback_reason()
            .unwrap_or("projection returned no steps")
            .to_string();
        let missing_info = if missing_fields.is_empty() {
            String::new()
        } else {
            format!(
                " (missing semantic fields: {})",
                missing_fields
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        #[cfg(debug_assertions)]
        eprintln!("[mira] projection fallback for {workflow_id}: {reason}{missing_info}");
        let reading = read_device_with_package(package, ctx)?;
        Ok(ProjectedReading {
            reading,
            projection_valid: false,
            fallback_reason: Some(format!("{reason}{missing_info}")),
            projected_step_count: 0,
        })
    }
}

fn maybe_merge_onboard_lighting(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
    capabilities: Option<&Value>,
    outputs: &mut BTreeMap<String, Value>,
) -> Result<(), String> {
    // 3.1 节：检查标准 `mouseLighting` output 是否已存在（由插件直接产出或
    // 通过 semanticMappings 映射）。如果已存在则跳过 onboard 合并。
    if outputs.contains_key("mouseLighting") {
        return Ok(());
    }
    let Some(feature_index) = object(outputs, "featureIndexOnboardProfiles")
        .and_then(|feature| feature.get("featureIndex"))
        .and_then(Value::as_u64)
    else {
        return Ok(());
    };
    if feature_index == 0 {
        return Ok(());
    }

    let Some(onboard_workflow_id) = onboard_mouse_lighting_workflow_id(capabilities) else {
        return Ok(());
    };
    if !package.has_workflow(&onboard_workflow_id) {
        return Ok(());
    }

    // 持锁期间直接遍历 cached_outputs 并插入，避免克隆整个 BTreeMap。
    if let Some(cache) = ctx.onboard_memory_cache {
        if let Ok(guard) = cache.lock() {
            if let Some((cached_outputs, _)) = guard.get(ctx.path) {
                for (key, value) in cached_outputs {
                    outputs.entry(key.clone()).or_insert(value.clone());
                }
                return Ok(());
            }
        }
    }
    let (onboard_outputs, _) = match package.execute_with_cache(
        ctx.api,
        ctx.path,
        &onboard_workflow_id,
        ctx.feature_index_cache,
        ctx.cached_handles,
        ctx.hid_io_stats,
    ) {
        Ok(onboard) => onboard,
        // onboard 灯光读取是可选增强：失败不应让整次设备读取失败。
        // 缺失的 mouseLighting 由宿主快照合并保留旧值（capabilities 缺失键粘性）。
        Err(error) => {
            #[cfg(debug_assertions)]
            eprintln!("[mira] onboard lighting read failed: {error}");
            return Ok(());
        }
    };
    for (key, value) in onboard_outputs {
        outputs.entry(key).or_insert(value);
    }
    Ok(())
}

pub fn execute_plugin_workflow(
    ctx: &ProtocolContext,
    workflow_id: &str,
) -> Result<BTreeMap<String, Value>, String> {
    let (outputs, _) = ProtocolPackage::from_files(ctx.files)?.execute_with_cache(
        ctx.api,
        ctx.path,
        workflow_id,
        ctx.feature_index_cache,
        ctx.cached_handles,
        ctx.hid_io_stats,
    )?;
    Ok(outputs)
}

pub fn writable_mutations(ctx: &ProtocolContext) -> Result<Vec<String>, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    Ok(package.mutation_ids(ctx.family, Some(&ctx.outputs)))
}

/// Like `writable_mutations` but reuses a pre-parsed `ProtocolPackage`.
pub fn writable_mutations_with_package(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
) -> Result<Vec<String>, String> {
    Ok(package.mutation_ids(ctx.family, Some(&ctx.outputs)))
}

pub fn mutate_device(
    ctx: &ProtocolContext,
    mutation: &str,
    params: &Map<String, Value>,
) -> Result<Value, String> {
    let package = ProtocolPackage::from_files(ctx.files)?;
    mutate_device_with_package(&package, ctx, mutation, params)
}

/// Like `mutate_device` but reuses a pre-parsed `ProtocolPackage`.
pub fn mutate_device_with_package(
    package: &ProtocolPackage,
    ctx: &ProtocolContext,
    mutation: &str,
    params: &Map<String, Value>,
) -> Result<Value, String> {
    let mutation_id = format!("{}-{mutation}", ctx.family);
    package.mutate(
        ctx.api,
        ctx.path,
        &mutation_id,
        params,
        &ctx.outputs,
        ctx.onboard_memory_cache,
        ctx.cached_handles,
        ctx.hid_io_stats,
    )
}

fn standard_reading(
    outputs: BTreeMap<String, Value>,
    capabilities: Option<Value>,
    read_statuses: BTreeMap<String, ReadStatus>,
) -> DeviceReading {
    let mut reading = DeviceReading {
        capabilities: outputs,
        read_statuses,
        ..DeviceReading::default()
    };

    // Prefer device-reported rates from the protocol; fall back to the static
    // plugin manifest so the UI always receives a supported list.
    if let Some(rates) = object(&reading.capabilities, "reportRateList")
        .or_else(|| object(&reading.capabilities, "reportRateListExtended"))
        .and_then(|value| value.get("supportedRates"))
        .and_then(Value::as_array)
    {
        let rates: Vec<u16> = rates
            .iter()
            .filter_map(|value| value.as_u64().and_then(|rate| u16::try_from(rate).ok()))
            .collect();
        if !rates.is_empty() {
            reading.supported_polling_rates_hz = Some(rates);
        }
    }

    if reading.supported_polling_rates_hz.is_none() {
        if let Some(caps) = capabilities.as_ref().and_then(Value::as_object) {
            if let Some(rates) = caps.get("pollingRatesHz").and_then(Value::as_array) {
                let rates: Vec<u16> = rates
                    .iter()
                    .filter_map(|value| value.as_u64().and_then(|rate| u16::try_from(rate).ok()))
                    .collect();
                if !rates.is_empty() {
                    reading.supported_polling_rates_hz = Some(rates);
                }
            }
        }
    }

    reading.display_name = object(&reading.capabilities, "deviceName")
        .and_then(|device| device.get("name"))
        .and_then(Value::as_str)
        .and_then(mira_core::normalize_device_display_name);
    reading.connection = object(&reading.capabilities, "device")
        .or_else(|| object(&reading.capabilities, "featureIndexDeviceInfo"))
        .and_then(|device| device.get("connection"))
        .and_then(Value::as_str)
        .and_then(|connection| match connection {
            "usb" => Some(ConnectionKind::Usb),
            "wireless" | "wireless-receiver" => Some(ConnectionKind::Wireless),
            "bluetooth" => Some(ConnectionKind::Bluetooth),
            _ => None,
        });

    if let Some(battery) = object(&reading.capabilities, "battery") {
        reading.battery_percent = reported_battery_percentage(battery, "percentage");
        reading.charging = battery_charging(battery, "charging");
        if let Some(percentage) = reading.battery_percent {
            reading.batteries.push(mira_core::DeviceBattery {
                id: "mouse".into(),
                label: "mock.mouseLabel".into(),
                percentage,
                charging: reading.charging,
            });
        }
    }

    // Receiver transports expose their status object alongside ordinary workflow outputs.
    let receiver_idle = object(&reading.capabilities, "receiverIdle");
    let receiver_proxy = object(&reading.capabilities, "receiver");
    let receiver = receiver_idle.or(receiver_proxy);
    // 接收器场景：根据 receiverIdle.mouseOnline 设置 mouse_ready。
    // - mouseOnline: true → Some(true)，鼠标已就位
    // - mouseOnline: false → Some(false)，鼠标未就位，UI 显示等待提示
    // - mouseOnline 字段缺失（旧版插件）→ None，状态未知，UI 按默认行为显示 Dashboard
    // receiver 代理对象不暴露 mouseOnline，保持 None。
    if let Some(receiver_idle_obj) = receiver_idle {
        reading.mouse_ready = boolean_like(receiver_idle_obj, "mouseOnline");
    }
    if let Some(receiver) = receiver {
        if reading.battery_percent.is_none() {
            reading.battery_percent = receiver_mouse_battery_percentage(receiver);
        }
        if reading.batteries.is_empty() {
            if let Some(percentage) = receiver_mouse_battery_percentage(receiver) {
                reading.batteries.push(mira_core::DeviceBattery {
                    id: "mouse".into(),
                    label: "mock.mouseLabel".into(),
                    percentage,
                    charging: false,
                });
            }
        }
        if let Some(percentage) = receiver_status_battery_percentage(receiver) {
            reading.batteries.push(mira_core::DeviceBattery {
                id: "receiver".into(),
                label: "mock.receiverLabel".into(),
                percentage,
                charging: protocol_a_receiver_battery_charging(percentage),
            });
        }
    }
    if let Some(receiver_battery) = object(&reading.capabilities, "receiverBattery") {
        if let Some(percentage) = reported_battery_percentage(receiver_battery, "percentage") {
            upsert_battery(
                &mut reading.batteries,
                mira_core::DeviceBattery {
                    id: "receiver".into(),
                    label: "mock.receiverLabel".into(),
                    percentage,
                    charging: battery_charging(receiver_battery, "charging"),
                },
            );
        }
    }

    reading.profile = crate::onboard_profiles::active_profile_index(&reading.capabilities);

    // If the plugin already emitted a structured "profile" capability, keep it.
    // Otherwise normalize 0x8101 Profile Management outputs into a single capability.
    if object(&reading.capabilities, "profile").is_none()
        && (crate::onboard_profiles::profile_count(&reading.capabilities).is_some()
            || crate::onboard_profiles::profile_management_info(&reading.capabilities).is_some())
    {
        let mut profile = serde_json::Map::new();
        if let Some(current) = reading.profile {
            profile.insert("current".into(), json!(current));
        }
        if let Some(count) = crate::onboard_profiles::profile_count(&reading.capabilities) {
            profile.insert("count".into(), json!(count));
        }
        if let Some(info) = crate::onboard_profiles::profile_management_info(&reading.capabilities)
        {
            profile.insert(
                "management".to_string(),
                json!({
                    "featureVersion": info.feature_version,
                    "maxProfileCount": info.max_profile_count,
                    "profileNameLength": info.profile_name_length,
                }),
            );
        }
        reading
            .capabilities
            .insert("profile".into(), Value::Object(profile));
    }

    if let Some(dpi) = object(&reading.capabilities, "dpi")
        .or_else(|| object(&reading.capabilities, "dpiExtended"))
    {
        let current = number(dpi, "currentStage").and_then(|value| usize::try_from(value).ok());
        let values = array(dpi, "dpiX");
        let colors = array(dpi, "stageColors");
        if let Some(values) = values {
            // Array-based DPI stages (e.g. AMaster protocol A).
            let count = number(dpi, "stageCount")
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(values.len())
                .min(8);
            let stages: Vec<_> = (0..count)
                .filter_map(|index| {
                    let value = values
                        .get(index)?
                        .as_u64()
                        .and_then(|value| u16::try_from(value).ok())?;
                    let color = colors
                        .and_then(|colors| colors.get(index)?.as_str())
                        .unwrap_or("#9a8bd0")
                        .to_string();
                    Some(mira_core::DpiStage {
                        value,
                        color,
                        enabled: true,
                        active: current.map(|c| c == index + 1).unwrap_or(index == 0),
                    })
                })
                .collect();
            reading.dpi = stages
                .iter()
                .find(|stage| stage.active)
                .map(|stage| stage.value);
            if !stages.is_empty() {
                reading.dpi_stages = Some(stages);
            }
        } else if let Some(value) = number(dpi, "dpiValue") {
            // Single-value DPI (e.g. HID++ 2.0 AdjustableDPI).
            if let Ok(value) = u16::try_from(value) {
                reading.dpi = Some(value);
                reading.dpi_stages = Some(vec![mira_core::DpiStage {
                    value,
                    color: "#9a8bd0".into(),
                    enabled: true,
                    active: true,
                }]);
            }
        }
    }

    if let Some(settings) = object(&reading.capabilities, "settings")
        .or_else(|| object(&reading.capabilities, "settingsExtended"))
        .or_else(|| object(&reading.capabilities, "pollingRate"))
    {
        reading.polling_rate_hz =
            number(settings, "pollingRate").and_then(|value| u16::try_from(value).ok());
    }

    // 3.1 节：宿主不再猜测插件原始字段（mouseLightMode/receiverLight/color1 等）。
    // 插件通过 capabilities.json 的 `semanticMappings` 声明如何把品牌原始 output
    // 映射为标准语义 output（mouseLighting/receiverLighting）。宿主只执行通用映射。
    apply_semantic_mappings(&mut reading.capabilities, capabilities.as_ref());

    // onboard profile lighting 仍是通用机制：插件通过 `normalizers.mouseLighting.onboardProfile`
    // 声明 onboard 布局，宿主按声明解析 chunk bytes。不由 semanticMappings 处理。
    if !reading.capabilities.contains_key("mouseLighting") {
        if let Some(onboard) = onboard_mouse_lighting(&reading.capabilities, capabilities.as_ref())
        {
            reading
                .capabilities
                .insert("mouseLighting".into(), Value::Object(onboard));
        }
    }

    reading.light_color = object(&reading.capabilities, "mouseLighting")
        .and_then(|lighting| lighting.get("color"))
        .and_then(Value::as_str)
        .map(str::to_string);

    reading
}

/// 3.1 节：通用语义映射引擎。
///
/// 插件通过 `capabilities.json` 的 `semanticMappings` 声明如何把品牌原始 output
/// 映射为标准语义 output。宿主只执行通用路径映射，不认识任何品牌字段名。
///
/// schema：
/// ```json
/// {
///   "semanticMappings": {
///     "mouseLighting": [
///       { "output": "mouseEffect" },
///       { "output": "mouseLightMode" },
///       { "output": "settings", "fieldMap": { "mouseLightStartColor": "color", "mouseLightEnabled": "enabled" } }
///     ],
///     "receiverLighting": [
///       { "output": "receiverLight", "fieldMap": { "type": ["effect", "option"], "color1": "color" } }
///     ]
///   }
/// }
/// ```
///
/// 规则：
/// - 目标 output 已存在时不覆盖（插件直接产出的标准 output 优先）。
/// - `{ "output": "X" }`：把 output X 的所有字段原样复制到目标。
/// - `{ "output": "X", "fieldMap": { "src": "dst" } }`：把 output X 的 src 字段复制为目标的 dst 字段。
/// - `{ "output": "X", "fieldMap": { "src": ["dst1", "dst2"] } }`：把 src 字段复制为 dst1 和 dst2。
/// - 字段级"先到先得"：sources 数组中先声明的源优先，后续源不覆盖已存在的字段。
///   插件通过数组顺序声明优先级（主源在前，fallback 在后）。
/// - 源 output 不存在或字段缺失时跳过，不报错。
/// - onboard profile lighting（通过 `normalizers.mouseLighting.onboardProfile` 声明）
///   仍是通用机制，不由 semanticMappings 处理。
fn apply_semantic_mappings(outputs: &mut BTreeMap<String, Value>, capabilities: Option<&Value>) {
    let Some(mappings) = capabilities
        .and_then(|caps| caps.get("semanticMappings"))
        .and_then(Value::as_object)
    else {
        return;
    };

    for (target_name, sources) in mappings {
        // 目标 output 已存在（由插件直接产出）时跳过映射。
        if outputs.contains_key(target_name) {
            continue;
        }

        let mut target: serde_json::Map<String, Value> = serde_json::Map::new();
        let Some(sources_arr) = sources.as_array() else {
            continue;
        };
        for source in sources_arr {
            let Some(source_obj) = source.as_object() else {
                continue;
            };
            let Some(source_output) = source_obj
                .get("output")
                .and_then(Value::as_str)
                .and_then(|name| outputs.get(name))
                .and_then(Value::as_object)
            else {
                continue;
            };

            if let Some(field_map) = source_obj.get("fieldMap").and_then(Value::as_object) {
                // 字段重命名映射：{ "srcField": "dstField" } 或 { "srcField": ["dst1", "dst2"] }
                // 与无 fieldMap 分支一致，使用 `or_insert` 不覆盖已有字段，
                // 让插件通过 sources 数组顺序声明优先级（先到先得）。
                for (src_field, dst_fields) in field_map {
                    let Some(value) = source_output.get(src_field) else {
                        continue;
                    };
                    match dst_fields {
                        Value::String(dst) => {
                            target.entry(dst.clone()).or_insert_with(|| value.clone());
                        }
                        Value::Array(dsts) => {
                            for dst in dsts {
                                if let Some(dst) = dst.as_str() {
                                    target
                                        .entry(dst.to_string())
                                        .or_insert_with(|| value.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                // 原样复制所有字段
                for (key, value) in source_obj.iter().filter(|(key, _)| *key != "output") {
                    target.insert(key.clone(), value.clone());
                }
                for (key, value) in source_output.iter() {
                    target.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }

        if !target.is_empty() {
            outputs.insert(target_name.clone(), Value::Object(target));
        }
    }
}

fn onboard_mouse_lighting(
    outputs: &BTreeMap<String, Value>,
    capabilities: Option<&Value>,
) -> Option<serde_json::Map<String, Value>> {
    let profile = onboard_mouse_lighting_normalizer(capabilities)?;
    if !onboard_profile_lighting_active(outputs) {
        return None;
    }
    let description_output = profile
        .get("sectorSize")
        .and_then(|reference| reference.get("output"))
        .and_then(Value::as_str)
        .unwrap_or("onboardDescription");
    let chunk_prefix = profile
        .get("chunkPrefix")
        .and_then(Value::as_str)
        .unwrap_or("onboardProfileChunk");
    let chunk_field = profile
        .get("chunkField")
        .and_then(Value::as_str)
        .unwrap_or("bytes");
    let description = object(outputs, description_output)?;
    let sector_size = profile
        .get("sectorSize")
        .and_then(|reference| reference.get("field"))
        .and_then(Value::as_str)
        .and_then(|field| number(description, field))
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(256);
    let bytes = onboard_profile_bytes(outputs, sector_size, chunk_prefix, chunk_field)?;
    let layout = profile
        .get("layouts")?
        .as_array()?
        .iter()
        .find(|layout| onboard_layout_matches(layout, description))?
        .as_object()?;
    let effect_offset = normalizer_offset(layout, "effectOffset")?;
    let color_offset = normalizer_offset(layout, "colorOffset")?;
    let speed_offset = normalizer_offset(layout, "speedOffset")?;
    let brightness_offset = normalizer_offset(layout, "brightnessOffset")?;
    let extra_color_offset = normalizer_offset(layout, "extraColorOffset")?;
    if bytes.len() <= extra_color_offset + 2
        || bytes.len() <= brightness_offset
        || bytes.len() <= effect_offset
        || bytes.len() <= color_offset + 2
        || bytes.len() <= speed_offset + 1
    {
        return None;
    }

    let effect = bytes[effect_offset];
    let enabled = profile
        .get("enabledOverride")
        .and_then(|reference| {
            let output = reference.get("output")?.as_str()?;
            let field = reference.get("field")?.as_str()?;
            object(outputs, output).and_then(|value| boolean_like(value, field))
        })
        .unwrap_or(effect != 0);
    let mut lighting = serde_json::Map::new();
    lighting.insert("enabled".into(), json!(enabled));
    lighting.insert("effect".into(), json!(effect));
    lighting.insert(
        "color".into(),
        json!(format!(
            "#{:02x}{:02x}{:02x}",
            bytes[color_offset],
            bytes[color_offset + 1],
            bytes[color_offset + 2]
        )),
    );
    lighting.insert(
        "speed".into(),
        json!(u16::from_be_bytes([
            bytes[speed_offset],
            bytes[speed_offset + 1]
        ])),
    );
    lighting.insert("brightness".into(), json!(bytes[brightness_offset]));
    lighting.insert(
        "extraColor".into(),
        json!(format!(
            "#{:02x}{:02x}{:02x}",
            bytes[extra_color_offset],
            bytes[extra_color_offset + 1],
            bytes[extra_color_offset + 2]
        )),
    );
    Some(lighting)
}

fn onboard_profile_lighting_active(outputs: &BTreeMap<String, Value>) -> bool {
    let mode = object(outputs, "onboardMode").or_else(|| object(outputs, "controlMode"));
    mode.and_then(|mode| number(mode, "mode"))
        .is_none_or(|mode| mode == 1)
}

fn onboard_mouse_lighting_normalizer(
    capabilities: Option<&Value>,
) -> Option<&serde_json::Map<String, Value>> {
    capabilities?
        .get("normalizers")?
        .get("mouseLighting")?
        .get("onboardProfile")?
        .as_object()
}

fn onboard_mouse_lighting_workflow_id(capabilities: Option<&Value>) -> Option<String> {
    onboard_mouse_lighting_normalizer(capabilities)?
        .get("sourceWorkflow")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn onboard_layout_matches(layout: &Value, description: &serde_json::Map<String, Value>) -> bool {
    let Some(layout) = layout.as_object() else {
        return false;
    };
    if layout
        .get("default")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    let Some(condition) = layout.get("when").and_then(Value::as_object) else {
        return false;
    };
    let Some(field) = condition.get("field").and_then(Value::as_str) else {
        return false;
    };
    let Some(expected) = condition.get("eq") else {
        return false;
    };
    description.get(field) == Some(expected)
}

fn normalizer_offset(layout: &serde_json::Map<String, Value>, key: &str) -> Option<usize> {
    layout
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn onboard_profile_bytes(
    outputs: &BTreeMap<String, Value>,
    sector_size: usize,
    chunk_prefix: &str,
    chunk_field: &str,
) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for index in 0.. {
        let key = format!("{chunk_prefix}{index:02}");
        let Some(chunk) = object(outputs, &key) else {
            break;
        };
        let chunk_bytes = chunk.get(chunk_field)?.as_array()?;
        for byte in chunk_bytes {
            bytes.push(u8::try_from(byte.as_u64()?).ok()?);
            if bytes.len() >= sector_size {
                return Some(bytes);
            }
        }
    }
    (bytes.len() >= sector_size).then_some(bytes)
}

fn object<'a>(
    outputs: &'a BTreeMap<String, Value>,
    key: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    outputs.get(key)?.as_object()
}

fn number(object: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key)?.as_u64()
}

fn percentage_value(object: &serde_json::Map<String, Value>, key: &str) -> Option<u8> {
    let value = number(object, key)?;
    (value <= 100).then(|| u8::try_from(value).ok()).flatten()
}

fn reported_battery_percentage(object: &serde_json::Map<String, Value>, key: &str) -> Option<u8> {
    // 仅根据 present 判断电池是否存在；valid 标志在不同连接模式下语义
    // 不一致（USB 直连时 offset 2 常为 0），保留它会误伤真实电量数据。
    if boolean_like(object, "present") == Some(false) {
        return None;
    }
    percentage_value(object, key)
}

fn receiver_mouse_battery_percentage(object: &serde_json::Map<String, Value>) -> Option<u8> {
    if boolean_like(object, "mouseOnline") == Some(false) {
        return None;
    }
    percentage_value(object, "mouseBattery")
}

fn receiver_status_battery_percentage(object: &serde_json::Map<String, Value>) -> Option<u8> {
    let percentage = percentage_value(object, "receiverBattery")?;
    // Official AMasterDriver (MouseDocker.get_0xf7) reads offset 10 raw and
    // only maps 0 → -1 (unavailable). It does NOT treat 0x32 (50) as a
    // placeholder — 50 is a legitimate battery level. Mirroring that here.
    if percentage == 0 {
        return None;
    }
    Some(percentage)
}

fn protocol_a_receiver_battery_charging(percentage: u8) -> bool {
    // Official AMasterDriver 1.3.8 maps the receiver-slot battery to
    // mouseDBatStatus: 1 while 0 < pct < 100, 2 at 100, and 0 when absent.
    // Its frontend defines status 1 as charging. Preserve that state so the
    // host does not treat the charging curve as an exact discharge reading.
    percentage > 0 && percentage < 100
}

/// 电池充电状态字段约定：原始字节值 1 表示充电中（与官方前端
/// `1 === mouseBatStatus` / `1 === dongleChargingStatus` 一致）。
/// 0 = 未充电，2 = 满电（或其他状态码）均不视为充电中。
/// 兼容旧 parser 输出的 bool 值（true 视为 1）。
fn battery_charging(object: &serde_json::Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_u64().map(|status| status == 1))
        })
        .unwrap_or(false)
}

fn boolean_like(object: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    object.get(key).and_then(|value| {
        value
            .as_bool()
            .or_else(|| value.as_u64().map(|number| number != 0))
    })
}

fn upsert_battery(
    batteries: &mut Vec<mira_core::DeviceBattery>,
    battery: mira_core::DeviceBattery,
) {
    if let Some(existing) = batteries
        .iter_mut()
        .find(|existing| existing.id == battery.id)
    {
        *existing = battery;
    } else {
        batteries.push(battery);
    }
}

fn array<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Vec<Value>> {
    object.get(key)?.as_array()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_contract_faults_from_runtime_inputs() {
        assert_eq!(
            classify_contract_fault(&json!({
                "kind": "transport",
                "error": "operation timed out"
            })),
            Ok("transport-timeout")
        );
        assert_eq!(
            classify_contract_fault(&json!({
                "kind": "transport",
                "error": "device disconnected"
            })),
            Ok("transaction-cancelled")
        );
        assert_eq!(
            classify_contract_fault(&json!({
                "kind": "parser",
                "error": "checksum mismatch"
            })),
            Ok("checksum-mismatch")
        );
        assert_eq!(
            classify_contract_fault(&json!({
                "kind": "battery-threshold",
                "previous": 10,
                "current": 9,
                "threshold": 9
            })),
            Ok("threshold-crossed-once")
        );
        assert_eq!(
            classify_contract_fault(&json!({
                "kind": "readback",
                "expected": { "pollingRateHz": 1000 },
                "actual": { "pollingRateHz": 500 }
            })),
            Ok("actual-state-shown-not-success")
        );
    }

    #[test]
    fn rejects_non_fault_contract_inputs() {
        assert!(classify_contract_fault(&json!({
            "kind": "battery-threshold",
            "previous": 8,
            "current": 9,
            "threshold": 9
        }))
        .is_err());
        assert!(classify_contract_fault(&json!({
            "kind": "readback",
            "expected": 1000,
            "actual": 1000
        }))
        .is_err());
    }

    #[test]
    fn semantic_mapping_collects_all_available_runtime_sources() {
        // 3.1 节：宿主只消费标准语义 output 名（mouseLighting），
        // 不再收集品牌原始 output 名（mouseLightMode/mouseEffect 等）。
        let available = BTreeSet::from([
            "settings".to_string(),
            "settingsExtended".to_string(),
            "pollingRate".to_string(),
            "profileMgmtCurrent".to_string(),
            "profile".to_string(),
            "mouseLighting".to_string(),
            "receiverLighting".to_string(),
        ]);
        let fields = BTreeSet::from([
            SemanticField::PollingRate,
            SemanticField::ActiveProfile,
            SemanticField::LightingState,
        ]);

        let (targets, missing) =
            map_semantic_fields_to_outputs(&available, &fields, &BTreeMap::new());

        assert!(missing.is_empty());
        for expected in [
            "settings",
            "settingsExtended",
            "pollingRate",
            "profileMgmtCurrent",
            "profile",
            "mouseLighting",
            "receiverLighting",
        ] {
            assert!(targets.contains(expected), "missing target {expected}");
        }
    }

    #[test]
    fn semantic_mapping_reuses_full_read_preferred_output() {
        let available = BTreeSet::from([
            "settings".to_string(),
            "settingsExtended".to_string(),
            "pollingRate".to_string(),
        ]);
        let fields = BTreeSet::from([SemanticField::PollingRate]);
        let preferred = BTreeMap::from([(
            "PollingRate".to_string(),
            BTreeSet::from(["pollingRate".to_string()]),
        )]);

        let (targets, missing) = map_semantic_fields_to_outputs(&available, &fields, &preferred);

        assert!(missing.is_empty());
        assert_eq!(targets, BTreeSet::from(["pollingRate".to_string()]));
    }

    #[test]
    fn semantic_mapping_keeps_composite_lighting_outputs() {
        // 3.1 节：宿主只消费标准语义 output 名（mouseLighting）。
        let available = BTreeSet::from(["settings".to_string(), "mouseLighting".to_string()]);
        let fields = BTreeSet::from([SemanticField::LightingState]);
        let preferred = BTreeMap::from([(
            "LightingState".to_string(),
            BTreeSet::from(["mouseLighting".to_string()]),
        )]);

        let (targets, missing) = map_semantic_fields_to_outputs(&available, &fields, &preferred);

        assert!(missing.is_empty());
        assert_eq!(targets, BTreeSet::from(["mouseLighting".to_string()]));
    }

    #[test]
    fn semantic_output_cache_rejects_unrelated_nonempty_settings() {
        // 3.1 节：settings output 是否对 LightingState 有用，只看标准字段（mouseLightEnabled
        // 由插件通过 semanticMappings 映射到 mouseLighting，不再由宿主直接检查 settings）。
        // settings 仍可包含 pollingRate 等非灯光字段，不应被误判为灯光来源。
        assert!(!semantic_output_is_useful(
            SemanticField::LightingState,
            "settings",
            &json!({"pollingRate": 1000})
        ));
        // settings 包含 mouseLightEnabled 时仍视为有用（兼容 Protocol A 的 settings 灯光字段）。
        assert!(semantic_output_is_useful(
            SemanticField::LightingState,
            "settings",
            &json!({"mouseLightEnabled": true})
        ));
    }

    #[test]
    fn normalizes_am35_quick_polling_profile_and_lighting_outputs() {
        // 3.1 节：AM35 插件通过 semanticMappings 声明 mouseLightMode + mouseLightColor
        // → mouseLighting 的映射。宿主执行通用映射，不认识品牌字段名。
        let outputs = BTreeMap::from([
            ("pollingRate".into(), json!({"pollingRate": 8000})),
            ("profile".into(), json!({"profile": 2})),
            ("mouseLightMode".into(), json!({"mode": 1, "enabled": true})),
            ("mouseLightColor".into(), json!({"color": "#12ABEF"})),
        ]);
        let capabilities = Some(json!({
            "semanticMappings": {
                "mouseLighting": [
                    { "output": "mouseLightMode" },
                    { "output": "mouseLightColor" }
                ]
            }
        }));

        let reading = standard_reading(outputs, capabilities, BTreeMap::new());

        assert_eq!(reading.polling_rate_hz, Some(8000));
        assert_eq!(reading.profile, Some(2));
        assert_eq!(reading.light_color.as_deref(), Some("#12ABEF"));
    }

    #[test]
    fn maps_standard_capabilities_without_protocol_offsets() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 83, "charging": false}),
            ),
            (
                "dpi".into(),
                json!({
                    "profile": 0,
                    "currentStage": 2,
                    "stageCount": 2,
                    "dpiX": [400, 800],
                    "stageColors": ["#112233", "#445566"]
                }),
            ),
            (
                "settings".into(),
                json!({"profile": 0, "pollingRate": 1000}),
            ),
            ("mouseEffect".into(), json!({"color": "#AABBCC"})),
        ]);
        // 3.1 节：插件通过 semanticMappings 声明 mouseEffect → mouseLighting 映射。
        let capabilities = Some(json!({
            "semanticMappings": {
                "mouseLighting": [{ "output": "mouseEffect" }]
            }
        }));
        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        assert_eq!(reading.battery_percent, Some(83));
        assert_eq!(reading.batteries.len(), 1);
        assert_eq!(reading.dpi, Some(800));
        assert_eq!(reading.polling_rate_hz, Some(1000));
        assert_eq!(reading.light_color.as_deref(), Some("#AABBCC"));
        assert!(reading.capabilities.contains_key("mouseLighting"));
    }

    #[test]
    fn normalizes_receiver_and_mouse_batteries() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 76, "charging": false}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 75, "receiverBattery": 100}),
            ),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[0].label, "mock.mouseLabel");
        assert_eq!(reading.batteries[1].label, "mock.receiverLabel");
        assert_eq!(reading.batteries[1].percentage, 100);
    }

    #[test]
    fn normalizes_protocol_a_receiver_battery_charging_status() {
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 88}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 88);
        assert!(reading.batteries[1].charging);
    }

    #[test]
    fn protocol_a_receiver_prefers_idle_status_battery() {
        let outputs = BTreeMap::from([
            (
                "receiverIdle".into(),
                json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 87}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 50}),
            ),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 87);
        assert!(reading.batteries[1].charging);
    }

    #[test]
    fn protocol_a_receiver_keeps_battery_level_50() {
        // Official AMasterDriver does NOT filter 0x32 (50) — it is a legitimate
        // battery level, not a placeholder. Only 0 maps to unavailable.
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 75, "mouseOnline": true, "receiverBattery": 50}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 50);
    }

    /// AMaster INFINITY MOUSE .100 (protocol-a-receiver) 端到端电池测试。
    /// 0xF7 receiver-status 报文：offset 2 = mouseBattery, offset 4 = mouseOnline,
    /// offset 10 = receiverBattery。dedicated 0xD6 battery response offset 3 = 鼠标主电量。
    /// 反编译证据：mouse_battery = res[2]; dongle_battery = res[10]。

    #[test]
    fn amaster_100_separates_mouse_and_receiver_batteries() {
        // mouse=17, receiver=40 两个值独立解析，互不覆盖。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 40}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        // Hero 主电量使用 mouse 17%，不是 receiver 40%。
        assert_eq!(reading.battery_percent, Some(17));
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[0].id, "mouse");
        assert_eq!(reading.batteries[0].percentage, 17);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 40);
    }

    #[test]
    fn amaster_100_battery_snapshot_contains_independent_entries() {
        // DeviceSnapshot batteries 包含独立的 mouse 与 receiver 条目。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 40}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        let mouse = reading
            .batteries
            .iter()
            .find(|b| b.id == "mouse")
            .expect("mouse battery entry must exist");
        let receiver = reading
            .batteries
            .iter()
            .find(|b| b.id == "receiver")
            .expect("receiver battery entry must exist");
        assert_eq!(mouse.percentage, 17);
        assert_eq!(receiver.percentage, 40);
        assert_ne!(mouse.percentage, receiver.percentage);
        assert_ne!(mouse.id, receiver.id);
    }

    #[test]
    fn amaster_100_offset10_zero_hides_receiver_battery() {
        // offset10=0 → 接收器电量不显示，不创建 0% 接收器组件。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 0}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.battery_percent, Some(17));
        assert_eq!(reading.batteries.len(), 1);
        assert_eq!(reading.batteries[0].id, "mouse");
        assert_eq!(reading.batteries[0].percentage, 17);
        assert!(
            reading.batteries.iter().all(|b| b.id != "receiver"),
            "receiver battery must not appear when offset10=0"
        );
    }

    #[test]
    fn amaster_100_offset10_full_reports_receiver_100() {
        // offset10=100 → 接收器满电 100%。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 100}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        let receiver = reading
            .batteries
            .iter()
            .find(|b| b.id == "receiver")
            .expect("receiver battery must exist at 100%");
        assert_eq!(receiver.percentage, 100);
    }

    #[test]
    fn amaster_100_offset10_invalid_does_not_clamp() {
        // offset10=101 → 无效，不 clamp 到 100%，不显示接收器电量。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 101}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.battery_percent, Some(17));
        assert!(
            reading.batteries.iter().all(|b| b.id != "receiver"),
            "receiver battery must not appear when offset10=101"
        );
        // 关键断言：不得 clamp 到 100%。
        assert!(
            reading
                .batteries
                .iter()
                .all(|b| !(b.id == "receiver" && b.percentage == 100)),
            "receiver battery must not be clamped to 100"
        );
    }

    #[test]
    fn amaster_100_mouse_offline_hides_mouse_battery() {
        // mouseOnline=false → mouse 电量不显示，receiver 不得顶替 mouse 成 Hero 主电量。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 0, "mouseOnline": false, "receiverBattery": 40}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        // Hero 主电量必须为 None（mouse 离线），不得用 receiver 40 顶替。
        assert_eq!(reading.battery_percent, None);
        assert!(
            reading.batteries.iter().all(|b| b.id != "mouse"),
            "mouse battery must not appear when mouseOnline=false"
        );
        // receiver 40 仍然作为独立组件显示。
        let receiver = reading
            .batteries
            .iter()
            .find(|b| b.id == "receiver")
            .expect("receiver battery must still appear");
        assert_eq!(receiver.percentage, 40);
    }

    #[test]
    fn amaster_100_survives_missing_dedicated_battery_response() {
        // dedicated 0xD6 mouse battery response 暂时缺失 → 优雅处理，
        // 从 receiver-status offset 2 回退获取 mouse 电量。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 40}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        // 没有 battery output 时，mouse 电量来自 receiver.mouseBattery。
        assert_eq!(reading.battery_percent, Some(17));
        let mouse = reading
            .batteries
            .iter()
            .find(|b| b.id == "mouse")
            .expect("mouse battery must fall back to receiver-status offset 2");
        assert_eq!(mouse.percentage, 17);
    }

    #[test]
    fn amaster_100_forbids_offset2_as_receiver_battery() {
        // 禁止：不得把 offset 2 (mouseBattery) 当作接收器电量。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 40}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        let receiver = reading
            .batteries
            .iter()
            .find(|b| b.id == "receiver")
            .expect("receiver entry must exist");
        // receiver 必须是 40（offset 10），绝不能是 17（offset 2）。
        assert_eq!(receiver.percentage, 40);
        assert_ne!(receiver.percentage, 17);
    }

    #[test]
    fn amaster_100_forbids_receiver_overwriting_mouse_hero() {
        // 禁止：不得把 receiver 40 覆盖 mouse 17 成为 Hero 主电量。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 40}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        // Hero 主电量必须是 17，不得被 receiver 40 覆盖。
        assert_eq!(reading.battery_percent, Some(17));
        assert_ne!(reading.battery_percent, Some(40));
    }

    #[test]
    fn amaster_100_forbids_receiver_as_hero_when_mouse_offline() {
        // 禁止：mouse 离线时不得把 receiver 当 Hero 主电量。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 0, "mouseOnline": false, "receiverBattery": 40}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(
            reading.battery_percent, None,
            "receiver must not become Hero battery when mouse is offline"
        );
    }

    #[test]
    fn amaster_100_forbids_byte_normalization_for_receiver() {
        // 禁止：不得按 0..255 归一化 receiver 电量。
        // offset10=101 必须被丢弃，而不是归一化为 (101/255)*100≈40 或 clamp 到 100。
        let outputs = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 101}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        // receiver 不得出现任何百分比（不 clamp、不归一化）。
        assert!(
            reading.batteries.iter().all(|b| b.id != "receiver"),
            "receiver battery must be dropped, not normalized from 0..255"
        );
        // 同理验证 255（0xFF）也不得归一化。
        let outputs_ff = BTreeMap::from([(
            "receiver".into(),
            json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 255}),
        )]);
        let reading_ff = standard_reading(outputs_ff, None, BTreeMap::new());
        assert!(
            reading_ff.batteries.iter().all(|b| b.id != "receiver"),
            "receiver battery 0xFF must be dropped, not normalized"
        );
    }

    #[test]
    fn amaster_100_dedicated_battery_response_takes_precedence_for_hero() {
        // 当 dedicated 0xD6 battery response 存在时，它提供 Hero 主电量；
        // receiver-status 的 mouseBattery 作为独立 mouse 条目，receiver 作为独立条目。
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 17, "charging": false, "valid": true}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 17, "mouseOnline": true, "receiverBattery": 40}),
            ),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        // Hero 主电量来自 dedicated battery response = 17。
        assert_eq!(reading.battery_percent, Some(17));
        // mouse 条目来自 battery output（id=mouse），receiver 条目来自 receiver-status。
        assert_eq!(reading.batteries.len(), 2);
        let mouse = reading.batteries.iter().find(|b| b.id == "mouse").unwrap();
        let receiver = reading
            .batteries
            .iter()
            .find(|b| b.id == "receiver")
            .unwrap();
        assert_eq!(mouse.percentage, 17);
        assert_eq!(receiver.percentage, 40);
    }

    #[test]
    fn drops_invalid_or_unavailable_battery_percentages() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 101, "charging": false, "valid": true}),
            ),
            (
                "receiverBattery".into(),
                json!({"percentage": 88, "charging": 1, "present": 0}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 80, "mouseOnline": false, "receiverBattery": 0}),
            ),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.battery_percent, None);
        assert!(reading.batteries.is_empty());
    }

    #[test]
    fn normalizes_am35_numeric_charging_and_receiver_battery_output() {
        let outputs = BTreeMap::from([
            (
                "battery".into(),
                json!({"percentage": 76, "charging": 1, "health": 100, "present": 1}),
            ),
            (
                "receiverBattery".into(),
                json!({"percentage": 95, "charging": 1, "health": 100, "present": 1}),
            ),
            (
                "receiver".into(),
                json!({"mouseBattery": 74, "receiverBattery": 88}),
            ),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.battery_percent, Some(76));
        assert!(reading.charging);
        assert_eq!(reading.batteries.len(), 2);
        assert_eq!(reading.batteries[0].id, "mouse");
        assert!(reading.batteries[0].charging);
        assert_eq!(reading.batteries[1].id, "receiver");
        assert_eq!(reading.batteries[1].percentage, 95);
        assert!(reading.batteries[1].charging);
    }

    /// 官方前端用 `1 === mouseBatStatus` / `1 === dongleChargingStatus` 判断充电中。
    /// status=0（未充电）和 status=2（满电）都不应显示充电图标。
    #[test]
    fn treats_only_status_one_as_charging() {
        for (status, expected_charging) in [(0u8, false), (1, true), (2, false)] {
            let outputs = BTreeMap::from([(
                "battery".into(),
                json!({"percentage": 80, "charging": status, "valid": true}),
            )]);
            let reading = standard_reading(outputs, None, BTreeMap::new());
            assert_eq!(
                reading.charging, expected_charging,
                "status {status} should report charging={expected_charging}"
            );
        }

        // AM35 接收器同理（receiverBattery output 携带 charging 字段）。
        for (status, expected_charging) in [(0u8, false), (1, true), (2, false)] {
            let outputs = BTreeMap::from([(
                "receiverBattery".into(),
                json!({"percentage": 90, "charging": status, "present": 1}),
            )]);
            let reading = standard_reading(outputs, None, BTreeMap::new());
            assert_eq!(reading.batteries.len(), 1);
            assert_eq!(
                reading.batteries[0].charging, expected_charging,
                "receiver status {status} should report charging={expected_charging}"
            );
        }
    }

    #[test]
    fn normalizes_plugin_reported_identity() {
        let outputs = BTreeMap::from([
            (
                "device".into(),
                json!({"deviceIndex": 1, "connection": "wireless"}),
            ),
            ("deviceName".into(), json!({"name": "G705 Mouse"})),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.display_name.as_deref(), Some("G705 Mouse"));
        assert_eq!(reading.connection, Some(ConnectionKind::Wireless));
    }

    #[test]
    fn normalizes_plugin_reported_usb_connection() {
        let outputs = BTreeMap::from([(
            "device".into(),
            json!({"deviceIndex": 255, "connection": "usb"}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.connection, Some(ConnectionKind::Usb));
    }

    #[test]
    fn limits_plugin_reported_device_name_for_host_layout() {
        let outputs = BTreeMap::from([(
            "deviceName".into(),
            json!({"name": "  Logitech Prototype Mouse With A Very Long Engineering Name  "}),
        )]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        let display_name = reading.display_name.unwrap();
        assert_eq!(
            display_name.chars().count(),
            mira_core::MAX_DEVICE_DISPLAY_NAME_CHARS
        );
        assert!(display_name.ends_with('…'));
        assert!(!display_name.starts_with(' '));
    }

    #[test]
    fn prefers_mouse_settings_color_over_receiver_lighting() {
        let outputs = BTreeMap::from([
            (
                "settings".into(),
                json!({"mouseLightStartColor": "#FB223C"}),
            ),
            ("receiverLighting".into(), json!({"color": "#4BBFB1"})),
        ]);
        // 3.1 节：插件声明 settings.mouseLightStartColor → mouseLighting.color 映射。
        let capabilities = Some(json!({
            "semanticMappings": {
                "mouseLighting": [
                    { "output": "settings", "fieldMap": { "mouseLightStartColor": "color" } }
                ]
            }
        }));
        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        assert_eq!(reading.light_color.as_deref(), Some("#FB223C"));
        assert_eq!(
            reading
                .capabilities
                .get("mouseLighting")
                .and_then(|value| value.get("color"))
                .and_then(Value::as_str),
            Some("#FB223C")
        );
    }

    #[test]
    fn never_treats_receiver_lighting_as_mouse_light_color() {
        let outputs = BTreeMap::from([
            ("lighting".into(), json!({"color": "#EEAA00"})),
            ("receiverLighting".into(), json!({"color": "#4BBFB1"})),
        ]);
        // 未声明 mouseLighting semanticMapping 时，宿主不应推断任何灯光颜色。
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.light_color, None);
    }

    #[test]
    fn prefers_explicit_mouse_light_color_over_receiver_lighting() {
        let outputs = BTreeMap::from([
            ("mouseLightColor".into(), json!({"color": "#FB223C"})),
            ("receiverLighting".into(), json!({"color": "#4BBFB1"})),
        ]);
        // 3.1 节：插件声明 mouseLightColor → mouseLighting 映射。
        let capabilities = Some(json!({
            "semanticMappings": {
                "mouseLighting": [{ "output": "mouseLightColor" }]
            }
        }));
        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        assert_eq!(reading.light_color.as_deref(), Some("#FB223C"));
    }

    #[test]
    fn normalizes_am35_mouse_and_receiver_lighting_separately() {
        let outputs = BTreeMap::from([
            (
                "mouseLightMode".into(),
                json!({"mode": 2, "modeName": "模式 2", "speed": 1, "brightness": 3}),
            ),
            ("mouseLightColor".into(), json!({"color": "#112233"})),
            (
                "receiverLight".into(),
                json!({"enabled": 1, "type": 7, "color1": "#AABBCC", "speed": 2, "brightness": 4}),
            ),
        ]);
        // 3.1 节：AMaster 插件通过 semanticMappings 声明品牌原始 output 到标准语义 output 的映射。
        let capabilities = Some(json!({
            "semanticMappings": {
                "mouseLighting": [
                    { "output": "mouseLightMode" },
                    { "output": "mouseLightColor" }
                ],
                "receiverLighting": [
                    { "output": "receiverLight", "fieldMap": { "type": ["effect", "option"], "color1": "color" } }
                ]
            }
        }));
        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        let mouse = reading
            .capabilities
            .get("mouseLighting")
            .and_then(Value::as_object)
            .unwrap();
        let receiver = reading
            .capabilities
            .get("receiverLighting")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(reading.light_color.as_deref(), Some("#112233"));
        assert_eq!(mouse.get("color").and_then(Value::as_str), Some("#112233"));
        assert_eq!(mouse.get("mode").and_then(Value::as_u64), Some(2));
        assert_eq!(
            receiver.get("color").and_then(Value::as_str),
            Some("#AABBCC")
        );
        assert_eq!(receiver.get("effect").and_then(Value::as_u64), Some(7));
        assert_eq!(receiver.get("option").and_then(Value::as_u64), Some(7));
    }

    #[test]
    fn single_value_dpi_produces_one_active_stage() {
        // HID++ 2.0 AdjustableDPI returns one DPI value for the active stage.
        // The runtime should expose it as a single-stage list so the UI can
        // render and edit it without a full stage array.
        let outputs = BTreeMap::from([("dpi".into(), json!({"dpiValue": 1600, "stageIndex": 0}))]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.dpi, Some(1600));
        let stages = reading.dpi_stages.expect("dpi stages");
        assert_eq!(stages.len(), 1);
        assert!(stages[0].active);
        assert_eq!(stages[0].value, 1600);
    }

    #[test]
    fn dpi_array_falls_back_to_default_color_when_missing() {
        // Plugins that don't expose per-stage colors should still produce
        // usable stages — the UI replaces the placeholder color later.
        let outputs =
            BTreeMap::from([("dpi".into(), json!({"stageCount": 2, "dpiX": [400, 800]}))]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        let stages = reading.dpi_stages.expect("dpi stages");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].color, "#9a8bd0");
    }

    #[test]
    fn reads_supported_polling_rates_from_report_rate_list() {
        // rateListFlags = 0b00001011 means 1 ms (1000), 2 ms (500), and 8 ms (125) are supported.
        let outputs = BTreeMap::from([
            (
                "reportRateList".into(),
                json!({"rateListFlags": 0x0B, "supportedRates": [1000, 500, 125]}),
            ),
            ("settings".into(), json!({"pollingRate": 500})),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.polling_rate_hz, Some(500));
        assert_eq!(
            reading.supported_polling_rates_hz,
            Some(vec![1000, 500, 125])
        );
    }

    #[test]
    fn reads_extended_hidpp_dpi_and_polling_rate() {
        let outputs = BTreeMap::from([
            (
                "dpiExtended".into(),
                json!({"dpiValue": 2400, "sensorIndex": 0}),
            ),
            ("settingsExtended".into(), json!({"pollingRate": 8000})),
            (
                "reportRateListExtended".into(),
                json!({"rateListFlags": 0x0078, "supportedRates": [1000, 2000, 4000, 8000]}),
            ),
        ]);
        let reading = standard_reading(outputs, None, BTreeMap::new());
        assert_eq!(reading.dpi, Some(2400));
        assert_eq!(reading.polling_rate_hz, Some(8000));
        assert_eq!(
            reading.supported_polling_rates_hz,
            Some(vec![1000, 2000, 4000, 8000])
        );
    }

    #[test]
    fn falls_back_polling_rates_to_capabilities() {
        let outputs = BTreeMap::from([("settings".into(), json!({"pollingRate": 1000}))]);
        let capabilities = Some(json!({"pollingRatesHz": [125, 250, 500, 1000]}));
        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        assert_eq!(
            reading.supported_polling_rates_hz,
            Some(vec![125, 250, 500, 1000])
        );
    }

    #[test]
    fn normalizes_hidpp_onboard_profile_lighting_from_plugin_metadata() {
        let mut outputs = BTreeMap::from([(
            "onboardDescription".into(),
            json!({"profileFormatId": 5, "sectorSize": 255}),
        )]);
        outputs.insert("rgbControl".into(), json!({"enabled": false}));
        outputs.insert(
            "rgbEffectsInfo".into(),
            json!({"supportsFixed": false, "supportsCycle": false, "supportsWave": true}),
        );
        for index in 0..16 {
            let mut chunk = vec![0; 16];
            if index == 13 {
                chunk[11] = 3;
                chunk[12] = 0xb8;
                chunk[13] = 0x7a;
                chunk[14] = 0xb0;
            }
            if index == 14 {
                chunk[0] = 100;
                chunk[1] = 100;
                chunk[2] = 0x12;
                chunk[3] = 0x34;
                chunk[4] = 0x56;
            }
            outputs.insert(
                format!("onboardProfileChunk{index:02}"),
                json!({"bytes": chunk}),
            );
        }

        assert!(!standard_reading(outputs.clone(), None, BTreeMap::new())
            .capabilities
            .contains_key("mouseLighting"));

        let capabilities = Some(json!({
            "normalizers": {
                "mouseLighting": {
                    "onboardProfile": {
                        "sectorSize": { "output": "onboardDescription", "field": "sectorSize" },
                        "enabledOverride": { "output": "rgbControl", "field": "enabled" },
                        "chunkPrefix": "onboardProfileChunk",
                        "chunkField": "bytes",
                        "layouts": [{
                            "when": { "field": "profileFormatId", "eq": 5 },
                            "effectOffset": 219,
                            "colorOffset": 220,
                            "speedOffset": 223,
                            "brightnessOffset": 225,
                            "extraColorOffset": 226
                        }]
                    }
                }
            }
        }));
        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        let mouse = reading
            .capabilities
            .get("mouseLighting")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(reading.light_color.as_deref(), Some("#b87ab0"));
        assert_eq!(mouse.get("enabled").and_then(Value::as_bool), Some(false));
        assert_eq!(mouse.get("effect").and_then(Value::as_u64), Some(3));
        assert_eq!(mouse.get("speed").and_then(Value::as_u64), Some(100));
        assert_eq!(mouse.get("brightness").and_then(Value::as_u64), Some(100));
        assert_eq!(
            mouse.get("extraColor").and_then(Value::as_str),
            Some("#123456")
        );
        assert_eq!(mouse.get("supportedEffects"), None);
    }

    #[test]
    fn normalizes_supported_lighting_effects_from_feature_info() {
        // 3.1 节：宿主不再从 colorLedInfo/rgbEffectsInfo 推断 supportedEffects。
        // 插件应在 workflow 中直接计算 supportedEffects 并通过 semanticMappings 映射。
        let outputs = BTreeMap::from([(
            "mouseEffect".into(),
            json!({"effect": 10, "color": "#123456", "enabled": true, "supportedEffects": [0, 1, 4, 10]}),
        )]);
        let capabilities = Some(json!({
            "semanticMappings": {
                "mouseLighting": [{ "output": "mouseEffect" }]
            }
        }));
        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        let mouse = reading
            .capabilities
            .get("mouseLighting")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(mouse.get("supportedEffects"), Some(&json!([0, 1, 4, 10])));
    }

    #[test]
    fn skips_onboard_profile_lighting_when_host_mode_is_active() {
        let mut outputs = BTreeMap::from([
            (
                "onboardDescription".into(),
                json!({"profileFormatId": 5, "sectorSize": 255}),
            ),
            ("onboardMode".into(), json!({"mode": 2, "modeName": "host"})),
        ]);
        outputs.insert("rgbControl".into(), json!({"enabled": false}));
        for index in 0..16 {
            outputs.insert(
                format!("onboardProfileChunk{index:02}"),
                json!({"bytes": vec![255; 16]}),
            );
        }
        let capabilities = Some(json!({
            "normalizers": {
                "mouseLighting": {
                    "onboardProfile": {
                        "sectorSize": { "output": "onboardDescription", "field": "sectorSize" },
                        "enabledOverride": { "output": "rgbControl", "field": "enabled" },
                        "chunkPrefix": "onboardProfileChunk",
                        "chunkField": "bytes",
                        "layouts": [{
                            "when": { "field": "profileFormatId", "eq": 5 },
                            "effectOffset": 219,
                            "colorOffset": 220,
                            "speedOffset": 223,
                            "brightnessOffset": 225,
                            "extraColorOffset": 226
                        }]
                    }
                }
            }
        }));

        let reading = standard_reading(outputs, capabilities, BTreeMap::new());
        assert!(!reading.capabilities.contains_key("mouseLighting"));
        assert_eq!(reading.light_color, None);
    }

    #[test]
    fn hid_io_stats_records_handle_cache_events() {
        let mut stats = HidIoStats::default();
        stats.record_cache_miss();
        stats.record_cache_hit();
        stats.record_returned();
        stats.record_open_failure();
        stats.record_lock_failure();

        assert_eq!(stats.handle_cache_misses, 1);
        assert_eq!(stats.open_path_attempts, 1);
        assert_eq!(stats.handle_cache_hits, 1);
        assert_eq!(stats.handles_returned, 1);
        assert_eq!(stats.open_path_failures, 1);
        assert_eq!(stats.handle_cache_lock_failures, 1);
    }
}
