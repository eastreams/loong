use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::{
        HeaderValue, Method, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN,
        },
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use clap::Subcommand;
use rand::random;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{CliResult, mvp, with_graceful_shutdown};

#[derive(Subcommand, Debug)]
pub enum WebCommand {
    /// Serve the local Web Console API surface
    Serve {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value = "127.0.0.1:4317")]
        bind: String,
    },
}

#[derive(Debug, Clone)]
struct WebApiState {
    config_path: Option<String>,
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
    message: ChatMessagePayload,
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
        WebCommand::Serve { config, bind } => run_web_serve(config.as_deref(), &bind).await,
    }
}

async fn run_web_serve(config_path: Option<&str>, bind: &str) -> CliResult<()> {
    let state = Arc::new(WebApiState {
        config_path: config_path.map(str::to_owned),
    });
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/meta", get(meta))
        .route("/api/dashboard/summary", get(dashboard_summary))
        .route("/api/dashboard/providers", get(dashboard_providers))
        .route("/api/chat/sessions", get(chat_sessions).post(create_chat_session))
        .route("/api/chat/sessions/{id}", delete(delete_chat_session))
        .route("/api/chat/sessions/{id}/turn", post(chat_turn))
        .route("/api/chat/sessions/{id}/history", get(chat_history))
        .layer(middleware::from_fn(local_web_cors))
        .with_state(state);

    let address: SocketAddr = bind
        .parse()
        .map_err(|error| format!("invalid web bind address `{bind}`: {error}"))?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("bind web api on {bind} failed: {error}"))?;

    println!("loongclaw web api listening on http://{address}");
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
    if request.method() == Method::OPTIONS {
        return with_cors_headers(StatusCode::NO_CONTENT.into_response());
    }

    let response = next.run(request).await;
    with_cors_headers(response)
}

fn with_cors_headers(mut response: Response) -> Response {
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
    );
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    response
}

async fn meta() -> Json<ApiEnvelope<MetaPayload>> {
    Json(ApiEnvelope {
        ok: true,
        data: MetaPayload {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            api_version: "v1",
            web_install_mode: "api_only",
            supported_locales: ["en", "zh-CN"],
            default_locale: "en",
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
            web_install_mode: "api_only",
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
    let history = load_session_messages(&snapshot.memory_config, &id)?;

    if history.is_empty() {
        return Err(WebApiError::not_found(format!(
            "session `{id}` was not found in sqlite memory"
        )));
    }

    let messages = history
        .into_iter()
        .filter(|turn| {
            !(turn.role.eq_ignore_ascii_case("assistant")
                && is_internal_assistant_record(&turn.content))
        })
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
    mvp::memory::clear_session_direct(&id, &snapshot.memory_config).map_err(WebApiError::internal)?;
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

    let assistant_text = run_chat_turn(state.as_ref(), &id, input).await?;
    let visible_history = load_session_messages(&load_web_snapshot(state.as_ref())?.memory_config, &id)?
        .into_iter()
        .filter(|turn| {
            turn.role.eq_ignore_ascii_case("assistant")
                && !is_internal_assistant_record(&turn.content)
        })
        .collect::<Vec<_>>();
    let latest_assistant_message = visible_history
        .last()
        .map(|turn| ChatMessagePayload {
            id: format!("{id}:{}", turn.ts),
            role: "assistant".to_owned(),
            content: turn.content.clone(),
            created_at: format_timestamp(turn.ts),
        })
        .unwrap_or_else(|| {
            let created_at = OffsetDateTime::now_utc().unix_timestamp();
            ChatMessagePayload {
                id: format!("{id}:{created_at}"),
                role: "assistant".to_owned(),
                content: assistant_text,
                created_at: format_timestamp(created_at),
            }
        });

    Ok(Json(ApiEnvelope {
        ok: true,
        data: ChatTurnPayload {
            session_id: id,
            message: latest_assistant_message,
        },
    }))
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
    let memory_config = mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
        &config.memory,
    );
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

fn provider_item_from_parts(
    id: String,
    provider: &mvp::config::ProviderConfig,
    enabled: bool,
    default_for_kind: bool,
) -> ProviderItemPayload {
    let api_key_value = provider
        .api_key
        .as_deref()
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
    turns.iter()
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

async fn run_chat_turn(
    state: &WebApiState,
    session_id: &str,
    input: &str,
) -> Result<String, WebApiError> {
    let snapshot = load_web_snapshot(state)?;
    mvp::runtime_env::initialize_runtime_environment(
        &snapshot.config,
        Some(&snapshot.resolved_path),
    );
    let sqlite_path = snapshot.config.memory.resolved_sqlite_path();
    mvp::memory::ensure_memory_db_ready(Some(sqlite_path), &snapshot.memory_config)
        .map_err(WebApiError::internal)?;
    let kernel_ctx =
        mvp::context::bootstrap_kernel_context("web-api", mvp::context::DEFAULT_TOKEN_TTL_S)
            .map_err(WebApiError::internal)?;
    let turn_config = snapshot
        .config
        .reload_provider_runtime_state_from_path(snapshot.resolved_path.as_path())
        .map_err(WebApiError::internal)?;
    let address = mvp::conversation::ConversationSessionAddress::from_session_id(session_id);
    let coordinator = mvp::conversation::ConversationTurnCoordinator::new();
    let acp_options = mvp::acp::AcpConversationTurnOptions::automatic();

    coordinator
        .handle_turn_with_address_and_acp_options(
            &turn_config,
            &address,
            input,
            mvp::conversation::ProviderErrorMode::InlineMessage,
            &acp_options,
            mvp::conversation::ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .map_err(WebApiError::internal)
}

fn generate_session_id() -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    format!("web-{now}-{:08x}", random::<u32>())
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
