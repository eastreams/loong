use super::*;

pub(super) async fn current_snapshot(
    state: &ControlPlaneHttpState,
) -> Result<ControlPlaneSnapshot, String> {
    let mut snapshot = map_snapshot(state.manager.snapshot());
    #[cfg(feature = "memory-sqlite")]
    if let Some(repository_view) = state.repository_view.as_ref() {
        let repository_snapshot = repository_view.snapshot_summary()?;
        snapshot.session_count = repository_snapshot.session_count;
        snapshot.pending_approval_count = repository_snapshot.pending_approval_count;
    }
    #[cfg(feature = "memory-sqlite")]
    if let Some(acp_view) = state.acp_view.as_ref() {
        snapshot.acp_session_count = acp_view.visible_session_count().await?;
    }
    Ok(snapshot)
}

pub(super) async fn readyz() -> impl IntoResponse {
    StatusCode::OK
}

pub(super) async fn control_challenge(State(state): State<ControlPlaneHttpState>) -> Response {
    let challenge = state.challenge_registry.issue();
    Json(ControlPlaneChallengeResponse {
        nonce: challenge.nonce,
        issued_at_ms: challenge.issued_at_ms,
        expires_at_ms: challenge.expires_at_ms,
    })
    .into_response()
}

pub(super) async fn healthz(State(state): State<ControlPlaneHttpState>) -> Response {
    match current_snapshot(&state).await {
        Ok(snapshot) => Json(ControlPlaneSnapshotResponse { snapshot }).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn control_snapshot(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/snapshot", &headers) {
        return *response;
    }
    match current_snapshot(&state).await {
        Ok(snapshot) => Json(ControlPlaneSnapshotResponse { snapshot }).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn control_events(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<EventQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/events", &headers) {
        return *response;
    }
    let limit = query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_EVENT_LIMIT);
    let events = if let Some(after_seq) = query.after_seq {
        let timeout_ms = query.timeout_ms.unwrap_or(CONTROL_PLANE_TICK_INTERVAL_MS);
        state
            .manager
            .wait_for_recent_events(after_seq, limit, query.include_targeted, timeout_ms)
            .await
    } else {
        state.manager.recent_events(limit, query.include_targeted)
    };
    let events = events.into_iter().map(map_event).collect::<Vec<_>>();
    Json(ControlPlaneRecentEventsResponse { events }).into_response()
}

pub(super) async fn control_subscribe(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<SubscribeQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/subscribe", &headers) {
        return *response;
    }
    let after_seq = query.after_seq.unwrap_or(0);
    let include_targeted = query.include_targeted;
    let manager = state.manager;
    let stream = control_plane_subscribe_stream(manager, after_seq, include_targeted);
    let keep_alive = KeepAlive::new()
        .interval(std::time::Duration::from_millis(
            CONTROL_PLANE_TICK_INTERVAL_MS,
        ))
        .text(CONTROL_PLANE_KEEPALIVE_TEXT);
    let sse = Sse::new(stream).keep_alive(keep_alive);
    sse.into_response()
}

pub(super) async fn control_ping(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/ping", &headers) {
        return *response;
    }
    match current_snapshot(&state).await {
        Ok(snapshot) => Json(serde_json::json!({
            "protocol": CONTROL_PLANE_PROTOCOL_VERSION,
            "state_version": snapshot.state_version,
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn control_connect(
    State(state): State<ControlPlaneHttpState>,
    Json(request): Json<ControlPlaneConnectRequest>,
) -> Response {
    if request.max_protocol < CONTROL_PLANE_PROTOCOL_VERSION
        || request.min_protocol > CONTROL_PLANE_PROTOCOL_VERSION
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("protocol mismatch: expected protocol {CONTROL_PLANE_PROTOCOL_VERSION}"),
        );
    }
    if let Err(response) = verify_remote_connect_bootstrap_auth(&state, &request) {
        return *response;
    }
    if let Err(response) = verify_connect_device_challenge(&state, &request) {
        return *response;
    }
    if let Some(device) = request.device.as_ref() {
        let requested_scopes = request
            .scopes
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let device_token = request
            .auth
            .as_ref()
            .and_then(|auth| auth.device_token.as_deref());
        let pairing_decision = match state.pairing_registry.evaluate_connect(
            &device.device_id,
            &request.client.id,
            &device.public_key,
            request.role.as_str(),
            &requested_scopes,
            device_token,
        ) {
            Ok(decision) => decision,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        };
        match pairing_decision {
            mvp::control_plane::ControlPlanePairingConnectDecision::Authorized => {}
            mvp::control_plane::ControlPlanePairingConnectDecision::PairingRequired {
                request: pairing_request,
                created,
            } => {
                if created {
                    let _ = state.manager.record_pairing_requested(serde_json::json!({
                        "pairing_request_id": pairing_request.pairing_request_id,
                        "device_id": pairing_request.device_id,
                        "client_id": pairing_request.client_id,
                        "role": pairing_request.role,
                    }));
                }
                return pairing_required_response(&pairing_request);
            }
            mvp::control_plane::ControlPlanePairingConnectDecision::DeviceTokenRequired => {
                return device_token_error_response(
                    ControlPlaneConnectErrorCode::DeviceTokenRequired,
                    format!(
                        "device `{}` is paired but must present auth.device_token on connect",
                        device.device_id
                    ),
                );
            }
            mvp::control_plane::ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                return device_token_error_response(
                    ControlPlaneConnectErrorCode::DeviceTokenInvalid,
                    format!(
                        "device `{}` presented an invalid auth.device_token",
                        device.device_id
                    ),
                );
            }
        }
    }

    let connection_id = format!(
        "cp-{:016x}",
        state.connection_counter.fetch_add(1, Ordering::Relaxed) + 1
    );
    let granted_scopes = granted_connect_scopes(&state, &request);
    let principal = principal_from_connect(&request, connection_id.clone(), granted_scopes.clone());
    let lease = state
        .connection_registry
        .issue(connection_principal_from_connect(
            &request,
            connection_id,
            &granted_scopes,
        ));
    let scoped_capabilities = connection_scoped_capabilities(&lease);
    let agent_id = lease.principal.client_id.clone();
    let issue_result =
        state
            .kernel_authority
            .issue_scoped_token(&lease.token, &agent_id, &scoped_capabilities);
    if let Err(error) = issue_result {
        let revoked = state.connection_registry.revoke(&lease.token);
        if revoked {
            state.kernel_authority.remove_binding(&lease.token);
        }
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    let snapshot = match current_snapshot(&state).await {
        Ok(snapshot) => snapshot,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };

    let response = ControlPlaneConnectResponse {
        protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        principal,
        connection_token: lease.token,
        connection_token_expires_at_ms: lease.expires_at_ms,
        snapshot,
        policy: default_policy(),
    };
    (StatusCode::OK, Json(response)).into_response()
}
