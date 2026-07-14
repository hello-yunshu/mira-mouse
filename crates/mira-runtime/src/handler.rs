// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mira-specific InvokeHandler implementation for battery prediction.

use mira_protocol::{BatteryModelConfig, BatteryPredictionInput, BATTERY_USAGE_CAPABILITY};
use rill_runtime::InvokeHandler;
use serde_json::Value;

#[derive(Debug)]
pub struct MiraInvokeHandler {
    config: BatteryModelConfig,
}

impl MiraInvokeHandler {
    pub fn from_model(model: &Value) -> Result<Self, String> {
        let config: BatteryModelConfig = serde_json::from_value(model.clone())
            .map_err(|e| format!("invalid model config: {e}"))?;
        config.validate().map_err(|e| e.to_string())?;
        Ok(Self { config })
    }
}

impl InvokeHandler for MiraInvokeHandler {
    fn invoke(&self, capability: &str, input: &Value) -> Result<Value, String> {
        if capability != BATTERY_USAGE_CAPABILITY {
            return Err(format!("unsupported capability: {capability}"));
        }
        let input: BatteryPredictionInput =
            serde_json::from_value(input.clone()).map_err(|e| format!("invalid input: {e}"))?;
        let output = crate::battery::predict(&input, &self.config).map_err(|e| e.to_string())?;
        serde_json::to_value(output).map_err(|e| format!("serialize output: {e}"))
    }
}
