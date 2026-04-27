use super::*;

pub(super) async fn turn_submit(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Json(request): Json<ControlPlaneTurnSubmitRequest>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "turn/submit", &headers) {
        return *response;
    }

    let Some(turn_runtime) = state.turn_runtime.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/submit requires runtime control-plane serve --config <path>",
        );
    };

    if !turn_runtime.config.acp.enabled {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/submit requires ACP to be enabled (`acp.enabled=true`)",
        );
    }

    let session_id = match normalize_required_text(request.session_id.as_str(), "session_id") {
        Ok(session_id) => session_id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };
    if let Some(response) = ensure_turn_session_visible(&state, session_id.as_str()) {
        return response;
    }
    let input = match require_nonempty_text(request.input.as_str(), "input") {
        Ok(input) => input,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    if let Err(error) = crate::build_acp_dispatch_address(
        session_id.as_str(),
        request.channel_id.as_deref(),
        request.conversation_id.as_deref(),
        request.account_id.as_deref(),
        request.participant_id.as_deref(),
        request.thread_id.as_deref(),
    ) {
        return error_response(StatusCode::BAD_REQUEST, error);
    }

    let turn_snapshot = turn_runtime.registry.issue_turn(session_id.as_str());
    let turn_id = turn_snapshot.turn_id.clone();
    let resolved_path = turn_runtime.resolved_path.clone();
    let config = turn_runtime.config.clone();
    let acp_manager = turn_runtime.acp_manager.clone();
    let turn_registry = turn_runtime.registry.clone();
    let manager = state.manager.clone();
    let spawned_turn_id = turn_id;
    let channel_id = request.channel_id.clone();
    let account_id = request.account_id.clone();
    let conversation_id = request.conversation_id.clone();
    let thread_id = request.thread_id.clone();
    let metadata = request.metadata.clone();
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
            channel_id,
            account_id,
            conversation_id,
            participant_id: request.participant_id.clone(),
            thread_id,
            metadata,
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

    let response = ControlPlaneTurnSubmitResponse {
        turn: map_turn_summary(&turn_snapshot),
    };
    (StatusCode::ACCEPTED, Json(response)).into_response()
}

pub(super) async fn turn_result(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TurnResultQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "turn/result", &headers) {
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
    if let Err(response) = authorize_control_plane_request(&state, "turn/stream", &headers) {
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
