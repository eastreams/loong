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

use crate::mvp::acp::{AcpConversationTurnOptions, AcpTurnResult, AcpTurnStopReason};

use super::control::{GatewayControlAppState, authorize_request_from_state};

#[derive(Debug, Deserialize)]
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
    fn from_acp_result(result: &AcpTurnResult) -> Self {
        let state = crate::acp_session_state_label(result.state).to_owned();
        Self {
            output_text: result.output_text.clone(),
            state,
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

fn extend_turn_metadata(target: &mut BTreeMap<String, String>, extra: &BTreeMap<String, String>) {
    for (key, value) in extra {
        let normalized_key = key.trim();
        let normalized_value = value.trim();
        if normalized_key.is_empty() {
            continue;
        }
        if normalized_value.is_empty() {
            continue;
        }

        let normalized_key = normalized_key.to_owned();
        let normalized_value = normalized_value.to_owned();
        target.entry(normalized_key).or_insert(normalized_value);
    }
}

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

    let turn_address = match crate::build_acp_dispatch_address(
        turn_request.session_id.as_str(),
        turn_request.channel_id.as_deref(),
        turn_request.conversation_id.as_deref(),
        turn_request.account_id.as_deref(),
        turn_request.thread_id.as_deref(),
    ) {
        Ok(turn_address) => turn_address,
        Err(error) => {
            let error_message = format!("invalid turn target: {error}");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": error_message})),
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

    let working_directory = turn_request.working_directory.as_deref().map(PathBuf::from);
    let turn_options = AcpConversationTurnOptions::explicit();
    let turn_options = turn_options.with_working_directory(working_directory.as_deref());
    let prepared_turn = crate::mvp::acp::prepare_acp_conversation_turn_for_address(
        config,
        &turn_address,
        turn_request.input.as_str(),
        &turn_options,
    );
    let prepared_turn = match prepared_turn {
        Ok(prepared_turn) => prepared_turn,
        Err(error) => {
            let error_message = format!("unable to prepare turn target: {error}");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": error_message})),
            );
        }
    };
    let mut bootstrap = prepared_turn.bootstrap;
    let mut acp_request = prepared_turn.request;
    extend_turn_metadata(&mut bootstrap.metadata, &turn_request.metadata);
    extend_turn_metadata(&mut acp_request.metadata, &turn_request.metadata);

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
            match serde_json::to_value(response) {
                Ok(value) => (StatusCode::OK, Json(value)),
                Err(error) => {
                    let error_message = format!("response serialization failed: {error}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": error_message})),
                    )
                }
            }
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
