use kernel::ConnectorCommand;
use loongclaw_bridge_runtime::{
    BridgeExecutionFailure, BridgeExecutionPolicy, execute_process_stdio_bridge_call,
};
use serde_json::Value;

use super::BridgeRuntimePolicy;

pub use loongclaw_bridge_runtime::{
    ProcessStdioExchangeOutcome, run_process_stdio_json_line_exchange,
};

#[allow(clippy::indexing_slicing)]
pub async fn execute_process_stdio_bridge(
    mut execution: Value,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    let execution_policy = BridgeExecutionPolicy {
        execute_process_stdio: runtime_policy.execute_process_stdio,
        execute_http_json: runtime_policy.execute_http_json,
        allowed_process_commands: runtime_policy.allowed_process_commands.clone(),
    };
    let execution_result =
        execute_process_stdio_bridge_call(provider, channel, command, &execution_policy).await;

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
