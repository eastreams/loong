use axum::{Json, http::StatusCode};
use serde_json::{Value, json};

pub(crate) type HealthResponse = (StatusCode, Json<Value>);

pub(crate) async fn handle_health() -> HealthResponse {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}
