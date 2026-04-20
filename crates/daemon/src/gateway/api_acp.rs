use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::CliResult;
use crate::build_acp_dispatch_address;

use super::control::{
    GatewayControlAppState, authorize_request_from_state, is_gateway_acp_not_found_error,
};
use super::read_models::{
    build_acp_dispatch_read_model, build_acp_observability_read_model, build_acp_status_read_model,
};

type GatewayAcpJsonResponse = (StatusCode, Json<Value>);

#[derive(Debug, Deserialize)]
pub(crate) struct GatewayAcpSessionQuery {
    pub(crate) session_id: String,
    #[serde(default)]
    pub(crate) channel_id: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<String>,
    #[serde(default)]
    pub(crate) conversation_id: Option<String>,
    #[serde(default)]
    pub(crate) participant_id: Option<String>,
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
}

pub(crate) async fn handle_acp_status(
    headers: HeaderMap,
    Query(query): Query<GatewayAcpSessionQuery>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayAcpJsonResponse {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return json_error(StatusCode::UNAUTHORIZED, error.as_str());
    }

    let acp_context = gateway_acp_runtime_context(app_state.as_ref());
    let (config, acp_manager) = match acp_context {
        Ok(context) => context,
        Err(response) => return response,
    };

    let address_result = build_query_address(&query);
    let address = match address_result {
        Ok(address) => address,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.as_str()),
    };

    let route_result = crate::mvp::acp::derive_acp_conversation_route_for_address(config, &address);
    let route = match route_result {
        Ok(route) => route,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.as_str()),
    };

    let status_result = acp_manager
        .get_status(config, route.session_key.as_str())
        .await;
    let status = match status_result {
        Ok(status) => status,
        Err(error) if is_gateway_acp_not_found_error(error.as_str()) => {
            let message = format!("ACP session `{}` is not registered", route.session_key);
            return json_error(StatusCode::NOT_FOUND, message.as_str());
        }
        Err(error) => {
            return internal_server_error("load gateway ACP status", error.as_str());
        }
    };

    let requested_route_session_id = route
        .binding
        .as_ref()
        .map(|binding| binding.route_session_id.as_str());
    let payload = build_acp_status_read_model(
        config_path_from_state(app_state.as_ref()),
        Some(query.session_id.as_str()),
        query.conversation_id.as_deref(),
        requested_route_session_id,
        route.session_key.as_str(),
        &status,
    );

    serialize_ok_json(&payload, "gateway ACP status payload")
}

pub(crate) async fn handle_acp_observability(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayAcpJsonResponse {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return json_error(StatusCode::UNAUTHORIZED, error.as_str());
    }

    let acp_context = gateway_acp_runtime_context(app_state.as_ref());
    let (config, acp_manager) = match acp_context {
        Ok(context) => context,
        Err(response) => return response,
    };

    let snapshot_result = acp_manager.observability_snapshot(config).await;
    let snapshot = match snapshot_result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return internal_server_error("load gateway ACP observability", error.as_str());
        }
    };

    let payload =
        build_acp_observability_read_model(config_path_from_state(app_state.as_ref()), &snapshot);

    serialize_ok_json(&payload, "gateway ACP observability payload")
}

pub(crate) async fn handle_acp_dispatch(
    headers: HeaderMap,
    Query(query): Query<GatewayAcpSessionQuery>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayAcpJsonResponse {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return json_error(StatusCode::UNAUTHORIZED, error.as_str());
    }

    let acp_context = gateway_acp_runtime_context(app_state.as_ref());
    let (config, _acp_manager) = match acp_context {
        Ok(context) => context,
        Err(response) => return response,
    };

    let address_result = build_query_address(&query);
    let address = match address_result {
        Ok(address) => address,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.as_str()),
    };

    let decision_result =
        crate::mvp::acp::evaluate_acp_conversation_dispatch_for_address(config, &address);
    let decision = match decision_result {
        Ok(decision) => decision,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.as_str()),
    };

    let payload = build_acp_dispatch_read_model(
        config_path_from_state(app_state.as_ref()),
        &address,
        query.session_id.as_str(),
        &decision,
    );

    serialize_ok_json(&payload, "gateway ACP dispatch payload")
}

fn gateway_acp_runtime_context(
    app_state: &GatewayControlAppState,
) -> Result<
    (
        &crate::mvp::config::LoongConfig,
        &crate::mvp::acp::AcpSessionManager,
    ),
    GatewayAcpJsonResponse,
> {
    let config = match app_state.config.as_ref() {
        Some(config) => config,
        None => {
            return Err(json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "gateway config is unavailable",
            ));
        }
    };

    let acp_manager = match app_state.acp_manager.as_ref() {
        Some(acp_manager) => acp_manager,
        None => {
            return Err(json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "ACP session manager not available",
            ));
        }
    };

    if !config.acp.enabled {
        return Err(json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "ACP is disabled by policy (`acp.enabled=false`)",
        ));
    }

    Ok((config, acp_manager))
}

fn build_query_address(
    query: &GatewayAcpSessionQuery,
) -> CliResult<crate::mvp::conversation::ConversationSessionAddress> {
    let session_id = query.session_id.as_str();
    let channel_id = query.channel_id.as_deref();
    let conversation_id = query.conversation_id.as_deref();
    let account_id = query.account_id.as_deref();
    let participant_id = query.participant_id.as_deref();
    let thread_id = query.thread_id.as_deref();

    build_acp_dispatch_address(
        session_id,
        channel_id,
        conversation_id,
        account_id,
        participant_id,
        thread_id,
    )
}

fn config_path_from_state(app_state: &GatewayControlAppState) -> &str {
    app_state.config_path.as_str()
}

fn serialize_ok_json<T>(value: &T, context: &str) -> GatewayAcpJsonResponse
where
    T: Serialize,
{
    let payload_result = serde_json::to_value(value);
    match payload_result {
        Ok(payload) => (StatusCode::OK, Json(payload)),
        Err(error) => {
            let error_message = error.to_string();
            internal_server_error(context, error_message.as_str())
        }
    }
}

fn json_error(status_code: StatusCode, message: &str) -> GatewayAcpJsonResponse {
    let error_code = gateway_acp_error_code(status_code);
    let payload = serde_json::json!({
        "error": {
            "code": error_code,
            "message": message,
        },
    });
    (status_code, Json(payload))
}

fn gateway_acp_error_code(status_code: StatusCode) -> &'static str {
    match status_code {
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
        StatusCode::INTERNAL_SERVER_ERROR => "internal_server_error",
        _ => "unknown_error",
    }
}

fn internal_server_error(context: &str, error: &str) -> GatewayAcpJsonResponse {
    tracing::error!(
        target: "loong.gateway",
        context = context,
        error = %error,
        "gateway ACP request failed"
    );
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}
