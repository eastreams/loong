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
pub(crate) struct GatewayTurnRequest {
    pub session_id: String,
    pub input: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GatewayTurnResponse {
    pub output_text: String,
    pub state: String,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub event_count: usize,
}

impl GatewayTurnResponse {
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

pub(crate) async fn handle_turn(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<Value>,
) -> TurnJsonResponse {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": error})));
    }

    let turn_request: GatewayTurnRequest = match serde_json::from_value(request) {
        Ok(req) => req,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid request: {error}")})),
            );
        }
    };

    if turn_request.input.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "input must not be empty"})),
        );
    }

    if let Err(error) = crate::build_acp_dispatch_address(
        turn_request.session_id.as_str(),
        turn_request.channel_id.as_deref(),
        turn_request.conversation_id.as_deref(),
        turn_request.account_id.as_deref(),
        turn_request.thread_id.as_deref(),
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("invalid turn target: {error}")})),
        );
    }

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

    let event_sink = app_state.event_bus.as_ref().map(|bus| bus.sink());
    let result = crate::mvp::agent_runtime::AgentRuntime::new()
        .run_turn_with_loaded_config_and_acp_manager(
            PathBuf::from(app_state.config_path.clone()),
            config.clone(),
            Some(turn_request.session_id.as_str()),
            &crate::mvp::agent_runtime::AgentTurnRequest {
                message: turn_request.input.clone(),
                turn_mode: crate::mvp::agent_runtime::AgentTurnMode::Acp,
                channel_id: turn_request.channel_id.clone(),
                account_id: turn_request.account_id.clone(),
                conversation_id: turn_request.conversation_id.clone(),
                thread_id: turn_request.thread_id.clone(),
                metadata: turn_request.metadata.clone(),
                acp: true,
                acp_event_stream: event_sink.is_some(),
                acp_bootstrap_mcp_servers: Vec::new(),
                acp_cwd: turn_request.working_directory.clone(),
                live_surface_enabled: false,
            },
            event_sink
                .as_ref()
                .map(|sink| sink as &dyn crate::mvp::acp::AcpTurnEventSink),
            acp_manager.clone(),
        )
        .await;

    match result {
        Ok(turn_result) => {
            let response = GatewayTurnResponse::from_agent_turn_result(&turn_result);
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
        let request = GatewayTurnRequest {
            session_id: "session-1".to_owned(),
            input: "hello".to_owned(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
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
