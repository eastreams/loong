use std::collections::BTreeSet;

use kernel::{
    Capability, CapabilityToken, ConnectorCommand, ExecutionRoute, HarnessOutcome, LoongClawKernel,
    PolicyEngine, TaskIntent, TaskState, TaskSupervisor,
};
use serde::Serialize;
use serde_json::json;

use crate::{CliResult, DEFAULT_AGENT_ID, DEFAULT_PACK_ID, PUBLIC_GITHUB_REPO, kernel_bootstrap};

#[derive(Debug, Clone, Serialize)]
pub struct DaemonTaskExecution {
    pub route: Option<ExecutionRoute>,
    pub outcome: Option<HarnessOutcome>,
    pub supervisor_state: TaskState,
    pub error: Option<String>,
}

pub(crate) async fn execute_daemon_task_with_supervisor<P: PolicyEngine>(
    kernel: &LoongClawKernel<P>,
    pack_id: &str,
    token: &CapabilityToken,
    intent: TaskIntent,
) -> CliResult<DaemonTaskExecution> {
    let mut supervisor = TaskSupervisor::new(intent);
    let dispatch_result = supervisor.execute(kernel, pack_id, token).await;
    let supervisor_state = supervisor.state().clone();

    match dispatch_result {
        Ok(dispatch) => Ok(DaemonTaskExecution {
            route: Some(dispatch.adapter_route),
            outcome: Some(dispatch.outcome),
            supervisor_state,
            error: None,
        }),
        Err(error) => {
            let error_message = format!("task dispatch failed: {error}");
            Ok(DaemonTaskExecution {
                route: None,
                outcome: None,
                supervisor_state,
                error: Some(error_message),
            })
        }
    }
}

fn require_successful_daemon_task_execution(
    execution: &DaemonTaskExecution,
) -> CliResult<(&ExecutionRoute, &HarnessOutcome)> {
    let route = execution.route.as_ref();
    let outcome = execution.outcome.as_ref();
    let error = execution.error.as_deref();

    match (route, outcome, error) {
        (Some(route), Some(outcome), None) => Ok((route, outcome)),
        (_, _, Some(error)) => Err(error.to_owned()),
        _ => Err("task dispatch returned an incomplete execution payload".to_owned()),
    }
}

pub async fn run_demo() -> CliResult<()> {
    let kernel = kernel_bootstrap::KernelBuilder::default().build();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 300)
        .map_err(|error| format!("token issue failed: {error}"))?;

    let task = TaskIntent {
        task_id: "task-bootstrap-01".to_owned(),
        objective: "summarize flaky test clusters".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::MemoryRead]),
        payload: json!({"repo": PUBLIC_GITHUB_REPO}),
    };

    let task_dispatch =
        execute_daemon_task_with_supervisor(&kernel, DEFAULT_PACK_ID, &token, task).await?;
    let (route, outcome) = require_successful_daemon_task_execution(&task_dispatch)?;

    println!(
        "task dispatched via {:?} with state {:?}: {}",
        route.harness_kind, task_dispatch.supervisor_state, outcome.output
    );

    let connector_dispatch = kernel
        .execute_connector_core(
            DEFAULT_PACK_ID,
            &token,
            None,
            ConnectorCommand {
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"channel": "ops-alerts", "message": "task complete"}),
            },
        )
        .await
        .map_err(|error| format!("connector dispatch failed: {error}"))?;

    println!("connector dispatch: {}", connector_dispatch.outcome.payload);
    Ok(())
}

pub async fn run_task_cli(objective: &str, payload_raw: &str) -> CliResult<()> {
    let payload = crate::parse_json_payload(payload_raw, "run-task payload")?;

    let kernel = kernel_bootstrap::KernelBuilder::default().build();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
        .map_err(|error| format!("token issue failed: {error}"))?;

    let dispatch = execute_daemon_task_with_supervisor(
        &kernel,
        DEFAULT_PACK_ID,
        &token,
        TaskIntent {
            task_id: "task-cli-01".to_owned(),
            objective: objective.to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::MemoryRead]),
            payload,
        },
    )
    .await?;

    let pretty = serde_json::to_string_pretty(&dispatch)
        .map_err(|error| format!("serialize task outcome failed: {error}"))?;
    println!("{pretty}");
    require_successful_daemon_task_execution(&dispatch)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_daemon_task_with_supervisor_reports_completed_state() {
        let kernel = kernel_bootstrap::KernelBuilder::default().build();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
            .expect("issue token");

        let execution = execute_daemon_task_with_supervisor(
            &kernel,
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-test-01".to_owned(),
                objective: "exercise daemon task supervisor".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"kind": "daemon-task-supervisor"}),
            },
        )
        .await
        .expect("execute daemon task");
        let outcome = execution
            .outcome
            .as_ref()
            .expect("successful execution should include outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.output["task"], "task-test-01");
        assert!(matches!(
            execution.supervisor_state,
            TaskState::Completed(ref outcome) if outcome.status == "ok"
        ));
        assert!(execution.error.is_none());
    }

    #[tokio::test]
    async fn daemon_task_execution_serializes_supervisor_state_for_cli_output() {
        let kernel = kernel_bootstrap::KernelBuilder::default().build();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
            .expect("issue token");

        let execution = execute_daemon_task_with_supervisor(
            &kernel,
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-cli-01".to_owned(),
                objective: "summarize flaky test clusters".to_owned(),
                required_capabilities: BTreeSet::from([
                    Capability::InvokeTool,
                    Capability::MemoryRead,
                ]),
                payload: json!({"repo":"loongclaw-ai/loongclaw"}),
            },
        )
        .await
        .expect("execute daemon task");
        let expected_route = execution
            .route
            .clone()
            .expect("successful execution should include route");

        let payload = serde_json::to_value(&execution).expect("serialize daemon task execution");
        let expected_route_payload =
            serde_json::to_value(expected_route).expect("serialize expected route");

        assert_eq!(payload["route"], expected_route_payload);
        assert_eq!(payload["outcome"]["status"], "ok");
        assert_eq!(payload["supervisor_state"]["Completed"]["status"], "ok");
        assert_eq!(
            payload["supervisor_state"]["Completed"]["output"]["task"],
            "task-cli-01"
        );
    }

    #[tokio::test]
    async fn execute_daemon_task_with_supervisor_preserves_faulted_state_on_dispatch_error() {
        let kernel = kernel_bootstrap::KernelBuilder::default().build();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
            .expect("issue token");

        let execution = execute_daemon_task_with_supervisor(
            &kernel,
            "missing-pack",
            &token,
            TaskIntent {
                task_id: "task-faulted-01".to_owned(),
                objective: "exercise daemon task supervisor fault".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"kind": "daemon-task-supervisor-fault"}),
            },
        )
        .await
        .expect("execute daemon task");
        let error = execution
            .error
            .as_deref()
            .expect("faulted execution should include an error");
        let payload = serde_json::to_value(&execution).expect("serialize daemon task execution");

        assert!(execution.route.is_none());
        assert!(execution.outcome.is_none());
        assert!(error.contains("task dispatch failed"));
        assert!(matches!(execution.supervisor_state, TaskState::Faulted(_)));
        assert!(payload["route"].is_null());
        assert!(payload["outcome"].is_null());
    }
}
