use super::*;

pub(super) async fn run_web_serve(
    config_path: Option<&str>,
    bind: &str,
    static_root: Option<&str>,
) -> CliResult<()> {
    let (local_token, local_token_path) = resolve_local_web_token()
        .map_err(|error| format!("initialize local web api token failed: {}", error.message))?;
    let token_path_display = local_token_path.display().to_string();
    let explicit_static_root = resolve_static_root(static_root)?;
    let resolved_static_root = if explicit_static_root.is_some() {
        explicit_static_root
    } else {
        let auto_dist = web_install_dist_dir(&default_web_install_dir());
        if auto_dist.join("index.html").is_file() {
            Some(auto_dist)
        } else {
            None
        }
    };
    let web_install_mode = if resolved_static_root.is_some() {
        "same_origin_static"
    } else {
        "api_only"
    };
    let address: SocketAddr = bind
        .parse()
        .map_err(|error| format!("invalid web bind address `{bind}`: {error}"))?;
    if web_install_mode == "same_origin_static" && !address.ip().is_loopback() {
        return Err(format!(
            "same-origin static mode only supports loopback binds, got `{bind}`"
        ));
    }
    let exact_origin = matches!(
        address.ip(),
        std::net::IpAddr::V4(_) | std::net::IpAddr::V6(_)
    )
    .then(|| format!("http://{address}"));
    let state = Arc::new(WebApiState {
        config_path: config_path.map(str::to_owned),
        local_token,
        local_token_path,
        web_install_mode,
        exact_origin,
        static_root: resolved_static_root.clone(),
        turn_streams: Mutex::new(HashMap::new()),
        debug_state: StdMutex::new(DebugConsoleRuntimeState::default()),
    });
    let public_api = Router::new()
        .route("/meta", get(meta))
        .route("/onboard/status", get(onboarding::onboard_status))
        .route(
            "/onboard/pairing/auto",
            post(onboarding::onboard_pairing_auto),
        )
        .route(
            "/onboard/pairing/clear",
            post(onboarding::onboard_pairing_clear),
        )
        .with_state(state.clone());
    let protected_api = Router::new()
        .route("/onboard/provider", post(onboarding::onboard_provider))
        .route(
            "/onboard/provider/apply",
            post(onboarding::onboard_provider_apply),
        )
        .route("/providers/catalog", get(provider_catalog))
        .route(
            "/onboard/preferences",
            post(onboarding::onboard_preferences),
        )
        .route("/onboard/validate", post(onboarding::onboard_validate))
        .route(
            "/abilities/personalization",
            get(abilities_personalization).post(abilities_personalization_save),
        )
        .route("/abilities/channels", get(abilities_channels))
        .route("/abilities/skills", get(abilities_skills))
        .route("/dashboard/summary", get(dashboard_summary))
        .route("/dashboard/providers", get(dashboard_providers))
        .route("/dashboard/runtime", get(dashboard_runtime))
        .route("/dashboard/connectivity", get(dashboard_connectivity))
        .route("/dashboard/config", get(dashboard_config))
        .route("/dashboard/tools", get(dashboard_tools))
        .route("/dashboard/approvals", get(dashboard_approvals))
        .route("/dashboard/debug-console", get(dashboard_debug_console))
        .route(
            "/chat/sessions",
            get(chat_sessions).post(create_chat_session),
        )
        .route("/chat/sessions/{id}", delete(delete_chat_session))
        .route("/chat/sessions/{id}/turn", post(chat_turn))
        .route(
            "/chat/sessions/{id}/turns/{turn_id}/stream",
            get(chat_turn_stream),
        )
        .route("/chat/sessions/{id}/history", get(chat_history))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_same_origin_write_origin,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_local_token,
        ))
        .with_state(state.clone());
    let app = Router::new()
        .route("/healthz", get(healthz))
        .nest("/api", public_api.merge(protected_api))
        .fallback(get(serve_web_static))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            local_web_cors,
        ))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("bind web api on {bind} failed: {error}"))?;

    println!("loongclaw web api listening on http://{address}");
    println!("loongclaw web api local token path: {token_path_display}");
    if let Some(static_root) = resolved_static_root.as_ref() {
        println!(
            "loongclaw web ui same-origin static root: {}",
            static_root.display()
        );
    }
    with_graceful_shutdown(async move {
        axum::serve(listener, app)
            .await
            .map_err(|error| format!("web api serve failed: {error}"))
    })
    .await
}

pub(super) async fn healthz() -> Json<ApiEnvelope<HealthPayload>> {
    Json(ApiEnvelope {
        ok: true,
        data: HealthPayload { status: "ok" },
    })
}

pub(super) async fn local_web_cors(
    State(state): State<Arc<WebApiState>>,
    request: Request,
    next: Next,
) -> Response {
    let allowed_origin = allowed_cors_origin(state.as_ref(), request.headers());
    if request.method() == Method::OPTIONS {
        return with_cors_headers(
            StatusCode::NO_CONTENT.into_response(),
            allowed_origin.as_deref(),
        );
    }

    let response = next.run(request).await;
    with_cors_headers(response, allowed_origin.as_deref())
}

fn allowed_cors_origin(state: &WebApiState, headers: &HeaderMap) -> Option<String> {
    if state.web_install_mode == "same_origin_static" {
        let origin = headers
            .get(ORIGIN)
            .and_then(|value| value.to_str().ok())
            .map(str::trim);
        return state
            .exact_origin
            .as_deref()
            .filter(|expected| origin == Some(*expected))
            .map(ToOwned::to_owned);
    }

    extract_allowed_local_origin(headers)
}

fn with_cors_headers(mut response: Response, allowed_origin: Option<&str>) -> Response {
    if let Some(origin) = allowed_origin
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        response
            .headers_mut()
            .insert(VARY, HeaderValue::from_static("Origin"));
    }
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
    );
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type, authorization, x-loongclaw-token"),
    );
    response
}

pub(super) async fn meta(State(state): State<Arc<WebApiState>>) -> Json<ApiEnvelope<MetaPayload>> {
    Json(ApiEnvelope {
        ok: true,
        data: MetaPayload {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            api_version: "v1",
            web_install_mode: state.web_install_mode,
            supported_locales: ["en", "zh-CN"],
            default_locale: "en",
            auth: MetaAuthPayload {
                required: true,
                scheme: if state.web_install_mode == "same_origin_static" {
                    "cookie"
                } else {
                    "bearer"
                },
                header: if state.web_install_mode == "same_origin_static" {
                    "Cookie"
                } else {
                    "Authorization"
                },
                token_path: if state.web_install_mode == "same_origin_static" {
                    String::new()
                } else {
                    state.local_token_path.display().to_string()
                },
                token_env: if state.web_install_mode == "same_origin_static" {
                    ""
                } else {
                    WEB_API_TOKEN_ENV
                },
                mode: if state.web_install_mode == "same_origin_static" {
                    "same_origin_session"
                } else {
                    "local_token"
                },
            },
        },
    })
}

fn resolve_static_root(static_root: Option<&str>) -> CliResult<Option<PathBuf>> {
    let Some(raw_root) = static_root else {
        return Ok(None);
    };
    let root = PathBuf::from(raw_root);
    if !root.exists() {
        return Err(format!(
            "web static root `{}` does not exist",
            root.display()
        ));
    }
    if !root.is_dir() {
        return Err(format!(
            "web static root `{}` is not a directory",
            root.display()
        ));
    }
    let index_path = root.join("index.html");
    if !index_path.is_file() {
        return Err(format!(
            "web static root `{}` is missing `index.html`",
            root.display()
        ));
    }
    Ok(Some(root))
}

pub(super) async fn serve_web_static(
    State(state): State<Arc<WebApiState>>,
    uri: Uri,
) -> Result<Response, WebApiError> {
    let Some(static_root) = state.static_root.as_ref() else {
        return Err(WebApiError::not_found("not found"));
    };

    let request_path = uri.path();
    let candidate = match resolve_static_asset_path(static_root, request_path) {
        Some(path) => path,
        None => return Err(WebApiError::not_found("not found")),
    };
    let effective_path = if candidate.is_file() {
        candidate
    } else if is_asset_like_path(request_path) {
        return Err(WebApiError::not_found("not found"));
    } else {
        static_root.join("index.html")
    };
    let bytes = tokio::fs::read(&effective_path).await.map_err(|error| {
        WebApiError::internal(format!(
            "read web static asset `{}` failed: {error}",
            effective_path.display()
        ))
    })?;
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, detect_static_content_type(&effective_path))
        .body(Body::from(bytes))
        .map_err(|error| WebApiError::internal(format!("build static response failed: {error}")))?;
    if state.web_install_mode == "same_origin_static" {
        response.headers_mut().append(
            SET_COOKIE,
            build_same_origin_session_cookie(state.local_token.as_str())?,
        );
    }
    Ok(response)
}

fn resolve_static_asset_path(static_root: &FsPath, request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some(static_root.join("index.html"));
    }

    let relative = FsPath::new(trimmed);
    let mut resolved = static_root.to_path_buf();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(segment) => resolved.push(segment),
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::CurDir
            | std::path::Component::ParentDir => return None,
        }
    }
    if resolved.is_dir() {
        Some(resolved.join("index.html"))
    } else {
        Some(resolved)
    }
}

fn is_asset_like_path(request_path: &str) -> bool {
    FsPath::new(request_path.trim_start_matches('/'))
        .extension()
        .is_some()
}

fn detect_static_content_type(path: &FsPath) -> &'static str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json; charset=utf-8",
        Some("webmanifest") => "application/manifest+json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}
