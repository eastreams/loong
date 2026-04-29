use std::collections::BTreeSet;

use async_trait::async_trait;
use kernel::{
    Capability, CapabilityToken, ConnectorCommand, ExecutionRoute, HarnessAdapter, HarnessError,
    HarnessKind, HarnessOutcome, HarnessRequest, LoongKernel, PolicyEngine, StaticPolicyEngine,
    TaskIntent, TaskState, TaskSupervisor,
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
        let (resolved_path, config) = loong_app::config::load(payload.config_path.as_deref())
            .map_err(HarnessError::Execution)?;
        crate::trusted_host_runtime::dispatch_turn_start_hook_for_request(
            &config,
            payload.session_hint.as_deref(),
            &turn_request,
        )
        .await
        .map_err(HarnessError::Execution)?;
        let runtime = loong_app::agent_runtime::AgentRuntime::new();
        let turn_result = runtime
            .run_turn_with_loaded_config(
                resolved_path,
                config.clone(),
                payload.session_hint.as_deref(),
                &turn_request,
                None,
            )
            .await;
        let turn_result = match turn_result {
            Ok(turn_result) => {
                crate::trusted_host_runtime::dispatch_turn_end_hook_for_success(
                    &config,
                    payload.session_hint.as_deref(),
                    &turn_request,
                    &turn_result,
                )
                .await
                .map_err(HarnessError::Execution)?;
                turn_result
            }
            Err(error) => {
                if let Err(turn_end_error) =
                    crate::trusted_host_runtime::dispatch_turn_end_hook_for_error(
                        &config,
                        payload.session_hint.as_deref(),
                        &turn_request,
                        error.as_str(),
                    )
                    .await
                {
                    return Err(HarnessError::Execution(format!(
                        "{error}; trusted host turn_end hook failed: {turn_end_error}"
                    )));
                }
                return Err(HarnessError::Execution(error));
            }
        };

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
    let mut kernel = kernel_bootstrap::BootstrapBuilder::default().into_builder();
    kernel.register_harness_adapter(EmbeddedAgentHarness);
    kernel
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
    let (_resolved_path, config) = loong_app::config::load(config_path)?;
    if !config.cli.enabled {
        return Err("CLI channel is disabled by config.cli.enabled=false".to_owned());
    }

    let kernel = build_daemon_runtime_kernel();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
        .map_err(|error| format!("token issue failed: {error}"))?;
    let payload = serde_json::to_value(DaemonTurnTaskPayload {
        config_path: config_path.map(ToOwned::to_owned),
        session_hint: session_hint.map(ToOwned::to_owned),
        message: Some(message.to_owned()),
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
    })
    .map_err(|error| format!("serialize turn payload failed: {error}"))?;

    let dispatch = execute_daemon_task_with_supervisor(
        &kernel,
        DEFAULT_PACK_ID,
        &token,
        TaskIntent {
            task_id: "turn-run-01".to_owned(),
            objective: message.to_owned(),
            required_capabilities: BTreeSet::from([
                Capability::InvokeTool,
                Capability::MemoryRead,
                Capability::MemoryWrite,
            ]),
            payload,
        },
    )
    .await?;
    let (_, outcome) = require_successful_daemon_task_execution(&dispatch)?;
    let result =
        serde_json::from_value::<loong_app::agent_runtime::AgentTurnResult>(outcome.output.clone())
            .map_err(|error| format!("parse turn result failed: {error}"))?;
    println!("{}", result.output_text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

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
    async fn daemon_runtime_kernel_overrides_pi_local_stub_harness() {
        let mut env = crate::test_support::ScopedEnv::new();
        let home = std::env::temp_dir().join(format!(
            "loong-daemon-runtime-harness-home-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&home).expect("create isolated daemon home");
        env.set("HOME", &home);
        env.remove("LOONG_HOME");
        env.remove("LOONG_CONFIG_PATH");
        env.remove("LOONGCLAW_CONFIG_PATH");

        let kernel = build_daemon_runtime_kernel();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
            .expect("issue token");
        let payload = serde_json::to_value(DaemonTurnTaskPayload {
            message: Some("hello".to_owned()),
            ..DaemonTurnTaskPayload::default()
        })
        .expect("serialize turn payload");

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
            .expect("missing config should fail through the real runtime harness");

        assert!(execution.outcome.is_none());
        assert!(
            error.contains("failed to read config"),
            "expected unified runtime harness failure, got: {error}"
        );
    }

    fn write_file(root: &Path, relative_path: &str, contents: &str) {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write file");
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create unique temp dir");
        path
    }

    fn install_blocked_trusted_host_runtime_plugin(root: &Path) {
        install_trusted_host_runtime_plugin(root, "blocked-node", "[\"turn_start\"]");
        write_file(
            root,
            "runtime-plugins/trusted-host/index.js",
            "#!/usr/bin/env node\nprocess.stdin.resume();\n",
        );
    }

    fn install_trusted_host_runtime_plugin(root: &Path, command: &str, host_hooks_json: &str) {
        let manifest = serde_json::json!({
            "api_version": "v1alpha1",
            "version": "1.0.0",
            "plugin_id": "trusted-host-extension",
            "provider_id": "trusted-host-extension",
            "connector_name": "trusted-host-extension",
            "capabilities": ["InvokeConnector"],
            "metadata": {
                "bridge_kind": "process_stdio",
                "adapter_family": "javascript-stdio-adapter",
                "entrypoint": "stdin/stdout::invoke",
                "source_language": "javascript",
                "command": command,
                "args_json": "[\"index.js\"]",
                "process_timeout_ms": "15000",
                "loong_extension_contract": "process_stdio_json_line_v1",
                "loong_extension_family": "trusted_host_extension",
                "loong_extension_trust_lane": "trusted_host",
                "loong_extension_methods_json": "[\"extension/event\"]",
                "loong_extension_host_hooks_json": host_hooks_json,
            }
        });
        write_file(
            root,
            "runtime-plugins/trusted-host/loong.plugin.json",
            &serde_json::to_string_pretty(&manifest).expect("serialize trusted host manifest"),
        );
    }

    struct TestAcpHarnessBackend {
        id: &'static str,
    }

    impl TestAcpHarnessBackend {
        fn new(id: &'static str) -> Self {
            Self { id }
        }
    }

    impl loong_app::acp::AcpRuntimeBackend for TestAcpHarnessBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> loong_app::acp::AcpBackendMetadata {
            loong_app::acp::AcpBackendMetadata::new(
                self.id(),
                std::iter::empty::<loong_app::acp::AcpCapability>(),
                "TaskExecution ACP Test Backend",
            )
        }

        fn ensure_session<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _config: &'life1 loong_app::config::LoongConfig,
            request: &'life2 loong_app::acp::AcpSessionBootstrap,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = CliResult<loong_app::acp::AcpSessionHandle>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                Ok(loong_app::acp::AcpSessionHandle {
                    session_key: request.session_key.clone(),
                    backend_id: self.id().to_owned(),
                    runtime_session_name: format!("task-runtime-{}", request.session_key),
                    working_directory: request.working_directory.clone(),
                    backend_session_id: Some(format!("backend-{}", request.session_key)),
                    agent_session_id: Some(format!("agent-{}", request.session_key)),
                    binding: request.binding.clone(),
                })
            })
        }

        fn run_turn<'life0, 'life1, 'life2, 'life3, 'async_trait>(
            &'life0 self,
            _config: &'life1 loong_app::config::LoongConfig,
            _session: &'life2 loong_app::acp::AcpSessionHandle,
            request: &'life3 loong_app::acp::AcpTurnRequest,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = CliResult<loong_app::acp::AcpTurnResult>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            'life3: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                Ok(loong_app::acp::AcpTurnResult {
                    output_text: format!("acp: {}", request.input),
                    state: loong_app::acp::AcpSessionState::Ready,
                    usage: Some(json!({"total_tokens": 3})),
                    events: Vec::new(),
                    stop_reason: Some(loong_app::acp::AcpTurnStopReason::Completed),
                })
            })
        }

        fn run_turn_with_sink<'life0, 'life1, 'life2, 'life3, 'life5, 'async_trait>(
            &'life0 self,
            config: &'life1 loong_app::config::LoongConfig,
            session: &'life2 loong_app::acp::AcpSessionHandle,
            request: &'life3 loong_app::acp::AcpTurnRequest,
            _abort: Option<loong_app::acp::AcpAbortSignal>,
            _sink: Option<&'life5 dyn loong_app::acp::AcpTurnEventSink>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = CliResult<loong_app::acp::AcpTurnResult>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            'life3: 'async_trait,
            'life5: 'async_trait,
            Self: 'async_trait,
        {
            self.run_turn(config, session, request)
        }

        fn cancel<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _config: &'life1 loong_app::config::LoongConfig,
            _session: &'life2 loong_app::acp::AcpSessionHandle,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { Ok(()) })
        }

        fn close<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _config: &'life1 loong_app::config::LoongConfig,
            _session: &'life2 loong_app::acp::AcpSessionHandle,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { Ok(()) })
        }
    }

    fn register_test_acp_backend(backend_id: &'static str) {
        loong_app::acp::register_acp_backend(backend_id, move || {
            Box::new(TestAcpHarnessBackend::new(backend_id))
        })
        .expect("register task execution ACP backend");
    }

    fn write_turn_end_marker_entrypoint(root: &Path) {
        write_file(
            root,
            "runtime-plugins/trusted-host/index.js",
            "#!/usr/bin/env node\nconst fs = require('fs');\nfunction emitResponse(line) { const trimmed = line.trim(); if (!trimmed) return; const request = JSON.parse(trimmed); const payload = request.payload ?? {}; const markerPath = payload.payload?.hook_payload?.metadata?.hook_marker_path ?? null; if (markerPath) { fs.writeFileSync(markerPath, payload.payload?.host_hook ?? 'unknown'); } const response = { method: request.method ?? '', id: request.id ?? null, payload: { handled_hook: payload.payload?.host_hook ?? null, outcome_status: payload.payload?.hook_payload?.outcome?.status ?? null } }; process.stdout.write(`${JSON.stringify(response)}\\n`); } process.stdin.setEncoding('utf8'); let buffered=''; process.stdin.on('data', chunk => { buffered += chunk; let newlineIndex = buffered.indexOf('\\n'); while (newlineIndex !== -1) { const line = buffered.slice(0, newlineIndex); buffered = buffered.slice(newlineIndex + 1); emitResponse(line); newlineIndex = buffered.indexOf('\\n'); } }); process.stdin.on('end', () => { if (buffered.trim()) emitResponse(buffered); }); process.stdin.resume();\n",
        );
    }

    #[tokio::test]
    async fn daemon_runtime_kernel_fails_closed_when_trusted_turn_start_hook_is_not_allowlisted() {
        let root = unique_temp_dir("loong-daemon-trusted-host-harness");
        install_blocked_trusted_host_runtime_plugin(&root);
        let config_path = root.join("loong.toml");
        let mut config = loong_app::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["process_stdio".to_owned()];
        config.runtime_plugins.allowed_process_commands = vec!["node".to_owned()];
        loong_app::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
            .expect("write config");

        let kernel = build_daemon_runtime_kernel();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
            .expect("issue token");
        let payload = serde_json::to_value(DaemonTurnTaskPayload {
            config_path: Some(config_path.display().to_string()),
            message: Some("hello".to_owned()),
            ..DaemonTurnTaskPayload::default()
        })
        .expect("serialize turn payload");

        let execution = execute_daemon_task_with_supervisor(
            &kernel,
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-trusted-host-hook-01".to_owned(),
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
            .expect("trusted host hook failure should fault the harness");

        assert!(execution.outcome.is_none());
        assert!(
            error.contains("trusted host extension trusted-host-extension"),
            "expected trusted host hook failure, got: {error}"
        );
        assert!(error.contains("runtime_plugins.allowed_process_commands"));
    }

    #[tokio::test]
    async fn daemon_runtime_kernel_dispatches_trusted_turn_end_hook_for_successful_acp_turn() {
        let root = unique_temp_dir("loong-daemon-trusted-turn-end-success");
        install_trusted_host_runtime_plugin(&root, "node", "[\"turn_end\"]");
        write_turn_end_marker_entrypoint(&root);
        let marker_path = root.join("turn-end-marker.txt");
        let config_path = root.join("loong.toml");
        let session_hint = format!(
            "trusted-turn-end-session-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        );
        let backend_id: &'static str = Box::leak(
            format!(
                "task-execution-turn-end-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after epoch")
                    .as_nanos()
            )
            .into_boxed_str(),
        );
        register_test_acp_backend(backend_id);

        let mut config = loong_app::config::LoongConfig::default();
        config.acp.enabled = true;
        config.acp.backend = Some(backend_id.to_owned());
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["process_stdio".to_owned()];
        config.runtime_plugins.allowed_process_commands = vec!["node".to_owned()];
        loong_app::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
            .expect("write config");

        let kernel = build_daemon_runtime_kernel();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
            .expect("issue token");
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert(
            "hook_marker_path".to_owned(),
            marker_path.display().to_string(),
        );
        let payload = serde_json::to_value(DaemonTurnTaskPayload {
            config_path: Some(config_path.display().to_string()),
            session_hint: Some(session_hint),
            message: Some("hello".to_owned()),
            turn_mode: loong_app::agent_runtime::AgentTurnMode::Acp,
            metadata,
            acp: true,
            ..DaemonTurnTaskPayload::default()
        })
        .expect("serialize turn payload");

        let execution = execute_daemon_task_with_supervisor(
            &kernel,
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-trusted-turn-end-success-01".to_owned(),
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

        assert!(
            execution.error.is_none(),
            "unexpected execution error: {:?}",
            execution.error
        );
        assert!(
            execution.outcome.is_some(),
            "expected successful harness outcome"
        );
        let marker_contents =
            fs::read_to_string(&marker_path).expect("turn_end hook should write marker");
        assert_eq!(marker_contents, "turn_end");
    }
}
