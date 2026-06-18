// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_steps: usize,
    pub max_report_bytes: usize,
    pub max_total_delay_ms: u64,
    pub max_reads: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 64,
            max_report_bytes: 1024,
            max_total_delay_ms: 2_000,
            max_reads: 16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workflow {
    pub id: String,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Operation {
    Write { report: Vec<u8> },
    Read { length: usize },
    Expect { offset: usize, bytes: Vec<u8> },
    Delay { milliseconds: u64 },
}

pub trait Transport {
    fn write(&mut self, report: &[u8]) -> Result<(), String>;
    fn read(&mut self, length: usize) -> Result<Vec<u8>, String>;
    fn delay(&mut self, milliseconds: u64) -> Result<(), String>;
}

pub fn execute_workflow(
    workflow: &Workflow,
    transport: &mut dyn Transport,
    limits: Limits,
) -> Result<Vec<u8>, DslError> {
    if workflow.operations.len() > limits.max_steps {
        return Err(DslError::Limit("steps"));
    }
    let mut reads = 0;
    let mut delay: u64 = 0;
    let mut last = Vec::new();
    for operation in &workflow.operations {
        match operation {
            Operation::Write { report } => {
                if report.len() > limits.max_report_bytes {
                    return Err(DslError::Limit("report bytes"));
                }
                transport.write(report).map_err(DslError::Transport)?;
            }
            Operation::Read { length } => {
                reads += 1;
                if reads > limits.max_reads || *length > limits.max_report_bytes {
                    return Err(DslError::Limit("reads"));
                }
                last = transport.read(*length).map_err(DslError::Transport)?;
                if last.len() > *length {
                    return Err(DslError::OversizedRead);
                }
            }
            Operation::Expect { offset, bytes } => {
                let end = offset
                    .checked_add(bytes.len())
                    .ok_or(DslError::Expectation)?;
                if last.get(*offset..end) != Some(bytes.as_slice()) {
                    return Err(DslError::Expectation);
                }
            }
            Operation::Delay { milliseconds } => {
                delay = delay
                    .checked_add(*milliseconds)
                    .ok_or(DslError::Limit("delay"))?;
                if delay > limits.max_total_delay_ms {
                    return Err(DslError::Limit("delay"));
                }
                transport
                    .delay(*milliseconds)
                    .map_err(DslError::Transport)?;
            }
        }
    }
    Ok(last)
}

#[derive(Debug, Error, PartialEq)]
pub enum DslError {
    #[error("workflow exceeded {0} limit")]
    Limit(&'static str),
    #[error("transport failed: {0}")]
    Transport(String),
    #[error("read returned more bytes than requested")]
    OversizedRead,
    #[error("response expectation failed")]
    Expectation,
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mock;
    impl Transport for Mock {
        fn write(&mut self, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn read(&mut self, length: usize) -> Result<Vec<u8>, String> {
            Ok(vec![7; length])
        }
        fn delay(&mut self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn enforces_delay_limit() {
        let workflow = Workflow {
            id: "bounded".into(),
            operations: vec![Operation::Delay {
                milliseconds: 2_001,
            }],
        };
        assert_eq!(
            execute_workflow(&workflow, &mut Mock, Limits::default()),
            Err(DslError::Limit("delay"))
        );
    }
}
