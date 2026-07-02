use serde_json::Value;

use crate::{CliResult, RuntimeSnapshotCliState, gateway};

pub fn build_runtime_snapshot_cli_json_payload(
    snapshot: &RuntimeSnapshotCliState,
) -> CliResult<Value> {
    let read_model = gateway::read_models::build_runtime_snapshot_read_model(snapshot);
    let payload = serde_json::to_value(read_model)
        .map_err(|error| format!("serialize runtime snapshot read model failed: {error}"))?;
    Ok(payload)
}
