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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandDefinition {
    request: RequestDefinition,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestDefinition {
    length: usize,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DerivedDefinition {
    kind: String,
    source: String,
    table: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransportsFile {
    schema_version: u32,
    transports: HashMap<String, TransportDefinition>,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::large_enum_variant)] // Mirrors the declarative transport schema; instances are few and immutable.
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowsFile {
    schema_version: u32,
    workflows: HashMap<String, WorkflowDefinition>,
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
    params: BTreeMap<String, u8>,
}

pub struct ProtocolPackage {
    commands: CommandsFile,
    parsers: ParsersFile,
    transports: TransportsFile,
    workflows: WorkflowsFile,
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
        let package = Self {
            commands: parse(files, "protocol/commands.json")?,
            parsers: parse(files, "protocol/parsers.json")?,
            transports: parse(files, "protocol/transports.json")?,
            workflows: parse(files, "protocol/workflows.json")?,
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
            let transport = step.transport.as_deref().unwrap_or(&workflow.transport);
            let response = session
                .execute_command(transport, &step.command, &step.params)
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

    fn build_command(&self, id: &str, params: &BTreeMap<String, u8>) -> Result<Vec<u8>, String> {
        let command = self
            .commands
            .commands
            .get(id)
            .ok_or_else(|| format!("missing command {id}"))?;
        if command.request.length == 0 || command.request.length > 1024 {
            return Err(format!("invalid command length for {id}"));
        }
        let mut report = vec![0u8; command.request.length];
        for byte in &command.request.bytes {
            let target = report
                .get_mut(byte.offset)
                .ok_or_else(|| format!("command {id} byte offset out of range"))?;
            *target = match (&byte.value, &byte.param) {
                (Some(value), None) => parse_byte(value)?,
                (None, Some(param)) => *params
                    .get(param)
                    .ok_or_else(|| format!("command {id} missing parameter {param}"))?,
                _ => return Err(format!("command {id} byte must have value or param")),
            };
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
            if derived.kind != "lookup" {
                return Err(format!("unsupported derived kind {}", derived.kind));
            }
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
        Ok(Value::Object(fields))
    }
}

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
        params: &BTreeMap<String, u8>,
    ) -> Result<Vec<u8>, String> {
        let transport = self
            .package
            .transports
            .transports
            .get(transport_id)
            .ok_or_else(|| format!("missing transport {transport_id}"))?;
        match transport {
            TransportDefinition::HidFeature { .. } => {
                let report = self.package.build_command(command_id, params)?;
                self.feature_exchange(transport_id, &report, true)
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
                    .build_command(&start_command, &BTreeMap::new())?;
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

                let inner = self.package.build_command(command_id, params)?;
                let length_params =
                    BTreeMap::from([("innerLength".to_string(), inner.len() as u8)]);
                let set_length = self
                    .package
                    .build_command(&set_length_command, &length_params)?;
                self.feature_exchange(&base_transport, &set_length, true)?;
                self.feature_exchange(&base_transport, &inner, true)?;
                self.poll_until(
                    &base_transport,
                    &poll_command,
                    &read_ready,
                    attempts,
                    delay_ms,
                )?;
                let read = self
                    .package
                    .build_command(&read_command, &BTreeMap::new())?;
                self.feature_exchange(&base_transport, &read, true)
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
        let report = self.package.build_command(command, &BTreeMap::new())?;
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
        self.delay(10)?;
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

fn parse_byte(value: &str) -> Result<u8, String> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|hex| u8::from_str_radix(hex, 16))
        .unwrap_or_else(|| value.parse::<u8>())
        .map_err(|_| format!("invalid byte {value}"))
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
}
