use super::*;

pub(super) async fn session_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<SessionListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "session/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "session/list", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "session/list requires runtime control-plane serve --config <path>",
            );
        };
        match repository_view.list_sessions(
            query.include_archived,
            query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT),
        ) {
            Ok(view) => Json(ControlPlaneSessionListResponse {
                current_session_id: view.current_session_id,
                matched_count: view.matched_count,
                returned_count: view.returned_count,
                sessions: view
                    .sessions
                    .into_iter()
                    .map(map_session_summary)
                    .collect::<Vec<_>>(),
            })
            .into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

pub(super) async fn session_read(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<SessionReadQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "session/read requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "session/read", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "session/read requires runtime control-plane serve --config <path>",
            );
        };
        match repository_view.read_session(
            &query.session_id,
            query
                .recent_event_limit
                .unwrap_or(CONTROL_PLANE_DEFAULT_SESSION_RECENT_LIMIT),
            query.tail_after_id,
            query
                .tail_page_limit
                .unwrap_or(CONTROL_PLANE_DEFAULT_SESSION_TAIL_LIMIT),
        ) {
            Ok(Some(observation)) => Json(ControlPlaneSessionReadResponse {
                current_session_id: repository_view.current_session_id().to_owned(),
                observation: map_session_observation(observation),
            })
            .into_response(),
            Ok(None) => error_response(
                StatusCode::NOT_FOUND,
                format!("session `{}` not found", query.session_id.trim()),
            ),
            Err(error) if error == "control_plane_session_id_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::FORBIDDEN, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

pub(super) async fn task_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TaskListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "task/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "task/list", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "task/list requires runtime control-plane serve --config <path>",
            );
        };
        let limit = query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT);
        match repository_view.list_background_tasks(query.include_archived, limit) {
            Ok(view) => {
                let tasks = view
                    .tasks
                    .into_iter()
                    .map(map_task_summary)
                    .collect::<Vec<_>>();
                let response = ControlPlaneTaskListResponse {
                    current_session_id: view.current_session_id,
                    matched_count: view.matched_count,
                    returned_count: view.returned_count,
                    tasks,
                };
                Json(response).into_response()
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

pub(super) async fn task_read(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TaskReadQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "task/read requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "task/read", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "task/read requires runtime control-plane serve --config <path>",
            );
        };
        match repository_view.read_background_task(&query.task_id) {
            Ok(Some(task_view)) => {
                let task = map_task_summary(task_view);
                let response = ControlPlaneTaskReadResponse {
                    current_session_id: repository_view.current_session_id().to_owned(),
                    task,
                };
                Json(response).into_response()
            }
            Ok(None) => error_response(
                StatusCode::NOT_FOUND,
                format!("background task `{}` not found", query.task_id.trim()),
            ),
            Err(error) if error == "control_plane_session_id_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::NOT_FOUND, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

pub(super) async fn approval_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<ApprovalListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "approval/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "approval/list", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "approval/list requires runtime control-plane serve --config <path>",
            );
        };
        let status = match query.status.as_deref() {
            Some(raw) => match parse_approval_request_status(raw) {
                Ok(status) => Some(status),
                Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
            },
            None => None,
        };
        match repository_view.list_approvals(
            query.session_id.as_deref(),
            status,
            query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT),
        ) {
            Ok(view) => Json(ControlPlaneApprovalListResponse {
                current_session_id: view.current_session_id,
                matched_count: view.matched_count,
                returned_count: view.returned_count,
                approvals: view
                    .approvals
                    .into_iter()
                    .map(map_approval_summary)
                    .collect::<Vec<_>>(),
            })
            .into_response(),
            Err(error) if error == "control_plane_session_id_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::FORBIDDEN, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

pub(super) async fn pairing_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<PairingListQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "pairing/list", &headers) {
        return *response;
    }
    let status = match query.status.as_deref() {
        Some(raw) => match parse_pairing_status(raw) {
            Ok(status) => Some(status),
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        },
        None => None,
    };
    let requests = state.pairing_registry.list_requests(
        status,
        query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT),
    );
    let matched_count = requests.len();
    let returned_count = matched_count;
    Json(ControlPlanePairingListResponse {
        matched_count,
        returned_count,
        requests: requests
            .into_iter()
            .map(map_pairing_request)
            .collect::<Vec<_>>(),
    })
    .into_response()
}

pub(super) async fn pairing_resolve(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Json(request): Json<ControlPlanePairingResolveRequest>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "pairing/resolve", &headers) {
        return *response;
    }
    match state
        .pairing_registry
        .resolve_request(&request.pairing_request_id, request.approve)
    {
        Ok(Some(record)) => {
            let _ = state.manager.record_pairing_resolved(
                serde_json::json!({
                    "pairing_request_id": record.pairing_request_id,
                    "device_id": record.device_id,
                    "status": record.status.as_str(),
                }),
                false,
            );
            Json(ControlPlanePairingResolveResponse {
                request: map_pairing_request(record.clone()),
                device_token: record.device_token,
            })
            .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            format!(
                "pairing request `{}` not found",
                request.pairing_request_id.trim()
            ),
        ),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn acp_session_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<AcpSessionListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "acp/session/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "acp/session/list", &headers)
        {
            return *response;
        }
        let Some(acp_view) = state.acp_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp/session/list requires runtime control-plane serve --config <path>",
            );
        };
        match acp_view.list_sessions(query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT)) {
            Ok(view) => Json(ControlPlaneAcpSessionListResponse {
                current_session_id: view.current_session_id,
                matched_count: view.matched_count,
                returned_count: view.returned_count,
                sessions: view
                    .sessions
                    .into_iter()
                    .map(map_acp_session_metadata)
                    .collect::<Vec<_>>(),
            })
            .into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

pub(super) async fn acp_session_read(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<AcpSessionReadQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "acp/session/read requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "acp/session/read", &headers)
        {
            return *response;
        }
        let Some(acp_view) = state.acp_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp/session/read requires runtime control-plane serve --config <path>",
            );
        };
        match acp_view.read_session(&query.session_key).await {
            Ok(Some(view)) => Json(ControlPlaneAcpSessionReadResponse {
                current_session_id: view.current_session_id,
                metadata: map_acp_session_metadata(view.metadata),
                status: map_acp_session_status(view.status),
            })
            .into_response(),
            Ok(None) => error_response(
                StatusCode::NOT_FOUND,
                format!("ACP session `{}` not found", query.session_key.trim()),
            ),
            Err(error) if error == "control_plane_acp_session_key_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::FORBIDDEN, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}
