use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;

use axum::response::sse::Event;
use futures_util::stream::{self, Stream};
use loong_protocol::ControlPlaneTurnSubmitRequest;
use loong_protocol::{
    ControlPlaneTurnEventEnvelope, ControlPlaneTurnResultResponse, ControlPlaneTurnStatus,
    ControlPlaneTurnSummary,
};

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

pub(crate) struct ControlPlaneTurnStreamState {
    turn_id: String,
    registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    pending_events: VecDeque<mvp::control_plane::ControlPlaneTurnEventRecord>,
    receiver: tokio::sync::broadcast::Receiver<mvp::control_plane::ControlPlaneTurnEventRecord>,
    last_seq: u64,
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

    pub(crate) fn acp_enabled(&self) -> bool {
        self.config.acp.enabled
    }

    pub(crate) fn submit(
        self: &Arc<Self>,
        manager: Arc<mvp::control_plane::ControlPlaneManager>,
        session_id: String,
        input: String,
        request: ControlPlaneTurnSubmitRequest,
    ) -> mvp::control_plane::ControlPlaneTurnSnapshot {
        submit_control_plane_turn(self.clone(), manager, session_id, input, request)
    }

    pub(crate) fn read_turn(
        &self,
        turn_id: &str,
    ) -> Result<Option<mvp::control_plane::ControlPlaneTurnSnapshot>, String> {
        self.registry.read_turn(turn_id)
    }

    pub(crate) fn stream(
        &self,
        turn_id: String,
        after_seq: u64,
    ) -> Result<impl Stream<Item = Result<Event, Infallible>> + 'static, String> {
        control_plane_turn_stream(self.registry.clone(), turn_id, after_seq)
    }

    pub(crate) fn snapshot_has_streamable_events(
        snapshot: &mvp::control_plane::ControlPlaneTurnSnapshot,
    ) -> bool {
        !(snapshot.status.is_terminal() && snapshot.event_count == 0)
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

pub(crate) fn map_turn_status(
    status: mvp::control_plane::ControlPlaneTurnStatus,
) -> ControlPlaneTurnStatus {
    match status {
        mvp::control_plane::ControlPlaneTurnStatus::Running => ControlPlaneTurnStatus::Running,
        mvp::control_plane::ControlPlaneTurnStatus::Completed => ControlPlaneTurnStatus::Completed,
        mvp::control_plane::ControlPlaneTurnStatus::Failed => ControlPlaneTurnStatus::Failed,
        mvp::control_plane::ControlPlaneTurnStatus::Cancelled => ControlPlaneTurnStatus::Cancelled,
    }
}

pub(crate) fn map_turn_summary(
    snapshot: &mvp::control_plane::ControlPlaneTurnSnapshot,
) -> ControlPlaneTurnSummary {
    ControlPlaneTurnSummary {
        turn_id: snapshot.turn_id.clone(),
        session_id: snapshot.session_id.clone(),
        status: map_turn_status(snapshot.status),
        submitted_at_ms: snapshot.submitted_at_ms,
        completed_at_ms: snapshot.completed_at_ms,
        event_count: snapshot.event_count,
    }
}

pub(crate) fn map_turn_result(
    snapshot: &mvp::control_plane::ControlPlaneTurnSnapshot,
) -> ControlPlaneTurnResultResponse {
    ControlPlaneTurnResultResponse {
        turn: map_turn_summary(snapshot),
        output_text: snapshot.output_text.clone(),
        stop_reason: snapshot.stop_reason.clone(),
        usage: snapshot.usage.clone(),
        error: snapshot.error.clone(),
    }
}

pub(crate) fn map_turn_event(
    record: mvp::control_plane::ControlPlaneTurnEventRecord,
) -> ControlPlaneTurnEventEnvelope {
    ControlPlaneTurnEventEnvelope {
        turn_id: record.turn_id,
        session_id: record.session_id,
        seq: record.seq,
        terminal: record.terminal,
        payload: record.payload,
    }
}

pub(crate) fn sse_event_from_turn_record(
    record: mvp::control_plane::ControlPlaneTurnEventRecord,
) -> Result<Event, String> {
    let seq = record.seq;
    let terminal = record.terminal;
    let envelope = map_turn_event(record);
    let event_name = if terminal {
        "turn.terminal"
    } else {
        "turn.event"
    };
    let event_id = seq.to_string();
    let base_event = Event::default();
    let named_event = base_event.event(event_name);
    let identified_event = named_event.id(event_id);

    identified_event
        .json_data(&envelope)
        .map_err(|error| format!("control-plane turn SSE event encoding failed: {error}"))
}

pub(crate) fn fallback_turn_sse_error_event(message: &str) -> Event {
    let error_message = format!("{{\"error\":\"{message}\"}}");
    let base_event = Event::default();
    let named_event = base_event.event("turn.error");
    named_event.data(error_message)
}

pub(crate) fn initial_turn_stream_state(
    registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    turn_id: &str,
    after_seq: u64,
) -> Result<ControlPlaneTurnStreamState, String> {
    let receiver = registry.subscribe();
    let pending_events = registry.recent_events_after(
        turn_id,
        after_seq,
        crate::control_plane_server::CONTROL_PLANE_DEFAULT_EVENT_LIMIT,
    )?;
    let pending_events = VecDeque::from(pending_events);

    Ok(ControlPlaneTurnStreamState {
        turn_id: turn_id.to_owned(),
        registry,
        pending_events,
        receiver,
        last_seq: after_seq,
    })
}

pub(crate) async fn next_turn_sse_item(
    mut state: ControlPlaneTurnStreamState,
) -> Option<(Result<Event, Infallible>, ControlPlaneTurnStreamState)> {
    loop {
        let pending_event = state.pending_events.pop_front();
        if let Some(record) = pending_event {
            state.last_seq = record.seq;
            let event = match sse_event_from_turn_record(record) {
                Ok(event) => event,
                Err(error) => fallback_turn_sse_error_event(error.as_str()),
            };
            return Some((Ok(event), state));
        }

        let snapshot = match state.registry.read_turn(state.turn_id.as_str()) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return None,
            Err(_) => return None,
        };
        if snapshot.status.is_terminal() {
            return None;
        }

        let receive_result = state.receiver.recv().await;
        match receive_result {
            Ok(record) => {
                if record.turn_id != state.turn_id {
                    continue;
                }
                if record.seq <= state.last_seq {
                    continue;
                }
                state.last_seq = record.seq;
                let event = match sse_event_from_turn_record(record) {
                    Ok(event) => event,
                    Err(error) => fallback_turn_sse_error_event(error.as_str()),
                };
                return Some((Ok(event), state));
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                let refill_result = state.registry.recent_events_after(
                    state.turn_id.as_str(),
                    state.last_seq,
                    crate::control_plane_server::CONTROL_PLANE_DEFAULT_EVENT_LIMIT,
                );
                let refill = refill_result.unwrap_or_default();
                state.pending_events = VecDeque::from(refill);
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

pub(crate) fn control_plane_turn_stream(
    registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    turn_id: String,
    after_seq: u64,
) -> Result<impl Stream<Item = Result<Event, Infallible>>, String> {
    let initial_state = initial_turn_stream_state(registry, turn_id.as_str(), after_seq)?;
    Ok(stream::unfold(initial_state, next_turn_sse_item))
}
