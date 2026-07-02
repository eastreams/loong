use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use loong_protocol::{
    ControlPlaneChallengeResponse, ControlPlaneConnectErrorCode, ControlPlaneConnectRequest,
    ControlPlanePairingListResponse, ControlPlanePairingResolveRequest,
    ControlPlanePairingResolveResponse, ControlPlaneScope,
};
use serde::Deserialize;

use super::api_events::{GatewayEventsQuery, bounded_gateway_event_limit, gateway_event_stream};
use super::control::{
    GatewayControlAppState, GatewayControlJsonResponse, GatewayControlRequest,
    GatewayPairingSessionRequest, authorize_request, gateway_control_payload_response,
    gateway_pairing_protocol_principal, gateway_pairing_registry, json_connect_error,
    json_connect_error_with_request, verify_gateway_pairing_device_challenge,
};
use super::event_bus::{GatewayEventBus, GatewayEventReplayWindow};
use super::lifecycle::json_error;
use super::pairing_runtime::{
    gateway_pairing_after_seq_is_stale, gateway_pairing_event_bus,
    gateway_pairing_stale_cursor_response, persist_gateway_pairing_runtime_state,
};
use super::read_models::{
    GatewayPairingSessionLeaseReadModel, build_gateway_node_inventory_from_registry_read_model,
    build_gateway_pairing_complete_read_model, build_gateway_pairing_events_read_model,
    build_gateway_pairing_session_read_model, build_gateway_pairing_start_read_model,
};

#[derive(Debug, Default, Deserialize)]
pub(super) struct GatewayPairingListQuery {
    pub(super) status: Option<String>,
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct GatewayPairingEventsQuery {
    pub(super) after_seq: Option<u64>,
    pub(super) limit: Option<usize>,
    pub(super) ack_seq: Option<u64>,
}

pub(super) async fn handle_gateway_pairing_requests(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Query(query): Query<GatewayPairingListQuery>,
) -> GatewayControlJsonResponse {
    let request = match GatewayControlRequest::authorize(&headers, app_state.as_ref()) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let pairing_registry = match request.pairing_registry() {
        Ok(pairing_registry) => pairing_registry,
        Err(response) => return response,
    };

    let status = match query.status.as_deref() {
        Some(raw) => {
            match crate::control_plane_server::pairing_projection::parse_pairing_status(raw) {
                Ok(status) => Some(status),
                Err(error) => {
                    return json_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_pairing_status",
                        error.as_str(),
                    );
                }
            }
        }
        None => None,
    };
    let limit = query.limit.unwrap_or(50);
    let requests = pairing_registry.list_requests(status, limit);
    let payload = ControlPlanePairingListResponse {
        matched_count: requests.len(),
        returned_count: requests.len(),
        requests: requests
            .into_iter()
            .map(crate::control_plane_server::pairing_projection::map_pairing_request_summary)
            .collect::<Vec<_>>(),
    };
    gateway_control_payload_response(&payload, "gateway pairing requests payload")
}

pub(super) async fn handle_gateway_pairing_start(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(response) = GatewayControlRequest::authorize(&headers, app_state.as_ref()) {
        return response;
    }

    let challenge = app_state.challenge_registry.issue();
    let challenge = ControlPlaneChallengeResponse {
        nonce: challenge.nonce,
        issued_at_ms: challenge.issued_at_ms,
        expires_at_ms: challenge.expires_at_ms,
    };
    let payload = build_gateway_pairing_start_read_model(challenge);
    gateway_control_payload_response(&payload, "gateway pairing start payload")
}

pub(super) async fn handle_gateway_nodes(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let pairing_registry = gateway_pairing_registry(app_state.as_ref()).ok();
    let payload = build_gateway_node_inventory_from_registry_read_model(
        app_state.config_path.as_str(),
        app_state.channel_inventory.as_ref(),
        pairing_registry.as_ref(),
    );
    gateway_control_payload_response(&payload, "gateway node inventory payload")
}

pub(super) async fn handle_gateway_pairing_resolve(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<ControlPlanePairingResolveRequest>,
) -> GatewayControlJsonResponse {
    let request_context = match GatewayControlRequest::authorize(&headers, app_state.as_ref()) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let pairing_registry = match request_context.pairing_registry() {
        Ok(pairing_registry) => pairing_registry,
        Err(response) => return response,
    };

    match pairing_registry.resolve_request(request.pairing_request_id.as_str(), request.approve) {
        Ok(Some(record)) => {
            let payload = ControlPlanePairingResolveResponse {
                request:
                    crate::control_plane_server::pairing_projection::map_pairing_request_summary(
                        record.clone(),
                    ),
                device_token: record.device_token,
            };
            gateway_control_payload_response(&payload, "gateway pairing resolve payload")
        }
        Ok(None) => json_error(
            StatusCode::NOT_FOUND,
            "pairing_not_found",
            format!(
                "pairing request `{}` not found",
                request.pairing_request_id.trim()
            )
            .as_str(),
        ),
        Err(error) => json_error(
            StatusCode::BAD_REQUEST,
            "pairing_resolve_failed",
            error.as_str(),
        ),
    }
}

pub(super) async fn handle_gateway_pairing_complete(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<ControlPlaneConnectRequest>,
) -> GatewayControlJsonResponse {
    let request_context = match GatewayControlRequest::authorize(&headers, app_state.as_ref()) {
        Ok(request) => request,
        Err(response) => return response,
    };

    if request.max_protocol < loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION
        || request.min_protocol > loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION
    {
        return json_connect_error(
            StatusCode::BAD_REQUEST,
            ControlPlaneConnectErrorCode::ProtocolMismatch,
            format!(
                "protocol mismatch: expected protocol {}",
                loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION
            ),
        );
    }

    let device = match request.device.as_ref() {
        Some(device) => device,
        None => {
            return json_connect_error(
                StatusCode::BAD_REQUEST,
                ControlPlaneConnectErrorCode::ChallengeRequired,
                "gateway pairing complete requires device identity",
            );
        }
    };

    if let Err(response) = verify_gateway_pairing_device_challenge(app_state.as_ref(), &request) {
        return response;
    }

    let pairing_registry = match request_context.pairing_registry() {
        Ok(pairing_registry) => pairing_registry,
        Err(response) => return response,
    };

    let pairing_outcome = match crate::control_plane_device_auth::evaluate_pairing_connect_outcome(
        &pairing_registry,
        &request,
    ) {
        Ok(Some(outcome)) => outcome,
        Ok(None) => {
            return json_connect_error(
                StatusCode::BAD_REQUEST,
                ControlPlaneConnectErrorCode::ChallengeRequired,
                "gateway pairing complete requires device identity",
            );
        }
        Err(error) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "pairing_complete_failed",
                error.as_str(),
            );
        }
    };
    match pairing_outcome {
        crate::control_plane_device_auth::PairingConnectOutcome::Authorized => {
            let requested_scopes = request.scopes.iter().copied().collect::<Vec<_>>();
            let lease =
                super::control::issue_gateway_pairing_session_lease(app_state.as_ref(), &request);
            let _ = persist_gateway_pairing_runtime_state(app_state.as_ref());
            let payload = build_gateway_pairing_complete_read_model(
                device.device_id.as_str(),
                request.client.id.as_str(),
                request.role,
                requested_scopes,
                lease,
            );
            gateway_control_payload_response(&payload, "gateway pairing complete payload")
        }
        crate::control_plane_device_auth::PairingConnectOutcome::PairingRequired {
            request: pairing_request,
            ..
        } => json_connect_error_with_request(
            StatusCode::FORBIDDEN,
            ControlPlaneConnectErrorCode::PairingRequired,
            format!(
                "device `{}` requires operator pairing approval before connect can complete",
                pairing_request.device_id
            ),
            Some(pairing_request.pairing_request_id),
        ),
        crate::control_plane_device_auth::PairingConnectOutcome::DeviceTokenRequired => {
            json_connect_error(
                StatusCode::UNAUTHORIZED,
                ControlPlaneConnectErrorCode::DeviceTokenRequired,
                format!(
                    "device `{}` is paired but must present auth.device_token on connect",
                    device.device_id
                ),
            )
        }
        crate::control_plane_device_auth::PairingConnectOutcome::DeviceTokenInvalid => {
            json_connect_error(
                StatusCode::UNAUTHORIZED,
                ControlPlaneConnectErrorCode::DeviceTokenInvalid,
                format!(
                    "device `{}` presented an invalid auth.device_token",
                    device.device_id
                ),
            )
        }
    }
}

pub(super) async fn handle_gateway_pairing_session(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    let session = match GatewayPairingSessionRequest::authorize(
        &headers,
        app_state.as_ref(),
        ControlPlaneScope::OperatorRead,
    ) {
        Ok(session) => session,
        Err(response) => return response,
    };

    let principal = gateway_pairing_protocol_principal(session.lease());
    let replay_window = app_state
        .event_bus
        .as_ref()
        .map(GatewayEventBus::replay_window)
        .unwrap_or(GatewayEventReplayWindow {
            oldest_retained_seq: None,
            latest_seq: None,
        });
    let payload = build_gateway_pairing_session_read_model(
        GatewayPairingSessionLeaseReadModel {
            connection_token: session.lease().token.clone(),
            connection_token_expires_at_ms: session.lease().expires_at_ms,
            principal,
            last_acknowledged_seq: session.lease().acknowledged_seq,
        },
        replay_window,
    );
    gateway_control_payload_response(&payload, "gateway pairing session payload")
}

pub(super) async fn handle_gateway_pairing_events(
    headers: HeaderMap,
    Query(query): Query<GatewayPairingEventsQuery>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    let session = match GatewayPairingSessionRequest::authorize(
        &headers,
        app_state.as_ref(),
        ControlPlaneScope::OperatorRead,
    ) {
        Ok(session) => session,
        Err(response) => return response,
    };

    let event_bus = match gateway_pairing_event_bus(app_state.as_ref()) {
        Ok(event_bus) => event_bus,
        Err(response) => return response,
    };

    let after_seq = query.after_seq.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 256);
    let session = if let Some(ack_seq) = query.ack_seq {
        match session.acknowledge_seq(app_state.as_ref(), ack_seq) {
            Ok(session) => session,
            Err(response) => return response,
        }
    } else {
        session
    };
    if query.ack_seq.is_some() {
        let _ = persist_gateway_pairing_runtime_state(app_state.as_ref());
    }
    let replay_window = event_bus.replay_window();
    if gateway_pairing_after_seq_is_stale(after_seq, replay_window) {
        return gateway_pairing_stale_cursor_response(
            after_seq,
            session.lease().acknowledged_seq,
            replay_window,
        );
    }
    let events = event_bus.recent_events_after(after_seq, limit);
    let payload = build_gateway_pairing_events_read_model(
        after_seq,
        session.lease().acknowledged_seq,
        replay_window,
        events,
    );
    gateway_control_payload_response(&payload, "gateway pairing events payload")
}

pub(super) async fn handle_gateway_pairing_stream(
    headers: HeaderMap,
    Query(query): Query<GatewayEventsQuery>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> Response {
    let session = match GatewayPairingSessionRequest::authorize(
        &headers,
        app_state.as_ref(),
        ControlPlaneScope::OperatorRead,
    ) {
        Ok(session) => session,
        Err(response) => return response.into_response(),
    };

    let event_bus = match gateway_pairing_event_bus(app_state.as_ref()) {
        Ok(event_bus) => event_bus,
        Err(response) => return response.into_response(),
    };

    let after_seq = query.after_seq.unwrap_or(0);
    let replay_window = event_bus.replay_window();
    if gateway_pairing_after_seq_is_stale(after_seq, replay_window) {
        return gateway_pairing_stale_cursor_response(
            after_seq,
            session.lease().acknowledged_seq,
            replay_window,
        )
        .into_response();
    }

    let limit = bounded_gateway_event_limit(query.limit);
    let event_stream = gateway_event_stream(event_bus.clone(), query.after_seq, limit);
    axum::response::sse::Sse::new(event_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}
