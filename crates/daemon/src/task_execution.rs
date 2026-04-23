use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use kernel::{
    AuditSink, Capability, CapabilityToken, ConnectorCommand, ExecutionRoute, HarnessAdapter,
    HarnessError, HarnessKind, HarnessOutcome, HarnessRequest, InMemoryAuditSink, LoongKernel,
    PolicyEngine, StaticPolicyEngine, SystemClock, TaskIntent, TaskState, TaskSupervisor,
    VerticalPackManifest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{CliResult, DEFAULT_AGENT_ID, DEFAULT_PACK_ID, PUBLIC_GITHUB_REPO, kernel_bootstrap};

#[derive(Debug, Clone, Serialize)]
pub struct DaemonTaskExecution {
    pub route: Option<ExecutionRoute>,
    pub outcome: Option<HarnessOutcome>,
    pub supervisor_state: TaskState,
    pub error: Option<String>,
}

/// Execute a daemon task intent through the task supervisor while preserving
/// route/outcome/state evidence for operator-facing callers.
///
/// This helper intentionally returns a structured execution record even when
/// dispatch fails so CLI/API surfaces can report the supervisor's terminal
/// state instead of collapsing everything into a plain transport error.
pub(crate) async fn execute_daemon_task_with_supervisor<P: PolicyEngine>(
    kernel: &LoongKernel<P>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct DaemonTurnTaskPayload {
    config_path: Option<String>,
    session_hint: Option<String>,
    message: Option<String>,
    turn_mode: loong_app::agent_runtime::AgentTurnMode,
    metadata: std::collections::BTreeMap<String, String>,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_servers: Vec<String>,
    acp_cwd: Option<String>,
}

struct EmbeddedAgentHarness;

#[async_trait]
impl HarnessAdapter for EmbeddedAgentHarness {
    fn name(&self) -> &str {
        "pi-local"
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::EmbeddedPi
    }

    async fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, HarnessError> {
        let payload = serde_json::from_value::<DaemonTurnTaskPayload>(request.payload)
            .map_err(|error| HarnessError::Execution(format!("invalid_turn_payload: {error}")))?;
        let message = payload.message.unwrap_or(request.objective);
        let turn_request = loong_app::agent_runtime::AgentTurnRequest {
            message,
            turn_mode: payload.turn_mode,
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            metadata: payload.metadata,
            acp: payload.acp,
            acp_event_stream: payload.acp_event_stream,
            acp_bootstrap_mcp_servers: payload.acp_bootstrap_mcp_servers,
            acp_cwd: payload.acp_cwd,
            live_surface_enabled: matches!(
                payload.turn_mode,
                loong_app::agent_runtime::AgentTurnMode::Interactive
            ),
        };
        let turn_service =
            loong_app::agent_runtime::load_turn_execution_service(payload.config_path.as_deref())
                .map_err(HarnessError::Execution)?;
        let turn_options = loong_app::agent_runtime::TurnExecutionOptions::default();
        let turn_result = turn_service
            .execute(payload.session_hint.as_deref(), &turn_request, turn_options)
            .await
            .map_err(HarnessError::Execution)?;

        Ok(HarnessOutcome {
            status: "ok".to_owned(),
            output: serde_json::to_value(turn_result).map_err(|error| {
                HarnessError::Execution(format!("serialize_turn_result_failed: {error}"))
            })?,
        })
    }
}

/// Build the daemon-side kernel used for generic task/turn execution.
///
/// This starts from the spec/kernel bootstrap defaults and then registers the
/// embedded agent harness so daemon task intents can route back into the shared
/// `AgentRuntime` pipeline without spawning an external process.
fn build_daemon_runtime_kernel() -> LoongKernel<StaticPolicyEngine> {
    let audit_sink = Arc::new(InMemoryAuditSink::default());
    let audit_sink = audit_sink as Arc<dyn AuditSink>;
    let clock = Arc::new(SystemClock) as Arc<dyn kernel::Clock>;
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit_sink);
    let pack = daemon_runtime_pack_manifest();
    let register_pack_result = kernel.register_pack(pack);
    register_pack_result.expect("daemon runtime pack should register");
    kernel.register_harness_adapter(EmbeddedAgentHarness);
    kernel
}

fn daemon_runtime_pack_manifest() -> VerticalPackManifest {
    let allowed_connectors = BTreeSet::new();
    let granted_capabilities = BTreeSet::from([
        Capability::InvokeTool,
        Capability::MemoryRead,
        Capability::MemoryWrite,
    ]);
    let metadata = BTreeMap::from([
        ("owner".to_owned(), "daemon-runtime".to_owned()),
        ("stage".to_owned(), "runtime".to_owned()),
    ]);
    let default_route = ExecutionRoute {
        harness_kind: HarnessKind::EmbeddedPi,
        adapter: Some("pi-local".to_owned()),
    };

    VerticalPackManifest {
        pack_id: DEFAULT_PACK_ID.to_owned(),
        domain: "engineering".to_owned(),
        version: "0.1.0".to_owned(),
        default_route,
        allowed_connectors,
        granted_capabilities,
        metadata,
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
    let payload = crate::cli_json::parse_json_payload(payload_raw, "run-task payload")?;

    let kernel = build_daemon_runtime_kernel();
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

/// Run a single daemon-managed turn through the task supervisor/harness path.
///
/// Unlike `chat`/`ask`, this exercises the same kernel-supervised dispatch lane
/// that daemon tasks use in production: the CLI request is wrapped as a
/// `TaskIntent`, routed through `EmbeddedAgentHarness`, and then decoded back
/// into an `AgentTurnResult` for presentation.
pub(crate) async fn run_turn_cli(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    message: &str,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> CliResult<()> {
    if message.trim().is_empty() {
        return Err("turn message must not be empty".to_owned());
    }
    let turn_service = loong_app::agent_runtime::load_turn_execution_service(config_path)?;
    let config = turn_service.config();
    if !config.cli.enabled {
        return Err("CLI channel is disabled by config.cli.enabled=false".to_owned());
    }

    let turn_request = loong_app::agent_runtime::AgentTurnRequest {
        message: message.to_owned(),
        turn_mode: if acp {
            loong_app::agent_runtime::AgentTurnMode::Acp
        } else {
            loong_app::agent_runtime::AgentTurnMode::Oneshot
        },
        metadata: std::collections::BTreeMap::new(),
        acp,
        acp_event_stream,
        acp_bootstrap_mcp_servers: acp_bootstrap_mcp_server.to_vec(),
        acp_cwd: acp_cwd.map(ToOwned::to_owned),
        ..Default::default()
    };
    let turn_options = loong_app::agent_runtime::TurnExecutionOptions::default();
    let result = turn_service
        .execute(session_hint, &turn_request, turn_options)
        .await?;
    println!("{}", result.output_text);
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
                payload: json!({"repo":"loong-ai/loong"}),
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

    #[tokio::test]
    async fn daemon_runtime_kernel_rejects_invalid_turn_payload_instead_of_using_stub_echo() {
        let kernel = build_daemon_runtime_kernel();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
            .expect("issue token");
        let payload = json!({
            "message": 42
        });

        let execution = execute_daemon_task_with_supervisor(
            &kernel,
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-runtime-harness-01".to_owned(),
                objective: "hello".to_owned(),
                required_capabilities: BTreeSet::from([
                    Capability::InvokeTool,
                    Capability::MemoryRead,
                    Capability::MemoryWrite,
                ]),
                payload,
            },
        )
        .await
        .expect("execute daemon task");

        let error = execution
            .error
            .as_deref()
            .expect("invalid payload should fail through the real runtime harness");

        assert!(execution.outcome.is_none());
        assert!(
            error.contains("invalid_turn_payload"),
            "expected unified runtime harness failure, got: {error}"
        );
    }
}
