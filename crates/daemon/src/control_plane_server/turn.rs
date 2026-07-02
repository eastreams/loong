use super::*;
use crate::task_execution::{ExplicitAcpTurnExecutionRequest, execute_explicit_acp_turn_request};

pub(super) async fn turn_submit(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Json(request): Json<ControlPlaneTurnSubmitRequest>,
) -> Response {
    let (turn_runtime, session_id, input) =
        match prepare_turn_submit(&state, &headers, &request).await {
            Ok(prepared) => prepared,
            Err(response) => return *response,
        };

    let turn_snapshot = turn_runtime.registry.issue_turn(session_id.as_str());
    let turn_id = turn_snapshot.turn_id.clone();
    let resolved_path = turn_runtime.resolved_path.clone();
    let config = turn_runtime.config.clone();
    let acp_manager = turn_runtime.acp_manager.clone();
    let turn_registry = turn_runtime.registry.clone();
    let manager = state.manager.clone();
    let spawned_turn_id = turn_id;
    let turn_request = ExplicitAcpTurnExecutionRequest::from(request)
        .with_required_text(session_id.clone(), input);

    spawn_control_plane_turn_execution(
        resolved_path,
        config,
        acp_manager,
        turn_registry,
        manager,
        spawned_turn_id,
        session_id,
        turn_request,
    );

    let response = ControlPlaneTurnSubmitResponse {
        turn: map_turn_summary(&turn_snapshot),
    };
    (StatusCode::ACCEPTED, Json(response)).into_response()
}

async fn prepare_turn_submit<'a>(
    state: &'a ControlPlaneHttpState,
    headers: &HeaderMap,
    request: &'a ControlPlaneTurnSubmitRequest,
) -> Result<(&'a Arc<ControlPlaneTurnRuntime>, String, String), Box<Response>> {
    authorize_control_plane_request(state, "turn/submit", headers).await?;

    let Some(turn_runtime) = state.turn_runtime.as_ref() else {
        return Err(Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/submit requires runtime control-plane serve --config <path>",
        )));
    };

    if !turn_runtime.config.acp.enabled {
        return Err(Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/submit requires ACP to be enabled (`acp.enabled=true`)",
        )));
    }

    let session_id = normalize_required_text(request.session_id.as_str(), "session_id")
        .map_err(|error| Box::new(error_response(StatusCode::BAD_REQUEST, error)))?;
    if let Some(response) = ensure_turn_session_visible(state, session_id.as_str()) {
        return Err(Box::new(response));
    }
    let input = require_nonempty_text(request.input.as_str(), "input")
        .map_err(|error| Box::new(error_response(StatusCode::BAD_REQUEST, error)))?;

    Ok((turn_runtime, session_id, input))
}

fn spawn_control_plane_turn_execution(
    resolved_path: std::path::PathBuf,
    config: mvp::config::LoongConfig,
    acp_manager: Arc<mvp::acp::AcpSessionManager>,
    turn_registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    turn_id: String,
    session_id: String,
    turn_request: ExplicitAcpTurnExecutionRequest,
) {
    tokio::spawn(async move {
        let event_forwarder = ControlPlaneTurnEventForwarder {
            manager: manager.clone(),
            registry: turn_registry.clone(),
            turn_id: turn_id.clone(),
        };
        let execution_result = execute_explicit_acp_turn_request(
            resolved_path,
            config,
            acp_manager,
            Some(&event_forwarder),
            turn_request,
        )
        .await;

        finalize_turn_execution(
            turn_registry,
            manager,
            turn_id,
            session_id,
            execution_result,
        );
    });
}

fn finalize_turn_execution(
    turn_registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    turn_id: String,
    session_id: String,
    execution_result: CliResult<loong_app::agent_runtime::AgentTurnResult>,
) {
    match execution_result {
        Ok(result) => {
            let completion = turn_registry.complete_success(
                turn_id.as_str(),
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
                turn_id = %turn_id,
                session_id = %session_id,
                error = %crate::observability::summarize_error(error.as_str()),
                "control-plane turn execution failed"
            );
            let completion = turn_registry.complete_failure(turn_id.as_str(), &error);
            if let Ok(record) = completion {
                let payload = map_turn_event_payload(&record);
                let _ = manager.record_acp_turn_event(payload, true);
            }
        }
    }
}

pub(super) async fn turn_result(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TurnResultQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "turn/result", &headers).await {
        return *response;
    }

    let Some(turn_runtime) = state.turn_runtime.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/result requires runtime control-plane serve --config <path>",
        );
    };

    let turn_id = match normalize_required_text(query.turn_id.as_str(), "turn_id") {
        Ok(turn_id) => turn_id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    let snapshot = match turn_runtime.registry.read_turn(turn_id.as_str()) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            let message = format!("turn `{}` not found", turn_id);
            return error_response(StatusCode::NOT_FOUND, message);
        }
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    if let Some(response) = ensure_turn_session_visible(&state, snapshot.session_id.as_str()) {
        return response;
    }

    Json(map_turn_result(&snapshot)).into_response()
}

pub(super) async fn turn_stream(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TurnStreamQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "turn/stream", &headers).await {
        return *response;
    }

    let Some(turn_runtime) = state.turn_runtime.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/stream requires runtime control-plane serve --config <path>",
        );
    };

    let turn_id = match normalize_required_text(query.turn_id.as_str(), "turn_id") {
        Ok(turn_id) => turn_id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    let snapshot = match turn_runtime.registry.read_turn(turn_id.as_str()) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            let message = format!("turn `{}` not found", turn_id);
            return error_response(StatusCode::NOT_FOUND, message);
        }
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    if let Some(response) = ensure_turn_session_visible(&state, snapshot.session_id.as_str()) {
        return response;
    }
    if snapshot.status.is_terminal() && snapshot.event_count == 0 {
        return error_response(
            StatusCode::CONFLICT,
            format!("turn `{}` completed without any streamable events", turn_id),
        );
    }

    let after_seq = query.after_seq.unwrap_or(0);
    let stream_result =
        control_plane_turn_stream(turn_runtime.registry.clone(), turn_id, after_seq);
    let stream = match stream_result {
        Ok(stream) => stream,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    let keep_alive = KeepAlive::new()
        .interval(std::time::Duration::from_millis(
            CONTROL_PLANE_TICK_INTERVAL_MS,
        ))
        .text(CONTROL_PLANE_KEEPALIVE_TEXT);
    Sse::new(stream).keep_alive(keep_alive).into_response()
}
