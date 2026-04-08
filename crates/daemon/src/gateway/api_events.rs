use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream::{self, Stream};
use serde::Deserialize;

use super::control::{GatewayControlAppState, authorize_request_from_state};
use super::event_bus::{GatewayEventBus, GatewayEventRecord};

const DEFAULT_GATEWAY_EVENT_LIMIT: usize = 50;
const MAX_GATEWAY_EVENT_LIMIT: usize = 256;

#[derive(Debug, Deserialize)]
pub(crate) struct GatewayEventsQuery {
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

struct GatewayEventStreamState {
    bus: GatewayEventBus,
    pending_events: VecDeque<GatewayEventRecord>,
    receiver: tokio::sync::broadcast::Receiver<GatewayEventRecord>,
    last_seq: u64,
    replay_limit: usize,
}

fn bounded_gateway_event_limit(raw_limit: Option<usize>) -> usize {
    let requested_limit = raw_limit.unwrap_or(DEFAULT_GATEWAY_EVENT_LIMIT);
    requested_limit.clamp(1, MAX_GATEWAY_EVENT_LIMIT)
}

fn initial_gateway_event_stream_state(
    bus: GatewayEventBus,
    after_seq: Option<u64>,
    limit: usize,
) -> GatewayEventStreamState {
    let receiver = bus.subscribe();
    let last_seq = after_seq.unwrap_or(0);
    let pending_events = if let Some(after_seq) = after_seq {
        let replay = bus.recent_events_after(after_seq, limit);
        VecDeque::from(replay)
    } else {
        VecDeque::new()
    };

    GatewayEventStreamState {
        bus,
        pending_events,
        receiver,
        last_seq,
        replay_limit: limit,
    }
}

fn sse_event_from_gateway_record(record: GatewayEventRecord) -> Result<Event, String> {
    let event_id = record.seq.to_string();
    let event_builder = Event::default();
    let event_builder = event_builder.id(event_id);
    event_builder
        .json_data(&record.payload)
        .map_err(|error| format!("gateway SSE event encoding failed: {error}"))
}

fn fallback_gateway_sse_error_event(message: &str) -> Event {
    let error_message = format!("{{\"error\":\"{message}\"}}");
    let base_event = Event::default();
    let named_event = base_event.event("gateway.error");
    named_event.data(error_message)
}

async fn next_gateway_sse_item(
    mut state: GatewayEventStreamState,
) -> Option<(Result<Event, Infallible>, GatewayEventStreamState)> {
    loop {
        let pending_event = state.pending_events.pop_front();
        if let Some(record) = pending_event {
            state.last_seq = record.seq;
            let sse_event_result = sse_event_from_gateway_record(record);
            let sse_event = match sse_event_result {
                Ok(event) => event,
                Err(error) => fallback_gateway_sse_error_event(error.as_str()),
            };
            return Some((Ok(sse_event), state));
        }

        let receive_result = state.receiver.recv().await;
        match receive_result {
            Ok(record) => {
                let already_seen = record.seq <= state.last_seq;
                if already_seen {
                    continue;
                }

                state.last_seq = record.seq;
                let sse_event_result = sse_event_from_gateway_record(record);
                let sse_event = match sse_event_result {
                    Ok(event) => event,
                    Err(error) => fallback_gateway_sse_error_event(error.as_str()),
                };
                return Some((Ok(sse_event), state));
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                let replay = state
                    .bus
                    .recent_events_after(state.last_seq, state.replay_limit);
                state.pending_events = VecDeque::from(replay);
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

fn gateway_event_stream(
    bus: GatewayEventBus,
    after_seq: Option<u64>,
    limit: usize,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let initial_state = initial_gateway_event_stream_state(bus, after_seq, limit);
    stream::unfold(initial_state, next_gateway_sse_item)
}

pub(crate) async fn handle_events(
    headers: HeaderMap,
    Query(query): Query<GatewayEventsQuery>,
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

    let after_seq = query.after_seq;
    let limit = bounded_gateway_event_limit(query.limit);
    let event_stream = gateway_event_stream(event_bus.clone(), after_seq, limit);

    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
