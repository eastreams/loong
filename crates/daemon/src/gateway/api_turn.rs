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

use crate::mvp::acp::{AcpSessionBootstrap, AcpTurnRequest, AcpTurnResult, AcpTurnStopReason};

use super::control::{GatewayControlAppState, authorize_request_from_state};

#[derive(Debug, Deserialize)]
pub(crate) struct GatewayTurnRequest {
    pub session_key: String,
    pub input: String,
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
    fn from_acp_result(result: &AcpTurnResult) -> Self {
        Self {
            output_text: result.output_text.clone(),
            state: serde_json::to_value(result.state)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "unknown".to_string()),
            stop_reason: result.stop_reason.as_ref().map(|r| match r {
                AcpTurnStopReason::Completed => "completed".to_string(),
                AcpTurnStopReason::Cancelled => "cancelled".to_string(),
            }),
            usage: result.usage.clone(),
            event_count: result.events.len(),
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

    let bootstrap = AcpSessionBootstrap {
        session_key: turn_request.session_key.clone(),
        conversation_id: None,
        binding: None,
        working_directory: turn_request.working_directory.as_deref().map(PathBuf::from),
        initial_prompt: None,
        mode: None,
        mcp_servers: vec![],
        metadata: turn_request.metadata.clone(),
    };

    let acp_request = AcpTurnRequest {
        session_key: turn_request.session_key,
        input: turn_request.input,
        working_directory: turn_request.working_directory.as_deref().map(PathBuf::from),
        metadata: turn_request.metadata,
    };

    let event_sink = app_state.event_bus.as_ref().map(|bus| bus.sink());
    let sink_ref = event_sink
        .as_ref()
        .map(|s| s as &dyn crate::mvp::acp::AcpTurnEventSink);

    let result = acp_manager
        .run_turn_with_sink(config, &bootstrap, &acp_request, sink_ref)
        .await;

    match result {
        Ok(turn_result) => {
            let response = GatewayTurnResponse::from_acp_result(&turn_result);
            (
                StatusCode::OK,
                Json(serde_json::to_value(response).unwrap_or(json!({}))),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        ),
    }
}

/// Test-only router with no ACP backend (returns 503 for valid turn requests).
#[doc(hidden)]
pub fn build_turn_test_router_no_backend(bearer_token: String) -> Router {
    let app_state = Arc::new(GatewayControlAppState::test_minimal(bearer_token));
    Router::new()
        .route("/v1/turn", post(handle_turn))
        .with_state(app_state)
}
