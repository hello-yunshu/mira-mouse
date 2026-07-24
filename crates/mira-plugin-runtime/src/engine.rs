// SPDX-License-Identifier: AGPL-3.0-or-later
use hidapi::{HidApi, HidDevice};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::CString;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::{
    FeatureIndexCache, HidEventSink, HidHandleCache, HidIoStats, OnboardMemoryCache, ReadStatus,
};

const MAX_COMMANDS: usize = 56;
const MAX_REPORTS: usize = 128;
const MAX_DELAY_MS: u64 = 5_000;
const MAX_OPERATION_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandsFile {
    schema_version: u32,
    commands: HashMap<String, CommandDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeaturesFile {
    #[serde(default)]
    features: HashMap<String, FeatureEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeatureEntry {
    decimal: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandDefinition {
    request: RequestDefinition,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestDefinition {
    length: usize,
    #[serde(default)]
    base: RequestBase,
    #[serde(default)]
    bytes: Vec<ByteDefinition>,
    checksum: Option<ChecksumDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ByteDefinition {
    offset: usize,
    value: Option<String>,
    param: Option<String>,
    encoding: Option<String>,
    indexed_by: Option<String>,
    #[serde(default)]
    index_base: i64,
    #[serde(default = "default_stride")]
    stride: usize,
    #[serde(default)]
    lookup: BTreeMap<String, u8>,
}

fn default_stride() -> usize {
    1
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum RequestBase {
    #[default]
    Zero,
    ReadResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChecksumDefinition {
    algorithm: String,
    start: usize,
    end_exclusive: usize,
    write_offset: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParsersFile {
    schema_version: u32,
    parsers: HashMap<String, ParserDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParserDefinition {
    #[serde(default)]
    valid_when: Vec<Condition>,
    #[serde(default)]
    fields: BTreeMap<String, FieldDefinition>,
    #[serde(default)]
    derived: BTreeMap<String, DerivedDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Condition {
    offset: usize,
    eq: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FieldDefinition {
    offset: usize,
    kind: String,
    count: Option<usize>,
    mask: Option<String>,
    #[serde(default)]
    invert: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BitmapEntry {
    bit: u8,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DerivedDefinition {
    kind: String,
    source: String,
    #[serde(default)]
    table: BTreeMap<String, Value>,
    #[serde(default)]
    bitmap: Vec<BitmapEntry>,
    /// `bit` derived kind: 从 source 字段（u64）提取指定位 → bool。
    /// 用于拆解 nvCaps/capabilities 位域为独立的 supports* 布尔字段。
    #[serde(default)]
    bit: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransportsFile {
    schema_version: u32,
    transports: HashMap<String, TransportDefinition>,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::enum_variant_names, clippy::large_enum_variant)]
// Variant names mirror the declarative transport schema.
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum TransportDefinition {
    HidFeature {
        report_id: u8,
        write_length: usize,
        read_length: usize,
        strip_report_id_on_read: bool,
        #[serde(default = "default_feature_delay_ms")]
        feature_delay_ms: u64,
        /// #8 超时统一治理：per-transport 超时声明（毫秒）。
        /// Host 强制 30s 上限，未声明时使用默认值。
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    HidFeatureProxy {
        base_transport: String,
        start_command: String,
        poll_command: String,
        set_length_command: String,
        read_command: String,
        send_ready: Condition,
        read_ready: Condition,
        online: Condition,
        status_parser: String,
        status_output: String,
        attempts: usize,
        delay_ms: u64,
        /// #8 超时统一治理：per-transport 超时声明（毫秒）。
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    HidOutputInput {
        report_id: u8,
        write_length: usize,
        read_length: usize,
        strip_report_id_on_read: bool,
        #[serde(default = "default_read_timeout_ms")]
        read_timeout_ms: i32,
        #[serde(default = "default_read_retries")]
        read_retries: u8,
        /// #8 超时统一治理：per-transport 超时声明（毫秒）。
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// AM35 RACE-style protocol over hidraw-compatible transports.
    /// Uses HID Output Report (ID 0x06) for writes and Input Report (ID 0x07)
    /// for reads, with a 3-byte framing header: [writeReportId, length, type].
    /// `race_type` is 0x00 for direct USB, 0x80 for receiver forwarding.
    /// Protocol data collected from AMasterDriver v1.0.6 reverse analysis;
    /// runtime execution is preparatory and pending hardware validation.
    HidRace {
        write_report_id: u8,
        read_report_id: u8,
        write_length: usize,
        read_length: usize,
        race_type: u8,
        strip_report_id_on_read: bool,
        #[serde(default)]
        read_mode: HidRaceReadMode,
        #[serde(default)]
        read_delay_ms: u64,
        #[serde(default = "default_read_timeout_ms")]
        read_timeout_ms: i32,
        #[serde(default = "default_read_retries")]
        read_retries: u8,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum HidRaceReadMode {
    #[default]
    Interrupt,
    InputReport,
}

fn default_read_timeout_ms() -> i32 {
    500
}

/// Default delay between sending a feature report and reading the response.
/// 10 ms matches the typical settling time for HID feature reports.
fn default_feature_delay_ms() -> u64 {
    10
}

/// Default number of read attempts for output/input exchanges before giving up.
/// 8 retries with the configured `read_timeout_ms` covers brief device stalls.
fn default_read_retries() -> u8 {
    8
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowsFile {
    schema_version: u32,
    workflows: HashMap<String, WorkflowDefinition>,
    #[serde(default)]
    mutations: HashMap<String, MutationDefinition>,
    /// #5 写事务与回滚：声明写事务边界和回滚策略。
    /// 事务期间的写操作在持锁状态下执行（依赖 #7 排队），
    /// 失败时按声明的回滚策略恢复板载配置。
    #[serde(default)]
    transactions: HashMap<String, TransactionDefinition>,
}

/// #5 写事务与回滚：事务边界声明。
/// 定义一组 mutation 的事务语义，包括快照点和回滚策略。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransactionDefinition {
    /// 事务包含的 mutation id 列表。
    mutations: Vec<String>,
    /// 快照 workflow id：事务开始前执行，读取板载配置作为回滚基准。
    snapshot_workflow: Option<String>,
    /// 回滚 workflow id：事务失败时执行，用快照数据恢复板载配置。
    rollback_workflow: Option<String>,
    /// 事务超时（毫秒），Host 强制 30s 上限。
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowDefinition {
    transport: String,
    steps: Vec<WorkflowStep>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowStep {
    command: String,
    parser: String,
    output: String,
    transport: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    #[serde(default)]
    param_candidates: BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    skip_if_zero: Vec<OutputReference>,
    /// Per-step failure policy. `abort` (default) preserves the historical
    /// behavior of aborting the whole workflow on the first error. `continue`
    /// records a `ReadStatus::Failed(reason)` for this step's output and lets
    /// subsequent steps run — useful for best-effort reads where one missing
    /// output (e.g. battery on a receiver-less device) must not block others.
    #[serde(default)]
    on_failure: StepFailurePolicy,
}

/// Per-step failure policy declared on `WorkflowStep`.
///
/// Brand-neutral: plugins opt into `continue` for best-effort read steps.
/// The default (`abort`) keeps backward compatibility with workflows written
/// before this field existed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepFailurePolicy {
    Abort,
    Continue,
}

impl Default for StepFailurePolicy {
    fn default() -> Self {
        Self::Abort
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OutputReference {
    output: String,
    field: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MutationDefinition {
    transport: String,
    inputs: BTreeMap<String, MutationInput>,
    #[serde(default)]
    skip_if_zero: Vec<OutputReference>,
    #[serde(default)]
    skip_if_non_zero: Vec<OutputReference>,
    #[serde(default)]
    skip_if_all_zero: Vec<OutputReference>,
    /// memory-only 路径跳过 writeCommand 的条件：任一引用为 0 则跳过。
    /// 用于设备无直写 feature（如 0x8070）但需走 memory 补丁的场景。
    #[serde(default)]
    write_skip_if_zero: Vec<OutputReference>,
    read: MutationCall,
    write_command: String,
    write_transport: Option<String>,
    #[serde(default)]
    write_params: BTreeMap<String, Value>,
    #[serde(default)]
    memory: Option<MemoryMutationDefinition>,
    #[serde(default)]
    post_writes: Vec<MutationWriteCall>,
    preserve_unknown: bool,
    #[serde(default)]
    settle_ms: u64,
    verify: MutationVerify,
    /// #8 超时统一治理：per-mutation 超时声明（毫秒）。
    /// 覆盖 transport 级别的 timeout_ms，Host 强制 30s 上限。
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemoryMutationDefinition {
    read_workflow: String,
    available_when: OutputReference,
    enabled_when: OutputCondition,
    #[serde(default)]
    required_when: Vec<OutputCondition>,
    size: OutputReference,
    chunk_output_prefix: String,
    chunk_field: String,
    chunk_size: usize,
    checksum: String,
    patches: Vec<ByteDefinition>,
    #[serde(default)]
    patch_params: BTreeMap<String, Value>,
    transport: String,
    start_command: String,
    chunk_command: String,
    end_transport: String,
    end_command: String,
    #[serde(default = "default_true")]
    end_expect_response: bool,
    #[serde(default)]
    context_params: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OutputCondition {
    output: String,
    field: String,
    #[serde(default)]
    eq: Option<Value>,
    #[serde(default)]
    ne: Option<Value>,
}

impl OutputCondition {
    /// 检查给定值是否满足条件。eq/ne 均为 Option，支持单独使用或组合。
    /// 当值缺失时返回 false；当 eq/ne 均为 None 时视为无约束（返回 true）。
    fn matches(&self, value: Option<&Value>) -> bool {
        match (value, &self.eq, &self.ne) {
            (Some(v), Some(eq), _) => v == eq,
            (Some(v), _, Some(ne)) => v != ne,
            (None, _, _) => false,
            (Some(_), None, None) => true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MutationInput {
    kind: String,
    min: Option<u64>,
    max: Option<u64>,
    step: Option<u64>,
    #[serde(default)]
    allowed: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MutationCall {
    command: String,
    parser: String,
    transport: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    #[serde(default)]
    skip_if_zero: Vec<OutputReference>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MutationWriteCall {
    command: String,
    transport: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    #[serde(default)]
    skip_if_zero: Vec<OutputReference>,
    #[serde(default)]
    settle_ms: u64,
    #[serde(default)]
    verify: Option<MutationVerify>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MutationVerify {
    command: String,
    parser: String,
    transport: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    #[serde(default)]
    skip_if_zero: Vec<OutputReference>,
    assertions: Vec<MutationAssertion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MutationAssertion {
    field: String,
    param: String,
    index_param: Option<String>,
    #[serde(default)]
    index_base: i64,
}

/// 工作流投影：从完整工作流中选取生成目标 output 所需的最小步骤子集。
///
/// 投影在宿主运行时内部完成，不修改插件包内容，不影响签名验证。
/// UI 不直接提供原始 output 名称，而是通过 `SemanticField` 声明语义需求，
/// 由宿主映射为目标 output 后计算投影。
#[derive(Debug, Clone)]
pub struct WorkflowProjection {
    /// 选中的 step 索引（按原始顺序排列）。
    selected_steps: Vec<usize>,
    /// 投影请求的 output 名称集合（内部使用，不暴露给 UI）。
    requested_outputs: BTreeSet<String>,
    /// 投影失败时的回退原因。`None` 表示投影成功。
    fallback_reason: Option<String>,
}

impl WorkflowProjection {
    /// 投影是否成功（有选中的步骤且无回退原因）。
    pub fn is_valid(&self) -> bool {
        !self.selected_steps.is_empty() && self.fallback_reason.is_none()
    }

    /// 返回回退原因（如果有）。
    pub fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    /// 返回选中步骤的数量。
    pub fn selected_step_count(&self) -> usize {
        self.selected_steps.len()
    }

    /// 返回请求的 output 名称集合（用于诊断）。
    pub fn requested_outputs(&self) -> &BTreeSet<String> {
        &self.requested_outputs
    }

    /// 返回选中步骤的索引（用于诊断和测试）。
    pub fn selected_steps(&self) -> &[usize] {
        &self.selected_steps
    }
}

/// 计算单个 workflow step 依赖的所有 output 名称。
///
/// 依赖来源：
/// - `params` 中的 `fromOutput` 引用
/// - `skip_if_zero` 中的 `OutputReference`
/// - `param_candidates` 中的 `fromOutput` 引用
fn step_dependencies(step: &WorkflowStep) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();

    // params 中的 fromOutput 引用
    for value in step.params.values() {
        if let Some(reference) = value.as_object() {
            if reference
                .keys()
                .all(|key| matches!(key.as_str(), "fromOutput" | "field" | "subtract"))
            {
                if let Some(output) = reference.get("fromOutput").and_then(Value::as_str) {
                    deps.insert(output.to_string());
                }
            }
        }
    }

    // skip_if_zero 中的引用
    for reference in &step.skip_if_zero {
        deps.insert(reference.output.clone());
    }

    // param_candidates 中的引用
    for candidates in step.param_candidates.values() {
        for value in candidates {
            if let Some(reference) = value.as_object() {
                if reference
                    .keys()
                    .all(|key| matches!(key.as_str(), "fromOutput" | "field" | "subtract"))
                {
                    if let Some(output) = reference.get("fromOutput").and_then(Value::as_str) {
                        deps.insert(output.to_string());
                    }
                }
            }
        }
    }

    deps
}

pub struct ProtocolPackage {
    commands: CommandsFile,
    parsers: ParsersFile,
    transports: TransportsFile,
    workflows: WorkflowsFile,
    capabilities: Option<Value>,
    /// 预编译的 derived lookup 表：parser_id → derived_name → (numeric_source → value)。
    /// 将 parsers.json 中的字符串键（"0x01"/"1"）在加载时解析为 u64，
    /// parse_response 时直接用数值键查找，避免每次格式化字符串。
    compiled_lookups: HashMap<String, HashMap<String, HashMap<u64, Value>>>,
    /// 预构建的无 param 命令 report 字节：command_id → report。
    /// 仅缓存 RequestBase::Zero 且所有 byte 定义都无 param/indexed_by 的命令。
    /// build_command 时命中缓存直接返回，避免重复构建 + checksum 计算。
    compiled_commands: HashMap<String, Vec<u8>>,
    /// Full 读取验证过的语义字段最佳 output。随 ProtocolPackage 生命周期
    /// 自动失效，不写入插件包、不影响签名。
    pub(crate) semantic_output_cache: Mutex<HashMap<String, BTreeMap<String, BTreeSet<String>>>>,
}

impl ProtocolPackage {
    pub fn from_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Self, String> {
        fn parse<T: for<'de> Deserialize<'de>>(
            files: &BTreeMap<String, Vec<u8>>,
            path: &str,
        ) -> Result<T, String> {
            let bytes = files.get(path).ok_or_else(|| format!("missing {path}"))?;
            serde_json::from_slice(bytes).map_err(|error| format!("invalid {path}: {error}"))
        }
        fn parse_optional<T: for<'de> Deserialize<'de>>(
            files: &BTreeMap<String, Vec<u8>>,
            path: &str,
        ) -> Result<Option<T>, String> {
            match files.get(path) {
                Some(bytes) => serde_json::from_slice(bytes)
                    .map_err(|error| format!("invalid {path}: {error}")),
                None => Ok(None),
            }
        }

        let features: Option<FeaturesFile> = parse_optional(files, "protocol/features.json")?;
        let feature_map = features.map(|file| file.features).unwrap_or_default();

        let mut workflows: WorkflowsFile = parse(files, "protocol/workflows.json")?;
        expand_feature_refs(&feature_map, &mut workflows)?;

        let capabilities: Option<Value> = parse_optional(files, "capabilities.json")?;

        let mut package = Self {
            commands: parse(files, "protocol/commands.json")?,
            parsers: parse(files, "protocol/parsers.json")?,
            transports: parse(files, "protocol/transports.json")?,
            workflows,
            capabilities,
            compiled_lookups: HashMap::new(),
            compiled_commands: HashMap::new(),
            semantic_output_cache: Mutex::new(HashMap::new()),
        };
        if package.commands.schema_version != 1
            || package.parsers.schema_version != 1
            || package.transports.schema_version != 1
            || package.workflows.schema_version != 1
        {
            return Err("unsupported protocol schema version".into());
        }
        package.compile_lookups();
        package.compile_commands();
        Ok(package)
    }

    /// 将 parsers.json 中 derived lookup 的字符串键（"0x01"/"1"）预编译为 u64 键。
    /// 在 from_files 后调用，结果存储在 compiled_lookups 中供 parse_response 使用。
    fn compile_lookups(&mut self) {
        for (parser_id, parser) in &self.parsers.parsers {
            for (derived_name, derived) in &parser.derived {
                if derived.kind != "lookup" || derived.table.is_empty() {
                    continue;
                }
                let mut compiled: HashMap<u64, Value> = HashMap::with_capacity(derived.table.len());
                for (key, value) in &derived.table {
                    if let Some(numeric) = parse_lookup_key(key) {
                        compiled.insert(numeric, value.clone());
                    }
                }
                if !compiled.is_empty() {
                    self.compiled_lookups
                        .entry(parser_id.clone())
                        .or_default()
                        .insert(derived_name.clone(), compiled);
                }
            }
        }
    }

    /// 预构建无 param 依赖的命令 report 字节。
    /// 仅缓存 RequestBase::Zero 且所有 byte 定义都无 param/indexed_by 的命令，
    /// 这些命令的 report 字节在每次调用时完全相同，缓存后 build_command 直接返回。
    fn compile_commands(&mut self) {
        let empty_params: BTreeMap<String, Value> = BTreeMap::new();
        for (id, command) in &self.commands.commands {
            // 仅缓存 Zero base（ReadResponse 需要运行时 base 数据）
            if command.request.base != RequestBase::Zero {
                continue;
            }
            let cacheable = command
                .request
                .bytes
                .iter()
                .all(|byte| byte.param.is_none() && byte.indexed_by.is_none());
            if !cacheable {
                continue;
            }
            // 预构建并缓存（忽略错误，运行时 build_command 会重新构建并报错）
            if let Ok(report) = self.build_command(id, &empty_params, None) {
                self.compiled_commands.insert(id.clone(), report);
            }
        }
    }

    /// 解析插件包文件，可选地应用型号覆盖。
    ///
    /// 型号覆盖是模式 C 的核心：`models/<model>/` 目录下的 JSON 文件
    /// 与父插件对应文件做 deep merge，型号文件的字段覆盖父插件。
    /// 这允许同一协议族插件为不同型号提供差异化配置（如不同的 DPI 范围、
    /// 灯光区域数等），而无需复制整个协议文件。
    ///
    /// `model` 为 None 或空字符串时，等价于 `from_files`（向后兼容）。
    pub fn from_files_with_model(
        files: &BTreeMap<String, Vec<u8>>,
        model: Option<&str>,
    ) -> Result<Self, String> {
        let model = match model {
            Some(m) if !m.is_empty() => m,
            _ => return Self::from_files(files),
        };
        // 防御性校验：model 不含路径分隔符，防止 models/<model>/ 路径拼接越界。
        // model 来自 devices.json 的 hardware_verified_models（插件作者控制），
        // BTreeMap 查找是精确字符串匹配，不含 '/' 即无法跳出 models/ 目录。
        if model.contains('/') || model.contains('\\') {
            return Err(format!(
                "invalid model name containing path separators: {model}"
            ));
        }

        // 型号覆盖文件路径与父插件路径的对应关系。
        // models/<model>/protocol/commands.json → protocol/commands.json
        const MERGE_PATHS: &[&str] = &[
            "protocol/commands.json",
            "protocol/parsers.json",
            "protocol/transports.json",
            "protocol/workflows.json",
            "protocol/features.json",
            "capabilities.json",
        ];

        let mut merged_files = files.clone();
        for path in MERGE_PATHS {
            let model_path = format!("models/{model}/{path}");
            if let Some(model_bytes) = files.get(&model_path) {
                let merged_bytes = match files.get(*path) {
                    Some(base_bytes) => {
                        let base: Value = serde_json::from_slice(base_bytes)
                            .map_err(|e| format!("invalid {path}: {e}"))?;
                        let overlay: Value = serde_json::from_slice(model_bytes)
                            .map_err(|e| format!("invalid {model_path}: {e}"))?;
                        let merged = deep_merge_json(base, overlay);
                        serde_json::to_vec(&merged)
                            .map_err(|e| format!("serialize merged {path}: {e}"))?
                    }
                    None => model_bytes.clone(),
                };
                merged_files.insert(path.to_string(), merged_bytes);
            }
        }

        Self::from_files(&merged_files)
    }

    /// Parse with optional model overrides and dependency transport reuse.
    /// Dependency packages may contribute only `protocol/transports.json`;
    /// main-plugin definitions win on key conflicts.
    pub fn from_files_with_model_and_dependencies(
        files: &BTreeMap<String, Vec<u8>>,
        model: Option<&str>,
        dependency_files: &[&BTreeMap<String, Vec<u8>>],
    ) -> Result<Self, String> {
        let mut merged_files = files.clone();
        for dependency in dependency_files {
            let (Some(dependency_transports), Some(current_transports)) = (
                dependency.get("protocol/transports.json"),
                merged_files.get("protocol/transports.json"),
            ) else {
                continue;
            };
            let dependency_json: Value = serde_json::from_slice(dependency_transports)
                .map_err(|error| format!("invalid dependency protocol/transports.json: {error}"))?;
            let current_json: Value = serde_json::from_slice(current_transports)
                .map_err(|error| format!("invalid protocol/transports.json: {error}"))?;
            let merged = deep_merge_json(dependency_json, current_json);
            let bytes = serde_json::to_vec(&merged)
                .map_err(|error| format!("serialize dependency transports: {error}"))?;
            merged_files.insert("protocol/transports.json".into(), bytes);
        }
        Self::from_files_with_model(&merged_files, model)
    }

    pub fn capabilities(&self) -> Option<&Value> {
        self.capabilities.as_ref()
    }

    pub fn has_workflow(&self, workflow_id: &str) -> bool {
        self.workflows.workflows.contains_key(workflow_id)
    }

    /// 返回指定工作流中所有 step 的 output 名称集合。
    /// 用于语义字段映射时检查目标 output 是否存在。
    pub fn available_outputs(&self, workflow_id: &str) -> BTreeSet<String> {
        self.workflows
            .workflows
            .get(workflow_id)
            .map(|w| w.steps.iter().map(|s| s.output.clone()).collect())
            .unwrap_or_default()
    }

    pub fn execute(
        &self,
        api: &HidApi,
        path: &str,
        workflow_id: &str,
    ) -> Result<BTreeMap<String, Value>, String> {
        self.execute_with_initial_outputs(
            api,
            path,
            workflow_id,
            BTreeMap::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .map(|(outputs, _)| outputs)
    }

    /// Like `execute` but with an optional feature index cache.
    /// When cache is provided, `root-get-feature` steps that hit the cache are
    /// skipped (the cached feature index is inserted directly into outputs).
    /// Misses are populated after execution for future calls.
    ///
    /// Returns both the workflow outputs and a per-output `ReadStatus` map.
    /// The status map records `Ok`/`Skipped`/`Failed` for each executed step,
    /// letting the host distinguish skipped/failed reads from absent outputs.
    pub fn execute_with_cache(
        &self,
        api: &HidApi,
        path: &str,
        workflow_id: &str,
        cache: Option<&Mutex<FeatureIndexCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
    ) -> Result<(BTreeMap<String, Value>, BTreeMap<String, ReadStatus>), String> {
        self.execute_with_initial_outputs(
            api,
            path,
            workflow_id,
            BTreeMap::new(),
            None,
            None,
            cache,
            cached_handles,
            hid_io_stats,
            None,
            None,
        )
    }

    /// 与 `execute_with_cache` 相同，但接收 HID 交换事件回调。
    /// 宿主通过实现 `HidEventSink` trait 接收 HID 交换、忙碌重试、响应不匹配等事件。
    pub fn execute_with_cache_and_sink(
        &self,
        api: &HidApi,
        path: &str,
        workflow_id: &str,
        feature_index_cache: Option<&Mutex<FeatureIndexCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
        sink: &dyn HidEventSink,
    ) -> Result<(BTreeMap<String, Value>, BTreeMap<String, ReadStatus>), String> {
        self.execute_with_initial_outputs(
            api,
            path,
            workflow_id,
            BTreeMap::new(),
            None,
            None,
            feature_index_cache,
            cached_handles,
            hid_io_stats,
            None,
            Some(sink),
        )
    }

    /// 根据目标 output 集合计算工作流投影。
    ///
    /// 给定一个 workflow_id 和目标 output 名称集合，返回生成这些 output
    /// 所需的最小 step 子集（包含递归依赖）。
    ///
    /// 投影在宿主运行时内部完成，不修改插件包内容，不影响签名验证。
    /// UI 不直接调用此方法，而是通过宿主语义字段映射后间接使用。
    pub fn compute_projection(
        &self,
        workflow_id: &str,
        target_outputs: &BTreeSet<String>,
    ) -> WorkflowProjection {
        let workflow = match self.workflows.workflows.get(workflow_id) {
            Some(w) => w,
            None => {
                return WorkflowProjection {
                    selected_steps: Vec::new(),
                    requested_outputs: target_outputs.clone(),
                    fallback_reason: Some(format!("missing workflow {workflow_id}")),
                }
            }
        };

        let steps = &workflow.steps;

        let available_outputs: BTreeSet<&String> = steps.iter().map(|s| &s.output).collect();

        let mut valid_targets: BTreeSet<String> = BTreeSet::new();
        let mut missing_targets: BTreeSet<String> = BTreeSet::new();
        for target in target_outputs {
            if available_outputs.contains(target) {
                valid_targets.insert(target.clone());
            } else {
                missing_targets.insert(target.clone());
            }
        }

        if valid_targets.is_empty() {
            return WorkflowProjection {
                selected_steps: Vec::new(),
                requested_outputs: target_outputs.clone(),
                fallback_reason: Some(format!("no target outputs found in workflow {workflow_id}")),
            };
        }

        // 第一步：找到直接产出目标 output 的 step
        let mut selected: BTreeSet<usize> = BTreeSet::new();
        for (i, step) in steps.iter().enumerate() {
            if valid_targets.contains(&step.output) {
                selected.insert(i);
            }
        }

        // 第二步：递归计算依赖闭包，重复直到不再变化。
        let mut changed = true;
        while changed {
            changed = false;
            for (i, step) in steps.iter().enumerate() {
                if selected.contains(&i) {
                    let deps = step_dependencies(step);
                    for dep_output in deps {
                        for (j, dep_step) in steps.iter().enumerate() {
                            if dep_step.output == dep_output && !selected.contains(&j) {
                                selected.insert(j);
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        let fallback_reason = if missing_targets.is_empty() {
            None
        } else {
            Some(format!(
                "some target outputs not found in workflow: {}",
                missing_targets
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        };

        WorkflowProjection {
            selected_steps: selected.into_iter().collect(),
            requested_outputs: target_outputs.clone(),
            fallback_reason,
        }
    }

    /// 执行投影后的工作流。
    ///
    /// 与 `execute_with_cache` 类似，但只执行 `projection.selected_steps` 中的 step。
    /// 复用现有的 feature index 缓存、HID 句柄缓存、超时和报告数量限制。
    ///
    /// 返回 workflow outputs 和 per-output `ReadStatus` map（与
    /// `execute_with_cache` 一致），供宿主填充 `DeviceSnapshot.read_statuses`。
    #[allow(clippy::too_many_arguments)]
    pub fn execute_projection_with_cache(
        &self,
        api: &HidApi,
        path: &str,
        workflow_id: &str,
        projection: &WorkflowProjection,
        cache: Option<&Mutex<FeatureIndexCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
    ) -> Result<(BTreeMap<String, Value>, BTreeMap<String, ReadStatus>), String> {
        self.execute_with_initial_outputs(
            api,
            path,
            workflow_id,
            BTreeMap::new(),
            None,
            None,
            cache,
            cached_handles,
            hid_io_stats,
            Some(&projection.selected_steps),
            None,
        )
    }

    /// 与 `execute_projection_with_cache` 相同，但接收 HID 交换事件回调。
    pub fn execute_projection_with_cache_and_sink(
        &self,
        api: &HidApi,
        path: &str,
        workflow_id: &str,
        projection: &WorkflowProjection,
        cache: Option<&Mutex<FeatureIndexCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
        sink: &dyn HidEventSink,
    ) -> Result<(BTreeMap<String, Value>, BTreeMap<String, ReadStatus>), String> {
        self.execute_with_initial_outputs(
            api,
            path,
            workflow_id,
            BTreeMap::new(),
            None,
            None,
            cache,
            cached_handles,
            hid_io_stats,
            Some(&projection.selected_steps),
            Some(sink),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_with_initial_outputs(
        &self,
        api: &HidApi,
        path: &str,
        workflow_id: &str,
        initial_outputs: BTreeMap<String, Value>,
        timeout_ms: Option<u64>,
        inherited_deadline: Option<Instant>,
        feature_index_cache: Option<&Mutex<FeatureIndexCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
        selected_steps: Option<&[usize]>,
        event_sink: Option<&dyn HidEventSink>,
    ) -> Result<(BTreeMap<String, Value>, BTreeMap<String, ReadStatus>), String> {
        let workflow = self
            .workflows
            .workflows
            .get(workflow_id)
            .ok_or_else(|| format!("missing workflow {workflow_id}"))?;
        if workflow.steps.len() > MAX_COMMANDS {
            return Err("workflow command limit exceeded".into());
        }
        let c_path = CString::new(path).map_err(|_| "invalid HID path".to_string())?;
        let device = take_or_open_device(api, &c_path, path, cached_handles, hid_io_stats)?;
        let mut session = Session {
            package: self,
            device,
            reports: 0,
            delay_ms: 0,
            outputs: initial_outputs,
            deadline: merge_deadline(inherited_deadline, timeout_ms)?,
            event_sink,
        };
        let mut read_statuses: BTreeMap<String, ReadStatus> = BTreeMap::new();
        for (index, step) in workflow.steps.iter().enumerate() {
            // 工作流投影：只执行选中的 step，跳过未选中的。
            // selected_steps 为 None 时执行所有 step（完整工作流）。
            if let Some(steps) = selected_steps {
                if !steps.contains(&index) {
                    continue;
                }
            }
            if step
                .skip_if_zero
                .iter()
                .any(|reference| output_reference_is_zero(&session.outputs, reference))
            {
                read_statuses.insert(step.output.clone(), ReadStatus::Skipped);
                continue;
            }
            let params = match resolve_workflow_params(&step.params, &session.outputs) {
                Ok(params) => params,
                Err(error) => {
                    let reason =
                        format!("workflow {workflow_id} step {} params: {error}", index + 1);
                    if step.on_failure == StepFailurePolicy::Continue {
                        read_statuses.insert(step.output.clone(), ReadStatus::Failed(reason));
                        continue;
                    }
                    // Abort: drop session (closes handle, not returned to cache).
                    return Err(reason);
                }
            };

            // Feature index 缓存：root-get-feature 命令命中缓存时跳过 HID 往返
            if step.command == "root-get-feature" {
                if let Some(cache) = feature_index_cache {
                    if let Some(feature_id) = params.get("featureId").and_then(Value::as_u64) {
                        let cache_hit = cache.lock().ok().and_then(|guard| {
                            guard
                                .get(path)
                                .and_then(|device_cache| device_cache.get(&(feature_id as u16)))
                                .cloned()
                        });
                        if let Some(cached_output) = cache_hit {
                            // 恢复完整的 parsed output（含 featureIndex、deviceIndex、connection 等），
                            // 避免后续 step 引用 device.deviceIndex 时报 "missing output reference"。
                            session.outputs.insert(step.output.clone(), cached_output);
                            read_statuses.insert(step.output.clone(), ReadStatus::Ok);
                            continue;
                        }
                    }
                }
            }

            let transport = step.transport.as_deref().unwrap_or(&workflow.transport);
            let response = match execute_with_candidates(
                &mut session,
                transport,
                &step.command,
                &params,
                &step.param_candidates,
            ) {
                Ok(response) => response,
                Err(error) => {
                    let reason = format!(
                        "workflow {workflow_id} step {} command {}: {error}",
                        index + 1,
                        step.command
                    );
                    if step.on_failure == StepFailurePolicy::Continue {
                        read_statuses.insert(step.output.clone(), ReadStatus::Failed(reason));
                        continue;
                    }
                    return Err(reason);
                }
            };
            let parsed = match self.parse_response(&step.parser, &response) {
                Ok(parsed) => parsed,
                Err(error) => {
                    let reason = format!(
                        "workflow {workflow_id} step {} parser {}: {error}",
                        index + 1,
                        step.parser
                    );
                    if step.on_failure == StepFailurePolicy::Continue {
                        read_statuses.insert(step.output.clone(), ReadStatus::Failed(reason));
                        continue;
                    }
                    return Err(reason);
                }
            };
            session.outputs.insert(step.output.clone(), parsed);
            read_statuses.insert(step.output.clone(), ReadStatus::Ok);

            // 缓存 root-get-feature 的完整 parsed output 供后续轮询使用。
            // 存储 complete Value（含 deviceIndex、connection 等）而非仅 featureIndex，
            // 确保缓存命中时后续 step 能正确引用所有 derived 字段。
            if step.command == "root-get-feature" {
                if let Some(cache) = feature_index_cache {
                    if let (Some(feature_id), Some(parsed_output)) = (
                        params.get("featureId").and_then(Value::as_u64),
                        session.outputs.get(&step.output).cloned(),
                    ) {
                        if parsed_output.get("featureIndex").is_some() {
                            if let Ok(mut guard) = cache.lock() {
                                guard
                                    .entry(path.to_string())
                                    .or_insert_with(HashMap::new)
                                    .insert(feature_id as u16, parsed_output);
                            }
                        }
                    }
                }
            }
        }
        let Session {
            device,
            reports,
            outputs,
            ..
        } = session;
        if let Some(stats) = hid_io_stats {
            if let Ok(mut guard) = stats.lock() {
                guard.record_reports_executed(reports);
            }
        }
        return_device(path, device, cached_handles, hid_io_stats);
        Ok((outputs, read_statuses))
    }

    pub fn mutation_ids(
        &self,
        family: &str,
        ctx_outputs: Option<&BTreeMap<String, Value>>,
    ) -> Vec<String> {
        let prefix = format!("{family}-");
        let mut ids: Vec<_> = self
            .workflows
            .mutations
            .iter()
            .filter(|(_id, mutation)| {
                ctx_outputs.is_none_or(|outputs| mutation_available(mutation, outputs))
            })
            .map(|(id, _mutation)| id)
            .filter_map(|id| id.strip_prefix(&prefix).map(str::to_owned))
            .collect();
        ids.sort();
        ids
    }

    fn read_memory_mutation(
        &self,
        api: &HidApi,
        path: &str,
        definition: &MemoryMutationDefinition,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
    ) -> Result<(BTreeMap<String, Value>, Vec<u8>), String> {
        let (outputs, _) = self.execute_with_initial_outputs(
            api,
            path,
            &definition.read_workflow,
            BTreeMap::new(),
            None,
            None,
            None,
            cached_handles,
            hid_io_stats,
            None,
            None,
        )?;
        let size = output_value(&outputs, &definition.size)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|size| (32..=4096).contains(size))
            .ok_or_else(|| "memory workflow returned an invalid size".to_string())?;
        if definition.chunk_size == 0 || definition.chunk_size > 64 {
            return Err("memory mutation chunk size is invalid".into());
        }
        let count = size.div_ceil(definition.chunk_size);
        let mut memory = vec![0u8; size];
        // Pre-allocate the chunk key buffer and reuse it across iterations to
        // avoid per-iteration String allocations.
        use std::fmt::Write;
        let mut key = String::with_capacity(definition.chunk_output_prefix.len() + 2);
        for index in 0..count {
            key.clear();
            write!(&mut key, "{}{index:02}", definition.chunk_output_prefix).unwrap();
            let bytes = outputs
                .get(&key)
                .and_then(Value::as_object)
                .and_then(|object| object.get(&definition.chunk_field))
                .and_then(Value::as_array)
                .ok_or_else(|| format!("memory workflow is missing {key}"))?;
            // Use explicit [start, end) bounds so the final partial chunk does not
            // overlap with the previous chunk. The old `offset.min(size - chunk_size)`
            // formula rewrote the tail of the previous chunk when `size` was not a
            // multiple of `chunk_size`.
            let start = index * definition.chunk_size;
            let end = (start + definition.chunk_size).min(size);
            for (target, byte) in memory[start..end]
                .iter_mut()
                .zip(bytes.iter().filter_map(Value::as_u64))
            {
                *target = u8::try_from(byte)
                    .map_err(|_| format!("memory workflow {key} contains an invalid byte"))?;
            }
        }
        // libratbag tolerates sector CRC mismatches (logs debug, continues using
        // the sector data). Some devices report a sectorSize that doesn't match
        // the actual on-wire sector layout, causing a stored/calculated CRC
        // divergence. The memory patch only touches specific byte offsets and
        // the post-write readback verifies the actual bytes, so a pre-existing
        // CRC mismatch is not fatal — log a warning and proceed.
        if let Err(error) = verify_memory_checksum(&memory, &definition.checksum) {
            eprintln!("[mira] warning: {error} — continuing with memory patch");
        }
        Ok((outputs, memory))
    }

    fn execute_memory_mutation(
        &self,
        mutation_id: &str,
        definition: &MemoryMutationDefinition,
        params: &BTreeMap<String, Value>,
        session: &mut Session<'_>,
        original: &[u8],
    ) -> Result<Vec<u8>, String> {
        let mut patch_params = params.clone();
        patch_params.extend(
            resolve_workflow_params(&definition.patch_params, &session.outputs)
                .map_err(|error| format!("mutation {mutation_id} memory patch params: {error}"))?,
        );
        let mut updated = original.to_vec();
        for patch in &definition.patches {
            apply_byte_definition(mutation_id, patch, &patch_params, &mut updated)?;
        }
        write_memory_checksum(&mut updated, &definition.checksum)?;
        if updated == original {
            return Ok(updated);
        }

        let mut context = resolve_workflow_params(&definition.context_params, &session.outputs)
            .map_err(|error| format!("mutation {mutation_id} memory params: {error}"))?;
        context.insert("count".into(), Value::from(updated.len()));
        session
            .execute_command(
                &definition.transport,
                &definition.start_command,
                &context,
                true,
                None,
                None,
            )
            .map_err(|error| format!("mutation {mutation_id} memory write start: {error}"))?;
        for (index, chunk) in updated.chunks(definition.chunk_size).enumerate() {
            let mut chunk_params = context.clone();
            chunk_params.insert(
                "data".into(),
                Value::Array(chunk.iter().copied().map(Value::from).collect()),
            );
            session
                .execute_command(
                    &definition.transport,
                    &definition.chunk_command,
                    &chunk_params,
                    true,
                    None,
                    None,
                )
                .map_err(|error| format!("mutation {mutation_id} memory chunk {index}: {error}"))?;
        }
        session
            .execute_command(
                &definition.end_transport,
                &definition.end_command,
                &context,
                definition.end_expect_response,
                None,
                None,
            )
            .map_err(|error| format!("mutation {mutation_id} memory write end: {error}"))?;
        Ok(updated)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mutate(
        &self,
        api: &HidApi,
        path: &str,
        mutation_id: &str,
        params: &Map<String, Value>,
        ctx_outputs: &BTreeMap<String, Value>,
        onboard_memory_cache: Option<&Mutex<OnboardMemoryCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
    ) -> Result<Value, String> {
        if let Some(transaction) = self.transaction_for_mutation(mutation_id)? {
            return self.mutate_in_transaction(
                api,
                path,
                mutation_id,
                params,
                ctx_outputs,
                transaction,
                onboard_memory_cache,
                cached_handles,
                hid_io_stats,
            );
        }
        self.mutate_inner(
            api,
            path,
            mutation_id,
            params,
            ctx_outputs,
            None,
            onboard_memory_cache,
            cached_handles,
            hid_io_stats,
        )
    }

    /// 查找声明了目标 mutation 的事务。
    ///
    /// 匹配规则：事务 `mutations` 列表中的 id 必须与 `mutation_id` **精确相等**。
    /// 早期实现用 `ends_with("-{id}")` 做宽松后缀匹配，但这会让
    /// `"fake-set-dpi"` 误匹配声明 `"set-dpi"` 的事务，触发非预期的
    /// snapshot/rollback。事务应声明完整的 mutation id。
    ///
    /// 若多个事务声明了同一 mutation，返回错误——一个 mutation 只能属于一个事务，
    /// 否则 snapshot/rollback 的选择将不确定。
    fn transaction_for_mutation(
        &self,
        mutation_id: &str,
    ) -> Result<Option<TransactionDefinition>, String> {
        let matches: Vec<&TransactionDefinition> = self
            .workflows
            .transactions
            .values()
            .filter(|transaction| transaction.mutations.iter().any(|id| id == mutation_id))
            .collect();
        match matches.len() {
            0 => Ok(None),
            1 => Ok(Some(matches[0].clone())),
            _ => Err(format!(
                "mutation {mutation_id} is declared in multiple transactions; a mutation may belong to at most one transaction"
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mutate_in_transaction(
        &self,
        api: &HidApi,
        path: &str,
        mutation_id: &str,
        params: &Map<String, Value>,
        ctx_outputs: &BTreeMap<String, Value>,
        transaction: TransactionDefinition,
        onboard_memory_cache: Option<&Mutex<OnboardMemoryCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
    ) -> Result<Value, String> {
        // 事务级 timeout 是整个事务（snapshot + mutation + rollback）的总预算：
        // 三阶段共享同一 deadline，而非各自独立计时（否则总时间可达 3×timeout）。
        let tx_deadline = timeout_deadline(transaction.timeout_ms)?;
        let snapshot_outputs = if let Some(workflow) = &transaction.snapshot_workflow {
            let (outputs, _) = self.execute_with_initial_outputs(
                api,
                path,
                workflow,
                ctx_outputs.clone(),
                None,
                tx_deadline,
                None,
                cached_handles,
                hid_io_stats,
                None,
                None,
            )?;
            outputs
        } else {
            ctx_outputs.clone()
        };
        match self.mutate_inner(
            api,
            path,
            mutation_id,
            params,
            ctx_outputs,
            tx_deadline,
            onboard_memory_cache,
            cached_handles,
            hid_io_stats,
        ) {
            Ok(value) => Ok(value),
            Err(error) => {
                // #5 事务可观测性：错误信息包含 snapshot/rollback workflow 名称，
                // 让用户通过通知了解事务执行详情（复用前端 notifyError）。
                if let Some(rollback_workflow) = &transaction.rollback_workflow {
                    match self.execute_with_initial_outputs(
                        api,
                        path,
                        rollback_workflow,
                        snapshot_outputs,
                        None,
                        tx_deadline,
                        None,
                        cached_handles,
                        hid_io_stats,
                        None,
                        None,
                    ) {
                        Ok((_outputs, _statuses)) => Err(format!(
                            "写入 {mutation_id} 失败：{error}。事务回滚已执行（回滚工作流：{rollback_workflow}），设备已恢复至写入前状态。"
                        )),
                        Err(rollback_error) => Err(format!(
                            "写入 {mutation_id} 失败：{error}。事务回滚失败（回滚工作流：{rollback_workflow}）：{rollback_error}。设备可能处于不一致状态，请重新读取设备。"
                        )),
                    }
                } else if let Some(snapshot_workflow) = &transaction.snapshot_workflow {
                    Err(format!(
                        "写入 {mutation_id} 失败：{error}。该事务声明了快照（{snapshot_workflow}）但无回滚策略。"
                    ))
                } else {
                    Err(error)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mutate_inner(
        &self,
        api: &HidApi,
        path: &str,
        mutation_id: &str,
        params: &Map<String, Value>,
        ctx_outputs: &BTreeMap<String, Value>,
        inherited_deadline: Option<Instant>,
        onboard_memory_cache: Option<&Mutex<OnboardMemoryCache>>,
        cached_handles: Option<&Mutex<HidHandleCache>>,
        hid_io_stats: Option<&Mutex<HidIoStats>>,
    ) -> Result<Value, String> {
        let mutation = self
            .workflows
            .mutations
            .get(mutation_id)
            .ok_or_else(|| format!("missing mutation {mutation_id}"))?;
        if !mutation_available(mutation, ctx_outputs) {
            return Err(format!(
                "mutation {mutation_id} is not available on this device"
            ));
        }
        let params = validate_mutation_inputs(&mutation.inputs, params)?;
        // Validate settle_ms before any I/O. The previous position (after the
        // write command) allowed the device to be modified before the error
        // was returned, skipping verification.
        if mutation.settle_ms > 1_000 {
            return Err(format!("mutation {mutation_id} settle delay exceeds limit"));
        }
        // Validate write strategy against command template before any I/O.
        // The previous position (after the pre-read) allowed the device to be
        // read (and memory to be read) before the contract mismatch was
        // detected, wasting I/O and potentially leaving the device in an
        // inconsistent state.
        let write_command = self
            .commands
            .commands
            .get(&mutation.write_command)
            .ok_or_else(|| format!("missing command {}", mutation.write_command))?;
        let preserves_response = write_command.request.base == RequestBase::ReadResponse;
        if preserves_response != mutation.preserve_unknown {
            return Err(format!(
                "mutation {mutation_id} write strategy does not match its command template"
            ));
        }
        let direct_write_skipped = mutation
            .write_skip_if_zero
            .iter()
            .any(|reference| output_reference_is_zero(ctx_outputs, reference));
        let mut memory_read = match &mutation.memory {
            Some(memory)
                if output_value(ctx_outputs, &memory.available_when)
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    != 0 =>
            {
                // Always pre-read live device memory before read-modify-write.
                // A cached sector can become stale when the user changes onboard
                // profiles from hardware controls or another app during the same
                // connection, and using it here would overwrite those changes.
                match self.read_memory_mutation(api, path, memory, cached_handles, hid_io_stats) {
                    Ok((outputs, bytes)) => {
                        let enabled = memory.enabled_when.matches(output_value(
                            &outputs,
                            &OutputReference {
                                output: memory.enabled_when.output.clone(),
                                field: memory.enabled_when.field.clone(),
                            },
                        )) && memory.required_when.iter().all(|condition| {
                            condition.matches(output_value(
                                &outputs,
                                &OutputReference {
                                    output: condition.output.clone(),
                                    field: condition.field.clone(),
                                },
                            ))
                        });
                        enabled.then_some((outputs, bytes))
                    }
                    Err(error) if is_memory_checksum_mismatch(&error) && !direct_write_skipped => {
                        // If the onboard profile sector is already corrupt or was
                        // changed by another tool, do not patch unknown bytes.
                        // Continue with the standard HID++ write path when the
                        // device exposes one; verification below still proves
                        // whether the live device accepted the value.
                        eprintln!(
                            "[mira] skipping onboard memory patch for {mutation_id}: {error}"
                        );
                        None
                    }
                    Err(error) => return Err(error),
                }
            }
            _ => None,
        };
        let c_path = CString::new(path).map_err(|_| "invalid HID path".to_string())?;
        let device = take_or_open_device(api, &c_path, path, cached_handles, hid_io_stats)?;
        let mut session = Session {
            package: self,
            device,
            reports: 0,
            delay_ms: 0,
            outputs: {
                let mut outputs = ctx_outputs.clone();
                // Move memory outputs into session (avoids cloning the BTreeMap).
                // std::mem::take leaves an empty map; later code only accesses
                // the Vec<u8> bytes for verification, not the outputs.
                if let Some((memory_outputs, _)) = &mut memory_read {
                    outputs.extend(std::mem::take(memory_outputs));
                }
                outputs
            },
            deadline: merge_deadline(inherited_deadline, mutation.timeout_ms)?,
            event_sink: None,
        };
        // mutation 自身的 timeout 与事务继承的 deadline 已在 session.deadline 取 min。
        // 这里不再 fallback 到事务 timeout——否则会覆盖 transport 自身更严格的 timeout。
        // execute_command 会用 mutation.timeout_ms.or(transport_timeout_ms)，
        // 再与 session.deadline（含事务总预算）merge 取 min。
        let mutation_timeout_ms = mutation.timeout_ms;

        let read_transport = mutation
            .read
            .transport
            .as_deref()
            .unwrap_or(&mutation.transport);
        let read_skipped = mutation
            .read
            .skip_if_zero
            .iter()
            .any(|reference| output_reference_is_zero(&session.outputs, reference));
        let before = if memory_read.is_some() || read_skipped {
            Vec::new()
        } else {
            let read_params = resolve_workflow_params(&mutation.read.params, &session.outputs)
                .map_err(|error| format!("mutation {mutation_id} read params: {error}"))?;
            let response = session
                .execute_command(
                    read_transport,
                    &mutation.read.command,
                    &read_params,
                    true,
                    None,
                    mutation_timeout_ms,
                )
                .map_err(|error| format!("mutation {mutation_id} pre-read command: {error}"))?;
            self.parse_response(&mutation.read.parser, &response)
                .map_err(|error| format!("mutation {mutation_id} pre-read: {error}"))?;
            response
        };

        let write_transport = mutation
            .write_transport
            .as_deref()
            .unwrap_or(&mutation.transport);

        if let (Some(memory), Some((_, original))) = (&mutation.memory, &memory_read) {
            self.execute_memory_mutation(mutation_id, memory, &params, &mut session, original)?;
        }

        // writeSkipIfZero: memory-only 路径下，当直写 feature 不存在（如无 0x8070）时跳过 writeCommand。
        // memory 补丁已由 execute_memory_mutation 完成，直写命令无意义且会失败。
        let write_skipped = mutation
            .write_skip_if_zero
            .iter()
            .any(|reference| output_reference_is_zero(&session.outputs, reference));
        if write_skipped && memory_read.is_none() {
            return Err(format!(
                "mutation {mutation_id} is not available on this device"
            ));
        }
        if !write_skipped {
            let write_params = resolve_workflow_params(&mutation.write_params, &session.outputs)
                .map_err(|error| format!("mutation {mutation_id} write params: {error}"))?;
            let mut write_inputs = write_params;
            write_inputs.extend(params.clone());
            session.execute_command(
                write_transport,
                &mutation.write_command,
                &write_inputs,
                false,
                Some(&before),
                mutation_timeout_ms,
            )?;
            session.delay(mutation.settle_ms)?;
        }

        for (index, post_write) in mutation.post_writes.iter().enumerate() {
            let skipped = post_write
                .skip_if_zero
                .iter()
                .any(|reference| output_reference_is_zero(&session.outputs, reference));
            if skipped {
                continue;
            }
            if post_write.settle_ms > 1_000 {
                return Err(format!(
                    "mutation {mutation_id} post write {index} settle delay exceeds limit"
                ));
            }
            let command = self
                .commands
                .commands
                .get(&post_write.command)
                .ok_or_else(|| format!("missing command {}", post_write.command))?;
            if command.request.base == RequestBase::ReadResponse {
                return Err(format!(
                    "mutation {mutation_id} post write {index} must use a direct write command"
                ));
            }
            let transport = post_write.transport.as_deref().unwrap_or(write_transport);
            let write_params = resolve_workflow_params(&post_write.params, &session.outputs)
                .map_err(|error| {
                    format!("mutation {mutation_id} post write {index} params: {error}")
                })?;
            let mut write_inputs = write_params;
            write_inputs.extend(params.clone());
            session.execute_command(
                transport,
                &post_write.command,
                &write_inputs,
                false,
                None,
                mutation_timeout_ms,
            )?;
            session.delay(post_write.settle_ms)?;
            if let Some(verify) = &post_write.verify {
                let verify_skipped = verify
                    .skip_if_zero
                    .iter()
                    .any(|reference| output_reference_is_zero(&session.outputs, reference));
                if !verify_skipped {
                    let verify_transport = verify.transport.as_deref().unwrap_or(transport);
                    let verify_params = resolve_workflow_params(&verify.params, &session.outputs)
                        .map_err(|error| {
                        format!("mutation {mutation_id} post write {index} verify params: {error}")
                    })?;
                    let response = session.execute_command(
                        verify_transport,
                        &verify.command,
                        &verify_params,
                        true,
                        None,
                        mutation_timeout_ms,
                    )?;
                    let parsed = self.parse_response(&verify.parser, &response).map_err(
                        |error| {
                            format!(
                                "mutation {mutation_id} post write {index} verification read: {error}"
                            )
                        },
                    )?;
                    verify_assertions(&parsed, &params, &verify.assertions).map_err(|error| {
                        format!(
                            "mutation {mutation_id} post write {index} verification failed: {error}"
                        )
                    })?;
                }
            }
        }

        let verify_transport = mutation
            .verify
            .transport
            .as_deref()
            .unwrap_or(&mutation.transport);
        let verify_params = resolve_workflow_params(&mutation.verify.params, &session.outputs)
            .map_err(|error| format!("mutation {mutation_id} verify params: {error}"))?;
        let verify_skipped = mutation
            .verify
            .skip_if_zero
            .iter()
            .any(|reference| output_reference_is_zero(&session.outputs, reference));
        let expected_memory =
            if let (Some(memory), Some((_, expected))) = (&mutation.memory, &memory_read) {
                let mut patch_params = params.clone();
                patch_params.extend(
                    resolve_workflow_params(&memory.patch_params, &session.outputs).map_err(
                        |error| format!("mutation {mutation_id} memory patch params: {error}"),
                    )?,
                );
                let mut patched = expected.clone();
                for patch in &memory.patches {
                    apply_byte_definition(mutation_id, patch, &patch_params, &mut patched)?;
                }
                write_memory_checksum(&mut patched, &memory.checksum)?;
                Some((memory, patched))
            } else {
                None
            };
        let verified = if verify_skipped {
            Value::Object(params.clone().into_iter().collect())
        } else {
            let response = session.execute_command(
                verify_transport,
                &mutation.verify.command,
                &verify_params,
                true,
                None,
                mutation_timeout_ms,
            )?;
            let parsed = self
                .parse_response(&mutation.verify.parser, &response)
                .map_err(|error| format!("mutation {mutation_id} verification read: {error}"))?;
            verify_assertions(&parsed, &params, &mutation.verify.assertions)
                .map_err(|error| format!("mutation {mutation_id} verification failed: {error}"))?;
            parsed
        };
        let Session {
            device, reports, ..
        } = session;
        if let Some(stats) = hid_io_stats {
            if let Ok(mut guard) = stats.lock() {
                guard.record_reports_executed(reports);
            }
        }
        return_device(path, device, cached_handles, hid_io_stats);
        if let Some((memory, expected)) = expected_memory {
            let (verify_outputs, actual) =
                self.read_memory_mutation(api, path, memory, cached_handles, hid_io_stats)?;
            // Compare data bytes excluding the trailing CRC. Some devices
            // (notably Logitech G705) report a sectorSize whose CRC position
            // doesn't match the engine's assumption — the device may
            // recalculate and store the CRC at a different offset after a
            // write. The engine already verified the patched data via the
            // feature-register verify step above; the memory readback
            // verifies that the patched bytes took effect. Skipping the CRC
            // bytes avoids a false mismatch when the device manages its own
            // CRC independently (mirrors libratbag, which does not compare
            // the sector CRC on readback).
            let crc_len = if memory.checksum.is_empty() { 0 } else { 2 };
            let data_len = actual.len().saturating_sub(crc_len);
            if actual[..data_len] != expected[..data_len] {
                return Err(format!("mutation {mutation_id} memory readback mismatch"));
            }
            // UX3: 验证读成功后更新 onboard memory 缓存，下次预读可命中缓存。
            if let Some(cache) = onboard_memory_cache {
                if let Ok(mut cache_guard) = cache.lock() {
                    cache_guard.insert(path.to_string(), (verify_outputs, actual));
                }
            }
        }
        Ok(verified)
    }

    fn build_command(
        &self,
        id: &str,
        params: &BTreeMap<String, Value>,
        base: Option<&[u8]>,
    ) -> Result<Vec<u8>, String> {
        // P3: 无 param 命令命中预构建缓存时直接返回，跳过字节构建 + checksum 计算。
        // 仅当 params 为空且 base 为 None 时才查缓存（有 params/base 说明调用方需要动态构建）。
        if params.is_empty() && base.is_none() {
            if let Some(cached) = self.compiled_commands.get(id) {
                return Ok(cached.clone());
            }
        }
        let command = self
            .commands
            .commands
            .get(id)
            .ok_or_else(|| format!("missing command {id}"))?;
        if command.request.length == 0 || command.request.length > 1024 {
            return Err(format!("invalid command length for {id}"));
        }
        let mut report = match command.request.base {
            RequestBase::Zero => vec![0u8; command.request.length],
            RequestBase::ReadResponse => {
                let source =
                    base.ok_or_else(|| format!("command {id} requires a read response"))?;
                if source.len() != command.request.length {
                    return Err(format!("command {id} base length mismatch"));
                }
                source.to_vec()
            }
        };
        for byte in &command.request.bytes {
            apply_byte_definition(id, byte, params, &mut report)?;
        }
        if let Some(checksum) = &command.request.checksum {
            let valid_algorithm = matches!(checksum.algorithm.as_str(), "ff-minus-sum8" | "xor8");
            if !valid_algorithm
                || checksum.start > checksum.end_exclusive
                || checksum.end_exclusive > report.len()
                || checksum.write_offset >= report.len()
            {
                return Err(format!("invalid checksum declaration for {id}"));
            }
            match checksum.algorithm.as_str() {
                "ff-minus-sum8" => {
                    let sum = report[checksum.start..checksum.end_exclusive]
                        .iter()
                        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
                    report[checksum.write_offset] = 0xFF - sum;
                }
                "xor8" => {
                    let mut x: u8 = 0;
                    for i in checksum.start..checksum.end_exclusive {
                        x ^= report[i];
                    }
                    report[checksum.write_offset] = x;
                }
                _ => {}
            }
        }
        Ok(report)
    }

    fn parse_response(&self, id: &str, response: &[u8]) -> Result<Value, String> {
        #[cfg(debug_assertions)]
        eprintln!(
            "[mira] raw {id}: {:02x?}",
            &response[..response.len().min(32)]
        );
        let parser = self
            .parsers
            .parsers
            .get(id)
            .ok_or_else(|| format!("missing parser {id}"))?;
        for condition in &parser.valid_when {
            if response.get(condition.offset) != Some(&condition.eq) {
                return Err(format!("parser {id} validity check failed"));
            }
        }
        let mut fields = Map::new();
        for (name, field) in &parser.fields {
            fields.insert(name.clone(), parse_field(field, response)?);
        }
        for (name, derived) in &parser.derived {
            match derived.kind.as_str() {
                "lookup" => {
                    let source = fields
                        .get(&derived.source)
                        .and_then(Value::as_u64)
                        .ok_or_else(|| format!("invalid lookup source {}", derived.source))?;
                    // P2: 优先使用预编译的数值键查找，避免每次格式化字符串。
                    let value = self
                        .compiled_lookups
                        .get(id)
                        .and_then(|by_parser| by_parser.get(name))
                        .and_then(|compiled| compiled.get(&source).cloned())
                        .unwrap_or_else(|| {
                            // 回退到原始字符串表查找（兼容未预编译的路径）。
                            let hex_key = format!("0x{source:02X}");
                            let decimal_key = source.to_string();
                            derived
                                .table
                                .get(&hex_key)
                                .or_else(|| derived.table.get(&decimal_key))
                                .cloned()
                                .unwrap_or(Value::Null)
                        });
                    fields.insert(name.clone(), value);
                }
                "bitmap" => {
                    let source = fields
                        .get(&derived.source)
                        .and_then(Value::as_u64)
                        .ok_or_else(|| format!("invalid bitmap source {}", derived.source))?;
                    let values: Vec<Value> = derived
                        .bitmap
                        .iter()
                        .filter(|entry| source & (1u64 << entry.bit) != 0)
                        .map(|entry| entry.value.clone())
                        .collect();
                    fields.insert(name.clone(), Value::Array(values));
                }
                "bit" => {
                    let source = fields
                        .get(&derived.source)
                        .and_then(Value::as_u64)
                        .ok_or_else(|| format!("invalid bit source {}", derived.source))?;
                    let bit = derived
                        .bit
                        .ok_or_else(|| format!("bit derived {} missing 'bit' field", name))?;
                    fields.insert(name.clone(), Value::Bool((source >> bit) & 1 == 1));
                }
                _ => return Err(format!("unsupported derived kind {}", derived.kind)),
            }
        }
        Ok(Value::Object(fields))
    }
}

fn expand_feature_refs(
    features: &HashMap<String, FeatureEntry>,
    workflows: &mut WorkflowsFile,
) -> Result<(), String> {
    for (workflow_id, workflow) in &mut workflows.workflows {
        for (step_index, step) in workflow.steps.iter_mut().enumerate() {
            let context = format!("workflow {workflow_id} step {}", step_index + 1);
            expand_map_feature_refs(features, &mut step.params, &context)?;
        }
    }
    for (mutation_id, mutation) in &mut workflows.mutations {
        expand_map_feature_refs(
            features,
            &mut mutation.read.params,
            &format!("mutation {mutation_id} read"),
        )?;
        expand_map_feature_refs(
            features,
            &mut mutation.write_params,
            &format!("mutation {mutation_id} write"),
        )?;
        expand_map_feature_refs(
            features,
            &mut mutation.verify.params,
            &format!("mutation {mutation_id} verify"),
        )?;
        if let Some(memory) = &mut mutation.memory {
            expand_map_feature_refs(
                features,
                &mut memory.context_params,
                &format!("mutation {mutation_id} memory context"),
            )?;
            expand_map_feature_refs(
                features,
                &mut memory.patch_params,
                &format!("mutation {mutation_id} memory patch"),
            )?;
        }
    }
    Ok(())
}

fn expand_map_feature_refs(
    features: &HashMap<String, FeatureEntry>,
    map: &mut BTreeMap<String, Value>,
    context: &str,
) -> Result<(), String> {
    if let Some(feature_ref) = map.remove("featureRef") {
        let name = feature_ref
            .as_str()
            .ok_or_else(|| format!("{context}: featureRef must be a string"))?;
        let entry = features
            .get(name)
            .ok_or_else(|| format!("{context}: unknown featureRef '{name}'"))?;
        map.insert("featureId".to_string(), Value::from(entry.decimal));
    }
    for (key, value) in map.iter_mut() {
        expand_value_feature_refs(features, value, &format!("{context} -> {key}"))?;
    }
    Ok(())
}

fn expand_value_feature_refs(
    features: &HashMap<String, FeatureEntry>,
    value: &mut Value,
    context: &str,
) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if let Some(feature_ref) = map.remove("featureRef") {
                let name = feature_ref
                    .as_str()
                    .ok_or_else(|| format!("{context}: featureRef must be a string"))?;
                let entry = features
                    .get(name)
                    .ok_or_else(|| format!("{context}: unknown featureRef '{name}'"))?;
                map.insert("featureId".to_string(), Value::from(entry.decimal));
            }
            for (key, child) in map.iter_mut() {
                expand_value_feature_refs(features, child, &format!("{context} -> {key}"))?;
            }
            Ok(())
        }
        Value::Array(array) => {
            for (index, child) in array.iter_mut().enumerate() {
                expand_value_feature_refs(features, child, &format!("{context} -> [{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn execute_with_candidates(
    session: &mut Session<'_>,
    transport: &str,
    command: &str,
    params: &BTreeMap<String, Value>,
    candidates: &BTreeMap<String, Vec<Value>>,
) -> Result<Vec<u8>, String> {
    if candidates.is_empty() {
        return session.execute_command(transport, command, params, true, None, None);
    }
    if candidates.len() != 1 {
        return Err("only one candidate parameter is supported per workflow step".into());
    }
    let (name, values) = candidates.iter().next().expect("checked non-empty");
    if values.is_empty() || values.len() > 16 {
        return Err(format!(
            "candidate parameter {name} must contain 1..16 values"
        ));
    }
    let mut last_error = None;
    for value in values {
        let mut attempt = params.clone();
        attempt.insert(name.clone(), value.clone());
        match session.execute_command(transport, command, &attempt, true, None, None) {
            Ok(response) => return Ok(response),
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "all candidates for {name} failed: {}",
        last_error.unwrap_or_else(|| "no candidates".into())
    ))
}

/// 从句柄缓存取出设备（命中时移除），未命中则 `open_path`。`HidDevice` 不可 Clone，
/// 采用取用-归还：调用方执行成功后通过 `return_device` 归还；出错时句柄随 session
/// 析构关闭，不归还（设备可能处于异常状态）。
fn take_or_open_device(
    api: &HidApi,
    c_path: &CString,
    path: &str,
    cached_handles: Option<&Mutex<HidHandleCache>>,
    hid_io_stats: Option<&Mutex<HidIoStats>>,
) -> Result<HidDevice, String> {
    if let Some(cache) = cached_handles {
        match cache.lock() {
            Ok(mut guard) => {
                if let Some(device) = guard.remove(path) {
                    record_hid_io_stat(hid_io_stats, HidIoStatEvent::CacheHit);
                    return Ok(device);
                }
            }
            Err(_) => record_hid_io_stat(hid_io_stats, HidIoStatEvent::LockFailure),
        }
    }
    record_hid_io_stat(hid_io_stats, HidIoStatEvent::CacheMiss);
    match api.open_path(c_path) {
        Ok(device) => Ok(device),
        Err(error) => {
            record_hid_io_stat(hid_io_stats, HidIoStatEvent::OpenFailure);
            Err(error.to_string())
        }
    }
}

/// 将设备归还到句柄缓存。锁中毒或无缓存时句柄直接析构关闭。
fn return_device(
    path: &str,
    device: HidDevice,
    cached_handles: Option<&Mutex<HidHandleCache>>,
    hid_io_stats: Option<&Mutex<HidIoStats>>,
) {
    if let Some(cache) = cached_handles {
        match cache.lock() {
            Ok(mut guard) => {
                guard.insert(path.to_string(), device);
                record_hid_io_stat(hid_io_stats, HidIoStatEvent::Returned);
            }
            Err(_) => record_hid_io_stat(hid_io_stats, HidIoStatEvent::LockFailure),
        }
    }
}

enum HidIoStatEvent {
    CacheHit,
    CacheMiss,
    OpenFailure,
    Returned,
    LockFailure,
}

fn record_hid_io_stat(stats: Option<&Mutex<HidIoStats>>, event: HidIoStatEvent) {
    let Some(stats) = stats else {
        return;
    };
    if let Ok(mut guard) = stats.lock() {
        match event {
            HidIoStatEvent::CacheHit => guard.record_cache_hit(),
            HidIoStatEvent::CacheMiss => guard.record_cache_miss(),
            HidIoStatEvent::OpenFailure => guard.record_open_failure(),
            HidIoStatEvent::Returned => guard.record_returned(),
            HidIoStatEvent::LockFailure => guard.record_lock_failure(),
        }
    }
}

/// Rate-limited HID session. Workflow definitions are capped by `MAX_COMMANDS`
/// and each session is bounded by:
/// - `MAX_REPORTS` (128): caps the total number of HID reports sent.
/// - `MAX_DELAY_MS` (5_000): caps the cumulative delay, bounding wall-clock time.
///
/// Together these prevent a malicious plugin from flooding the device or
/// holding the HID handle for too long.
struct Session<'a> {
    package: &'a ProtocolPackage,
    device: HidDevice,
    reports: usize,
    delay_ms: u64,
    outputs: BTreeMap<String, Value>,
    deadline: Option<Instant>,
    /// HID 交换事件回调（可选）。None 时使用 NullHidEventSink。
    event_sink: Option<&'a dyn HidEventSink>,
}

impl Session<'_> {
    fn execute_command(
        &mut self,
        transport_id: &str,
        command_id: &str,
        params: &BTreeMap<String, Value>,
        expect_response: bool,
        base: Option<&[u8]>,
        timeout_override_ms: Option<u64>,
    ) -> Result<Vec<u8>, String> {
        let transport = self
            .package
            .transports
            .transports
            .get(transport_id)
            .ok_or_else(|| format!("missing transport {transport_id}"))?;
        let transport_timeout_ms = match transport {
            TransportDefinition::HidFeature { timeout_ms, .. }
            | TransportDefinition::HidFeatureProxy { timeout_ms, .. }
            | TransportDefinition::HidOutputInput { timeout_ms, .. }
            | TransportDefinition::HidRace { timeout_ms, .. } => *timeout_ms,
        };
        let previous_deadline = self.deadline;
        self.deadline = merge_deadline(
            previous_deadline,
            timeout_override_ms.or(transport_timeout_ms),
        )?;
        let result = match transport {
            TransportDefinition::HidFeature { .. } => {
                let report = self.package.build_command(command_id, params, base)?;
                self.feature_exchange(transport_id, command_id, &report, expect_response)
            }
            TransportDefinition::HidFeatureProxy {
                base_transport,
                start_command,
                poll_command,
                set_length_command,
                read_command,
                send_ready,
                read_ready,
                online,
                status_parser,
                status_output,
                attempts,
                delay_ms,
                ..
            } => {
                let base_transport = base_transport.clone();
                let start_command = start_command.clone();
                let poll_command = poll_command.clone();
                let set_length_command = set_length_command.clone();
                let read_command = read_command.clone();
                let status_parser = status_parser.clone();
                let status_output = status_output.clone();
                let send_ready = Condition {
                    offset: send_ready.offset,
                    eq: send_ready.eq,
                };
                let read_ready = Condition {
                    offset: read_ready.offset,
                    eq: read_ready.eq,
                };
                let online = Condition {
                    offset: online.offset,
                    eq: online.eq,
                };
                let attempts = *attempts;
                let delay_ms = *delay_ms;

                let start = self
                    .package
                    .build_command(&start_command, &BTreeMap::new(), None)?;
                self.feature_exchange(&base_transport, &start_command, &start, false)?;
                let status = self.poll_until(
                    &base_transport,
                    &poll_command,
                    &send_ready,
                    attempts,
                    delay_ms,
                )?;
                if status.get(online.offset) != Some(&online.eq) {
                    return Err("proxy target is offline".into());
                }
                let parsed_status = self.package.parse_response(&status_parser, &status)?;
                self.outputs.insert(status_output, parsed_status);

                let inner = self.package.build_command(command_id, params, base)?;
                let length_params =
                    BTreeMap::from([("innerLength".to_string(), Value::from(inner.len()))]);
                let set_length =
                    self.package
                        .build_command(&set_length_command, &length_params, None)?;
                self.feature_exchange(&base_transport, &set_length_command, &set_length, true)?;
                self.feature_exchange(&base_transport, command_id, &inner, expect_response)?;
                if !expect_response {
                    self.poll_until(
                        &base_transport,
                        &poll_command,
                        &send_ready,
                        attempts,
                        delay_ms,
                    )?;
                    return Ok(Vec::new());
                }
                self.poll_until(
                    &base_transport,
                    &poll_command,
                    &read_ready,
                    attempts,
                    delay_ms,
                )?;
                let read = self
                    .package
                    .build_command(&read_command, &BTreeMap::new(), None)?;
                self.feature_exchange(&base_transport, &read_command, &read, true)
            }
            TransportDefinition::HidOutputInput { .. } => {
                let report = self.package.build_command(command_id, params, base)?;
                self.output_input_exchange(transport_id, command_id, &report, expect_response)
            }
            TransportDefinition::HidRace { .. } => {
                let race_payload = self.package.build_command(command_id, params, base)?;
                self.race_exchange(transport_id, command_id, &race_payload, expect_response)
            }
        };
        self.deadline = previous_deadline;
        result
    }

    fn poll_until(
        &mut self,
        transport: &str,
        command: &str,
        condition: &Condition,
        attempts: usize,
        delay_ms: u64,
    ) -> Result<Vec<u8>, String> {
        let report = self
            .package
            .build_command(command, &BTreeMap::new(), None)?;
        // Cap attempts at 32 to bound worst-case latency. Plugin authors who
        // declare a higher value will still get at most 32 polls.
        for attempt in 0..attempts.min(32) {
            let response = self.feature_exchange(transport, command, &report, true)?;
            if response.get(condition.offset) == Some(&condition.eq) {
                return Ok(response);
            }
            // 条件未满足：设备忙碌，通知宿主记录 hid-busy-retry 事件。
            if attempt > 0 {
                if let Some(sink) = self.event_sink {
                    sink.on_hid_busy_retry(transport, command, attempt + 1);
                }
            }
            self.delay(delay_ms)?;
        }
        Err(format!(
            "condition at offset {} timed out",
            condition.offset
        ))
    }

    /// 验证响应校验和。返回 `(expected, actual)` 字节对，`None` 表示无法验证
    /// （命令未声明 checksum 或响应长度不足）。
    ///
    /// 复用请求的 `ChecksumDefinition`：雷蛇协议中请求和响应的校验和算法与
    /// 位置通常相同（`start..end_exclusive` 范围计算，`write_offset` 位置存储）。
    fn verify_response_checksum(&self, command_id: &str, response: &[u8]) -> Option<(u8, u8)> {
        let command = self.package.commands.commands.get(command_id)?;
        let checksum = command.request.checksum.as_ref()?;
        if checksum.end_exclusive > response.len() || checksum.write_offset >= response.len() {
            return None;
        }
        match checksum.algorithm.as_str() {
            "xor8" => {
                let mut x: u8 = 0;
                for i in checksum.start..checksum.end_exclusive {
                    x ^= response[i];
                }
                Some((x, response[checksum.write_offset]))
            }
            "ff-minus-sum8" => {
                let sum = response[checksum.start..checksum.end_exclusive]
                    .iter()
                    .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
                Some((0xFF - sum, response[checksum.write_offset]))
            }
            _ => None,
        }
    }

    /// 计算校验和验证结果并在失败时发射 `on_hid_checksum_failed` 事件。
    /// 返回 `Some(bool)` 表示已验证（true=通过，false=失败），`None` 表示无法验证。
    fn check_checksum_and_emit(
        &self,
        transport_id: &str,
        command_id: &str,
        response: &[u8],
    ) -> Option<bool> {
        let (expected, actual) = self.verify_response_checksum(command_id, response)?;
        let valid = expected == actual;
        if !valid {
            if let Some(sink) = self.event_sink {
                sink.on_hid_checksum_failed(
                    transport_id,
                    command_id,
                    &hex::encode(&[expected]),
                    &hex::encode(&[actual]),
                );
            }
        }
        Some(valid)
    }

    fn feature_exchange(
        &mut self,
        transport_id: &str,
        command_id: &str,
        payload: &[u8],
        expect_response: bool,
    ) -> Result<Vec<u8>, String> {
        let Some(TransportDefinition::HidFeature {
            report_id,
            write_length,
            read_length,
            strip_report_id_on_read,
            feature_delay_ms,
            ..
        }) = self.package.transports.transports.get(transport_id)
        else {
            return Err(format!("transport {transport_id} is not hid-feature"));
        };
        if payload.len() + 1 != *write_length || *write_length > 1025 || *read_length > 1025 {
            return Err("feature report length mismatch".into());
        }
        self.reports += 1;
        if self.reports > MAX_REPORTS {
            return Err("report limit exceeded".into());
        }
        deadline_remaining_ms(self.deadline)?;
        let mut report = Vec::with_capacity(*write_length);
        report.push(*report_id);
        report.extend_from_slice(payload);
        let exchange_start = Instant::now();
        self.device
            .send_feature_report(&report)
            .map_err(|error| format!("send feature report: {error}"))?;
        if !expect_response {
            let duration_ms = exchange_start.elapsed().as_millis() as u64;
            if let Some(sink) = self.event_sink {
                sink.on_hid_exchange(
                    transport_id,
                    command_id,
                    &hex::encode(payload),
                    "",
                    duration_ms,
                    0,
                    None,
                );
            }
            return Ok(Vec::new());
        }
        self.delay(*feature_delay_ms)?;
        deadline_remaining_ms(self.deadline)?;
        let mut response = vec![0u8; *read_length];
        let count = self
            .device
            .get_feature_report(&mut response)
            .map_err(|error| format!("get feature report: {error}"))?;
        response.truncate(count);
        if *strip_report_id_on_read && !response.is_empty() {
            response.remove(0);
        }
        let duration_ms = exchange_start.elapsed().as_millis() as u64;
        let checksum_valid = self.check_checksum_and_emit(transport_id, command_id, &response);
        if let Some(sink) = self.event_sink {
            sink.on_hid_exchange(
                transport_id,
                command_id,
                &hex::encode(payload),
                &hex::encode(&response),
                duration_ms,
                0,
                checksum_valid,
            );
        }
        Ok(response)
    }

    fn output_input_exchange(
        &mut self,
        transport_id: &str,
        command_id: &str,
        payload: &[u8],
        expect_response: bool,
    ) -> Result<Vec<u8>, String> {
        let Some(TransportDefinition::HidOutputInput {
            report_id,
            write_length,
            read_length,
            strip_report_id_on_read,
            read_timeout_ms,
            read_retries,
            ..
        }) = self.package.transports.transports.get(transport_id)
        else {
            return Err(format!("transport {transport_id} is not hid-output-input"));
        };
        if payload.len() + 1 != *write_length || *write_length > 1025 || *read_length > 1025 {
            return Err("output/input report length mismatch".into());
        }
        self.reports += 1;
        if self.reports > MAX_REPORTS {
            return Err("report limit exceeded".into());
        }
        deadline_remaining_ms(self.deadline)?;
        let mut report = Vec::with_capacity(*write_length);
        report.push(*report_id);
        report.extend_from_slice(payload);
        let exchange_start = Instant::now();
        let written = write_output_report_with_fallback(&self.device, &report)
            .map_err(|error| format!("send output report: {error}"))?;
        if written != report.len() {
            return Err(format!(
                "short output report write: {written}/{}",
                report.len()
            ));
        }
        if !expect_response {
            let duration_ms = exchange_start.elapsed().as_millis() as u64;
            if let Some(sink) = self.event_sink {
                sink.on_hid_exchange(
                    transport_id,
                    command_id,
                    &hex::encode(payload),
                    "",
                    duration_ms,
                    0,
                    None,
                );
            }
            return Ok(Vec::new());
        }

        for _ in 0..*read_retries {
            let read_timeout_ms = match deadline_remaining_ms(self.deadline)? {
                Some(remaining) => {
                    (*read_timeout_ms).min(i32::try_from(remaining).unwrap_or(i32::MAX))
                }
                None => *read_timeout_ms,
            };
            let mut response = vec![0u8; *read_length];
            let count = self
                .device
                .read_timeout(&mut response, read_timeout_ms)
                .map_err(|error| format!("read input report: {error}"))?;
            if count == 0 {
                continue;
            }
            response.truncate(count);
            // HID++ error responses reference payload[0..3]. Only evaluate
            // these branches when the payload is long enough; otherwise a
            // short payload (e.g. write_length < 4) would panic on indexing.
            if payload.len() >= 3
                && response.len() >= 6
                && response[0] == 0x10
                && response[1] == payload[0]
                && response[2] == 0x8F
                && response[3] == payload[1]
                && response[4] == payload[2]
            {
                return Err(format!("HID++ 1.0 transport error 0x{:02X}", response[5]));
            }
            let Some(response) =
                input_payload_from_report(response, *report_id, *strip_report_id_on_read)
            else {
                continue;
            };
            if payload.len() >= 3
                && response.len() >= 5
                && response[1] == 0xFF
                && response[2] == payload[1]
                && response[3] == payload[2]
            {
                return Err(format!("HID++ 2.0 error 0x{:02X}", response[4]));
            }
            if response.get(..3) == payload.get(..3) {
                let duration_ms = exchange_start.elapsed().as_millis() as u64;
                let checksum_valid =
                    self.check_checksum_and_emit(transport_id, command_id, &response);
                if let Some(sink) = self.event_sink {
                    sink.on_hid_exchange(
                        transport_id,
                        command_id,
                        &hex::encode(payload),
                        &hex::encode(&response),
                        duration_ms,
                        0,
                        checksum_valid,
                    );
                }
                return Ok(response);
            }
            // Response mismatch: the response's first 3 bytes (device index,
            // feature index, function ID) don't match the request. Notify the
            // sink so the host can record a hid-response-mismatch event.
            if let Some(sink) = self.event_sink {
                sink.on_hid_response_mismatch(
                    transport_id,
                    command_id,
                    &hex::encode(payload.get(..3).unwrap_or(&[])),
                    &hex::encode(response.get(..3).unwrap_or(&[])),
                );
            }
        }
        Err("timed out waiting for matching input report".into())
    }

    /// AM35 RACE-style exchange: frames the RACE payload with a 3-byte header
    /// ([writeReportId, payloadLength, raceType]), writes via HID Output Report,
    /// and reads the response via HID Input Report.
    fn race_exchange(
        &mut self,
        transport_id: &str,
        command_id: &str,
        race_payload: &[u8],
        expect_response: bool,
    ) -> Result<Vec<u8>, String> {
        let Some(TransportDefinition::HidRace {
            write_report_id,
            read_report_id,
            write_length,
            read_length,
            race_type,
            strip_report_id_on_read,
            read_mode,
            read_delay_ms,
            read_timeout_ms,
            read_retries,
            ..
        }) = self.package.transports.transports.get(transport_id)
        else {
            return Err(format!("transport {transport_id} is not hid-race"));
        };
        if race_payload.len() + 3 > *write_length || *write_length > 1025 || *read_length > 1025 {
            return Err("race report length mismatch".into());
        }
        // 修复 P-1：race_payload 长度字段为单字节（u8），当 write_length > 258 时
        // payload 可超过 255 字节，`as u8` 会静默截断导致协议帧损坏。
        // 当前 amaster 插件 writeLength=62 不触发，但需防御未来插件。
        if race_payload.len() > 255 {
            return Err("race payload exceeds single-byte length field".into());
        }
        self.reports += 1;
        if self.reports > MAX_REPORTS {
            return Err("report limit exceeded".into());
        }
        deadline_remaining_ms(self.deadline)?;
        // Frame: [writeReportId, racePayloadLength, raceType, ...payload, ...zeros]
        let mut report = Vec::with_capacity(*write_length);
        report.push(*write_report_id);
        report.push(race_payload.len() as u8);
        report.push(*race_type);
        report.extend_from_slice(race_payload);
        report.resize(*write_length, 0);
        let exchange_start = Instant::now();
        let written = self
            .device
            .write(&report)
            .map_err(|error| format!("send race output report: {error}"))?;
        if written != report.len() {
            return Err(format!(
                "short race output report write: {written}/{}",
                report.len()
            ));
        }
        if !expect_response {
            let duration_ms = exchange_start.elapsed().as_millis() as u64;
            if let Some(sink) = self.event_sink {
                sink.on_hid_exchange(
                    transport_id,
                    command_id,
                    &hex::encode(race_payload),
                    "",
                    duration_ms,
                    0,
                    None,
                );
            }
            return Ok(Vec::new());
        }
        for _ in 0..*read_retries {
            let mut response = vec![0u8; *read_length];
            let count = match read_mode {
                HidRaceReadMode::Interrupt => {
                    let timeout = match deadline_remaining_ms(self.deadline)? {
                        Some(remaining) => {
                            (*read_timeout_ms).min(i32::try_from(remaining).unwrap_or(i32::MAX))
                        }
                        None => *read_timeout_ms,
                    };
                    self.device
                        .read_timeout(&mut response, timeout)
                        .map_err(|error| format!("read race interrupt report: {error}"))?
                }
                HidRaceReadMode::InputReport => {
                    if *read_delay_ms > 0 {
                        self.delay(*read_delay_ms)?;
                    }
                    response[0] = *read_report_id;
                    self.device
                        .get_input_report(&mut response)
                        .map_err(|error| format!("get race input report: {error}"))?
                }
            };
            if count == 0 {
                continue;
            }
            response.truncate(count);
            if response.first() != Some(read_report_id) {
                continue;
            }
            if *strip_report_id_on_read && !response.is_empty() {
                response.remove(0);
            }
            if !race_response_matches_request(&response, race_payload, *strip_report_id_on_read) {
                // RACE response command ID doesn't match the request. Notify
                // the sink so the host can record a hid-response-mismatch event.
                if let Some(sink) = self.event_sink {
                    let request_id = race_payload.get(4..6).unwrap_or(&[]);
                    let response_id_offset = if *strip_report_id_on_read { 6 } else { 7 };
                    let response_id = response
                        .get(response_id_offset..response_id_offset + 2)
                        .unwrap_or(&[]);
                    sink.on_hid_response_mismatch(
                        transport_id,
                        command_id,
                        &hex::encode(request_id),
                        &hex::encode(response_id),
                    );
                }
                continue;
            }
            let duration_ms = exchange_start.elapsed().as_millis() as u64;
            let checksum_valid = self.check_checksum_and_emit(transport_id, command_id, &response);
            if let Some(sink) = self.event_sink {
                sink.on_hid_exchange(
                    transport_id,
                    command_id,
                    &hex::encode(race_payload),
                    &hex::encode(&response),
                    duration_ms,
                    0,
                    checksum_valid,
                );
            }
            return Ok(response);
        }
        Err("timed out waiting for race input report".into())
    }

    fn delay(&mut self, milliseconds: u64) -> Result<(), String> {
        self.delay_ms = self
            .delay_ms
            .checked_add(milliseconds)
            .ok_or_else(|| "delay limit exceeded".to_string())?;
        if self.delay_ms > MAX_DELAY_MS {
            return Err("delay limit exceeded".into());
        }
        if let Some(remaining) = deadline_remaining_ms(self.deadline)? {
            if milliseconds > remaining {
                return Err("operation timeout exceeded".into());
            }
        }
        thread::sleep(Duration::from_millis(milliseconds));
        Ok(())
    }
}

fn race_response_matches_request(
    response: &[u8],
    request: &[u8],
    report_id_stripped: bool,
) -> bool {
    let Some(request_id) = request.get(4..6) else {
        return true;
    };
    let response_id_offset = if report_id_stripped { 6 } else { 7 };
    response
        .get(response_id_offset..response_id_offset + 2)
        .is_some_and(|response_id| response_id == request_id)
}

fn input_payload_from_report(
    mut response: Vec<u8>,
    report_id: u8,
    strip_report_id_on_read: bool,
) -> Option<Vec<u8>> {
    if response.first() == Some(&report_id) {
        if strip_report_id_on_read {
            response.remove(0);
        }
        return Some(response);
    }

    if strip_report_id_on_read {
        return Some(response);
    }

    None
}

fn write_output_report_with_fallback(device: &HidDevice, report: &[u8]) -> Result<usize, String> {
    match device.write(report) {
        Ok(written) => Ok(written),
        Err(error) => {
            let output_error = error.to_string();
            if output_report_write_needs_feature_fallback(&output_error) {
                if let Some(long_report) = hidpp_short_output_as_long_report(report) {
                    match device.write(&long_report) {
                        Ok(written) if written == long_report.len() => {
                            return Ok(report.len());
                        }
                        Ok(written) => {
                            return Err(format!(
                                "{output_error}; fallback long output report: short write {written}/{}",
                                long_report.len()
                            ));
                        }
                        Err(long_error) => {
                            let feature_result =
                                device.send_feature_report(report).map(|_| report.len());
                            return feature_result.map_err(|feature_error| {
                                format!(
                                    "{output_error}; fallback long output report: {long_error}; fallback feature report: {feature_error}"
                                )
                            });
                        }
                    }
                }
                device
                    .send_feature_report(report)
                    .map(|_| report.len())
                    .map_err(|feature_error| {
                        format!("{output_error}; fallback feature report: {feature_error}")
                    })
            } else {
                Err(output_error)
            }
        }
    }
}

fn hidpp_short_output_as_long_report(report: &[u8]) -> Option<Vec<u8>> {
    if report.len() != 7 || report.first() != Some(&0x10) {
        return None;
    }
    let mut long_report = Vec::with_capacity(20);
    long_report.push(0x11);
    long_report.extend_from_slice(&report[1..]);
    long_report.resize(20, 0);
    Some(long_report)
}

fn output_report_write_needs_feature_fallback(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("writefile")
        && (lower.contains("0x00000001") || lower.contains("incorrect function"))
}

fn output_value<'a>(
    outputs: &'a BTreeMap<String, Value>,
    reference: &OutputReference,
) -> Option<&'a Value> {
    outputs
        .get(&reference.output)?
        .as_object()?
        .get(&reference.field)
}

/// Checks if the referenced output value should be considered "zero" for
/// skip-condition purposes. Treats numeric 0 and bool `false` as zero;
/// missing values are treated as non-zero (don't skip).
///
/// `Value::Bool(false).as_u64()` returns `None` rather than `Some(0)`, so a
/// direct `as_u64() == Some(0)` check fails for bool fields. This helper
/// bridges that gap so `skipIfZero` works with `bool` parser fields.
fn output_reference_is_zero(
    outputs: &BTreeMap<String, Value>,
    reference: &OutputReference,
) -> bool {
    match output_value(outputs, reference) {
        Some(Value::Bool(false)) => true,
        Some(value) => value.as_u64() == Some(0),
        None => false,
    }
}

fn output_condition_value<'a>(
    outputs: &'a BTreeMap<String, Value>,
    condition: &OutputCondition,
) -> Option<&'a Value> {
    output_value(
        outputs,
        &OutputReference {
            output: condition.output.clone(),
            field: condition.field.clone(),
        },
    )
}

fn mutation_base_available(
    mutation: &MutationDefinition,
    outputs: &BTreeMap<String, Value>,
) -> bool {
    let zero_skipped = mutation
        .skip_if_zero
        .iter()
        .any(|reference| output_reference_is_zero(outputs, reference));
    let non_zero_skipped = mutation.skip_if_non_zero.iter().any(|reference| {
        output_value(outputs, reference)
            .and_then(Value::as_u64)
            .unwrap_or(0)
            != 0
    });
    let all_zero_skipped = !mutation.skip_if_all_zero.is_empty()
        && mutation
            .skip_if_all_zero
            .iter()
            .all(|reference| output_reference_is_zero(outputs, reference));
    !(zero_skipped || non_zero_skipped || all_zero_skipped)
}

fn direct_write_available(
    mutation: &MutationDefinition,
    outputs: &BTreeMap<String, Value>,
) -> bool {
    !mutation
        .write_skip_if_zero
        .iter()
        .any(|reference| output_reference_is_zero(outputs, reference))
}

fn memory_write_available(
    mutation: &MutationDefinition,
    outputs: &BTreeMap<String, Value>,
) -> bool {
    let Some(memory) = &mutation.memory else {
        return false;
    };
    output_value(outputs, &memory.available_when)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        != 0
        && memory
            .enabled_when
            .matches(output_condition_value(outputs, &memory.enabled_when))
        && memory
            .required_when
            .iter()
            .all(|condition| condition.matches(output_condition_value(outputs, condition)))
}

fn mutation_available(mutation: &MutationDefinition, outputs: &BTreeMap<String, Value>) -> bool {
    mutation_base_available(mutation, outputs)
        && (direct_write_available(mutation, outputs) || memory_write_available(mutation, outputs))
}

/// Deep merge two JSON values for model override loading.
///
/// 合并策略：
/// - Object + Object：递归合并，overlay 的字段覆盖 base 的同名字段。
/// - 其他类型组合：overlay 完全覆盖 base（包括 Array，不做元素级合并）。
///
/// 这让型号覆盖文件只需提供 diff（变更的字段），其余字段从父插件继承。
fn deep_merge_json(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.remove(&key) {
                    Some(base_value) => {
                        base_map.insert(key, deep_merge_json(base_value, value));
                    }
                    None => {
                        base_map.insert(key, value);
                    }
                }
            }
            Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}

fn resolve_workflow_params(
    params: &BTreeMap<String, Value>,
    outputs: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, String> {
    params
        .iter()
        .map(|(name, value)| {
            let resolved = if let Some(reference) = value.as_object() {
                let output = reference.get("fromOutput").and_then(Value::as_str);
                let field = reference.get("field").and_then(Value::as_str);
                match (output, field) {
                    (Some(output), Some(field))
                        if reference.keys().all(|key| {
                            matches!(key.as_str(), "fromOutput" | "field" | "subtract")
                        }) =>
                    {
                        let source = outputs
                            .get(output)
                            .and_then(Value::as_object)
                            .and_then(|object| object.get(field))
                            .cloned()
                            .ok_or_else(|| format!("missing output reference {output}.{field}"))?;
                        if let Some(subtract) = reference.get("subtract").and_then(Value::as_u64) {
                            let value = source.as_u64().ok_or_else(|| {
                                format!("output reference {output}.{field} is not numeric")
                            })?;
                            Value::from(value.checked_sub(subtract).ok_or_else(|| {
                                format!("output reference {output}.{field} subtraction underflow")
                            })?)
                        } else {
                            source
                        }
                    }
                    _ => value.clone(),
                }
            } else {
                value.clone()
            };
            Ok((name.clone(), resolved))
        })
        .collect()
}

fn parse_byte(value: &str) -> Result<u8, String> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|hex| u8::from_str_radix(hex, 16))
        .unwrap_or_else(|| value.parse::<u8>())
        .map_err(|_| format!("invalid byte {value}"))
}

fn apply_byte_definition(
    command_id: &str,
    definition: &ByteDefinition,
    params: &BTreeMap<String, Value>,
    report: &mut [u8],
) -> Result<(), String> {
    let index = if let Some(param) = &definition.indexed_by {
        let value = params
            .get(param)
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("command {command_id} missing integer parameter {param}"))?;
        let index = value
            .checked_sub(definition.index_base)
            .ok_or_else(|| format!("command {command_id} index underflow"))?;
        let index = usize::try_from(index)
            .map_err(|_| format!("command {command_id} index out of range"))?;
        definition
            .offset
            .checked_add(index.saturating_mul(definition.stride))
            .ok_or_else(|| format!("command {command_id} byte offset overflow"))?
    } else {
        definition.offset
    };

    let encoded = match (&definition.value, &definition.param) {
        (Some(value), None) => vec![parse_byte(value)?],
        (None, Some(param)) => {
            let value = params
                .get(param)
                .ok_or_else(|| format!("command {command_id} missing parameter {param}"))?;
            match definition.encoding.as_deref().unwrap_or("u8") {
                "u8" => {
                    let raw = value
                        .as_u64()
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or_else(|| {
                            format!("command {command_id} parameter {param} is not u8")
                        })?;
                    vec![raw]
                }
                "bool" => vec![u8::from(value.as_bool().ok_or_else(|| {
                    format!("command {command_id} parameter {param} is not boolean")
                })?)],
                "le-u16" => {
                    let raw = value
                        .as_u64()
                        .and_then(|value| u16::try_from(value).ok())
                        .ok_or_else(|| {
                            format!("command {command_id} parameter {param} is not u16")
                        })?;
                    raw.to_le_bytes().to_vec()
                }
                "be-u16" => {
                    let raw = value
                        .as_u64()
                        .and_then(|value| u16::try_from(value).ok())
                        .ok_or_else(|| {
                            format!("command {command_id} parameter {param} is not u16")
                        })?;
                    raw.to_be_bytes().to_vec()
                }
                "rgb" => parse_rgb(value.as_str().ok_or_else(|| {
                    format!("command {command_id} parameter {param} is not a color")
                })?)?
                .to_vec(),
                "hue-index-be-u16" => {
                    let [r, g, b] = parse_rgb(value.as_str().ok_or_else(|| {
                        format!("command {command_id} parameter {param} is not a color")
                    })?)?;
                    rgb_to_hue_index(r, g, b).to_be_bytes().to_vec()
                }
                "bytes" => value
                    .as_array()
                    .ok_or_else(|| {
                        format!("command {command_id} parameter {param} is not a byte array")
                    })?
                    .iter()
                    .map(|byte| {
                        byte.as_u64()
                            .and_then(|byte| u8::try_from(byte).ok())
                            .ok_or_else(|| {
                                format!(
                                    "command {command_id} parameter {param} contains an invalid byte"
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                "lookup-u8" => {
                    let key = value
                        .as_u64()
                        .ok_or_else(|| {
                            format!("command {command_id} parameter {param} is not integer")
                        })?
                        .to_string();
                    vec![*definition.lookup.get(&key).ok_or_else(|| {
                        format!("command {command_id} parameter {param} has no encoding")
                    })?]
                }
                "bool-lookup-u8" => {
                    let key = value
                        .as_bool()
                        .ok_or_else(|| {
                            format!("command {command_id} parameter {param} is not boolean")
                        })?
                        .to_string();
                    vec![*definition.lookup.get(&key).ok_or_else(|| {
                        format!("command {command_id} parameter {param} has no encoding")
                    })?]
                }
                encoding => {
                    return Err(format!(
                        "command {command_id} uses unsupported parameter encoding {encoding}"
                    ))
                }
            }
        }
        _ => {
            return Err(format!(
                "command {command_id} byte must have exactly one source"
            ))
        }
    };
    let target = report
        .get_mut(index..index + encoded.len())
        .ok_or_else(|| format!("command {command_id} byte range out of bounds"))?;
    target.copy_from_slice(&encoded);
    Ok(())
}

fn timeout_deadline(timeout_ms: Option<u64>) -> Result<Option<Instant>, String> {
    timeout_ms
        .map(|value| {
            if value == 0 {
                return Err("timeout_ms must be greater than zero".into());
            }
            Ok(Instant::now() + Duration::from_millis(value.min(MAX_OPERATION_TIMEOUT_MS)))
        })
        .transpose()
}

fn merge_deadline(
    current: Option<Instant>,
    timeout_ms: Option<u64>,
) -> Result<Option<Instant>, String> {
    let Some(next) = timeout_deadline(timeout_ms)? else {
        return Ok(current);
    };
    Ok(Some(match current {
        Some(existing) => existing.min(next),
        None => next,
    }))
}

fn deadline_remaining_ms(deadline: Option<Instant>) -> Result<Option<u64>, String> {
    match deadline {
        Some(deadline) => {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "operation timeout exceeded".to_string())?;
            Ok(Some(
                remaining.as_millis().max(1).min(u128::from(u64::MAX)) as u64
            ))
        }
        None => Ok(None),
    }
}

fn parse_rgb(value: &str) -> Result<[u8; 3], String> {
    let hex = value
        .strip_prefix('#')
        .filter(|value| value.len() == 6)
        .ok_or_else(|| "color must use #RRGGBB".to_string())?;
    Ok([
        u8::from_str_radix(&hex[0..2], 16).map_err(|_| "invalid red channel")?,
        u8::from_str_radix(&hex[2..4], 16).map_err(|_| "invalid green channel")?,
        u8::from_str_radix(&hex[4..6], 16).map_err(|_| "invalid blue channel")?,
    ])
}

fn verify_memory_checksum(memory: &[u8], algorithm: &str) -> Result<(), String> {
    if algorithm != "crc-ccitt-false" || memory.len() < 2 {
        return Err("unsupported memory checksum declaration".into());
    }
    let offset = memory.len() - 2;
    let stored = u16::from_be_bytes([memory[offset], memory[offset + 1]]);
    let calculated = crc_ccitt_false(&memory[..offset]);
    if stored != calculated {
        return Err(format!(
            "memory checksum mismatch: stored 0x{stored:04x}, calculated 0x{calculated:04x}"
        ));
    }
    Ok(())
}

fn is_memory_checksum_mismatch(error: &str) -> bool {
    error.contains("memory checksum mismatch:")
}

fn write_memory_checksum(memory: &mut [u8], algorithm: &str) -> Result<(), String> {
    if algorithm != "crc-ccitt-false" || memory.len() < 2 {
        return Err("unsupported memory checksum declaration".into());
    }
    let offset = memory.len() - 2;
    let checksum = crc_ccitt_false(&memory[..offset]).to_be_bytes();
    memory[offset..].copy_from_slice(&checksum);
    Ok(())
}

fn crc_ccitt_false(data: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in data {
        let temp = (crc >> 8) ^ u16::from(*byte);
        crc <<= 8;
        let quick = temp ^ (temp >> 4);
        crc ^= quick ^ (quick << 5) ^ (quick << 12);
    }
    crc
}

fn validate_mutation_inputs(
    definitions: &BTreeMap<String, MutationInput>,
    supplied: &Map<String, Value>,
) -> Result<BTreeMap<String, Value>, String> {
    // 只校验已声明的参数，忽略 supplied 中多余的字段。
    // 向后兼容：后端可向只声明 color+enabled 的旧插件 mutation 传递 effect/speed/brightness 等扩展参数。
    definitions
        .iter()
        .map(|(name, definition)| {
            let value = supplied
                .get(name)
                .ok_or_else(|| format!("missing mutation parameter {name}"))?;
            let normalized = match definition.kind.as_str() {
                "integer" => {
                    let integer = value
                        .as_u64()
                        .ok_or_else(|| format!("mutation parameter {name} must be an integer"))?;
                    if definition.min.is_some_and(|min| integer < min)
                        || definition.max.is_some_and(|max| integer > max)
                        || (!definition.allowed.is_empty()
                            && !definition.allowed.contains(&integer))
                        || definition.step.is_some_and(|step| {
                            step == 0
                                || integer.saturating_sub(definition.min.unwrap_or(0)) % step != 0
                        })
                    {
                        return Err(format!("mutation parameter {name} is out of range"));
                    }
                    Value::from(integer)
                }
                "boolean" => Value::from(
                    value
                        .as_bool()
                        .ok_or_else(|| format!("mutation parameter {name} must be boolean"))?,
                ),
                "color" => {
                    let color = value
                        .as_str()
                        .ok_or_else(|| format!("mutation parameter {name} must be a color"))?;
                    parse_rgb(color)?;
                    Value::from(color.to_ascii_uppercase())
                }
                kind => return Err(format!("unsupported mutation parameter kind {kind}")),
            };
            Ok((name.clone(), normalized))
        })
        .collect()
}

fn verify_assertions(
    value: &Value,
    params: &BTreeMap<String, Value>,
    assertions: &[MutationAssertion],
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "verification parser did not return an object".to_string())?;
    for assertion in assertions {
        let mut actual = object
            .get(&assertion.field)
            .ok_or_else(|| format!("missing verification field {}", assertion.field))?;
        if let Some(index_param) = &assertion.index_param {
            let index = params
                .get(index_param)
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("missing verification index {index_param}"))?
                .checked_sub(assertion.index_base)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| format!("invalid verification index {index_param}"))?;
            actual = actual
                .as_array()
                .and_then(|values| values.get(index))
                .ok_or_else(|| format!("verification index {index} is out of range"))?;
        }
        let expected = params
            .get(&assertion.param)
            .ok_or_else(|| format!("missing verification parameter {}", assertion.param))?;
        if actual != expected {
            return Err(format!(
                "field {} expected {} but read {}",
                assertion.field, expected, actual
            ));
        }
    }
    Ok(())
}

/// 将 derived lookup 表的字符串键（"0x01" 或 "1"）解析为 u64。
/// 用于预编译阶段，避免每次 parse_response 都格式化字符串。
fn parse_lookup_key(key: &str) -> Option<u64> {
    if let Some(hex) = key.strip_prefix("0x").or_else(|| key.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        key.parse::<u64>().ok()
    }
}

fn parse_field(field: &FieldDefinition, response: &[u8]) -> Result<Value, String> {
    let byte = || {
        response
            .get(field.offset)
            .copied()
            .ok_or_else(|| "parser offset out of range".to_string())
    };
    match field.kind.as_str() {
        "u8" => {
            let mut value = byte()?;
            if let Some(mask) = &field.mask {
                value &= parse_byte(mask)?;
            }
            if field.invert {
                value = u8::from(value == 0);
            }
            Ok(Value::from(value))
        }
        "bool" => {
            let mut value = byte()?;
            if let Some(mask) = &field.mask {
                value &= parse_byte(mask)?;
            }
            let value = value != 0;
            Ok(Value::Bool(if field.invert { !value } else { value }))
        }
        "le-u16" => {
            let bytes = response
                .get(field.offset..field.offset + 2)
                .ok_or_else(|| "parser u16 offset out of range".to_string())?;
            Ok(Value::from(u16::from_le_bytes([bytes[0], bytes[1]])))
        }
        "be-u16" => {
            let bytes = response
                .get(field.offset..field.offset + 2)
                .ok_or_else(|| "parser u16 offset out of range".to_string())?;
            Ok(Value::from(u16::from_be_bytes([bytes[0], bytes[1]])))
        }
        "le-u16-array" => {
            let count = field
                .count
                .ok_or_else(|| "array count missing".to_string())?;
            let bytes = response
                .get(field.offset..field.offset + count * 2)
                .ok_or_else(|| "parser array offset out of range".to_string())?;
            Ok(Value::Array(
                bytes
                    .chunks_exact(2)
                    .map(|pair| Value::from(u16::from_le_bytes([pair[0], pair[1]])))
                    .collect(),
            ))
        }
        "be-u16-array" => {
            let count = field
                .count
                .ok_or_else(|| "array count missing".to_string())?;
            let bytes = response
                .get(field.offset..field.offset + count * 2)
                .ok_or_else(|| "parser array offset out of range".to_string())?;
            Ok(Value::Array(
                bytes
                    .chunks_exact(2)
                    .map(|pair| Value::from(u16::from_be_bytes([pair[0], pair[1]])))
                    .collect(),
            ))
        }
        "bytes" => {
            let count = field
                .count
                .ok_or_else(|| "byte count missing".to_string())?;
            let bytes = response
                .get(field.offset..field.offset + count)
                .ok_or_else(|| "parser byte range out of range".to_string())?;
            Ok(Value::Array(
                bytes.iter().copied().map(Value::from).collect(),
            ))
        }
        "ascii" => {
            let count = field
                .count
                .ok_or_else(|| "parser ASCII field missing count".to_string())?;
            let bytes = response
                .get(field.offset..field.offset + count)
                .ok_or_else(|| "parser ASCII range out of bounds".to_string())?;
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            let text = std::str::from_utf8(&bytes[..end])
                .map_err(|_| "parser ASCII field is not valid UTF-8".to_string())?;
            if !text.is_ascii() {
                return Err("parser ASCII field contains non-ASCII text".into());
            }
            Ok(Value::from(text))
        }
        "rgb" => {
            let bytes = response
                .get(field.offset..field.offset + 3)
                .ok_or_else(|| "parser RGB offset out of range".to_string())?;
            Ok(Value::String(format!(
                "#{:02X}{:02X}{:02X}",
                bytes[0], bytes[1], bytes[2]
            )))
        }
        "rgb-array" => {
            let count = field
                .count
                .ok_or_else(|| "array count missing".to_string())?;
            let bytes = response
                .get(field.offset..field.offset + count * 3)
                .ok_or_else(|| "parser RGB array offset out of range".to_string())?;
            Ok(Value::Array(
                bytes
                    .chunks_exact(3)
                    .map(|rgb| {
                        Value::String(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]))
                    })
                    .collect(),
            ))
        }
        "hue-index-be-u16" => {
            let bytes = response
                .get(field.offset..field.offset + 2)
                .ok_or_else(|| "parser hue-index offset out of range".to_string())?;
            let idx = u16::from_be_bytes([bytes[0], bytes[1]]);
            Ok(Value::String(hue_index_to_hex(idx)))
        }
        other => Err(format!("unsupported parser field kind {other}")),
    }
}

/// 将 0-1530 的彩虹色相索引转换为 "#RRGGBB" 字符串。
///
/// 该编码使用 6 段渐变（每段 255），覆盖 HSV 色相轮的一圈：
///   0-255   红→黄   (r=255, g=0..255, b=0)
///   256-510 黄→绿   (r=255..0, g=255, b=0)
///   511-765 绿→青   (r=0, g=255, b=0..255)
///   766-1020 青→蓝  (r=0, g=255..0, b=255)
///   1021-1275 蓝→洋红 (r=0..255, g=0, b=255)
///   1276-1530 洋红→红 (r=255, g=0, b=255..0)
///
/// 65535 或其他超出 0-1530 范围的值返回白色 "#FFFFFF"。
fn hue_index_to_hex(idx: u16) -> String {
    if idx == 65535 || idx > 1530 {
        return "#FFFFFF".to_string();
    }
    let idx = idx as i32;
    let (r, g, b) = if idx <= 255 {
        (255, idx, 0)
    } else if idx <= 510 {
        (510 - idx, 255, 0)
    } else if idx <= 765 {
        (0, 255, idx - 510)
    } else if idx <= 1020 {
        (0, 1020 - idx, 255)
    } else if idx <= 1275 {
        (idx - 1020, 0, 255)
    } else {
        (255, 0, 1530 - idx)
    };
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

/// 将 RGB 颜色转换为 0-1530 的彩虹色相索引（`hue_index_to_hex` 的逆函数）。
///
/// 只接受恰好落在彩虹轮 6 段渐变上的"纯色"（如 r=255 且 b=0，或 g=255 且 b=0 等）。
/// 不满足任何段的混合色返回 65535（设备端显示为白色）。
fn rgb_to_hue_index(r: u8, g: u8, b: u8) -> u16 {
    let (r, g, b) = (r as u16, g as u16, b as u16);
    if r == 255 && b == 0 {
        g
    } else if g == 255 && b == 0 {
        510 - r
    } else if r == 0 && g == 255 {
        510 + b
    } else if r == 0 && b == 255 {
        1020 - g
    } else if g == 0 && b == 255 {
        1020 + r
    } else if r == 255 && g == 0 {
        1530 - b
    } else {
        65535
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    #[test]
    fn race_response_matches_request_command_after_report_id_is_stripped() {
        let request = [0x05, 0x5a, 0x02, 0x00, 0xcf, 0x30];
        let matching = [0x3d, 0x00, 0x05, 0x5b, 0x04, 0x00, 0xcf, 0x30, 0x01];
        let stale = [0x3d, 0x00, 0x05, 0x5b, 0x04, 0x00, 0xc5, 0x30, 0x01];
        assert!(race_response_matches_request(&matching, &request, true));
        assert!(!race_response_matches_request(&stale, &request, true));
    }

    #[test]
    fn race_response_matches_request_command_with_report_id_present() {
        let request = [0x05, 0x5a, 0x02, 0x00, 0xcf, 0x30];
        let response = [0x07, 0x3d, 0x00, 0x05, 0x5b, 0x04, 0x00, 0xcf, 0x30, 0x01];
        assert!(race_response_matches_request(&response, &request, false));
    }

    #[test]
    fn input_payload_accepts_report_id_or_already_stripped_payload() {
        assert_eq!(
            input_payload_from_report(vec![0x11, 0x01, 0x02, 0x03], 0x11, true),
            Some(vec![0x01, 0x02, 0x03])
        );
        assert_eq!(
            input_payload_from_report(vec![0x01, 0x02, 0x03], 0x11, true),
            Some(vec![0x01, 0x02, 0x03])
        );
        assert_eq!(
            input_payload_from_report(vec![0x01, 0x02, 0x03], 0x11, false),
            None
        );
    }

    #[test]
    fn output_report_fallback_only_matches_windows_invalid_function() {
        assert!(output_report_write_needs_feature_fallback(
            "hidapi error: WriteFile: (0x00000001) Incorrect function."
        ));
        assert!(!output_report_write_needs_feature_fallback(
            "hidapi error: WriteFile: (0x00000005) Access is denied."
        ));
        assert!(!output_report_write_needs_feature_fallback(
            "hidapi error: timeout waiting for device"
        ));
    }

    #[test]
    fn hidpp_short_output_can_be_padded_as_long_output() {
        assert_eq!(
            hidpp_short_output_as_long_report(&[0x10, 0x01, 0x0b, 0x81, 0, 0, 0]),
            Some(vec![
                0x11, 0x01, 0x0b, 0x81, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ])
        );
        assert_eq!(hidpp_short_output_as_long_report(&[0x11, 1, 2, 3]), None);
        assert_eq!(hidpp_short_output_as_long_report(&[0x10, 1, 2]), None);
    }

    #[test]
    fn parses_supported_field_kinds() {
        let response = [0x81, 0x34, 0x12, 1, 2, 3];
        let u16_field = FieldDefinition {
            offset: 1,
            kind: "le-u16".into(),
            count: None,
            mask: None,
            invert: false,
        };
        let rgb_field = FieldDefinition {
            offset: 3,
            kind: "rgb".into(),
            count: None,
            mask: None,
            invert: false,
        };
        assert_eq!(
            parse_field(&u16_field, &response).unwrap(),
            Value::from(0x1234)
        );
        assert_eq!(
            parse_field(&rgb_field, &response).unwrap(),
            Value::from("#010203")
        );
    }

    #[test]
    fn parses_be_u16_and_be_u16_array() {
        // HID++ 2.0 encodes DPI as big-endian u16: 0x06 0x40 = 1600.
        let response = [0x05, 0x10, 0x06, 0x40, 0x00, 0x00, 0x06, 0x40];
        let be_u16_field = FieldDefinition {
            offset: 2,
            kind: "be-u16".into(),
            count: None,
            mask: None,
            invert: false,
        };
        assert_eq!(
            parse_field(&be_u16_field, &response).unwrap(),
            Value::from(1600)
        );
        let be_u16_array_field = FieldDefinition {
            offset: 2,
            kind: "be-u16-array".into(),
            count: Some(3),
            mask: None,
            invert: false,
        };
        assert_eq!(
            parse_field(&be_u16_array_field, &response).unwrap(),
            Value::Array(vec![Value::from(1600), Value::from(0), Value::from(1600)])
        );
    }

    #[test]
    fn parses_hue_index_be_u16() {
        // 2 字节大端序色相索引（0-1530 的 6 段彩虹映射）。
        let response: [u8; 18] = [
            0x00, 0x00, // idx=0     → 红
            0x00, 0xFF, // idx=255   → 黄
            0x01, 0xFE, // idx=510   → 绿
            0x02, 0xFD, // idx=765   → 青
            0x03, 0xFC, // idx=1020  → 蓝
            0x04, 0xFB, // idx=1275  → 洋红
            0x05, 0xFA, // idx=1530  → 红（回到起点）
            0xFF, 0xFF, // idx=65535 → 白
            0x07, 0xD0, // idx=2000  → 白（超出范围）
        ];
        let cases = [
            (0usize, "#FF0000"),
            (2, "#FFFF00"),
            (4, "#00FF00"),
            (6, "#00FFFF"),
            (8, "#0000FF"),
            (10, "#FF00FF"),
            (12, "#FF0000"),
            (14, "#FFFFFF"),
            (16, "#FFFFFF"),
        ];
        for (offset, expected) in cases {
            let field = FieldDefinition {
                offset,
                kind: "hue-index-be-u16".into(),
                count: None,
                mask: None,
                invert: false,
            };
            assert_eq!(
                parse_field(&field, &response).unwrap(),
                Value::from(expected),
                "offset {offset} should map to {expected}"
            );
        }
    }

    #[test]
    fn rgb_to_hue_index_round_trips_pure_colors() {
        // 彩虹轮 6 段端点颜色应与 hue_index_to_hex 互逆。
        // 注意：#FF0000 同时是 idx=0（红→黄起点）和 idx=1530（洋红→红终点），
        // rgb_to_hue_index 返回第一个匹配（0），所以 1530 只做正向验证。
        let round_trip = [
            (0u16, "#FF0000"), // 红
            (255, "#FFFF00"),  // 黄
            (510, "#00FF00"),  // 绿
            (765, "#00FFFF"),  // 青
            (1020, "#0000FF"), // 蓝
            (1275, "#FF00FF"), // 洋红
        ];
        for (idx, hex) in round_trip {
            assert_eq!(hue_index_to_hex(idx), hex, "idx {idx} should map to {hex}");
            let [r, g, b] = parse_rgb(hex).unwrap();
            assert_eq!(
                rgb_to_hue_index(r, g, b),
                idx,
                "{hex} should map back to idx {idx}"
            );
        }
        // 1530 正向映射到 #FF0000，但逆向回到 0（红色是彩虹轮的起点和终点）
        assert_eq!(hue_index_to_hex(1530), "#FF0000");
    }

    #[test]
    fn rgb_to_hue_index_returns_white_for_mixed_colors() {
        // 不在彩虹轮上的混合色返回 65535。
        let mixed = ["#AABBCC", "#808080", "#112233", "#7F7F7F"];
        for hex in mixed {
            let [r, g, b] = parse_rgb(hex).unwrap();
            assert_eq!(rgb_to_hue_index(r, g, b), 65535, "{hex} should be 65535");
        }
    }

    #[test]
    fn parses_nul_terminated_ascii() {
        let response = b"\x01\x03\x11G705 Mouse\0unused";
        let field = FieldDefinition {
            offset: 3,
            kind: "ascii".into(),
            count: Some(16),
            mask: None,
            invert: false,
        };
        assert_eq!(
            parse_field(&field, response).unwrap(),
            Value::from("G705 Mouse")
        );
    }

    #[test]
    fn patches_indexed_values_without_replacing_unknown_bytes() {
        let mut report = vec![0xAA; 64];
        let params = BTreeMap::from([
            ("stage".into(), Value::from(2)),
            ("dpi".into(), Value::from(1600)),
        ]);
        let definition = ByteDefinition {
            offset: 8,
            value: None,
            param: Some("dpi".into()),
            encoding: Some("le-u16".into()),
            indexed_by: Some("stage".into()),
            index_base: 1,
            stride: 2,
            lookup: BTreeMap::new(),
        };
        apply_byte_definition("dpi-value-write", &definition, &params, &mut report).unwrap();
        assert_eq!(&report[10..12], &1600u16.to_le_bytes());
        assert_eq!(report[9], 0xAA);
        assert_eq!(report[12], 0xAA);
    }

    #[test]
    fn encodes_boolean_lookup_values() {
        let mut report = vec![0x00; 8];
        let params = BTreeMap::from([("enabled".into(), Value::from(true))]);
        let definition = ByteDefinition {
            offset: 4,
            value: None,
            param: Some("enabled".into()),
            encoding: Some("bool-lookup-u8".into()),
            indexed_by: None,
            index_base: 0,
            stride: 1,
            lookup: BTreeMap::from([("true".into(), 0x03), ("false".into(), 0x00)]),
        };
        apply_byte_definition("rgb-control-set", &definition, &params, &mut report).unwrap();
        assert_eq!(report[4], 0x03);
    }

    #[test]
    fn mutation_inputs_are_exact_and_bounded() {
        let definitions = BTreeMap::from([(
            "dpi".into(),
            MutationInput {
                kind: "integer".into(),
                min: Some(50),
                max: Some(30_000),
                step: Some(50),
                allowed: Vec::new(),
            },
        )]);
        assert!(validate_mutation_inputs(
            &definitions,
            &Map::from_iter([("dpi".into(), Value::from(800))])
        )
        .is_ok());
        assert!(validate_mutation_inputs(
            &definitions,
            &Map::from_iter([("dpi".into(), Value::from(825))])
        )
        .is_err());
        // 多余参数被忽略（向后兼容：后端可向旧插件传递扩展参数）
        assert!(validate_mutation_inputs(
            &definitions,
            &Map::from_iter([
                ("dpi".into(), Value::from(800)),
                ("raw".into(), Value::from(1)),
            ])
        )
        .is_ok());
    }

    #[test]
    fn validate_mutation_inputs_ignores_extra_params() {
        // 模拟 amaster set-mouse-lighting：只声明 color+enabled，但后端传递 effect/speed/brightness
        let definitions = BTreeMap::from([
            (
                "color".into(),
                MutationInput {
                    kind: "color".into(),
                    min: None,
                    max: None,
                    step: None,
                    allowed: Vec::new(),
                },
            ),
            (
                "enabled".into(),
                MutationInput {
                    kind: "boolean".into(),
                    min: None,
                    max: None,
                    step: None,
                    allowed: Vec::new(),
                },
            ),
        ]);
        let supplied = Map::from_iter([
            ("color".into(), Value::from("#FF0000")),
            ("enabled".into(), Value::from(true)),
            ("effect".into(), Value::from(0)),
            ("speed".into(), Value::from(100)),
            ("brightness".into(), Value::from(50)),
        ]);
        let result = validate_mutation_inputs(&definitions, &supplied);
        assert!(result.is_ok(), "extra params should be ignored");
        let validated = result.unwrap();
        assert_eq!(
            validated.len(),
            2,
            "only declared params should be returned"
        );
        assert_eq!(validated.get("color"), Some(&Value::from("#FF0000")));
        assert_eq!(validated.get("enabled"), Some(&Value::from(true)));
    }

    #[test]
    fn validate_mutation_inputs_still_validates_declared_params() {
        let definitions = BTreeMap::from([(
            "effect".into(),
            MutationInput {
                kind: "integer".into(),
                min: Some(0),
                max: Some(12),
                step: None,
                allowed: vec![0, 1, 3, 4, 5, 10, 11, 12],
            },
        )]);
        // 有效值 + 多余参数
        assert!(validate_mutation_inputs(
            &definitions,
            &Map::from_iter([
                ("effect".into(), Value::from(5)),
                ("extra".into(), Value::from("ignored")),
            ])
        )
        .is_ok());
        assert!(validate_mutation_inputs(
            &definitions,
            &Map::from_iter([
                ("effect".into(), Value::from(99)),
                ("extra".into(), Value::from("ignored")),
            ])
        )
        .is_err());
    }

    #[test]
    fn resolves_workflow_params_from_prior_outputs() {
        let params = BTreeMap::from([(
            "featureIndex".into(),
            serde_json::json!({"fromOutput": "batteryFeature", "field": "featureIndex"}),
        )]);
        let outputs = BTreeMap::from([(
            "batteryFeature".into(),
            serde_json::json!({"featureIndex": 7}),
        )]);
        let resolved = resolve_workflow_params(&params, &outputs).unwrap();
        assert_eq!(resolved.get("featureIndex"), Some(&Value::from(7)));

        let offset_params = BTreeMap::from([(
            "offset".into(),
            serde_json::json!({"fromOutput": "description", "field": "sectorSize", "subtract": 16}),
        )]);
        let offset_outputs =
            BTreeMap::from([("description".into(), serde_json::json!({"sectorSize": 254}))]);
        let resolved = resolve_workflow_params(&offset_params, &offset_outputs).unwrap();
        assert_eq!(resolved.get("offset"), Some(&Value::from(238)));

        let guard = OutputReference {
            output: "batteryFeature".into(),
            field: "featureIndex".into(),
        };
        assert_eq!(output_value(&outputs, &guard), Some(&Value::from(7)));
    }

    #[test]
    fn memory_checksum_round_trip_detects_corruption() {
        let mut memory: Vec<u8> = (0..255).map(|value| value as u8).collect();
        write_memory_checksum(&mut memory, "crc-ccitt-false").unwrap();
        assert!(verify_memory_checksum(&memory, "crc-ccitt-false").is_ok());
        memory[17] ^= 0xff;
        // verify_memory_checksum still returns Err for mismatches — the calling
        // code (read_memory_mutation) decides whether to tolerate the mismatch
        // (logging a warning) or fail. This mirrors libratbag's behavior where
        // hidpp20_onboard_profiles_is_sector_valid logs a debug message but
        // does not abort the sector read.
        assert!(verify_memory_checksum(&memory, "crc-ccitt-false").is_err());
    }

    #[test]
    fn identifies_memory_checksum_mismatch_errors() {
        assert!(is_memory_checksum_mismatch(
            "memory checksum mismatch: stored 0x038e, calculated 0xbef1"
        ));
        assert!(!is_memory_checksum_mismatch(
            "memory workflow returned an invalid size"
        ));
    }

    #[test]
    fn xor8_request_checksum_is_computed_correctly() {
        let commands = r#"{
            "schemaVersion": 1,
            "commands": {
                "xor8-cmd": {
                    "request": {
                        "length": 6,
                        "base": "zero",
                        "bytes": [
                            {"offset": 0, "value": "0x01"},
                            {"offset": 1, "value": "0x02"},
                            {"offset": 2, "value": "0x03"},
                            {"offset": 3, "value": "0x04"}
                        ],
                        "checksum": {
                            "algorithm": "xor8",
                            "start": 0,
                            "endExclusive": 4,
                            "writeOffset": 5
                        }
                    }
                }
            }
        }"#;
        let package = build_test_package(
            commands,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            r#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#,
        );
        let report = package
            .build_command("xor8-cmd", &BTreeMap::new(), None)
            .unwrap();
        // 0x01 ^ 0x02 ^ 0x03 ^ 0x04 = 0x04, written to offset 5.
        assert_eq!(report, vec![0x01, 0x02, 0x03, 0x04, 0x00, 0x04]);
    }

    #[test]
    fn ff_minus_sum8_request_checksum_still_supported() {
        let commands = r#"{
            "schemaVersion": 1,
            "commands": {
                "sum8-cmd": {
                    "request": {
                        "length": 4,
                        "base": "zero",
                        "bytes": [
                            {"offset": 0, "value": "0x01"},
                            {"offset": 1, "value": "0x02"}
                        ],
                        "checksum": {
                            "algorithm": "ff-minus-sum8",
                            "start": 0,
                            "endExclusive": 2,
                            "writeOffset": 3
                        }
                    }
                }
            }
        }"#;
        let package = build_test_package(
            commands,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            r#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#,
        );
        let report = package
            .build_command("sum8-cmd", &BTreeMap::new(), None)
            .unwrap();
        // 0xFF - (0x01 + 0x02) = 0xFC, written to offset 3.
        assert_eq!(report, vec![0x01, 0x02, 0x00, 0xFC]);
    }

    #[test]
    fn unknown_request_checksum_algorithm_is_rejected() {
        let commands = r#"{
            "schemaVersion": 1,
            "commands": {
                "bad-cmd": {
                    "request": {
                        "length": 4,
                        "base": "zero",
                        "bytes": [],
                        "checksum": {
                            "algorithm": "crc32",
                            "start": 0,
                            "endExclusive": 2,
                            "writeOffset": 3
                        }
                    }
                }
            }
        }"#;
        let package = build_test_package(
            commands,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            r#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#,
        );
        // compile_commands 在加载时预构建无 param 命令并忽略错误，
        // 因此缓存为空；运行时 build_command 重新构建并返回错误。
        assert!(package
            .build_command("bad-cmd", &BTreeMap::new(), None)
            .is_err());
    }

    #[test]
    fn parses_bitmap_derived_field() {
        let mut files = BTreeMap::new();
        files.insert(
            "protocol/commands.json".into(),
            br#"{"schemaVersion": 1, "commands": {}}"#.to_vec(),
        );
        files.insert(
            "protocol/parsers.json".into(),
            br#"{
                "schemaVersion": 1,
                "parsers": {
                    "rate-list": {
                        "validWhen": [],
                        "fields": {
                            "rateListFlags": {"offset": 3, "kind": "u8"}
                        },
                        "derived": {
                            "supportedRates": {
                                "kind": "bitmap",
                                "source": "rateListFlags",
                                "bitmap": [
                                    {"bit": 0, "value": 1000},
                                    {"bit": 1, "value": 500},
                                    {"bit": 3, "value": 250},
                                    {"bit": 7, "value": 125}
                                ]
                            }
                        }
                    }
                }
            }"#
            .to_vec(),
        );
        files.insert(
            "protocol/transports.json".into(),
            br#"{"schemaVersion": 1, "transports": {}}"#.to_vec(),
        );
        files.insert(
            "protocol/workflows.json".into(),
            br#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#.to_vec(),
        );
        let package = ProtocolPackage::from_files(&files).unwrap();
        // 0b00001011 -> bit 0 (1000), bit 1 (500), bit 3 (250) are set.
        let response = [0u8; 20];
        let mut response = response;
        response[3] = 0x0B;
        let parsed = package.parse_response("rate-list", &response).unwrap();
        let rates = parsed
            .get("supportedRates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap();
        assert_eq!(
            rates,
            vec![Value::from(1000), Value::from(500), Value::from(250)]
        );
    }

    /// Build a `ProtocolPackage` from raw JSON file contents for testing.
    fn build_test_package(
        commands: &str,
        parsers: &str,
        transports: &str,
        workflows: &str,
    ) -> ProtocolPackage {
        let mut files = BTreeMap::new();
        files.insert(
            "protocol/commands.json".into(),
            commands.as_bytes().to_vec(),
        );
        files.insert("protocol/parsers.json".into(), parsers.as_bytes().to_vec());
        files.insert(
            "protocol/transports.json".into(),
            transports.as_bytes().to_vec(),
        );
        files.insert(
            "protocol/workflows.json".into(),
            workflows.as_bytes().to_vec(),
        );
        ProtocolPackage::from_files(&files).unwrap()
    }

    fn test_files(
        commands: &str,
        parsers: &str,
        transports: &str,
        workflows: &str,
    ) -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            (
                "protocol/commands.json".into(),
                commands.as_bytes().to_vec(),
            ),
            ("protocol/parsers.json".into(), parsers.as_bytes().to_vec()),
            (
                "protocol/transports.json".into(),
                transports.as_bytes().to_vec(),
            ),
            (
                "protocol/workflows.json".into(),
                workflows.as_bytes().to_vec(),
            ),
        ])
    }

    #[test]
    fn mutation_ids_keep_lighting_writes_scoped_to_protocol_family() {
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            r#"{
                "schemaVersion": 1,
                "workflows": {},
                "mutations": {
                    "am35-receiver-set-mouse-lighting": {
                        "transport": "am35-receiver",
                        "inputs": {},
                        "read": { "command": "noop", "parser": "noop" },
                        "writeCommand": "noop",
                        "preserveUnknown": false,
                        "verify": { "command": "noop", "parser": "noop", "assertions": [] }
                    },
                    "protocol-a-receiver-set-receiver-lighting": {
                        "transport": "protocol-a",
                        "inputs": {},
                        "read": { "command": "noop", "parser": "noop" },
                        "writeCommand": "noop",
                        "preserveUnknown": false,
                        "verify": { "command": "noop", "parser": "noop", "assertions": [] }
                    }
                }
            }"#,
        );

        assert_eq!(
            package.mutation_ids("am35-receiver", None),
            vec!["set-mouse-lighting"]
        );
        assert_eq!(
            package.mutation_ids("protocol-a-receiver", None),
            vec!["set-receiver-lighting"]
        );
    }

    /// 构建工作流投影测试用的包。
    ///
    /// 工作流结构：
    /// - step 0: root-get-feature → "device" (设备信息，无依赖)
    /// - step 1: get-battery → "battery" (params 引用 device.deviceIndex)
    /// - step 2: get-dpi → "dpi" (params 引用 device.deviceIndex)
    /// - step 3: get-firmware → "firmware" (无依赖)
    /// - step 4: get-lighting → "lighting" (skip_if_zero 引用 device.supportsLighting)
    fn build_projection_test_package() -> ProtocolPackage {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {
                "test-read": {
                    "transport": "feature",
                    "steps": [
                        {
                            "command": "root-get-feature",
                            "parser": "device-info",
                            "output": "device"
                        },
                        {
                            "command": "get-battery",
                            "parser": "battery",
                            "output": "battery",
                            "params": {
                                "deviceIndex": {"fromOutput": "device", "field": "deviceIndex"}
                            }
                        },
                        {
                            "command": "get-dpi",
                            "parser": "dpi",
                            "output": "dpi",
                            "params": {
                                "deviceIndex": {"fromOutput": "device", "field": "deviceIndex"}
                            }
                        },
                        {
                            "command": "get-firmware",
                            "parser": "firmware",
                            "output": "firmware"
                        },
                        {
                            "command": "get-lighting",
                            "parser": "lighting",
                            "output": "lighting",
                            "skipIfZero": [
                                {"output": "device", "field": "supportsLighting"}
                            ]
                        }
                    ]
                }
            },
            "mutations": {}
        }"#;
        build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {
                "feature": {"kind": "hid-feature", "reportId": 16, "writeLength": 20, "readLength": 20, "stripReportIdOnRead": true}
            }}"#,
            workflows,
        )
    }

    #[test]
    fn projection_selects_target_and_dependencies() {
        let package = build_projection_test_package();
        let mut targets = BTreeSet::new();
        targets.insert("battery".to_string());

        let projection = package.compute_projection("test-read", &targets);
        assert!(projection.is_valid());
        // 应选中 step 0 (device) 和 step 1 (battery)
        assert_eq!(projection.selected_steps(), &[0, 1]);
    }

    #[test]
    fn projection_selects_multiple_targets_with_shared_dependency() {
        let package = build_projection_test_package();
        let mut targets = BTreeSet::new();
        targets.insert("battery".to_string());
        targets.insert("dpi".to_string());

        let projection = package.compute_projection("test-read", &targets);
        assert!(projection.is_valid());
        // 应选中 step 0 (device), step 1 (battery), step 2 (dpi)
        // device 是 battery 和 dpi 的共享依赖
        assert_eq!(projection.selected_steps(), &[0, 1, 2]);
    }

    #[test]
    fn projection_independent_step_has_no_dependency() {
        let package = build_projection_test_package();
        let mut targets = BTreeSet::new();
        targets.insert("firmware".to_string());

        let projection = package.compute_projection("test-read", &targets);
        assert!(projection.is_valid());
        // firmware 无依赖，只选中 step 3
        assert_eq!(projection.selected_steps(), &[3]);
    }

    #[test]
    fn projection_includes_skip_if_zero_dependency() {
        let package = build_projection_test_package();
        let mut targets = BTreeSet::new();
        targets.insert("lighting".to_string());

        let projection = package.compute_projection("test-read", &targets);
        assert!(projection.is_valid());
        // lighting 的 skip_if_zero 引用 device.supportsLighting
        // 所以 device (step 0) 是依赖
        assert_eq!(projection.selected_steps(), &[0, 4]);
    }

    #[test]
    fn projection_all_targets_selects_all_dependent_steps() {
        let package = build_projection_test_package();
        let mut targets = BTreeSet::new();
        targets.insert("battery".to_string());
        targets.insert("dpi".to_string());
        targets.insert("lighting".to_string());

        let projection = package.compute_projection("test-read", &targets);
        assert!(projection.is_valid());
        // 选中 step 0 (device, 共享依赖), 1 (battery), 2 (dpi), 4 (lighting)
        // step 3 (firmware) 不在依赖链中
        assert_eq!(projection.selected_steps(), &[0, 1, 2, 4]);
    }

    #[test]
    fn projection_missing_target_returns_fallback_reason() {
        let package = build_projection_test_package();
        let mut targets = BTreeSet::new();
        targets.insert("nonexistent".to_string());

        let projection = package.compute_projection("test-read", &targets);
        assert!(!projection.is_valid());
        assert!(projection.fallback_reason().is_some());
    }

    #[test]
    fn projection_missing_workflow_returns_fallback_reason() {
        let package = build_projection_test_package();
        let targets = BTreeSet::new();

        let projection = package.compute_projection("nonexistent-read", &targets);
        assert!(!projection.is_valid());
        assert!(projection.fallback_reason().is_some());
    }

    #[test]
    fn projection_preserves_step_order() {
        let package = build_projection_test_package();
        let mut targets = BTreeSet::new();
        // 请求靠后的 output，验证依赖步骤按原始顺序排列
        targets.insert("lighting".to_string());
        targets.insert("battery".to_string());

        let projection = package.compute_projection("test-read", &targets);
        assert!(projection.is_valid());
        // 选中步骤应按原始顺序：0, 1, 4
        assert_eq!(projection.selected_steps(), &[0, 1, 4]);
    }

    #[test]
    fn available_outputs_returns_all_step_outputs() {
        let package = build_projection_test_package();
        let outputs = package.available_outputs("test-read");
        assert_eq!(outputs.len(), 5);
        assert!(outputs.contains("device"));
        assert!(outputs.contains("battery"));
        assert!(outputs.contains("dpi"));
        assert!(outputs.contains("firmware"));
        assert!(outputs.contains("lighting"));
    }

    #[test]
    fn dependency_transports_are_merged_without_overriding_main() {
        let main = test_files(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {
                "main": {"kind": "hid-feature", "reportId": 16, "writeLength": 2, "readLength": 2, "stripReportIdOnRead": true},
                "shared": {"kind": "hid-feature", "reportId": 17, "writeLength": 2, "readLength": 2, "stripReportIdOnRead": true}
            }}"#,
            r#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#,
        );
        let dependency = test_files(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {
                "base": {"kind": "hid-feature", "reportId": 18, "writeLength": 2, "readLength": 2, "stripReportIdOnRead": true},
                "shared": {"kind": "hid-feature", "reportId": 19, "writeLength": 2, "readLength": 2, "stripReportIdOnRead": true}
            }}"#,
            r#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#,
        );
        let package =
            ProtocolPackage::from_files_with_model_and_dependencies(&main, None, &[&dependency])
                .unwrap();
        assert!(package.transports.transports.contains_key("base"));
        match package.transports.transports.get("shared").unwrap() {
            TransportDefinition::HidFeature { report_id, .. } => assert_eq!(*report_id, 17),
            _ => panic!("unexpected transport kind"),
        }
    }

    #[test]
    fn transaction_lookup_requires_exact_mutation_id() {
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            r#"{
                "schemaVersion": 1,
                "workflows": {},
                "mutations": {},
                "transactions": {
                    "profile": {
                        "mutations": ["mouse-set-dpi"],
                        "snapshotWorkflow": "snapshot",
                        "rollbackWorkflow": "rollback",
                        "timeoutMs": 500
                    }
                }
            }"#,
        );
        // 精确匹配：事务声明完整 mutation id，短 id 或无关 id 不再匹配。
        assert!(package
            .transaction_for_mutation("mouse-set-dpi")
            .unwrap()
            .is_some());
        assert!(package
            .transaction_for_mutation("set-dpi")
            .unwrap()
            .is_none());
        assert!(package
            .transaction_for_mutation("set-rate")
            .unwrap()
            .is_none());
    }

    #[test]
    fn transaction_lookup_rejects_mutation_in_multiple_transactions() {
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            r#"{
                "schemaVersion": 1,
                "workflows": {},
                "mutations": {},
                "transactions": {
                    "profile-a": {
                        "mutations": ["mouse-set-dpi"],
                        "snapshotWorkflow": "snapshot",
                        "rollbackWorkflow": "rollback",
                        "timeoutMs": 500
                    },
                    "profile-b": {
                        "mutations": ["mouse-set-dpi"],
                        "snapshotWorkflow": "snapshot",
                        "rollbackWorkflow": "rollback",
                        "timeoutMs": 500
                    }
                }
            }"#,
        );
        // 同一 mutation 属于多个事务时必须报错，避免 snapshot/rollback 选择不确定。
        assert!(package.transaction_for_mutation("mouse-set-dpi").is_err());
    }

    #[test]
    fn timeout_deadline_rejects_zero_and_caps_large_values() {
        assert!(timeout_deadline(Some(0)).is_err());
        assert!(timeout_deadline(Some(MAX_OPERATION_TIMEOUT_MS + 1)).is_ok());
        assert!(merge_deadline(None, Some(10)).unwrap().is_some());
    }

    /// Shared `HidApi` instance for tests. `HidApi::new()` enumerates all HID
    /// devices and can crash (SIGTRAP) if called multiple times on macOS;
    /// `OnceLock` ensures it is initialised only once.
    static TEST_HID_API: OnceLock<HidApi> = OnceLock::new();
    fn test_hid_api() -> &'static HidApi {
        TEST_HID_API.get_or_init(|| HidApi::new().unwrap())
    }

    /// `mutate` must reject unknown mutation ids before touching the device.
    #[test]
    fn mutate_rejects_missing_mutation() {
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            r#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#,
        );
        let api = test_hid_api();
        let outputs = BTreeMap::new();
        let result = package.mutate(
            api,
            "test-path",
            "nonexistent",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing mutation"));
    }

    /// `mutate` must honor `skipIfZero` and reject the mutation without I/O.
    #[test]
    fn mutate_rejects_when_skip_if_zero_matches() {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-dpi": {
                    "transport": "feature",
                    "inputs": {},
                    "skipIfZero": [{"output": "dpi", "field": "value"}],
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        let outputs = BTreeMap::from([("dpi".into(), serde_json::json!({"value": 0}))]);
        let result = package.mutate(
            api,
            "test-path",
            "test-set-dpi",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available on this device"));
    }

    /// `skipIfZero` must also trigger for bool `false` values, not just numeric 0.
    /// `Value::Bool(false).as_u64()` returns `None` (not `Some(0)`), so a naive
    /// `as_u64() == Some(0)` check fails. This test guards the
    /// `output_reference_is_zero` helper that bridges that gap.
    #[test]
    fn mutate_rejects_when_skip_if_zero_matches_bool_false() {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-dpi": {
                    "transport": "feature",
                    "inputs": {},
                    "skipIfZero": [{"output": "receiverIdle", "field": "mouseOnline"}],
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        let outputs = BTreeMap::from([(
            "receiverIdle".into(),
            serde_json::json!({"mouseOnline": false}),
        )]);
        let result = package.mutate(
            api,
            "test-path",
            "test-set-dpi",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available on this device"));
    }

    /// `skipIfNonZero` must reject the mutation when any referenced output is non-zero.
    /// This gates `set-mouse-lighting-onboard` away from devices that have COLOR_LED_EFFECTS (0x8070)
    /// or already use the onboard format V5 path.
    #[test]
    fn mutate_rejects_when_skip_if_non_zero_matches() {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-lighting": {
                    "transport": "feature",
                    "inputs": {},
                    "skipIfNonZero": [{"output": "colorLed", "field": "featureIndex"}],
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        // featureIndex=1 (non-zero) → skipIfNonZero triggers
        let outputs = BTreeMap::from([("colorLed".into(), serde_json::json!({"featureIndex": 1}))]);
        let result = package.mutate(
            api,
            "test-path",
            "test-set-lighting",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available on this device"));
    }

    /// `skipIfAllZero` must reject the mutation when ALL referenced outputs are zero.
    /// This gates `set-mouse-lighting-onboard` away from devices that lack ONBOARD_PROFILES (0x8100).
    #[test]
    fn mutate_rejects_when_skip_if_all_zero_matches() {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-lighting": {
                    "transport": "feature",
                    "inputs": {},
                    "skipIfAllZero": [{"output": "onboardProfiles", "field": "featureIndex"}],
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        // featureIndex=0 → skipIfAllZero triggers (all refs are zero)
        let outputs = BTreeMap::from([(
            "onboardProfiles".into(),
            serde_json::json!({"featureIndex": 0}),
        )]);
        let result = package.mutate(
            api,
            "test-path",
            "test-set-lighting",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available on this device"));
    }

    #[test]
    fn mutation_ids_require_a_real_direct_or_memory_write_path() {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-lighting": {
                    "transport": "feature",
                    "inputs": {},
                    "skipIfAllZero": [
                        {"output": "colorLed", "field": "featureIndex"},
                        {"output": "onboardProfiles", "field": "featureIndex"}
                    ],
                    "writeSkipIfZero": [{"output": "colorLed", "field": "featureIndex"}],
                    "read": {"command": "read-lighting", "parser": "lighting", "params": {}},
                    "writeCommand": "write-lighting",
                    "writeParams": {},
                    "memory": {
                        "readWorkflow": "test-onboard-read",
                        "availableWhen": {"output": "onboardProfiles", "field": "featureIndex"},
                        "enabledWhen": {"output": "onboardMode", "field": "mode", "eq": 1},
                        "requiredWhen": [{"output": "onboardDescription", "field": "profileFormatId", "eq": 5}],
                        "size": {"output": "onboardDescription", "field": "sectorSize"},
                        "chunkOutputPrefix": "chunk",
                        "chunkField": "bytes",
                        "chunkSize": 16,
                        "checksum": "crc-ccitt-false",
                        "patches": [{"offset": 0, "value": "0x01"}],
                        "transport": "feature",
                        "startCommand": "memory-start",
                        "chunkCommand": "memory-chunk",
                        "endTransport": "feature",
                        "endCommand": "memory-end"
                    },
                    "preserveUnknown": false,
                    "verify": {"command": "verify-lighting", "parser": "lighting", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );

        let direct_outputs = BTreeMap::from([
            ("colorLed".into(), serde_json::json!({"featureIndex": 7})),
            (
                "onboardProfiles".into(),
                serde_json::json!({"featureIndex": 11}),
            ),
            ("onboardMode".into(), serde_json::json!({"mode": 1})),
            (
                "onboardDescription".into(),
                serde_json::json!({"profileFormatId": 3, "sectorSize": 255}),
            ),
        ]);
        assert_eq!(
            package.mutation_ids("test", Some(&direct_outputs)),
            vec!["set-lighting"]
        );

        let memory_outputs = BTreeMap::from([
            ("colorLed".into(), serde_json::json!({"featureIndex": 0})),
            (
                "onboardProfiles".into(),
                serde_json::json!({"featureIndex": 11}),
            ),
            ("onboardMode".into(), serde_json::json!({"mode": 1})),
            (
                "onboardDescription".into(),
                serde_json::json!({"profileFormatId": 5, "sectorSize": 255}),
            ),
        ]);
        assert_eq!(
            package.mutation_ids("test", Some(&memory_outputs)),
            vec!["set-lighting"]
        );

        let unavailable_outputs = BTreeMap::from([
            ("colorLed".into(), serde_json::json!({"featureIndex": 0})),
            (
                "onboardProfiles".into(),
                serde_json::json!({"featureIndex": 11}),
            ),
            ("onboardMode".into(), serde_json::json!({"mode": 1})),
            (
                "onboardDescription".into(),
                serde_json::json!({"profileFormatId": 3, "sectorSize": 255}),
            ),
        ]);
        assert!(package
            .mutation_ids("test", Some(&unavailable_outputs))
            .is_empty());

        let api = test_hid_api();
        let result = package.mutate(
            api,
            "test-path",
            "test-set-lighting",
            &Map::new(),
            &unavailable_outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available on this device"));
    }

    /// `lookup` derived kind must return the mapped value when the source matches a table key,
    /// and `Value::Null` when the source does not match any key.
    /// This mirrors the `onboardFormatV5` derived field: profileFormatId=5 → 1, other → null.
    #[test]
    fn parses_lookup_derived_field() {
        let mut files = BTreeMap::new();
        files.insert(
            "protocol/commands.json".into(),
            br#"{"schemaVersion": 1, "commands": {}}"#.to_vec(),
        );
        files.insert(
            "protocol/parsers.json".into(),
            br#"{
                "schemaVersion": 1,
                "parsers": {
                    "onboard-description": {
                        "validWhen": [],
                        "fields": {
                            "profileFormatId": {"offset": 3, "kind": "u8"}
                        },
                        "derived": {
                            "onboardFormatV5": {
                                "kind": "lookup",
                                "source": "profileFormatId",
                                "table": {"5": 1}
                            }
                        }
                    }
                }
            }"#
            .to_vec(),
        );
        files.insert(
            "protocol/transports.json".into(),
            br#"{"schemaVersion": 1, "transports": {}}"#.to_vec(),
        );
        files.insert(
            "protocol/workflows.json".into(),
            br#"{"schemaVersion": 1, "workflows": {}, "mutations": {}}"#.to_vec(),
        );
        let package = ProtocolPackage::from_files(&files).unwrap();

        // profileFormatId=5 → onboardFormatV5=1
        let mut response = [0u8; 20];
        response[3] = 5;
        let parsed = package
            .parse_response("onboard-description", &response)
            .unwrap();
        assert_eq!(parsed.get("onboardFormatV5"), Some(&Value::from(1)));

        // profileFormatId=3 → onboardFormatV5=null (not in table)
        let mut response = [0u8; 20];
        response[3] = 3;
        let parsed = package
            .parse_response("onboard-description", &response)
            .unwrap();
        assert_eq!(parsed.get("onboardFormatV5"), Some(&Value::Null));
    }

    /// `mutate` must reject `settleMs` exceeding the 1-second safety limit.
    #[test]
    fn mutate_rejects_excessive_settle_ms() {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-dpi": {
                    "transport": "feature",
                    "inputs": {},
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "settleMs": 2000,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        let outputs = BTreeMap::new();
        let result = package.mutate(
            api,
            "test-path",
            "test-set-dpi",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("settle delay exceeds limit"));
    }

    /// `mutate` must reject when the write command is missing from commands.json.
    #[test]
    fn mutate_rejects_missing_write_command() {
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-dpi": {
                    "transport": "feature",
                    "inputs": {},
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            r#"{"schemaVersion": 1, "commands": {}}"#,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        let outputs = BTreeMap::new();
        let result = package.mutate(
            api,
            "test-path",
            "test-set-dpi",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing command"));
    }

    /// `mutate` must reject when `preserveUnknown` doesn't match the write
    /// command's `base` field (ReadResponse vs Zero).
    #[test]
    fn mutate_rejects_write_strategy_mismatch() {
        let commands = r#"{
            "schemaVersion": 1,
            "commands": {
                "write-dpi": {
                    "request": {
                        "length": 7,
                        "base": "read-response",
                        "bytes": []
                    }
                }
            }
        }"#;
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-dpi": {
                    "transport": "feature",
                    "inputs": {},
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            commands,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        let outputs = BTreeMap::new();
        let result = package.mutate(
            api,
            "test-path",
            "test-set-dpi",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("write strategy does not match"));
    }

    /// `mutate` must accept a consistent write strategy (base=zero +
    /// preserveUnknown=false) and proceed to I/O, where it fails on the
    /// nonexistent device path — proving validation passed.
    #[test]
    fn mutate_accepts_consistent_write_strategy() {
        let commands = r#"{
            "schemaVersion": 1,
            "commands": {
                "write-dpi": {
                    "request": {
                        "length": 7,
                        "base": "zero",
                        "bytes": []
                    }
                }
            }
        }"#;
        let workflows = r#"{
            "schemaVersion": 1,
            "workflows": {},
            "mutations": {
                "test-set-dpi": {
                    "transport": "feature",
                    "inputs": {},
                    "read": {"command": "read-dpi", "parser": "dpi", "params": {}},
                    "writeCommand": "write-dpi",
                    "preserveUnknown": false,
                    "verify": {"command": "verify-dpi", "parser": "dpi", "params": {}, "assertions": []}
                }
            }
        }"#;
        let package = build_test_package(
            commands,
            r#"{"schemaVersion": 1, "parsers": {}}"#,
            r#"{"schemaVersion": 1, "transports": {}}"#,
            workflows,
        );
        let api = test_hid_api();
        let outputs = BTreeMap::new();
        let result = package.mutate(
            api,
            "nonexistent-path",
            "test-set-dpi",
            &Map::new(),
            &outputs,
            None,
            None,
            None,
        );
        // Validation passed; the error should be from I/O (open_path), not validation.
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            !err.contains("write strategy"),
            "should not fail on validation: {err}"
        );
        assert!(
            !err.contains("missing command"),
            "should not fail on validation: {err}"
        );
    }

    #[test]
    fn deep_merge_objects_recursive() {
        // 修复 #2：Object+Object 应递归合并
        let base = serde_json::json!({"a": 1, "b": {"c": 2, "d": 3}});
        let overlay = serde_json::json!({"b": {"d": 99}, "e": 4});
        let merged = deep_merge_json(base, overlay);
        assert_eq!(
            merged,
            serde_json::json!({"a": 1, "b": {"c": 2, "d": 99}, "e": 4})
        );
    }

    #[test]
    fn deep_merge_array_replaces_entirely() {
        // Array 整体替换，不做元素级合并
        let base = serde_json::json!({"items": [1, 2, 3]});
        let overlay = serde_json::json!({"items": [9]});
        let merged = deep_merge_json(base, overlay);
        assert_eq!(merged, serde_json::json!({"items": [9]}));
    }

    #[test]
    fn deep_merge_null_overrides() {
        // null 作为 overlay 覆盖 base
        let base = serde_json::json!({"a": 1});
        let overlay = serde_json::json!({"a": null});
        let merged = deep_merge_json(base, overlay);
        assert_eq!(merged, serde_json::json!({"a": null}));
    }

    #[test]
    fn deep_merge_scalar_overrides() {
        let base = serde_json::json!("base");
        let overlay = serde_json::json!("overlay");
        let merged = deep_merge_json(base, overlay);
        assert_eq!(merged, serde_json::json!("overlay"));
    }

    #[test]
    fn from_files_with_model_rejects_path_separators() {
        // 修复 #2：model 含路径分隔符应被拒绝，防止路径遍历
        let files = BTreeMap::new();
        let result = ProtocolPackage::from_files_with_model(&files, Some("../escape"));
        match result {
            Err(err) => assert!(err.contains("path separators"), "unexpected error: {err}"),
            Ok(_) => panic!("expected error for path separator in model"),
        }
    }

    #[test]
    fn from_files_with_model_rejects_backslash() {
        let files = BTreeMap::new();
        let result = ProtocolPackage::from_files_with_model(&files, Some("a\\b"));
        match result {
            Err(err) => assert!(err.contains("path separators"), "unexpected error: {err}"),
            Ok(_) => panic!("expected error for backslash in model"),
        }
    }

    #[test]
    fn from_files_with_model_empty_model_skips_validation() {
        // model 为空字符串时应回退到 from_files，不触发路径校验
        let files = BTreeMap::new();
        let result = ProtocolPackage::from_files_with_model(&files, Some(""));
        match result {
            Err(err) => assert!(
                !err.contains("path separators"),
                "empty model should not trigger path validation: {err}"
            ),
            Ok(_) => panic!("expected error from from_files with empty file set"),
        }
    }
}
