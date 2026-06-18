// SPDX-License-Identifier: AGPL-3.0-or-later
use mira_plugin_runtime::Transport;
use std::collections::VecDeque;

#[derive(Default)]
pub struct MockTransport {
    pub writes: Vec<Vec<u8>>,
    pub reads: VecDeque<Result<Vec<u8>, String>>,
    pub delays: Vec<u64>,
    pub unplugged: bool,
}

impl Transport for MockTransport {
    fn write(&mut self, report: &[u8]) -> Result<(), String> {
        if self.unplugged {
            return Err("device unplugged".into());
        }
        self.writes.push(report.to_vec());
        Ok(())
    }
    fn read(&mut self, _: usize) -> Result<Vec<u8>, String> {
        if self.unplugged {
            return Err("device unplugged".into());
        }
        self.reads
            .pop_front()
            .unwrap_or_else(|| Err("timeout".into()))
    }
    fn delay(&mut self, milliseconds: u64) -> Result<(), String> {
        self.delays.push(milliseconds);
        Ok(())
    }
}
