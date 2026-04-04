use super::*;

pub(super) async fn chat_sessions(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<ChatSessionsPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let items = snapshot
        .sessions
        .iter()
        .map(|session| ChatSessionItemPayload {
            id: session.id.clone(),
            title: session.title.clone(),
            updated_at: format_timestamp(session.latest_turn_ts),
        })
        .collect();

    Ok(Json(ApiEnvelope {
        ok: true,
        data: ChatSessionsPayload { items },
    }))
}

pub(super) async fn create_chat_session(
    Json(payload): Json<CreateChatSessionRequest>,
) -> Json<ApiEnvelope<CreateChatSessionPayload>> {
    let session_id = payload
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(session_id_from_title)
        .unwrap_or_else(generate_session_id);

    Json(ApiEnvelope {
        ok: true,
        data: CreateChatSessionPayload { session_id },
    })
}

pub(super) async fn chat_history(
    State(state): State<Arc<WebApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiEnvelope<ChatHistoryPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let history = load_visible_session_messages(&snapshot.memory_config, &id, 128, 256)?;

    if history.is_empty() {
        return Err(WebApiError::not_found(format!(
            "session `{id}` was not found in sqlite memory"
        )));
    }

    let messages = history
        .into_iter()
        .enumerate()
        .map(|(index, turn)| ChatMessagePayload {
            id: format!("{id}:{index}"),
            role: turn.role,
            content: turn.content,
            created_at: format_timestamp(turn.ts),
        })
        .collect();

    Ok(Json(ApiEnvelope {
        ok: true,
        data: ChatHistoryPayload {
            session_id: id,
            messages,
        },
    }))
}

pub(super) async fn delete_chat_session(
    State(state): State<Arc<WebApiState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    mvp::memory::clear_session_direct(&id, &snapshot.memory_config)
        .map_err(WebApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn chat_turn(
    State(state): State<Arc<WebApiState>>,
    Path(id): Path<String>,
    Json(payload): Json<ChatTurnRequest>,
) -> Result<Json<ApiEnvelope<ChatTurnPayload>>, WebApiError> {
    let input = payload.input.trim();
    if input.is_empty() {
        return Err(WebApiError {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: "chat turn input must not be empty".to_owned(),
        });
    }

    let turn_id = generate_turn_id();
    let (sender, receiver) = mpsc::unbounded_channel();
    let now = OffsetDateTime::now_utc().unix_timestamp();

    {
        let mut streams = state.turn_streams.lock().await;
        // GC: Clear out unconsumed streams older than 60 seconds to prevent memory leaks
        streams.retain(|_, (ts, _)| now - *ts < 60);
        streams.insert(turn_id.clone(), (now, receiver));
    }

    let state_for_turn = state.clone();
    let session_id = id.clone();
    let turn_id_for_task = turn_id.clone();
    let input_owned = input.to_owned();
    tokio::spawn(async move {
        let _ = run_chat_turn_stream(
            state_for_turn,
            session_id,
            turn_id_for_task,
            input_owned,
            sender,
        )
        .await;
    });

    Ok(Json(ApiEnvelope {
        ok: true,
        data: ChatTurnPayload {
            session_id: id,
            turn_id,
            status: "accepted",
        },
    }))
}

pub(super) async fn chat_turn_stream(
    State(state): State<Arc<WebApiState>>,
    Path((_session_id, turn_id)): Path<(String, String)>,
) -> Result<Response, WebApiError> {
    let receiver = state
        .turn_streams
        .lock()
        .await
        .remove(&turn_id)
        .map(|(_, rx)| rx)
        .ok_or_else(|| WebApiError::not_found(format!("turn `{turn_id}` was not found")))?;

    let body_stream = stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .map(|line| (Ok::<String, Infallible>(format!("{line}\n")), receiver))
    });

    let mut response = Response::new(Body::from_stream(body_stream));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    Ok(response)
}
