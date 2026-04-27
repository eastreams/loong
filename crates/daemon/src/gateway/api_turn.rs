use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::control::{GatewayControlAppState, authorize_request_from_state};

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct GatewayHttpTurnRequest {
    pub session_id: String,
    pub input: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub participant_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GatewayHttpTurnResponse {
    pub output_text: String,
    pub state: String,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub event_count: usize,
}

impl GatewayHttpTurnResponse {
    fn from_agent_turn_result(result: &crate::mvp::agent_runtime::AgentTurnResult) -> Self {
        Self {
            output_text: result.output_text.clone(),
            state: result
                .state
                .clone()
                .unwrap_or_else(|| "completed".to_owned()),
            stop_reason: result.stop_reason.clone(),
            usage: result.usage.clone(),
            event_count: result.event_count,
        }
    }
}

type TurnJsonResponse = (StatusCode, Json<Value>);

/// Execute one ACP-backed agent turn through the gateway HTTP surface.
///
/// This endpoint validates the structured session/channel address first, then
/// reuses the gateway's shared ACP manager and loaded config snapshot to run a
/// single shared turn-gateway request. It is intentionally narrower than the
/// CLI chat path: the request is always executed as an ACP turn and never owns
/// long-lived interactive surface state.
pub(crate) async fn handle_turn(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<Value>,
) -> TurnJsonResponse {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": error})));
    }

    let turn_request: GatewayHttpTurnRequest = match serde_json::from_value(request) {
        Ok(req) => req,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid request: {error}")})),
            );
        }
    };

    if turn_request.input.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "input must not be empty"})),
        );
    }

    let address = match crate::build_acp_dispatch_address(
        turn_request.session_id.as_str(),
        turn_request.channel_id.as_deref(),
        turn_request.conversation_id.as_deref(),
        turn_request.account_id.as_deref(),
        turn_request.participant_id.as_deref(),
        turn_request.thread_id.as_deref(),
    ) {
        Ok(address) => address,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid turn target: {error}")})),
            );
        }
    };

    let (Some(acp_manager), Some(config)) = (&app_state.acp_manager, &app_state.config) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "ACP session manager not available"})),
        );
    };
    if !config.acp.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "ACP is disabled by policy (`acp.enabled=false`)"})),
        );
    }

    let working_directory = turn_request
        .working_directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let event_sink = app_state.event_bus.as_ref().map(|bus| bus.sink());
    let execution = crate::mvp::turn_gateway::TurnGatewayExecution {
        resolved_path: PathBuf::from(app_state.config_path.clone()),
        config: config.clone(),
        kernel_ctx: None,
        acp_manager: Some(acp_manager.clone()),
        event_sink: event_sink
            .as_ref()
            .map(|sink| sink as &dyn crate::mvp::acp::AcpTurnEventSink),
        initialize_runtime_environment: false,
    };
    let turn_request = crate::mvp::turn_gateway::TurnGatewayRequest {
        address,
        message: turn_request.input.clone(),
        metadata: turn_request.metadata.clone(),
        turn_mode: crate::mvp::agent_runtime::AgentTurnMode::Acp,
        acp: true,
        acp_event_stream: event_sink.is_some(),
        acp_bootstrap_mcp_servers: Vec::new(),
        acp_cwd: working_directory,
        live_surface_enabled: false,
        ingress: None,
        observer: None,
        provenance: crate::mvp::turn_gateway::TurnGatewayProvenance::default(),
        provider_error_mode: crate::mvp::conversation::ProviderErrorMode::InlineMessage,
        retry_progress: None,
    };
    let result = crate::mvp::turn_gateway::run_turn_gateway(execution, turn_request).await;

    match result {
        Ok(turn_result) => {
            let response = GatewayHttpTurnResponse::from_agent_turn_result(&turn_result);
            match serde_json::to_value(response) {
                Ok(value) => (StatusCode::OK, Json(value)),
                Err(error) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("response serialization failed: {error}")})),
                ),
            }
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        ),
    }
}

#[doc(hidden)]
pub fn build_turn_test_router_no_backend(bearer_token: String) -> Router {
    let app_state = Arc::new(GatewayControlAppState::test_minimal(bearer_token));
    Router::new()
        .route("/v1/turn", post(handle_turn))
        .with_state(app_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn gateway_turn_returns_service_unavailable_without_acp_backend() {
        let token = "gateway-test-token";
        let router = build_turn_test_router_no_backend(token.to_owned());
        let request = GatewayHttpTurnRequest {
            session_id: "session-1".to_owned(),
            input: "hello".to_owned(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            working_directory: None,
            metadata: BTreeMap::new(),
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/turn")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode gateway turn request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("gateway turn response");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("gateway turn response body");
        let payload: Value = serde_json::from_slice(&body).expect("gateway turn error payload");
        assert_eq!(payload["error"], "ACP session manager not available");
    }
}
