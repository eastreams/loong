use kernel::ConnectorCommand;
use loong_bridge_runtime::{BridgeExecutionFailure, execute_http_json_bridge_call};
use serde_json::Value;

#[allow(clippy::indexing_slicing)]
pub async fn execute_http_json_bridge(
    mut execution: Value,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
) -> Value {
    let execution_result = execute_http_json_bridge_call(provider, channel, command).await;

    match execution_result {
        Ok(success) => {
            execution["status"] = Value::String("executed".to_owned());
            execution["runtime"] = success.runtime_evidence;
            execution
        }
        Err(failure) => apply_bridge_execution_failure(execution, failure),
    }
}

#[allow(clippy::indexing_slicing)]
fn apply_bridge_execution_failure(mut execution: Value, failure: BridgeExecutionFailure) -> Value {
    let status = if failure.blocked { "blocked" } else { "failed" };
    execution["status"] = Value::String(status.to_owned());
    execution["reason"] = Value::String(failure.reason);

    let runtime_evidence_is_null = failure.runtime_evidence.is_null();
    if !runtime_evidence_is_null {
        execution["runtime"] = failure.runtime_evidence;
    }

    execution
}
