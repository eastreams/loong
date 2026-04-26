use std::sync::Arc;

use loong_protocol::ControlPlaneTurnSubmitRequest;

use crate::{CliResult, mvp};

/// Shared dependencies for ad-hoc turn execution launched from the control
/// plane HTTP surface.
///
/// This is intentionally narrower than the full control-plane router state: it
/// keeps just enough config, ACP ownership, and per-turn event registry state
/// to materialize `AgentRuntime` turns on demand.
pub(crate) struct ControlPlaneTurnRuntime {
    pub(crate) resolved_path: std::path::PathBuf,
    pub(crate) config: mvp::config::LoongConfig,
    pub(crate) acp_manager: Arc<mvp::acp::AcpSessionManager>,
    pub(crate) registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
}

struct ControlPlaneTurnEventForwarder {
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    turn_id: String,
}

impl ControlPlaneTurnRuntime {
    /// Build a control-plane turn runtime from a config snapshot and the shared
    /// ACP manager that should back all HTTP-triggered turns for that process.
    pub(crate) fn new(
        resolved_path: std::path::PathBuf,
        config: mvp::config::LoongConfig,
    ) -> Result<Self, String> {
        let acp_manager = mvp::acp::shared_acp_session_manager(&config)?;
        Ok(Self::with_manager(resolved_path, config, acp_manager))
    }

    /// Test/advanced constructor that reuses an already prepared ACP manager
    /// while still allocating a fresh turn registry for this runtime shell.
    pub(crate) fn with_manager(
        resolved_path: std::path::PathBuf,
        config: mvp::config::LoongConfig,
        acp_manager: Arc<mvp::acp::AcpSessionManager>,
    ) -> Self {
        Self {
            resolved_path,
            config,
            acp_manager,
            registry: Arc::new(mvp::control_plane::ControlPlaneTurnRegistry::new()),
        }
    }
}

impl mvp::acp::AcpTurnEventSink for ControlPlaneTurnEventForwarder {
    fn on_event(&self, event: &serde_json::Value) -> CliResult<()> {
        let recorded_event = self
            .registry
            .record_runtime_event(self.turn_id.as_str(), event.clone())?;
        let payload = map_turn_event_payload(&recorded_event);
        let _ = self.manager.record_acp_turn_event(payload, true);
        Ok(())
    }
}

pub(crate) fn submit_control_plane_turn(
    turn_runtime: Arc<ControlPlaneTurnRuntime>,
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    session_id: String,
    input: String,
    request: ControlPlaneTurnSubmitRequest,
) -> mvp::control_plane::ControlPlaneTurnSnapshot {
    let turn_snapshot = turn_runtime.registry.issue_turn(session_id.as_str());
    let turn_id = turn_snapshot.turn_id.clone();
    let resolved_path = turn_runtime.resolved_path.clone();
    let config = turn_runtime.config.clone();
    let acp_manager = turn_runtime.acp_manager.clone();
    let turn_registry = turn_runtime.registry.clone();
    let spawned_turn_id = turn_id;
    let working_directory = request
        .working_directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    tokio::spawn(async move {
        let event_forwarder = ControlPlaneTurnEventForwarder {
            manager: manager.clone(),
            registry: turn_registry.clone(),
            turn_id: spawned_turn_id.clone(),
        };
        let turn_request = mvp::agent_runtime::AgentTurnRequest {
            message: input,
            turn_mode: mvp::agent_runtime::AgentTurnMode::Acp,
            channel_id: request.channel_id,
            account_id: request.account_id,
            conversation_id: request.conversation_id,
            participant_id: request.participant_id,
            thread_id: request.thread_id,
            metadata: request.metadata,
            acp: true,
            acp_event_stream: true,
            acp_bootstrap_mcp_servers: Vec::new(),
            acp_cwd: working_directory,
            live_surface_enabled: false,
        };
        let turn_service =
            crate::mvp::agent_runtime::TurnExecutionService::new(resolved_path, config)
                .with_acp_manager(acp_manager)
                .without_runtime_environment_init();
        let turn_options = crate::mvp::agent_runtime::TurnExecutionOptions {
            event_sink: Some(&event_forwarder),
            ..Default::default()
        };
        let execution_result = turn_service
            .execute(Some(session_id.as_str()), &turn_request, turn_options)
            .await;

        match execution_result {
            Ok(result) => {
                let completion = turn_registry.complete_success(
                    spawned_turn_id.as_str(),
                    result.output_text.as_str(),
                    result.stop_reason.as_deref(),
                    result.usage.clone(),
                );
                if let Ok(record) = completion {
                    let payload = map_turn_event_payload(&record);
                    let _ = manager.record_acp_turn_event(payload, true);
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "loong.control-plane",
                    turn_id = %spawned_turn_id,
                    session_id = %session_id,
                    error = %crate::observability::summarize_error(error.as_str()),
                    "control-plane turn execution failed"
                );
                let completion = turn_registry.complete_failure(spawned_turn_id.as_str(), &error);
                if let Ok(record) = completion {
                    let payload = map_turn_event_payload(&record);
                    let _ = manager.record_acp_turn_event(payload, true);
                }
            }
        }
    });

    turn_snapshot
}

fn map_turn_event_payload(
    record: &mvp::control_plane::ControlPlaneTurnEventRecord,
) -> serde_json::Value {
    serde_json::json!({
        "turn_id": record.turn_id,
        "session_id": record.session_id,
        "seq": record.seq,
        "terminal": record.terminal,
        "payload": record.payload,
    })
}
