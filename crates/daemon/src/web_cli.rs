use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    env, fs,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
};

use ::time::{OffsetDateTime, format_description::well_known::Rfc3339};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Request, State},
    http::{
        HeaderMap, HeaderValue, Method, StatusCode, Uri,
        header::{
            ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS,
            ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN, AUTHORIZATION, CONTENT_TYPE,
            COOKIE, ORIGIN, SET_COOKIE, VARY,
        },
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use clap::Subcommand;
use futures_util::stream;
use rand::random;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, mpsc},
    time::{self, Duration},
};

use crate::{CliResult, mvp, with_graceful_shutdown};

mod auth;
mod debug_console;
mod onboarding;

use auth::{
    build_clear_pairing_cookie, build_clear_same_origin_session_cookie, build_pairing_cookie,
    build_same_origin_session_cookie, extract_allowed_local_origin, extract_request_token,
    request_is_authenticated, require_local_token, require_same_origin_write_origin,
};
use debug_console::{dashboard_debug_console, record_debug_operation};

#[derive(Subcommand, Debug)]
pub enum WebCommand {
    /// Serve the local Web Console API surface
    Serve {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value = "127.0.0.1:4317")]
        bind: String,
        /// Path to the built frontend assets. If omitted, uses installed assets
        /// from `web install` when available, otherwise runs in API-only mode.
        #[arg(long)]
        static_root: Option<String>,
    },
    /// Install the Web Console UI assets to ~/.loongclaw/web
    Install {
        /// Path to the built frontend assets directory (e.g. web/dist)
        #[arg(long)]
        source: String,
    },
    /// Show Web Console installation status
    Status,
    /// Remove the installed Web Console UI assets
    Remove {
        /// Skip the confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct WebInstallManifest {
    installed_at: String,
    source_path: String,
    install_dir: String,
}

const WEB_API_TOKEN_ENV: &str = "LOONGCLAW_WEB_TOKEN";
const WEB_API_TOKEN_FILE: &str = "web-api-token";
const WEB_API_PAIRING_COOKIE: &str = "loongclaw-web-pair";
const WEB_API_SESSION_COOKIE: &str = "loongclaw-web-session";

#[derive(Debug)]
struct WebApiState {
    config_path: Option<String>,
    local_token: String,
    local_token_path: PathBuf,
    web_install_mode: &'static str,
    exact_origin: Option<String>,
    static_root: Option<PathBuf>,
    turn_streams: Mutex<HashMap<String, mpsc::UnboundedReceiver<String>>>,
    debug_state: StdMutex<DebugConsoleRuntimeState>,
}

struct WebTurnEventSink {
    state: Arc<WebApiState>,
    turn_id: String,
    sender: mpsc::UnboundedSender<String>,
    emitted_text: Arc<AtomicBool>,
}

#[derive(Debug, Default, Clone)]
struct DebugConsoleRuntimeState {
    recent_blocks: Vec<DebugConsoleBlock>,
}

#[derive(Debug, Clone)]
struct DebugConsoleBlock {
    id: String,
    kind: &'static str,
    header: String,
    started_at: String,
    lines: Vec<String>,
    tool_calls: usize,
    delta_chunks: usize,
    delta_chars: usize,
}

impl DebugConsoleBlock {
    fn operation(id: String, kind: &'static str, header: String) -> Self {
        let started_at = format_timestamp(OffsetDateTime::now_utc().unix_timestamp());
        Self {
            id,
            kind,
            header,
            started_at,
            lines: Vec::new(),
            tool_calls: 0,
            delta_chunks: 0,
            delta_chars: 0,
        }
    }
}

impl mvp::acp::AcpTurnEventSink for WebTurnEventSink {
    fn on_event(&self, event: &Value) -> CliResult<()> {
        let Some(delta) = extract_stream_text_delta(event) else {
            return Ok(());
        };
        if delta.is_empty() {
            return Ok(());
        }
        self.emitted_text.store(true, Ordering::Relaxed);
        send_stream_event(
            &self.sender,
            json!({
                "type": "message.delta",
                "turnId": self.turn_id,
                "role": "assistant",
                "delta": delta,
            }),
        )
        .map_err(|error| error.message)?;
        record_message_delta(&self.state, &self.turn_id, delta.as_str());
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct ApiEnvelope<T> {
    ok: bool,
    data: T,
}

#[derive(Debug, Serialize)]
struct ApiErrorEnvelope {
    ok: bool,
    error: ApiErrorPayload,
}

#[derive(Debug, Serialize)]
struct ApiErrorPayload {
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct HealthPayload {
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetaPayload {
    app_version: String,
    api_version: &'static str,
    web_install_mode: &'static str,
    supported_locales: [&'static str; 2],
    default_locale: &'static str,
    auth: MetaAuthPayload,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetaAuthPayload {
    required: bool,
    scheme: &'static str,
    header: &'static str,
    token_path: String,
    token_env: &'static str,
    mode: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardSummaryPayload {
    runtime_status: &'static str,
    active_provider: Option<String>,
    active_model: String,
    memory_backend: &'static str,
    session_count: usize,
    web_install_mode: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardProvidersPayload {
    active_provider: Option<String>,
    items: Vec<ProviderItemPayload>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardRuntimePayload {
    status: &'static str,
    source: &'static str,
    config_path: String,
    memory_backend: &'static str,
    memory_mode: &'static str,
    ingest_mode: &'static str,
    web_install_mode: &'static str,
    active_provider: Option<String>,
    active_model: String,
    acp_enabled: bool,
    strict_memory: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardConnectivityPayload {
    status: &'static str,
    endpoint: String,
    host: String,
    dns_addresses: Vec<String>,
    probe_status: &'static str,
    probe_status_code: Option<u16>,
    fake_ip_detected: bool,
    proxy_env_detected: bool,
    recommendation: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardConfigPayload {
    active_provider: Option<String>,
    last_provider: Option<String>,
    model: String,
    endpoint: String,
    api_key_configured: bool,
    api_key_masked: Option<String>,
    personality: String,
    prompt_mode: &'static str,
    prompt_addendum_configured: bool,
    memory_profile: String,
    memory_system: &'static str,
    sqlite_path: String,
    file_root: String,
    sliding_window: usize,
    summary_max_chars: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardToolsPayload {
    approval_mode: String,
    shell_default_mode: String,
    shell_allow_count: usize,
    shell_deny_count: usize,
    items: Vec<DashboardToolItemPayload>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardToolItemPayload {
    id: &'static str,
    enabled: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderItemPayload {
    id: String,
    label: String,
    enabled: bool,
    model: String,
    endpoint: String,
    api_key_configured: bool,
    api_key_masked: Option<String>,
    default_for_kind: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatSessionsPayload {
    items: Vec<ChatSessionItemPayload>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatSessionItemPayload {
    id: String,
    title: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatHistoryPayload {
    session_id: String,
    messages: Vec<ChatMessagePayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateChatSessionRequest {
    title: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateChatSessionPayload {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatTurnRequest {
    input: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatTurnPayload {
    session_id: String,
    turn_id: String,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatMessagePayload {
    id: String,
    role: String,
    content: String,
    created_at: String,
}

#[derive(Debug)]
struct StreamToolEvent {
    tool_id: String,
    label: String,
    outcome: Option<&'static str>,
}

#[derive(Debug)]
struct WebApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl WebApiError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }
}

impl IntoResponse for WebApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorEnvelope {
                ok: false,
                error: ApiErrorPayload {
                    code: self.code,
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

pub async fn run_web_command(command: WebCommand) -> CliResult<()> {
    match command {
        WebCommand::Serve {
            config,
            bind,
            static_root,
        } => run_web_serve(config.as_deref(), &bind, static_root.as_deref()).await,
        WebCommand::Install { source } => run_web_install(&source),
        WebCommand::Status => run_web_status(),
        WebCommand::Remove { force } => run_web_remove(force),
    }
}

async fn run_web_serve(
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
        .route(
            "/onboard/preferences",
            post(onboarding::onboard_preferences),
        )
        .route("/onboard/validate", post(onboarding::onboard_validate))
        .route("/dashboard/summary", get(dashboard_summary))
        .route("/dashboard/providers", get(dashboard_providers))
        .route("/dashboard/runtime", get(dashboard_runtime))
        .route("/dashboard/connectivity", get(dashboard_connectivity))
        .route("/dashboard/config", get(dashboard_config))
        .route("/dashboard/tools", get(dashboard_tools))
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
        .layer(middleware::from_fn(local_web_cors))
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

async fn healthz() -> Json<ApiEnvelope<HealthPayload>> {
    Json(ApiEnvelope {
        ok: true,
        data: HealthPayload { status: "ok" },
    })
}

async fn local_web_cors(request: Request, next: Next) -> Response {
    let allowed_origin = extract_allowed_local_origin(request.headers());
    if request.method() == Method::OPTIONS {
        return with_cors_headers(
            StatusCode::NO_CONTENT.into_response(),
            allowed_origin.as_deref(),
        );
    }

    let response = next.run(request).await;
    with_cors_headers(response, allowed_origin.as_deref())
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

async fn meta(State(state): State<Arc<WebApiState>>) -> Json<ApiEnvelope<MetaPayload>> {
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

async fn dashboard_summary(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardSummaryPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardSummaryPayload {
            runtime_status: "ready",
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            active_model: snapshot.config.provider.model.clone(),
            memory_backend: "sqlite",
            session_count: snapshot.sessions.len(),
            web_install_mode: state.web_install_mode,
        },
    }))
}

async fn dashboard_providers(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardProvidersPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardProvidersPayload {
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            items: build_provider_items(&snapshot.config),
        },
    }))
}

async fn dashboard_runtime(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardRuntimePayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardRuntimePayload {
            status: "ready",
            source: "local_daemon",
            config_path: snapshot.resolved_path.display().to_string(),
            memory_backend: snapshot.config.memory.resolved_backend().as_str(),
            memory_mode: snapshot.config.memory.resolved_mode().as_str(),
            ingest_mode: snapshot.config.memory.ingest_mode.as_str(),
            web_install_mode: state.web_install_mode,
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            active_model: snapshot.config.provider.model.clone(),
            acp_enabled: snapshot.config.acp.enabled,
            strict_memory: !snapshot.config.memory.effective_fail_open(),
        },
    }))
}

async fn dashboard_connectivity(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardConnectivityPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let endpoint = snapshot.config.provider.endpoint();
    let parsed = reqwest::Url::parse(&endpoint).map_err(|error| {
        WebApiError::internal(format!(
            "parse provider endpoint for connectivity failed: {error}"
        ))
    })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| WebApiError::internal("provider endpoint host was missing"))?
        .to_owned();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let dns_addresses = resolve_provider_host_addresses(host.as_str(), port).await;
    let fake_ip_detected = dns_addresses
        .iter()
        .any(|address| is_fake_ip_address(address));
    let proxy_env_detected = has_proxy_environment();
    let (probe_status, probe_status_code) = probe_provider_endpoint(endpoint.as_str()).await;
    let degraded = fake_ip_detected || probe_status != "reachable";
    let recommendation = if fake_ip_detected {
        Some("direct_host_and_fake_ip_filter")
    } else if probe_status != "reachable" {
        Some("check_network_route")
    } else {
        None
    };

    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardConnectivityPayload {
            status: if degraded { "degraded" } else { "healthy" },
            endpoint,
            host,
            dns_addresses,
            probe_status,
            probe_status_code,
            fake_ip_detected,
            proxy_env_detected,
            recommendation,
        },
    }))
}

async fn dashboard_config(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardConfigPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let active_provider = build_provider_items(&snapshot.config)
        .into_iter()
        .find(|item| item.enabled);

    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardConfigPayload {
            active_provider: snapshot.config.active_provider_id().map(str::to_owned),
            last_provider: snapshot.config.last_provider.clone(),
            model: snapshot.config.provider.model.clone(),
            endpoint: snapshot.config.provider.endpoint(),
            api_key_configured: active_provider
                .as_ref()
                .map(|item| item.api_key_configured)
                .unwrap_or(false),
            api_key_masked: active_provider.and_then(|item| item.api_key_masked),
            personality: prompt_personality_id(snapshot.config.cli.resolved_personality())
                .to_owned(),
            prompt_mode: if snapshot.config.cli.uses_native_prompt_pack() {
                "native_prompt_pack"
            } else {
                "inline_prompt"
            },
            prompt_addendum_configured: snapshot
                .config
                .cli
                .system_prompt_addendum
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty()),
            memory_profile: snapshot
                .config
                .memory
                .resolved_profile()
                .as_str()
                .to_owned(),
            memory_system: snapshot.config.memory.resolved_system().as_str(),
            sqlite_path: snapshot
                .config
                .memory
                .resolved_sqlite_path()
                .display()
                .to_string(),
            file_root: snapshot
                .config
                .tools
                .resolved_file_root()
                .display()
                .to_string(),
            sliding_window: snapshot.config.memory.sliding_window,
            summary_max_chars: snapshot.config.memory.summary_max_chars,
        },
    }))
}

async fn dashboard_tools(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<DashboardToolsPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        &snapshot.config,
        None,
    );
    Ok(Json(ApiEnvelope {
        ok: true,
        data: DashboardToolsPayload {
            approval_mode: approval_mode_label(snapshot.config.tools.approval.mode).to_owned(),
            shell_default_mode: snapshot.config.tools.shell_default_mode.clone(),
            shell_allow_count: snapshot.config.tools.shell_allow.len(),
            shell_deny_count: snapshot.config.tools.shell_deny.len(),
            items: build_tool_items(&snapshot.config, &tool_runtime),
        },
    }))
}

async fn chat_sessions(
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

async fn create_chat_session(
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

async fn chat_history(
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

async fn delete_chat_session(
    State(state): State<Arc<WebApiState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    mvp::memory::clear_session_direct(&id, &snapshot.memory_config)
        .map_err(WebApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn chat_turn(
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

    state
        .turn_streams
        .lock()
        .await
        .insert(turn_id.clone(), receiver);

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

async fn chat_turn_stream(
    State(state): State<Arc<WebApiState>>,
    Path((_session_id, turn_id)): Path<(String, String)>,
) -> Result<Response, WebApiError> {
    let receiver = state
        .turn_streams
        .lock()
        .await
        .remove(&turn_id)
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

struct WebSnapshot {
    resolved_path: PathBuf,
    config: mvp::config::LoongClawConfig,
    memory_config: mvp::memory::runtime_config::MemoryRuntimeConfig,
    sessions: Vec<WebSessionSummary>,
}

struct WebSessionSummary {
    id: String,
    title: String,
    latest_turn_ts: i64,
}

fn load_web_snapshot(state: &WebApiState) -> Result<WebSnapshot, WebApiError> {
    let (resolved_path, config) =
        mvp::config::load(state.config_path.as_deref()).map_err(WebApiError::internal)?;
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let sessions = list_sessions(&memory_config)?;

    Ok(WebSnapshot {
        resolved_path,
        config,
        memory_config,
        sessions,
    })
}

fn list_sessions(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
) -> Result<Vec<WebSessionSummary>, WebApiError> {
    let sessions = mvp::memory::list_recent_sessions_direct(24, memory_config)
        .map_err(WebApiError::internal)?;

    sessions
        .into_iter()
        .map(|session| {
            let title = load_session_messages(memory_config, &session.session_id)
                .ok()
                .and_then(|messages| derive_session_title(&messages))
                .unwrap_or_else(|| session.session_id.clone());

            Ok(WebSessionSummary {
                id: session.session_id,
                title,
                latest_turn_ts: session.latest_turn_ts,
            })
        })
        .collect()
}

fn load_session_messages(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    session_id: &str,
) -> Result<Vec<mvp::memory::ConversationTurn>, WebApiError> {
    mvp::memory::window_direct(session_id, 64, memory_config).map_err(WebApiError::internal)
}

fn load_visible_session_messages(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    session_id: &str,
    visible_limit: usize,
    raw_limit: usize,
) -> Result<Vec<mvp::memory::ConversationTurn>, WebApiError> {
    let mut turns = mvp::memory::window_direct(session_id, raw_limit, memory_config)
        .map_err(WebApiError::internal)?;
    turns.retain(|turn| {
        !(turn.role.eq_ignore_ascii_case("assistant")
            && is_internal_assistant_record(&turn.content))
    });

    if turns.len() > visible_limit {
        let start = turns.len() - visible_limit;
        Ok(turns.split_off(start))
    } else {
        Ok(turns)
    }
}

fn build_tool_items(
    config: &mvp::config::LoongClawConfig,
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Vec<DashboardToolItemPayload> {
    vec![
        DashboardToolItemPayload {
            id: "shell_policy",
            enabled: true,
            detail: format!(
                "{} default, {} allow / {} deny",
                config.tools.shell_default_mode,
                config.tools.shell_allow.len(),
                config.tools.shell_deny.len()
            ),
        },
        DashboardToolItemPayload {
            id: "sessions",
            enabled: config.tools.sessions.enabled,
            detail: format!(
                "{} visibility, list {} / history {}",
                match config.tools.sessions.visibility {
                    mvp::config::SessionVisibility::SelfOnly => "self",
                    mvp::config::SessionVisibility::Children => "children",
                },
                config.tools.sessions.list_limit,
                config.tools.sessions.history_limit
            ),
        },
        DashboardToolItemPayload {
            id: "messages",
            enabled: config.tools.messages.enabled,
            detail: "message tool surface".to_owned(),
        },
        DashboardToolItemPayload {
            id: "delegate",
            enabled: config.tools.delegate.enabled,
            detail: format!(
                "depth {}, active children {}",
                config.tools.delegate.max_depth, config.tools.delegate.max_active_children
            ),
        },
        DashboardToolItemPayload {
            id: "browser",
            enabled: config.tools.browser.enabled,
            detail: format!(
                "{} sessions, {} links, {} chars",
                config.tools.browser.max_sessions,
                config.tools.browser.max_links,
                config.tools.browser.max_text_chars
            ),
        },
        DashboardToolItemPayload {
            id: "browser_companion",
            enabled: config.tools.browser_companion.enabled,
            // Prefer runtime-ready signals here so the dashboard reflects whether
            // the companion can actually be used right now, not just how it is configured.
            detail: format!(
                "{}, {}, {}s timeout",
                if runtime.browser_companion.is_runtime_ready() {
                    "ready"
                } else {
                    "not ready"
                },
                if runtime.browser_companion.command.is_some() {
                    "command configured"
                } else {
                    "no command"
                },
                runtime.browser_companion.timeout_seconds
            ),
        },
        DashboardToolItemPayload {
            id: "web_fetch",
            enabled: config.tools.web.enabled,
            detail: format!(
                "{}s timeout, {} bytes, {} redirects",
                config.tools.web.timeout_seconds,
                config.tools.web.max_bytes,
                config.tools.web.max_redirects
            ),
        },
        DashboardToolItemPayload {
            id: "web_search",
            enabled: config.tools.web_search.enabled,
            detail: format!(
                "{} provider, {}s timeout, {} results",
                runtime.web_search.default_provider,
                runtime.web_search.timeout_seconds,
                runtime.web_search.max_results
            ),
        },
        DashboardToolItemPayload {
            id: "file_tools",
            enabled: true,
            detail: format!(
                "read / write / edit within {}",
                config.tools.resolved_file_root().display()
            ),
        },
        DashboardToolItemPayload {
            id: "external_skills",
            enabled: config.external_skills.enabled,
            detail: if config.external_skills.auto_expose_installed {
                "auto expose installed".to_owned()
            } else {
                "manual expose".to_owned()
            },
        },
    ]
}

fn truncate_debug_value(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            output.push_str("...");
            break;
        }
        output.push(ch);
    }
    output
}

fn approval_mode_label(mode: mvp::config::GovernedToolApprovalMode) -> &'static str {
    match mode {
        mvp::config::GovernedToolApprovalMode::Disabled => "disabled",
        mvp::config::GovernedToolApprovalMode::MediumBalanced => "medium_balanced",
        mvp::config::GovernedToolApprovalMode::Strict => "strict",
    }
}

async fn resolve_provider_host_addresses(host: &str, port: u16) -> Vec<String> {
    let mut values = HashSet::new();
    if let Ok(addresses) = tokio::net::lookup_host((host, port)).await {
        for address in addresses {
            values.insert(address.ip().to_string());
        }
    }

    let mut addresses = values.into_iter().collect::<Vec<_>>();
    addresses.sort();
    addresses
}

fn is_fake_ip_address(address: &str) -> bool {
    let Ok(parsed) = address.parse::<std::net::IpAddr>() else {
        return false;
    };

    match parsed {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            octets[0] == 198 && (octets[1] == 18 || octets[1] == 19)
        }
        std::net::IpAddr::V6(_) => false,
    }
}

fn has_proxy_environment() -> bool {
    [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]
    .into_iter()
    .any(|key| {
        env::var(key)
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    })
}

async fn probe_provider_endpoint(endpoint: &str) -> (&'static str, Option<u16>) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return ("transport_failure", None);
    };

    match client.head(endpoint).send().await {
        Ok(response) => ("reachable", Some(response.status().as_u16())),
        Err(_) => ("transport_failure", None),
    }
}

fn build_provider_items(config: &mvp::config::LoongClawConfig) -> Vec<ProviderItemPayload> {
    if config.providers.is_empty() {
        return vec![provider_item_from_parts(
            config.provider.kind.profile().id.to_owned(),
            &config.provider,
            true,
            true,
        )];
    }

    config
        .providers
        .iter()
        .map(|(profile_id, profile)| {
            provider_item_from_parts(
                profile_id.clone(),
                &profile.provider,
                Some(profile_id.as_str()) == config.active_provider_id(),
                profile.default_for_kind,
            )
        })
        .collect()
}

fn prompt_personality_id(personality: mvp::prompt::PromptPersonality) -> &'static str {
    match personality {
        mvp::prompt::PromptPersonality::CalmEngineering => "calm_engineering",
        mvp::prompt::PromptPersonality::FriendlyCollab => "friendly_collab",
        mvp::prompt::PromptPersonality::AutonomousExecutor => "autonomous_executor",
    }
}

fn provider_item_from_parts(
    id: String,
    provider: &mvp::config::ProviderConfig,
    enabled: bool,
    default_for_kind: bool,
) -> ProviderItemPayload {
    let api_key_value = provider
        .api_key
        .as_ref()
        .and_then(|secret| secret.inline_value())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let api_key_env = provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    ProviderItemPayload {
        label: id.clone(),
        id,
        enabled,
        model: provider.model.clone(),
        endpoint: provider.endpoint(),
        api_key_configured: api_key_value.is_some() || api_key_env.is_some(),
        api_key_masked: api_key_value
            .map(mask_secret)
            .or_else(|| api_key_env.map(|_| "(env reference)".to_owned())),
        default_for_kind,
    }
}

fn derive_session_title(turns: &[mvp::memory::ConversationTurn]) -> Option<String> {
    turns
        .iter()
        .find(|turn| turn.role.eq_ignore_ascii_case("user"))
        .or_else(|| turns.first())
        .map(|turn| truncate_title(turn.content.as_str(), 56))
}

fn truncate_title(input: &str, max_chars: usize) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "Untitled session".to_owned();
    }

    let mut output = String::new();
    for (index, ch) in trimmed.chars().enumerate() {
        if index >= max_chars {
            output.push('…');
            break;
        }
        output.push(ch);
    }
    output
}

fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "****".to_owned();
    }

    if trimmed.starts_with('$') || trimmed.starts_with("env:") || trimmed.starts_with('%') {
        return "(env reference)".to_owned();
    }

    let suffix: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("****{suffix}")
}

fn resolve_local_web_token() -> Result<(String, PathBuf), WebApiError> {
    let loongclaw_home = mvp::config::default_loongclaw_home();
    fs::create_dir_all(&loongclaw_home)
        .map_err(|error| WebApiError::internal(format!("create loongclaw home failed: {error}")))?;

    let token_path = loongclaw_home.join(WEB_API_TOKEN_FILE);
    if let Ok(raw_env_token) = env::var(WEB_API_TOKEN_ENV) {
        let token = raw_env_token.trim();
        if !token.is_empty() {
            return Ok((token.to_owned(), token_path));
        }
    }

    if let Ok(existing) = fs::read_to_string(&token_path) {
        let token = existing.trim();
        if !token.is_empty() {
            return Ok((token.to_owned(), token_path));
        }
    }

    let token = format!(
        "{:016x}{:016x}{:016x}{:016x}",
        random::<u64>(),
        random::<u64>(),
        random::<u64>(),
        random::<u64>()
    );
    fs::write(&token_path, format!("{token}\n")).map_err(|error| {
        WebApiError::internal(format!("write local web api token failed: {error}"))
    })?;
    Ok((token, token_path))
}

// ── Web install helpers ──────────────────────────────────────────────────────

fn default_web_install_dir() -> PathBuf {
    mvp::config::default_loongclaw_home().join("web")
}

fn web_install_dist_dir(install_dir: &FsPath) -> PathBuf {
    install_dir.join("dist")
}

fn web_install_manifest_path(install_dir: &FsPath) -> PathBuf {
    install_dir.join("install.json")
}

fn copy_dir_all(src: &FsPath, dst: &FsPath) -> CliResult<()> {
    for entry in
        fs::read_dir(src).map_err(|error| format!("failed to read `{}`: {error}", src.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            fs::create_dir_all(&dst_path)
                .map_err(|error| format!("failed to create `{}`: {error}", dst_path.display()))?;
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|error| {
                format!(
                    "failed to copy `{}` to `{}`: {error}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn run_web_install(source: &str) -> CliResult<()> {
    let source_path = PathBuf::from(source);
    if !source_path.exists() {
        return Err(format!(
            "source path `{}` does not exist",
            source_path.display()
        ));
    }
    if !source_path.is_dir() {
        return Err(format!(
            "source path `{}` is not a directory",
            source_path.display()
        ));
    }
    if !source_path.join("index.html").is_file() {
        return Err(format!(
            "source path `{}` is missing `index.html` — run `npm run build` first",
            source_path.display()
        ));
    }

    let install_dir = default_web_install_dir();
    let dist_dir = web_install_dist_dir(&install_dir);

    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir).map_err(|error| {
            format!(
                "failed to remove existing install at `{}`: {error}",
                dist_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&dist_dir).map_err(|error| {
        format!(
            "failed to create install directory `{}`: {error}",
            dist_dir.display()
        )
    })?;

    copy_dir_all(&source_path, &dist_dir)?;

    let canonical_source = source_path
        .canonicalize()
        .unwrap_or_else(|_| source_path.clone());
    let manifest = WebInstallManifest {
        installed_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        source_path: canonical_source.display().to_string(),
        install_dir: install_dir.display().to_string(),
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("failed to serialize install manifest: {error}"))?;
    fs::write(web_install_manifest_path(&install_dir), manifest_json)
        .map_err(|error| format!("failed to write install manifest: {error}"))?;

    println!("Web Console installed to: {}", dist_dir.display());
    println!("Run `loongclaw web serve` to start the same-origin Web Console.");
    Ok(())
}

fn run_web_status() -> CliResult<()> {
    let install_dir = default_web_install_dir();
    let manifest_path = web_install_manifest_path(&install_dir);
    let dist_dir = web_install_dist_dir(&install_dir);

    if !manifest_path.exists() {
        println!("Web Console: not installed");
        println!("Run `loongclaw web install --source <path/to/web/dist>` to install.");
        return Ok(());
    }

    let manifest_raw = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read install manifest: {error}"))?;
    let manifest: WebInstallManifest = serde_json::from_str(&manifest_raw)
        .map_err(|error| format!("failed to parse install manifest: {error}"))?;

    let assets_ok = dist_dir.join("index.html").is_file();
    println!("Web Console: installed");
    println!("Install dir:  {}", manifest.install_dir);
    println!("Installed at: {}", manifest.installed_at);
    println!("Source:       {}", manifest.source_path);
    println!(
        "Assets:       {}",
        if assets_ok {
            "ok"
        } else {
            "missing (dist/index.html not found — re-run `web install`)"
        }
    );
    Ok(())
}

fn run_web_remove(force: bool) -> CliResult<()> {
    let install_dir = default_web_install_dir();
    let manifest_path = web_install_manifest_path(&install_dir);
    let dist_dir = web_install_dist_dir(&install_dir);

    if !manifest_path.exists() && !dist_dir.exists() {
        println!("Web Console: not installed, nothing to remove.");
        return Ok(());
    }

    if !force {
        println!("This will remove: {}", install_dir.display());
        println!("Re-run with --force to confirm removal.");
        return Ok(());
    }

    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir)
            .map_err(|error| format!("failed to remove `{}`: {error}", dist_dir.display()))?;
    }
    if manifest_path.exists() {
        fs::remove_file(&manifest_path)
            .map_err(|error| format!("failed to remove `{}`: {error}", manifest_path.display()))?;
    }

    println!("Web Console removed from: {}", install_dir.display());
    Ok(())
}

// ── Config / static root helpers ─────────────────────────────────────────────

fn resolve_web_config_path(state: &WebApiState) -> PathBuf {
    state
        .config_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(mvp::config::default_config_path)
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

async fn serve_web_static(
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
    let bytes = fs::read(&effective_path).map_err(|error| {
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

fn format_timestamp(unix_seconds: i64) -> String {
    OffsetDateTime::from_unix_timestamp(unix_seconds)
        .ok()
        .and_then(|timestamp| timestamp.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned())
}

fn is_internal_assistant_record(content: &str) -> bool {
    content.contains("\"_loongclaw_internal\":true")
        && (content.contains("\"type\":\"conversation_event\"")
            || content.contains("\"type\":\"tool_decision\"")
            || content.contains("\"type\":\"tool_outcome\""))
}

fn extract_stream_text_delta(event: &Value) -> Option<String> {
    // Provider/runtime event shapes are not fully uniform yet, so accept the
    // common text-bearing variants we already see from streaming-capable paths.
    let kind = event.get("type").and_then(Value::as_str);
    if kind == Some("text") {
        return event
            .get("content")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    if kind == Some("agent_message_chunk") {
        return extract_nested_text(event);
    }
    if event.get("sessionUpdate").and_then(Value::as_str) == Some("agent_message_chunk") {
        return extract_nested_text(event);
    }
    let payload = event
        .get("params")
        .and_then(|params| params.get("update"))?;
    if payload.get("sessionUpdate").and_then(Value::as_str) == Some("agent_message_chunk") {
        return extract_nested_text(payload);
    }
    None
}

fn extract_nested_text(value: &Value) -> Option<String> {
    value
        .get("content")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .get("delta")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

async fn run_chat_turn_stream(
    state: Arc<WebApiState>,
    session_id: String,
    turn_id: String,
    input: String,
    sender: mpsc::UnboundedSender<String>,
) -> Result<(), WebApiError> {
    let stream_result: Result<(), WebApiError> = async {
        let snapshot = load_web_snapshot(state.as_ref())?;
        let mut seen_internal_records = collect_internal_record_keys(&load_session_messages(
            &snapshot.memory_config,
            &session_id,
        )?);

        send_stream_event(
            &sender,
            json!({
                "type": "turn.started",
                "turnId": turn_id,
                "sessionId": session_id,
                "createdAt": format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
            }),
        )?;
        record_turn_started(&state, &session_id, &turn_id);

        mvp::runtime_env::initialize_runtime_environment(
            &snapshot.config,
            Some(&snapshot.resolved_path),
        );
        let sqlite_path = snapshot.config.memory.resolved_sqlite_path();
        mvp::memory::ensure_memory_db_ready(Some(sqlite_path), &snapshot.memory_config)
            .map_err(WebApiError::internal)?;
        let kernel_ctx = mvp::context::bootstrap_kernel_context_with_config(
            "web-api",
            mvp::context::DEFAULT_TOKEN_TTL_S,
            &snapshot.config,
        )
        .map_err(WebApiError::internal)?;
        let turn_config = snapshot
            .config
            .reload_provider_runtime_state_from_path(snapshot.resolved_path.as_path())
            .map_err(WebApiError::internal)?;
        let address = mvp::conversation::ConversationSessionAddress::from_session_id(&session_id);
        let coordinator = mvp::conversation::ConversationTurnCoordinator::new();
        let emitted_text = Arc::new(AtomicBool::new(false));
        let event_sink = WebTurnEventSink {
            state: state.clone(),
            turn_id: turn_id.clone(),
            sender: sender.clone(),
            emitted_text: emitted_text.clone(),
        };
        let acp_options =
            mvp::acp::AcpConversationTurnOptions::automatic().with_event_sink(Some(&event_sink));

        let turn_future = coordinator.handle_turn_with_address_and_acp_options(
            &turn_config,
            &address,
            &input,
            mvp::conversation::ProviderErrorMode::InlineMessage,
            &acp_options,
            mvp::conversation::ConversationRuntimeBinding::kernel(&kernel_ctx),
        );
        tokio::pin!(turn_future);

        let mut poll_interval = time::interval(Duration::from_millis(150));
        poll_interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

        let assistant_text = loop {
            tokio::select! {
                result = &mut turn_future => {
                    break result.map_err(WebApiError::internal)?;
                }
                _ = poll_interval.tick() => {
                    emit_internal_tool_events(
                        &state,
                        &snapshot.memory_config,
                        &session_id,
                        &turn_id,
                        &mut seen_internal_records,
                        &sender,
                    )?;
                }
            }
        };

        emit_internal_tool_events(
            &state,
            &snapshot.memory_config,
            &session_id,
            &turn_id,
            &mut seen_internal_records,
            &sender,
        )?;

        let final_message =
            latest_assistant_message(&snapshot.memory_config, &session_id, &assistant_text);
        if !emitted_text.load(Ordering::Relaxed) {
            // Older buffered providers still produce only the final assistant text.
            // Preserve the previous chunked fallback so the Web stream stays compatible.
            for delta in chunk_text(final_message.content.as_str(), 48) {
                send_stream_event(
                    &sender,
                    json!({
                        "type": "message.delta",
                        "turnId": turn_id,
                        "role": "assistant",
                        "delta": delta,
                    }),
                )?;
                record_message_delta(&state, &turn_id, delta.as_str());
                time::sleep(Duration::from_millis(18)).await;
            }
        }

        send_stream_event(
            &sender,
            json!({
                "type": "turn.completed",
                "turnId": turn_id,
                "message": final_message,
            }),
        )?;
        record_turn_completed(&state, &turn_id);

        Ok(())
    }
    .await;

    if let Err(error) = stream_result {
        let _ = send_stream_event(
            &sender,
            json!({
                "type": "turn.failed",
                "turnId": turn_id,
                "code": error.code,
                "message": error.message,
            }),
        );
        record_turn_failed(
            &state,
            &session_id,
            &turn_id,
            error.code,
            error.message.as_str(),
        );
    }

    Ok(())
}

fn generate_session_id() -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    format!("web-{now}-{:08x}", random::<u32>())
}

fn generate_turn_id() -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    format!("turn-{now}-{:08x}", random::<u32>())
}

fn send_stream_event(
    sender: &mpsc::UnboundedSender<String>,
    payload: Value,
) -> Result<(), WebApiError> {
    let line = serde_json::to_string(&payload).map_err(|error| {
        WebApiError::internal(format!("serialize stream event failed: {error}"))
    })?;
    sender
        .send(line)
        .map_err(|_error| WebApiError::internal("web turn stream receiver dropped"))
}

fn record_turn_started(state: &Arc<WebApiState>, session_id: &str, turn_id: &str) {
    let Ok(mut debug) = state.debug_state.lock() else {
        return;
    };
    let started_at = format_timestamp(OffsetDateTime::now_utc().unix_timestamp());
    let mut block = DebugConsoleBlock::operation(
        format!("turn:{turn_id}"),
        "turn",
        format!("{started_at} dialogue {turn_id}"),
    );
    block.lines.push(format!(
        "{started_at} turn.started session={session_id} turn={turn_id}"
    ));
    push_debug_block(&mut debug.recent_blocks, block);
}

fn record_message_delta(state: &Arc<WebApiState>, turn_id: &str, delta: &str) {
    let Ok(mut debug) = state.debug_state.lock() else {
        return;
    };
    let Some(last_turn) =
        find_debug_block_mut(&mut debug.recent_blocks, &format!("turn:{turn_id}"))
    else {
        return;
    };

    last_turn.delta_chunks += 1;
    last_turn.delta_chars += delta.chars().count();
    if last_turn.delta_chunks == 1 {
        last_turn.lines.push(format!(
            "{} message.delta first_chunk chars={}",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
            delta.chars().count()
        ));
    }
}

fn record_tool_started(
    state: &Arc<WebApiState>,
    session_id: &str,
    turn_id: &str,
    tool_id: &str,
    label: &str,
) {
    let Ok(mut debug) = state.debug_state.lock() else {
        return;
    };
    if let Some(last_turn) =
        find_debug_block_mut(&mut debug.recent_blocks, &format!("turn:{turn_id}"))
    {
        last_turn.tool_calls += 1;
        last_turn.lines.push(format!(
            "{} tool.started {} ({})",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
            label,
            tool_id
        ));
    }
    let _ = session_id;
}

fn record_tool_finished(
    state: &Arc<WebApiState>,
    session_id: &str,
    turn_id: &str,
    tool_id: &str,
    label: &str,
    outcome: &str,
) {
    let Ok(mut debug) = state.debug_state.lock() else {
        return;
    };
    let at = format_timestamp(OffsetDateTime::now_utc().unix_timestamp());
    if let Some(last_turn) =
        find_debug_block_mut(&mut debug.recent_blocks, &format!("turn:{turn_id}"))
    {
        last_turn.lines.push(format!(
            "{at} tool.finished {label} ({tool_id}) outcome={outcome}"
        ));
    }
    let _ = session_id;
}

fn record_turn_completed(state: &Arc<WebApiState>, turn_id: &str) {
    let Ok(mut debug) = state.debug_state.lock() else {
        return;
    };
    let Some(last_turn) =
        find_debug_block_mut(&mut debug.recent_blocks, &format!("turn:{turn_id}"))
    else {
        return;
    };
    last_turn.lines.push(format!(
        "{} turn.completed delta_chunks={} delta_chars={} tool_calls={}",
        format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
        last_turn.delta_chunks,
        last_turn.delta_chars,
        last_turn.tool_calls
    ));
    if last_turn.tool_calls == 0 {
        last_turn.lines.push(format!(
            "{} tool.none no real tool invocation was recorded for this turn",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp())
        ));
    }
}

fn record_turn_failed(
    state: &Arc<WebApiState>,
    session_id: &str,
    turn_id: &str,
    code: &str,
    message: &str,
) {
    let Ok(mut debug) = state.debug_state.lock() else {
        return;
    };
    let at = format_timestamp(OffsetDateTime::now_utc().unix_timestamp());
    if let Some(last_turn) =
        find_debug_block_mut(&mut debug.recent_blocks, &format!("turn:{turn_id}"))
    {
        last_turn.lines.push(format!(
            "{} turn.failed code={} tool_calls={} message={}",
            at,
            code,
            last_turn.tool_calls,
            truncate_debug_value(message, 180)
        ));
        if last_turn.tool_calls == 0 {
            last_turn.lines.push(format!(
                "{} tool.none no real tool invocation was recorded for this turn",
                format_timestamp(OffsetDateTime::now_utc().unix_timestamp())
            ));
        }
    }
    let _ = session_id;
}

fn push_debug_block(blocks: &mut Vec<DebugConsoleBlock>, block: DebugConsoleBlock) {
    blocks.push(block);
    trim_debug_blocks(blocks);
}

fn trim_debug_blocks(blocks: &mut Vec<DebugConsoleBlock>) {
    if blocks.len() > 24 {
        let overflow = blocks.len() - 24;
        blocks.drain(0..overflow);
    }
}

fn find_debug_block_mut<'a>(
    blocks: &'a mut [DebugConsoleBlock],
    id: &str,
) -> Option<&'a mut DebugConsoleBlock> {
    blocks.iter_mut().find(|block| block.id == id)
}

fn collect_internal_record_keys(turns: &[mvp::memory::ConversationTurn]) -> HashSet<String> {
    turns.iter().filter_map(internal_record_key).collect()
}

fn internal_record_key(turn: &mvp::memory::ConversationTurn) -> Option<String> {
    is_internal_assistant_record(&turn.content)
        .then(|| format!("{}:{}:{}", turn.ts, turn.role, turn.content))
}

fn emit_internal_tool_events(
    state: &Arc<WebApiState>,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    session_id: &str,
    turn_id: &str,
    seen_internal_records: &mut HashSet<String>,
    sender: &mpsc::UnboundedSender<String>,
) -> Result<(), WebApiError> {
    let history = load_session_messages(memory_config, session_id)?;
    for turn in history {
        let Some(record_key) = internal_record_key(&turn) else {
            continue;
        };
        if !seen_internal_records.insert(record_key) {
            continue;
        }
        let Some(event) = stream_tool_event_from_record(turn.content.as_str()) else {
            continue;
        };
        let event_type = if event.outcome.is_some() {
            "tool.finished"
        } else {
            "tool.started"
        };
        let payload = if let Some(ref outcome) = event.outcome {
            json!({
                "type": event_type,
                "turnId": turn_id,
                "toolId": event.tool_id,
                "label": event.label,
                "outcome": outcome,
            })
        } else {
            json!({
                "type": event_type,
                "turnId": turn_id,
                "toolId": event.tool_id,
                "label": event.label,
            })
        };
        send_stream_event(sender, payload)?;
        match event.outcome {
            Some(outcome) => record_tool_finished(
                state,
                session_id,
                turn_id,
                event.tool_id.as_str(),
                event.label.as_str(),
                outcome,
            ),
            None => record_tool_started(
                state,
                session_id,
                turn_id,
                event.tool_id.as_str(),
                event.label.as_str(),
            ),
        }
    }
    Ok(())
}

fn stream_tool_event_from_record(content: &str) -> Option<StreamToolEvent> {
    let parsed: Value = serde_json::from_str(content).ok()?;
    let record_type = parsed.get("type")?.as_str()?;
    let tool_id = parsed.get("tool_call_id")?.as_str()?.to_owned();
    let label = parsed
        .pointer("/decision/tool_name")
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .pointer("/outcome/payload/tool")
                .and_then(Value::as_str)
        })
        .or_else(|| parsed.pointer("/outcome/tool_name").and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| tool_id.clone());

    match record_type {
        "tool_decision" => Some(StreamToolEvent {
            tool_id,
            label,
            outcome: None,
        }),
        "tool_outcome" => {
            let outcome = if parsed
                .pointer("/outcome/status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                == "ok"
            {
                "ok"
            } else {
                "error"
            };
            Some(StreamToolEvent {
                tool_id,
                label,
                outcome: Some(outcome),
            })
        }
        _ => None,
    }
}

fn latest_assistant_message(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    session_id: &str,
    fallback_content: &str,
) -> ChatMessagePayload {
    let visible_history = load_session_messages(memory_config, session_id)
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter(|turn| {
            turn.role.eq_ignore_ascii_case("assistant")
                && !is_internal_assistant_record(&turn.content)
        })
        .collect::<Vec<_>>();

    visible_history
        .last()
        .map(|turn| ChatMessagePayload {
            id: format!("{session_id}:{}", turn.ts),
            role: "assistant".to_owned(),
            content: turn.content.clone(),
            created_at: format_timestamp(turn.ts),
        })
        .unwrap_or_else(|| {
            let created_at = OffsetDateTime::now_utc().unix_timestamp();
            ChatMessagePayload {
                id: format!("{session_id}:{created_at}"),
                role: "assistant".to_owned(),
                content: fallback_content.to_owned(),
                created_at: format_timestamp(created_at),
            }
        })
}

fn chunk_text(content: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for ch in content.chars() {
        current.push(ch);
        current_len += 1;
        if current_len >= chunk_size {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    chunks
}

fn session_id_from_title(title: &str) -> String {
    let slug = title
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let normalized = slug
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if normalized.is_empty() {
        generate_session_id()
    } else {
        format!("{normalized}-{:08x}", random::<u32>())
    }
}
