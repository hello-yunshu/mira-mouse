// SPDX-License-Identifier: AGPL-3.0-or-later
use hidapi::{HidApi, HidDevice};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::ffi::CString;
use std::thread;
use std::time::Duration;

const MAX_COMMANDS: usize = 32;
const MAX_REPORTS: usize = 128;
const MAX_DELAY_MS: u64 = 5_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandsFile {
    schema_version: u32,
    commands: HashMap<String, CommandDefinition>,
    #[serde(default, rename = "am35")]
    _am35: Option<Value>,
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
    },
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
    read: MutationCall,
    write_command: String,
    write_transport: Option<String>,
    #[serde(default)]
    write_params: BTreeMap<String, Value>,
    #[serde(default)]
    memory: Option<MemoryMutationDefinition>,
    preserve_unknown: bool,
    #[serde(default)]
    settle_ms: u64,
    verify: MutationVerify,
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
    eq: Value,
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

pub struct ProtocolPackage {
    commands: CommandsFile,
    parsers: ParsersFile,
    transports: TransportsFile,
    workflows: WorkflowsFile,
    capabilities: Option<Value>,
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

        let package = Self {
            commands: parse(files, "protocol/commands.json")?,
            parsers: parse(files, "protocol/parsers.json")?,
            transports: parse(files, "protocol/transports.json")?,
            workflows,
            capabilities,
        };
        if package.commands.schema_version != 1
            || package.parsers.schema_version != 1
            || package.transports.schema_version != 1
            || package.workflows.schema_version != 1
        {
            return Err("unsupported protocol schema version".into());
        }
        Ok(package)
    }

    pub fn capabilities(&self) -> Option<&Value> {
        self.capabilities.as_ref()
    }

    pub fn execute(
        &self,
        api: &HidApi,
        path: &str,
        workflow_id: &str,
    ) -> Result<BTreeMap<String, Value>, String> {
        let workflow = self
            .workflows
            .workflows
            .get(workflow_id)
            .ok_or_else(|| format!("missing workflow {workflow_id}"))?;
        if workflow.steps.len() > MAX_COMMANDS {
            return Err("workflow command limit exceeded".into());
        }
        let c_path = CString::new(path).map_err(|_| "invalid HID path".to_string())?;
        let device = api.open_path(&c_path).map_err(|error| error.to_string())?;
        let mut session = Session {
            package: self,
            device,
            reports: 0,
            delay_ms: 0,
            outputs: BTreeMap::new(),
        };
        for (index, step) in workflow.steps.iter().enumerate() {
            if step.skip_if_zero.iter().any(|reference| {
                output_value(&session.outputs, reference).and_then(Value::as_u64) == Some(0)
            }) {
                continue;
            }
            let params =
                resolve_workflow_params(&step.params, &session.outputs).map_err(|error| {
                    format!("workflow {workflow_id} step {} params: {error}", index + 1)
                })?;
            let transport = step.transport.as_deref().unwrap_or(&workflow.transport);
            let response = execute_with_candidates(
                &mut session,
                transport,
                &step.command,
                &params,
                &step.param_candidates,
            )
            .map_err(|error| {
                format!(
                    "workflow {workflow_id} step {} command {}: {error}",
                    index + 1,
                    step.command
                )
            })?;
            let parsed = self
                .parse_response(&step.parser, &response)
                .map_err(|error| {
                    format!(
                        "workflow {workflow_id} step {} parser {}: {error}",
                        index + 1,
                        step.parser
                    )
                })?;
            session.outputs.insert(step.output.clone(), parsed);
        }
        Ok(session.outputs)
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
                if let Some(outputs) = ctx_outputs {
                    let zero_skipped = mutation.skip_if_zero.iter().any(|reference| {
                        output_value(outputs, reference).and_then(Value::as_u64) == Some(0)
                    });
                    let non_zero_skipped = mutation.skip_if_non_zero.iter().any(|reference| {
                        output_value(outputs, reference)
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            != 0
                    });
                    let all_zero_skipped = !mutation.skip_if_all_zero.is_empty()
                        && mutation.skip_if_all_zero.iter().all(|reference| {
                            output_value(outputs, reference).and_then(Value::as_u64) == Some(0)
                        });
                    !(zero_skipped || non_zero_skipped || all_zero_skipped)
                } else {
                    true
                }
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
    ) -> Result<(BTreeMap<String, Value>, Vec<u8>), String> {
        let outputs = self.execute(api, path, &definition.read_workflow)?;
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
        for index in 0..count {
            let key = format!("{}{index:02}", definition.chunk_output_prefix);
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
        verify_memory_checksum(&memory, &definition.checksum)?;
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
            )
            .map_err(|error| format!("mutation {mutation_id} memory write end: {error}"))?;
        Ok(updated)
    }

    pub fn mutate(
        &self,
        api: &HidApi,
        path: &str,
        mutation_id: &str,
        params: &Map<String, Value>,
        ctx_outputs: &BTreeMap<String, Value>,
    ) -> Result<Value, String> {
        let mutation = self
            .workflows
            .mutations
            .get(mutation_id)
            .ok_or_else(|| format!("missing mutation {mutation_id}"))?;
        let zero_skipped = mutation.skip_if_zero.iter().any(|reference| {
            output_value(ctx_outputs, reference).and_then(Value::as_u64) == Some(0)
        });
        let non_zero_skipped = mutation.skip_if_non_zero.iter().any(|reference| {
            output_value(ctx_outputs, reference)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                != 0
        });
        let all_zero_skipped = !mutation.skip_if_all_zero.is_empty()
            && mutation.skip_if_all_zero.iter().all(|reference| {
                output_value(ctx_outputs, reference).and_then(Value::as_u64) == Some(0)
            });
        if zero_skipped || non_zero_skipped || all_zero_skipped {
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
        let memory_read = match &mutation.memory {
            Some(memory)
                if output_value(ctx_outputs, &memory.available_when)
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    != 0 =>
            {
                let (outputs, bytes) = self.read_memory_mutation(api, path, memory)?;
                let enabled = output_value(
                    &outputs,
                    &OutputReference {
                        output: memory.enabled_when.output.clone(),
                        field: memory.enabled_when.field.clone(),
                    },
                ) == Some(&memory.enabled_when.eq)
                    && memory.required_when.iter().all(|condition| {
                        output_value(
                            &outputs,
                            &OutputReference {
                                output: condition.output.clone(),
                                field: condition.field.clone(),
                            },
                        ) == Some(&condition.eq)
                    });
                enabled.then_some((outputs, bytes))
            }
            _ => None,
        };
        let c_path = CString::new(path).map_err(|_| "invalid HID path".to_string())?;
        let device = api.open_path(&c_path).map_err(|error| error.to_string())?;
        let mut session = Session {
            package: self,
            device,
            reports: 0,
            delay_ms: 0,
            outputs: {
                let mut outputs = ctx_outputs.clone();
                if let Some((memory_outputs, _)) = &memory_read {
                    outputs.extend(memory_outputs.clone());
                }
                outputs
            },
        };

        let read_transport = mutation
            .read
            .transport
            .as_deref()
            .unwrap_or(&mutation.transport);
        let read_skipped = mutation.read.skip_if_zero.iter().any(|reference| {
            output_value(&session.outputs, reference).and_then(Value::as_u64) == Some(0)
        });
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
        )?;
        session.delay(mutation.settle_ms)?;

        let verify_transport = mutation
            .verify
            .transport
            .as_deref()
            .unwrap_or(&mutation.transport);
        let verify_params = resolve_workflow_params(&mutation.verify.params, &session.outputs)
            .map_err(|error| format!("mutation {mutation_id} verify params: {error}"))?;
        let verify_skipped = mutation.verify.skip_if_zero.iter().any(|reference| {
            output_value(&session.outputs, reference).and_then(Value::as_u64) == Some(0)
        });
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
            )?;
            let parsed = self
                .parse_response(&mutation.verify.parser, &response)
                .map_err(|error| format!("mutation {mutation_id} verification read: {error}"))?;
            verify_assertions(&parsed, &params, &mutation.verify.assertions)
                .map_err(|error| format!("mutation {mutation_id} verification failed: {error}"))?;
            parsed
        };
        drop(session);
        if let Some((memory, expected)) = expected_memory {
            let (_, actual) = self.read_memory_mutation(api, path, memory)?;
            if actual != expected {
                return Err(format!("mutation {mutation_id} memory readback mismatch"));
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
            if checksum.algorithm != "ff-minus-sum8"
                || checksum.start > checksum.end_exclusive
                || checksum.end_exclusive > report.len()
                || checksum.write_offset >= report.len()
            {
                return Err(format!("invalid checksum declaration for {id}"));
            }
            let sum = report[checksum.start..checksum.end_exclusive]
                .iter()
                .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
            report[checksum.write_offset] = 0xFF - sum;
        }
        Ok(report)
    }

    fn parse_response(&self, id: &str, response: &[u8]) -> Result<Value, String> {
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
                    let hex_key = format!("0x{source:02X}");
                    let decimal_key = source.to_string();
                    let value = derived
                        .table
                        .get(&hex_key)
                        .or_else(|| derived.table.get(&decimal_key))
                        .cloned()
                        .unwrap_or(Value::Null);
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
        return session.execute_command(transport, command, params, true, None);
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
        match session.execute_command(transport, command, &attempt, true, None) {
            Ok(response) => return Ok(response),
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "all candidates for {name} failed: {}",
        last_error.unwrap_or_else(|| "no candidates".into())
    ))
}

/// Rate-limited HID session. Each session is bounded by:
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
}

impl Session<'_> {
    fn execute_command(
        &mut self,
        transport_id: &str,
        command_id: &str,
        params: &BTreeMap<String, Value>,
        expect_response: bool,
        base: Option<&[u8]>,
    ) -> Result<Vec<u8>, String> {
        let transport = self
            .package
            .transports
            .transports
            .get(transport_id)
            .ok_or_else(|| format!("missing transport {transport_id}"))?;
        match transport {
            TransportDefinition::HidFeature { .. } => {
                let report = self.package.build_command(command_id, params, base)?;
                self.feature_exchange(transport_id, &report, expect_response)
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
                self.feature_exchange(&base_transport, &start, false)?;
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
                self.feature_exchange(&base_transport, &set_length, true)?;
                self.feature_exchange(&base_transport, &inner, expect_response)?;
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
                self.feature_exchange(&base_transport, &read, true)
            }
            TransportDefinition::HidOutputInput { .. } => {
                let report = self.package.build_command(command_id, params, base)?;
                self.output_input_exchange(transport_id, &report, expect_response)
            }
        }
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
        for _ in 0..attempts.min(32) {
            let response = self.feature_exchange(transport, &report, true)?;
            if response.get(condition.offset) == Some(&condition.eq) {
                return Ok(response);
            }
            self.delay(delay_ms)?;
        }
        Err(format!(
            "condition at offset {} timed out",
            condition.offset
        ))
    }

    fn feature_exchange(
        &mut self,
        transport_id: &str,
        payload: &[u8],
        expect_response: bool,
    ) -> Result<Vec<u8>, String> {
        let Some(TransportDefinition::HidFeature {
            report_id,
            write_length,
            read_length,
            strip_report_id_on_read,
            feature_delay_ms,
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
        let mut report = Vec::with_capacity(*write_length);
        report.push(*report_id);
        report.extend_from_slice(payload);
        self.device
            .send_feature_report(&report)
            .map_err(|error| format!("send feature report: {error}"))?;
        if !expect_response {
            return Ok(Vec::new());
        }
        self.delay(*feature_delay_ms)?;
        let mut response = vec![0u8; *read_length];
        let count = self
            .device
            .get_feature_report(&mut response)
            .map_err(|error| format!("get feature report: {error}"))?;
        response.truncate(count);
        if *strip_report_id_on_read && !response.is_empty() {
            response.remove(0);
        }
        Ok(response)
    }

    fn output_input_exchange(
        &mut self,
        transport_id: &str,
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
        let mut report = Vec::with_capacity(*write_length);
        report.push(*report_id);
        report.extend_from_slice(payload);
        let written = self
            .device
            .write(&report)
            .map_err(|error| format!("send output report: {error}"))?;
        if written != report.len() {
            return Err(format!(
                "short output report write: {written}/{}",
                report.len()
            ));
        }
        if !expect_response {
            return Ok(Vec::new());
        }

        for _ in 0..*read_retries {
            let mut response = vec![0u8; *read_length];
            let count = self
                .device
                .read_timeout(&mut response, *read_timeout_ms)
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
            if response.first() != Some(report_id) {
                continue;
            }
            if *strip_report_id_on_read {
                response.remove(0);
            }
            if payload.len() >= 3
                && response.len() >= 5
                && response[1] == 0xFF
                && response[2] == payload[1]
                && response[3] == payload[2]
            {
                return Err(format!("HID++ 2.0 error 0x{:02X}", response[4]));
            }
            if response.get(..3) == payload.get(..3) {
                return Ok(response);
            }
        }
        Err("timed out waiting for matching input report".into())
    }

    fn delay(&mut self, milliseconds: u64) -> Result<(), String> {
        self.delay_ms = self
            .delay_ms
            .checked_add(milliseconds)
            .ok_or_else(|| "delay limit exceeded".to_string())?;
        if self.delay_ms > MAX_DELAY_MS {
            return Err("delay limit exceeded".into());
        }
        thread::sleep(Duration::from_millis(milliseconds));
        Ok(())
    }
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
    if supplied.len() != definitions.len() {
        return Err("mutation parameters do not match the declared schema".into());
    }
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
        other => Err(format!("unsupported parser field kind {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

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
        assert!(validate_mutation_inputs(
            &definitions,
            &Map::from_iter([
                ("dpi".into(), Value::from(800)),
                ("raw".into(), Value::from(1)),
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
        assert!(verify_memory_checksum(&memory, "crc-ccitt-false").is_err());
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
        let result = package.mutate(api, "test-path", "nonexistent", &Map::new(), &outputs);
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
        let result = package.mutate(api, "test-path", "test-set-dpi", &Map::new(), &outputs);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available on this device"));
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
        let result = package.mutate(api, "test-path", "test-set-dpi", &Map::new(), &outputs);
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
        let result = package.mutate(api, "test-path", "test-set-dpi", &Map::new(), &outputs);
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
        let result = package.mutate(api, "test-path", "test-set-dpi", &Map::new(), &outputs);
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
}
