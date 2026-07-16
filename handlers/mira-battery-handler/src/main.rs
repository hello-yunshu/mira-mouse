//! Mira-owned battery prediction handler for the generic Rill runtime.
//!
//! Device semantics deliberately stop here: Rill validates, sandboxes and
//! invokes this component but does not know anything about Mira batteries.

use std::sync::OnceLock;

use mira_protocol::{BatteryModelConfig, BatteryPredictionInput, BATTERY_USAGE_CAPABILITY};

wit_bindgen::generate!({
    path: "wit/rill-handler.wit",
    world: "invoke-handler",
});

const HANDLER_ID: &str = "mira.battery.handler";
const HANDLER_API_VERSION: u32 = 1;
const HANDLER_VERSION: &str = match option_env!("MIRA_HANDLER_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

static MODEL: OnceLock<BatteryModelConfig> = OnceLock::new();

struct MiraBatteryHandler;

impl Guest for MiraBatteryHandler {
    fn metadata() -> HandlerMetadata {
        HandlerMetadata {
            id: HANDLER_ID.into(),
            version: HANDLER_VERSION.into(),
            api_version: HANDLER_API_VERSION,
            capabilities: vec![BATTERY_USAGE_CAPABILITY.into()],
        }
    }

    fn configure(model_json: Vec<u8>) -> Result<(), HandlerError> {
        let config: BatteryModelConfig = serde_json::from_slice(&model_json)
            .map_err(|_| HandlerError::InvalidModel("invalid battery model JSON".into()))?;
        config
            .validate()
            .map_err(|message| HandlerError::InvalidModel(message.into()))?;
        MODEL
            .set(config)
            .map_err(|_| HandlerError::InvalidModel("handler is already configured".into()))
    }

    fn invoke(capability: String, input_json: Vec<u8>) -> Result<Vec<u8>, HandlerError> {
        if capability != BATTERY_USAGE_CAPABILITY {
            return Err(HandlerError::UnsupportedCapability(capability));
        }
        let config = MODEL
            .get()
            .ok_or_else(|| HandlerError::ExecutionFailed("handler is not configured".into()))?;
        let input: BatteryPredictionInput = serde_json::from_slice(&input_json)
            .map_err(|_| HandlerError::InvalidInput("invalid battery prediction input".into()))?;
        let output = mira_local_ai::predict(&input, config)
            .map_err(|_| HandlerError::ExecutionFailed("battery prediction failed".into()))?;
        serde_json::to_vec(&output)
            .map_err(|_| HandlerError::ExecutionFailed("failed to encode prediction".into()))
    }
}

export!(MiraBatteryHandler);

fn main() {}
