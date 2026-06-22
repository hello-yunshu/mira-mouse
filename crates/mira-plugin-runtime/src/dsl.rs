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

    /// Mock that returns more bytes than requested, to test the OversizedRead guard.
    struct OversizedMock;
    impl Transport for OversizedMock {
        fn write(&mut self, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn read(&mut self, length: usize) -> Result<Vec<u8>, String> {
            Ok(vec![0; length + 1])
        }
        fn delay(&mut self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }

    /// Mock that returns a fixed payload, for Expect tests.
    struct FixedRead(Vec<u8>);
    impl Transport for FixedRead {
        fn write(&mut self, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn read(&mut self, length: usize) -> Result<Vec<u8>, String> {
            Ok(self.0[..length].to_vec())
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

    /// A Write whose report exceeds `max_report_bytes` must be rejected.
    #[test]
    fn rejects_oversized_write() {
        let big_report = vec![0u8; 1025];
        let workflow = Workflow {
            id: "big-write".into(),
            operations: vec![Operation::Write { report: big_report }],
        };
        assert_eq!(
            execute_workflow(&workflow, &mut Mock, Limits::default()),
            Err(DslError::Limit("report bytes"))
        );
    }

    /// A Read that returns more bytes than requested must trigger OversizedRead.
    #[test]
    fn rejects_oversized_read() {
        let workflow = Workflow {
            id: "oversized-read".into(),
            operations: vec![Operation::Read { length: 4 }],
        };
        assert_eq!(
            execute_workflow(&workflow, &mut OversizedMock, Limits::default()),
            Err(DslError::OversizedRead)
        );
    }

    /// Exceeding the max_reads limit must be rejected.
    #[test]
    fn rejects_too_many_reads() {
        let ops: Vec<Operation> = (0..17).map(|_| Operation::Read { length: 1 }).collect();
        let workflow = Workflow {
            id: "many-reads".into(),
            operations: ops,
        };
        assert_eq!(
            execute_workflow(&workflow, &mut Mock, Limits::default()),
            Err(DslError::Limit("reads"))
        );
    }

    /// Exceeding the max_steps limit must be rejected before any I/O.
    #[test]
    fn rejects_too_many_steps() {
        let ops: Vec<Operation> = (0..65)
            .map(|_| Operation::Delay { milliseconds: 1 })
            .collect();
        let workflow = Workflow {
            id: "many-steps".into(),
            operations: ops,
        };
        assert_eq!(
            execute_workflow(&workflow, &mut Mock, Limits::default()),
            Err(DslError::Limit("steps"))
        );
    }

    /// Expect must pass when the bytes at the given offset match.
    #[test]
    fn expect_matches_at_offset() {
        let payload = vec![0x00, 0x01, 0x02, 0x03, 0x04];
        let workflow = Workflow {
            id: "expect-ok".into(),
            operations: vec![
                Operation::Read { length: 5 },
                Operation::Expect {
                    offset: 2,
                    bytes: vec![0x02, 0x03],
                },
            ],
        };
        assert!(execute_workflow(&workflow, &mut FixedRead(payload), Limits::default()).is_ok());
    }

    /// Expect must fail when the bytes at the given offset don't match.
    #[test]
    fn expect_fails_on_mismatch() {
        let payload = vec![0x00, 0x01, 0x02, 0x03, 0x04];
        let workflow = Workflow {
            id: "expect-fail".into(),
            operations: vec![
                Operation::Read { length: 5 },
                Operation::Expect {
                    offset: 2,
                    bytes: vec![0xFF, 0xFF],
                },
            ],
        };
        assert_eq!(
            execute_workflow(&workflow, &mut FixedRead(payload), Limits::default()),
            Err(DslError::Expectation)
        );
    }

    /// Expect at an offset that would overflow the read buffer must fail.
    #[test]
    fn expect_fails_on_overflow() {
        let payload = vec![0x00, 0x01];
        let workflow = Workflow {
            id: "expect-overflow".into(),
            operations: vec![
                Operation::Read { length: 2 },
                Operation::Expect {
                    offset: 1,
                    bytes: vec![0x01, 0x02, 0x03],
                },
            ],
        };
        assert_eq!(
            execute_workflow(&workflow, &mut FixedRead(payload), Limits::default()),
            Err(DslError::Expectation)
        );
    }

    /// A read length exceeding `max_report_bytes` must be rejected.
    #[test]
    fn rejects_oversized_read_length() {
        let workflow = Workflow {
            id: "big-read".into(),
            operations: vec![Operation::Read { length: 2048 }],
        };
        assert_eq!(
            execute_workflow(&workflow, &mut Mock, Limits::default()),
            Err(DslError::Limit("reads"))
        );
    }
}
