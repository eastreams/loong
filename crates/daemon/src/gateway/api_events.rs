use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use super::control::{GatewayControlAppState, authorize_request_from_state};

pub(crate) async fn handle_events(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> Response {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error": error})),
        )
            .into_response();
    }

    let Some(ref event_bus) = app_state.event_bus else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "event streaming not available"})),
        )
            .into_response();
    };

    let rx = event_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(value) => Some(Ok::<_, Infallible>(
            Event::default()
                .json_data(value)
                .unwrap_or_else(|_| Event::default().data("error: serialization failed")),
        )),
        Err(_) => None, // lagged — skip
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
