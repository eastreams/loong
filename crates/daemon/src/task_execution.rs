use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use kernel::{
    AuditSink, Capability, CapabilityToken, ConnectorCommand, ExecutionRoute, HarnessAdapter,
    HarnessError, HarnessKind, HarnessOutcome, HarnessRequest, InMemoryAuditSink, Kernel,
    SystemClock, TaskIntent, TaskState, TaskSupervisor, VerticalPackManifest,
};
use loong_spec::{SpecContextFactory, SpecExecutionContext};
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
pub(crate) async fn execute_daemon_task_with_supervisor(
    kernel: &Kernel<SpecContextFactory>,
    pack_id: &str,
    token: &CapabilityToken,
    intent: TaskIntent,
) -> CliResult<DaemonTaskExecution> {
    let mut supervisor = TaskSupervisor::new(intent);
    let policy_context = SpecExecutionContext::new(token);
    let dispatch_result = supervisor
        .execute(kernel, pack_id, token, &policy_context)
        .await;
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

pub(crate) async fn execute_daemon_turn_gateway_request(
    turn_service: &loong_app::agent_runtime::TurnExecutionService,
    session_hint: Option<&str>,
    mut request: loong_app::turn_gateway::TurnGatewayRequest,
    observer: Option<loong_app::conversation::ConversationTurnObserverHandle>,
    provider_error_mode: loong_app::conversation::ProviderErrorMode,
) -> CliResult<loong_app::agent_runtime::AgentTurnResult> {
    request.observer = observer;
    request.provider_error_mode = provider_error_mode;
    loong_app::turn_gateway::execute_projected_turn_gateway_request(
        turn_service,
        session_hint,
        &request,
        None,
    )
    .await
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExplicitAcpTurnExecutionRequest {
    pub(crate) session_id: String,
    pub(crate) input: String,
    pub(crate) channel_id: Option<String>,
    pub(crate) account_id: Option<String>,
    pub(crate) conversation_id: Option<String>,
    pub(crate) participant_id: Option<String>,
    pub(crate) thread_id: Option<String>,
    pub(crate) metadata: BTreeMap<String, String>,
    pub(crate) working_directory: Option<String>,
}

impl ExplicitAcpTurnExecutionRequest {
    pub(crate) fn with_required_text(mut self, session_id: String, input: String) -> Self {
        self.session_id = session_id;
        self.input = input;
        self
    }
}

impl From<crate::gateway::api_turn::GatewayHttpTurnRequest> for ExplicitAcpTurnExecutionRequest {
    fn from(request: crate::gateway::api_turn::GatewayHttpTurnRequest) -> Self {
        Self {
            session_id: request.session_id,
            input: request.input,
            channel_id: request.channel_id,
            account_id: request.account_id,
            conversation_id: request.conversation_id,
            participant_id: request.participant_id,
            thread_id: request.thread_id,
            metadata: request.metadata,
            working_directory: request.working_directory,
        }
    }
}

impl From<loong_protocol::ControlPlaneTurnSubmitRequest> for ExplicitAcpTurnExecutionRequest {
    fn from(request: loong_protocol::ControlPlaneTurnSubmitRequest) -> Self {
        Self {
            session_id: request.session_id,
            input: request.input,
            channel_id: request.channel_id,
            account_id: request.account_id,
            conversation_id: request.conversation_id,
            participant_id: request.participant_id,
            thread_id: request.thread_id,
            metadata: request.metadata,
            working_directory: request.working_directory,
        }
    }
}

pub(crate) fn normalize_explicit_acp_turn_execution_request(
    request: ExplicitAcpTurnExecutionRequest,
) -> Result<
    (
        loong_app::conversation::ConversationSessionAddress,
        loong_app::turn_gateway::TurnGatewayRequest,
    ),
    String,
> {
    let session_id = request.session_id.trim();
    if session_id.is_empty() {
        return Err("session_id is required".to_owned());
    }
    let input = request.input.trim();
    if input.is_empty() {
        return Err("input is required".to_owned());
    }

    let address = crate::build_acp_dispatch_address(
        session_id,
        request.channel_id.as_deref(),
        request.conversation_id.as_deref(),
        request.account_id.as_deref(),
        request.participant_id.as_deref(),
        request.thread_id.as_deref(),
    )
    .map_err(|error| format!("invalid turn target: {error}"))?;

    let working_directory = request
        .working_directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let gateway_request = loong_app::turn_gateway::build_turn_gateway_request(
        address.clone(),
        request.input,
        request.metadata,
        loong_app::agent_runtime::AgentTurnMode::Oneshot,
        loong_app::acp::AcpRoutingIntent::Explicit,
        false,
        Vec::new(),
        working_directory,
        false,
    );

    Ok((address, gateway_request))
}

pub(crate) async fn execute_explicit_acp_turn_request(
    resolved_path: std::path::PathBuf,
    config: loong_app::config::LoongConfig,
    acp_manager: Arc<loong_app::acp::AcpSessionManager>,
    event_sink: Option<&dyn loong_app::acp::AcpTurnEventSink>,
    request: ExplicitAcpTurnExecutionRequest,
) -> CliResult<loong_app::agent_runtime::AgentTurnResult> {
    let (_address, gateway_request) = normalize_explicit_acp_turn_execution_request(request)?;
    execute_explicit_acp_turn_gateway_request(
        resolved_path,
        config,
        acp_manager,
        event_sink,
        gateway_request,
    )
    .await
}

pub(crate) async fn execute_explicit_acp_turn_gateway_request(
    resolved_path: std::path::PathBuf,
    config: loong_app::config::LoongConfig,
    acp_manager: Arc<loong_app::acp::AcpSessionManager>,
    event_sink: Option<&dyn loong_app::acp::AcpTurnEventSink>,
    mut request: loong_app::turn_gateway::TurnGatewayRequest,
) -> CliResult<loong_app::agent_runtime::AgentTurnResult> {
    let execution = loong_app::turn_gateway::TurnGatewayExecution {
        resolved_path,
        config,
        app_ctx: None,
        acp_manager: Some(acp_manager),
        event_sink,
        initialize_runtime_environment: false,
    };
    request.acp_event_stream = event_sink.is_some();
    loong_app::turn_gateway::run_turn_gateway(execution, request).await
}

pub(crate) struct SeededGatewayTurnExecution {
    pub(crate) request_id: String,
    pub(crate) session_id: String,
    pub(crate) model: String,
    pub(crate) run_config: loong_app::config::LoongConfig,
    pub(crate) input: String,
    pub(crate) resolved_path: Option<std::path::PathBuf>,
}

pub(crate) fn build_seeded_gateway_turn_execution(
    request_id: String,
    model: String,
    input: String,
    history_turns: &[loong_app::memory::WindowTurn],
    mut run_config: loong_app::config::LoongConfig,
    resolved_path: Option<std::path::PathBuf>,
) -> Result<SeededGatewayTurnExecution, String> {
    #[cfg(feature = "memory-sqlite")]
    {
        let session_store_config =
            loong_app::session::store::session_store_config_from_memory_config_without_env_overrides(
                &run_config.memory,
            );
        let repo = loong_app::session::repository::SessionRepository::new(&session_store_config)?;
        repo.ensure_session(loong_app::session::repository::NewSessionRecord {
            session_id: request_id.clone(),
            kind: loong_app::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some(request_id.clone()),
            state: loong_app::session::repository::SessionState::Ready,
        })?;
    }

    let memory_config =
        loong_app::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
            &run_config.memory,
        );
    loong_app::memory::execute_memory_core_with_config(
        loong_app::memory::build_replace_turns_request(request_id.as_str(), history_turns),
        &memory_config,
    )
    .map_err(|error| format!("seed gateway turn session failed: {error}"))?;

    let session_id = request_id.clone();
    run_config.last_provider = None;

    Ok(SeededGatewayTurnExecution {
        request_id,
        session_id,
        model,
        run_config,
        input,
        resolved_path,
    })
}

pub(crate) async fn execute_seeded_gateway_turn(
    execution: &SeededGatewayTurnExecution,
    observer: Option<loong_app::conversation::ConversationTurnObserverHandle>,
) -> Result<loong_app::agent_runtime::AgentTurnResult, String> {
    let request = loong_app::turn_gateway::build_turn_gateway_request(
        loong_app::conversation::ConversationSessionAddress::from_session_id(
            execution.session_id.as_str(),
        ),
        execution.input.clone(),
        BTreeMap::new(),
        loong_app::agent_runtime::AgentTurnMode::Oneshot,
        loong_app::acp::AcpRoutingIntent::Automatic,
        false,
        Vec::new(),
        None,
        false,
    );
    let resolved_path = execution
        .resolved_path
        .clone()
        .ok_or_else(|| "seeded gateway turn execution requires resolved_path".to_owned())?;
    let turn_service = loong_app::agent_runtime::TurnExecutionService::new(
        resolved_path,
        execution.run_config.clone(),
    )
    .without_runtime_environment_init();
    execute_daemon_turn_gateway_request(
        &turn_service,
        Some(execution.session_id.as_str()),
        request,
        observer,
        loong_app::conversation::ProviderErrorMode::Propagate,
    )
    .await
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
        let projection_request = loong_app::turn_gateway::build_turn_gateway_request(
            loong_app::conversation::ConversationSessionAddress::from_session_id(
                payload
                    .session_hint
                    .clone()
                    .unwrap_or_else(|| "default".to_owned()),
            ),
            message,
            payload.metadata,
            payload.turn_mode,
            if payload.acp {
                loong_app::acp::AcpRoutingIntent::Explicit
            } else {
                loong_app::acp::AcpRoutingIntent::Automatic
            },
            payload.acp_event_stream,
            payload.acp_bootstrap_mcp_servers.clone(),
            payload.acp_cwd.clone(),
            matches!(
                payload.turn_mode,
                loong_app::agent_runtime::AgentTurnMode::Interactive
            ),
        );
        let turn_service =
            loong_app::agent_runtime::load_turn_execution_service(payload.config_path.as_deref())
                .map_err(HarnessError::Execution)?;
        let turn_result = execute_daemon_turn_gateway_request(
            &turn_service,
            payload.session_hint.as_deref(),
            projection_request,
            None,
            loong_app::conversation::ProviderErrorMode::InlineMessage,
        )
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
fn build_daemon_runtime_kernel() -> Kernel<SpecContextFactory> {
    let audit_sink = Arc::new(InMemoryAuditSink::default());
    let audit_sink = audit_sink as Arc<dyn AuditSink>;
    let clock = Arc::new(SystemClock) as Arc<dyn kernel::Clock>;
    let mut kernel = Kernel::<SpecContextFactory>::with_legacy_allow_runtime(clock, audit_sink);
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

    let policy_context = SpecExecutionContext::new(&token);
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
            &policy_context,
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

    #[tokio::test]
    async fn shared_daemon_turn_executor_preserves_propagated_provider_errors() {
        let resolved_path = std::env::temp_dir().join("loong-daemon-turn-propagate.toml");
        let mut config = loong_app::config::LoongConfig::default();
        config.memory.sqlite_path = std::env::temp_dir()
            .join("loong-daemon-turn-propagate.sqlite3")
            .display()
            .to_string();
        config.audit.mode = loong_app::config::AuditMode::InMemory;
        let sqlite_path = config.memory.resolved_sqlite_path();
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = loong_app::session::store::SessionStoreConfig {
                sqlite_path: Some(sqlite_path.clone()),
                runtime_config: None,
            };
            let repo = loong_app::session::repository::SessionRepository::new(&memory_config)
                .expect("provider propagation session repository");
            repo.ensure_session(loong_app::session::repository::NewSessionRecord {
                session_id: "turn-propagate".to_owned(),
                kind: loong_app::session::repository::SessionKind::Root,
                parent_session_id: None,
                label: Some("turn-propagate".to_owned()),
                state: loong_app::session::repository::SessionState::Ready,
            })
            .expect("seed provider propagation session");
        }
        loong_app::config::write(
            Some(resolved_path.to_string_lossy().as_ref()),
            &config,
            true,
        )
        .expect("write test config");
        let turn_service =
            loong_app::agent_runtime::TurnExecutionService::new(resolved_path.clone(), config)
                .without_runtime_environment_init();
        let request = loong_app::turn_gateway::build_turn_gateway_request(
            loong_app::conversation::ConversationSessionAddress::from_session_id("turn-propagate"),
            "hello".to_owned(),
            BTreeMap::new(),
            loong_app::agent_runtime::AgentTurnMode::Oneshot,
            loong_app::acp::AcpRoutingIntent::Automatic,
            false,
            Vec::new(),
            None,
            false,
        );

        let error = execute_daemon_turn_gateway_request(
            &turn_service,
            Some("turn-propagate"),
            request,
            None,
            loong_app::conversation::ProviderErrorMode::Propagate,
        )
        .await
        .expect_err("missing provider credentials should propagate");

        assert!(
            error.contains("OPENAI_API_KEY")
                || error.contains("ANTHROPIC_API_KEY")
                || error.contains("API key")
                || error.contains("unsupported_country_region_territory")
                || error.contains("provider model-list returned status"),
            "expected propagated provider configuration failure, got: {error}"
        );

        let _ = std::fs::remove_file(resolved_path);
        let _ = std::fs::remove_file(sqlite_path);
    }
}
