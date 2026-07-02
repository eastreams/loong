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
        if let Err(response) =
            authorize_control_plane_request(&state, "session/list", &headers).await
        {
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
        if let Err(response) =
            authorize_control_plane_request(&state, "session/read", &headers).await
        {
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
        if let Err(response) = authorize_control_plane_request(&state, "task/list", &headers).await
        {
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
        if let Err(response) = authorize_control_plane_request(&state, "task/read", &headers).await
        {
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
        if let Err(response) =
            authorize_control_plane_request(&state, "approval/list", &headers).await
        {
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
