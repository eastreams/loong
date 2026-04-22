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

mod abilities;
mod auth;
mod chat;
mod dashboard;
mod debug_console;
mod install;
mod onboarding;
mod serve;

use abilities::{
    abilities_channels, abilities_personalization, abilities_personalization_save, abilities_skills,
};
use auth::{
    build_clear_pairing_cookie, build_clear_same_origin_session_cookie, build_pairing_cookie,
    build_same_origin_session_cookie, extract_allowed_local_origin, extract_request_token,
    request_is_authenticated, require_local_token, require_same_origin_write_origin,
};
use chat::{
    chat_history, chat_sessions, chat_turn, chat_turn_stream, create_chat_session,
    delete_chat_session,
};
use dashboard::{
    dashboard_approvals, dashboard_config, dashboard_connectivity, dashboard_providers, dashboard_runtime,
    dashboard_summary, dashboard_tools, provider_catalog,
};
use debug_console::{dashboard_debug_console, record_debug_operation};
use install::{
    default_web_install_dir, run_web_install, run_web_remove, run_web_status, web_install_dist_dir,
};
use serve::run_web_serve;

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
    turn_streams: Mutex<HashMap<String, (i64, mpsc::UnboundedReceiver<String>)>>,
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
    recent_tool_activity: Vec<DebugToolActivity>,
    active_tool_starts: HashMap<String, i64>,
    last_failure: Option<DebugConsoleFailure>,
}

#[derive(Debug, Clone)]
struct DebugConsoleBlock {
    id: String,
    kind: &'static str,
    lines: Vec<String>,
    tool_calls: usize,
    delta_chunks: usize,
    delta_chars: usize,
    started_at_ms: i64,
    first_delta_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
    status: &'static str,
    session_id: Option<String>,
    failure_code: Option<String>,
    failure_message: Option<String>,
}

#[derive(Debug, Clone)]
struct DebugToolActivity {
    id: String,
    label: String,
    outcome: &'static str,
    detail: String,
    duration_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct DebugConsoleFailure {
    at: String,
    category: &'static str,
    detail: String,
    hint: &'static str,
}

impl DebugConsoleBlock {
    fn operation(id: String, kind: &'static str, _header: String) -> Self {
        let started_at_ms = current_timestamp_ms();
        Self {
            id,
            kind,
            lines: Vec::new(),
            tool_calls: 0,
            delta_chunks: 0,
            delta_chars: 0,
            started_at_ms,
            first_delta_at_ms: None,
            finished_at_ms: None,
            status: "info",
            session_id: None,
            failure_code: None,
            failure_message: None,
        }
    }
}

fn current_timestamp_ms() -> i64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

fn upsert_debug_tool_activity(
    activities: &mut Vec<DebugToolActivity>,
    activity: DebugToolActivity,
) {
    if let Some(index) = activities
        .iter()
        .position(|existing| existing.id == activity.id)
    {
        activities.remove(index);
    }
    activities.push(activity);
    if activities.len() > 8 {
        let overflow = activities.len() - 8;
        activities.drain(0..overflow);
    }
}

fn classify_failure(code: &str, message: &str) -> (&'static str, &'static str) {
    let normalized = format!("{code} {message}").to_lowercase();
    if normalized.contains("policy") || normalized.contains("denied") {
        ("policy", "review policy or approval settings")
    } else if normalized.contains("timeout")
        || normalized.contains("transport")
        || normalized.contains("dns")
        || normalized.contains("network")
    {
        ("network", "check route, proxy, or endpoint reachability")
    } else if normalized.contains("runtime")
        || normalized.contains("unavailable")
        || normalized.contains("not ready")
    {
        (
            "runtime",
            "check runtime readiness and companion availability",
        )
    } else if normalized.contains("provider") || normalized.contains("credential") {
        ("provider", "check provider endpoint and credentials")
    } else {
        ("turn", "inspect recent tool activity and raw events")
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

impl mvp::conversation::ConversationTurnObserver for WebTurnEventSink {
    fn on_phase(&self, event: mvp::conversation::ConversationTurnPhaseEvent) {
        let lane = match event.lane {
            Some(mvp::conversation::ExecutionLane::Fast) => Some("fast"),
            Some(mvp::conversation::ExecutionLane::Safe) => Some("safe"),
            None => None,
        };
        let _ = send_stream_event(
            &self.sender,
            json!({
                "type": "turn.phase",
                "turnId": self.turn_id,
                "phase": event.phase.as_str(),
                "providerRound": event.provider_round,
                "lane": lane,
                "toolCallCount": event.tool_call_count,
                "messageCount": event.message_count,
                "estimatedTokens": event.estimated_tokens,
            }),
        );
    }

    fn on_tool(&self, event: mvp::conversation::ConversationTurnToolEvent) {
        match event.state {
            mvp::conversation::ConversationTurnToolState::Running => {
                let _ = send_stream_event(
                    &self.sender,
                    json!({
                        "type": "tool.started",
                        "turnId": self.turn_id,
                        "toolId": event.tool_call_id,
                        "label": event.tool_name,
                    }),
                );
                record_tool_started(
                    &self.state,
                    "",
                    &self.turn_id,
                    event.tool_call_id.as_str(),
                    event.tool_name.as_str(),
                );
            }
            mvp::conversation::ConversationTurnToolState::Completed
            | mvp::conversation::ConversationTurnToolState::NeedsApproval
            | mvp::conversation::ConversationTurnToolState::Denied
            | mvp::conversation::ConversationTurnToolState::Failed
            | mvp::conversation::ConversationTurnToolState::Interrupted => {
                let outcome = if matches!(
                    event.state,
                    mvp::conversation::ConversationTurnToolState::Completed
                ) {
                    "ok"
                } else {
                    "error"
                };
                let detail = event
                    .detail
                    .as_deref()
                    .or(event.request_summary.as_deref())
                    .map(str::to_owned);
                let _ = send_stream_event(
                    &self.sender,
                    json!({
                        "type": "tool.finished",
                        "turnId": self.turn_id,
                        "toolId": event.tool_call_id,
                        "label": event.tool_name,
                        "outcome": outcome,
                        "detail": detail,
                    }),
                );
                record_tool_finished(
                    &self.state,
                    "",
                    &self.turn_id,
                    event.tool_call_id.as_str(),
                    event.tool_name.as_str(),
                    outcome,
                );
            }
        }
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
struct ProviderCatalogPayload {
    items: Vec<ProviderCatalogItemPayload>,
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
    provider_base_url: String,
    provider_endpoint_explicit: bool,
    endpoint: String,
    api_key_configured: bool,
    api_key_masked: Option<String>,
    personality: String,
    prompt_mode: &'static str,
    prompt_addendum_configured: bool,
    prompt_addendum: String,
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
    autonomy_profile: String,
    consent_default_mode: String,
    shell_default_mode: String,
    shell_allow_count: usize,
    shell_deny_count: usize,
    sessions_allow_mutation: bool,
    external_skills_require_download_approval: bool,
    external_skills_auto_expose_installed: bool,
    external_skills_blocked_domain_count: usize,
    items: Vec<DashboardToolItemPayload>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardApprovalsPayload {
    pending_approval_count: usize,
    active_approval_count: usize,
    matched_count: usize,
    returned_count: usize,
    items: Vec<DashboardApprovalItemPayload>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardApprovalItemPayload {
    approval_request_id: String,
    session_id: String,
    session_title: String,
    visible_tool_name: String,
    tool_name: String,
    status: String,
    decision: Option<String>,
    request_summary: String,
    requested_at: String,
    resolved_at: Option<String>,
    executed_at: Option<String>,
    reason: Option<String>,
    rule_id: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesPersonalizationPayload {
    configured: bool,
    has_operator_preferences: bool,
    suppressed: bool,
    prompt_state: &'static str,
    updated_at: Option<String>,
    preferred_name: Option<String>,
    response_density: Option<&'static str>,
    initiative_level: Option<&'static str>,
    standing_boundaries: Option<String>,
    locale: Option<String>,
    timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesPersonalizationWriteRequest {
    preferred_name: Option<String>,
    response_density: Option<String>,
    initiative_level: Option<String>,
    standing_boundaries: Option<String>,
    locale: Option<String>,
    timezone: Option<String>,
    prompt_state: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesChannelsPayload {
    catalog_channel_count: usize,
    configured_channel_count: usize,
    configured_account_count: usize,
    enabled_account_count: usize,
    misconfigured_account_count: usize,
    runtime_backed_channel_count: usize,
    enabled_service_channel_count: usize,
    ready_service_channel_count: usize,
    surfaces: Vec<AbilitiesChannelSurfacePayload>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesChannelSurfacePayload {
    id: String,
    label: String,
    source: &'static str,
    configured_account_count: usize,
    enabled_account_count: usize,
    misconfigured_account_count: usize,
    ready_send_account_count: usize,
    ready_serve_account_count: usize,
    default_configured_account_id: Option<String>,
    service_enabled: bool,
    service_ready: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesSkillsPayload {
    visible_runtime_tool_count: usize,
    visible_runtime_direct_tool_count: usize,
    hidden_tool_count: usize,
    visible_runtime_tools: Vec<String>,
    visible_runtime_catalog: Vec<AbilitiesVisibleToolPayload>,
    hidden_tool_surfaces: Vec<AbilitiesHiddenToolSurfacePayload>,
    approval_mode: String,
    autonomy_profile: String,
    consent_default_mode: String,
    sessions_allow_mutation: bool,
    browser_companion: AbilitiesBrowserCompanionPayload,
    external_skills: AbilitiesExternalSkillsPayload,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesVisibleToolPayload {
    visible_name: String,
    canonical_name: String,
    display_name: String,
    summary: String,
    surface_id: Option<String>,
    exposure: String,
    execution_kind: String,
    capability_action_class: String,
    usage_guidance: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesHiddenToolSurfacePayload {
    surface_id: String,
    tool_count: usize,
    visible_tool_names: Vec<String>,
    usage_guidance: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesBrowserCompanionPayload {
    enabled: bool,
    ready: bool,
    command_configured: bool,
    expected_version: Option<String>,
    execution_tier: String,
    timeout_seconds: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AbilitiesExternalSkillsPayload {
    enabled: bool,
    override_active: bool,
    inventory_status: String,
    inventory_error: Option<String>,
    require_download_approval: bool,
    auto_expose_installed: bool,
    install_root: Option<String>,
    allowed_domain_count: usize,
    blocked_domain_count: usize,
    resolved_skill_count: usize,
    shadowed_skill_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardToolItemPayload {
    id: &'static str,
    enabled: bool,
    source: &'static str,
    capability_state: &'static str,
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
struct ProviderCatalogItemPayload {
    kind: String,
    display_name: String,
    default_base_url: String,
    default_chat_path: String,
    default_models_path: Option<String>,
    auth_scheme: String,
    feature_family: String,
    is_coding_variant: bool,
    aliases: Vec<String>,
    configuration_hint: Option<String>,
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

struct WebSnapshot {
    resolved_path: PathBuf,
    config: mvp::config::LoongConfig,
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
    config: &mvp::config::LoongConfig,
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Vec<DashboardToolItemPayload> {
    vec![
        DashboardToolItemPayload {
            id: "bash_exec",
            enabled: true,
            source: "native",
            capability_state: if config.tools.shell_default_mode == "deny"
                && config.tools.shell_allow.is_empty()
            {
                "policy_limited"
            } else {
                "executable"
            },
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
            source: "native",
            capability_state: if config.tools.sessions.enabled {
                "executable"
            } else {
                "discoverable"
            },
            detail: format!(
                "{} visibility, list {} / history {} / search / status / events / wait",
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
            source: "native",
            capability_state: if config.tools.messages.enabled {
                "executable"
            } else {
                "discoverable"
            },
            detail: "message tool surface".to_owned(),
        },
        DashboardToolItemPayload {
            id: "delegate",
            enabled: config.tools.delegate.enabled,
            source: "native",
            capability_state: if config.tools.delegate.enabled {
                "executable"
            } else {
                "discoverable"
            },
            detail: format!(
                "depth {}, active children {}",
                config.tools.delegate.max_depth, config.tools.delegate.max_active_children
            ),
        },
        DashboardToolItemPayload {
            id: "browser",
            enabled: config.tools.browser.enabled,
            source: "native",
            capability_state: if config.tools.browser.enabled {
                "executable"
            } else {
                "discoverable"
            },
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
            source: "companion",
            capability_state: if !config.tools.browser_companion.enabled {
                "discoverable"
            } else if runtime.browser_companion.is_runtime_ready() {
                "executable"
            } else {
                "runtime_unavailable"
            },
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
            source: "native",
            capability_state: if config.tools.web.enabled {
                "executable"
            } else {
                "discoverable"
            },
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
            source: "provider",
            capability_state: if config.tools.web_search.enabled {
                "executable"
            } else {
                "discoverable"
            },
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
            source: "local",
            capability_state: "executable",
            detail: format!(
                "read / write / edit within {}",
                config.tools.resolved_file_root().display()
            ),
        },
        DashboardToolItemPayload {
            id: "external_skills",
            enabled: config.external_skills.enabled,
            source: "catalog",
            capability_state: if !config.external_skills.enabled {
                "discoverable"
            } else if config.external_skills.auto_expose_installed {
                "executable"
            } else {
                "discoverable"
            },
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

fn build_provider_items(config: &mvp::config::LoongConfig) -> Vec<ProviderItemPayload> {
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
        .map(|(profile_id, profile): (&String, &mvp::config::ProviderProfileConfig)| {
            provider_item_from_parts(
                profile_id.clone(),
                &profile.provider,
                Some(profile_id.as_str()) == config.active_provider_id(),
                profile.default_for_kind,
            )
        })
        .collect()
}

fn build_provider_catalog_items() -> Vec<ProviderCatalogItemPayload> {
    mvp::config::provider_catalog_entries()
        .into_iter()
        .map(|entry| ProviderCatalogItemPayload {
            kind: entry.kind,
            display_name: entry.display_name,
            default_base_url: entry.default_base_url,
            default_chat_path: entry.default_chat_path,
            default_models_path: entry.default_models_path,
            auth_scheme: entry.auth_scheme,
            feature_family: entry.feature_family,
            is_coding_variant: entry.is_coding_variant,
            aliases: entry.aliases,
            configuration_hint: entry.configuration_hint,
        })
        .collect()
}

fn prompt_personality_id(personality: mvp::prompt::PromptPersonality) -> &'static str {
    crate::onboard_cli::prompt_personality_id(personality)
}

fn provider_item_from_parts(
    id: String,
    provider: &mvp::config::ProviderConfig,
    enabled: bool,
    default_for_kind: bool,
) -> ProviderItemPayload {
    let default_profile_id = provider.inferred_profile_id();
    let label = if id == default_profile_id {
        provider.kind.display_name().to_owned()
    } else {
        format!("{} ({id})", provider.kind.display_name())
    };
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
        label,
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
    let loongclaw_home = mvp::config::default_loong_home();
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

// ==== Config / static root helpers ====

fn resolve_web_config_path(state: &WebApiState) -> PathBuf {
    state
        .config_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(mvp::config::default_config_path)
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
        let event_sink = Arc::new(WebTurnEventSink {
            state: state.clone(),
            turn_id: turn_id.clone(),
            sender: sender.clone(),
            emitted_text: emitted_text.clone(),
        });
        let observer_handle: mvp::conversation::ConversationTurnObserverHandle =
            event_sink.clone();
        let acp_options =
            mvp::acp::AcpConversationTurnOptions::automatic().with_event_sink(Some(event_sink.as_ref()));

        let turn_future = coordinator.handle_production_turn_with_address_and_acp_options_and_observer(
            &turn_config,
            &address,
            &input,
            mvp::conversation::ProviderErrorMode::InlineMessage,
            &acp_options,
            mvp::conversation::ConversationRuntimeBinding::kernel(&kernel_ctx),
            Some(observer_handle),
        );
        tokio::pin!(turn_future);

        let assistant_text: String = loop {
            tokio::select! {
                result = &mut turn_future => {
                    break result.map_err(WebApiError::internal)?;
                }
            }
        };

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
    block.status = "running";
    block.session_id = Some(session_id.to_owned());
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
        last_turn.first_delta_at_ms = Some(current_timestamp_ms());
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
    let tool_key = format!("{turn_id}:{tool_id}");
    debug
        .active_tool_starts
        .insert(tool_key.clone(), current_timestamp_ms());
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
    upsert_debug_tool_activity(
        &mut debug.recent_tool_activity,
        DebugToolActivity {
            id: tool_key,
            label: label.to_owned(),
            outcome: "running",
            detail: "started".to_owned(),
            duration_ms: None,
        },
    );
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
    let duration_ms = debug
        .active_tool_starts
        .remove(&format!("{turn_id}:{tool_id}"))
        .map(|started_at| current_timestamp_ms().saturating_sub(started_at));
    if let Some(last_turn) =
        find_debug_block_mut(&mut debug.recent_blocks, &format!("turn:{turn_id}"))
    {
        last_turn.lines.push(format!(
            "{at} tool.finished {label} ({tool_id}) outcome={outcome}"
        ));
    }
    upsert_debug_tool_activity(
        &mut debug.recent_tool_activity,
        DebugToolActivity {
            id: format!("{turn_id}:{tool_id}"),
            label: label.to_owned(),
            outcome: if outcome == "ok" { "ok" } else { "error" },
            detail: if outcome == "ok" {
                "completed".to_owned()
            } else {
                format!("failed outcome={outcome}")
            },
            duration_ms,
        },
    );
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
    last_turn.status = "completed";
    last_turn.finished_at_ms = Some(current_timestamp_ms());
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
    debug
        .active_tool_starts
        .retain(|key, _| !key.starts_with(&format!("{turn_id}:")));
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
    let (category, hint) = classify_failure(code, message);
    if let Some(last_turn) =
        find_debug_block_mut(&mut debug.recent_blocks, &format!("turn:{turn_id}"))
    {
        last_turn.status = "failed";
        last_turn.finished_at_ms = Some(current_timestamp_ms());
        last_turn.failure_code = Some(code.to_owned());
        last_turn.failure_message = Some(message.to_owned());
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
    let interrupted_keys = debug
        .active_tool_starts
        .iter()
        .filter_map(|(key, started_at)| {
            key.starts_with(&format!("{turn_id}:"))
                .then_some((key.clone(), *started_at))
        })
        .collect::<Vec<_>>();
    for (tool_key, started_at) in interrupted_keys {
        debug.active_tool_starts.remove(&tool_key);
        upsert_debug_tool_activity(
            &mut debug.recent_tool_activity,
            DebugToolActivity {
                id: tool_key,
                label: "tool".to_owned(),
                outcome: "error",
                detail: "interrupted".to_owned(),
                duration_ms: Some(current_timestamp_ms().saturating_sub(started_at)),
            },
        );
    }
    debug.last_failure = Some(DebugConsoleFailure {
        at,
        category,
        detail: truncate_debug_value(message, 180),
        hint,
    });
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
