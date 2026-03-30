use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::CliResult;
use crate::KernelContext;
use crate::channel::{
    ChannelDelivery, ChannelInboundMessage, ChannelOutboundTarget, ChannelOutboundTargetKind,
    ChannelPlatform, ChannelSession, ChannelTurnFeedbackPolicy, process_inbound_with_provider,
    runtime_state::ChannelOperationRuntimeTracker,
};
use crate::config::{LoongClawConfig, ResolvedWhatsappChannelConfig};

// ---------------------------------------------------------------------------
// RecentIdCache — copied from feishu/webhook.rs (private struct)
// ---------------------------------------------------------------------------

struct RecentIdCache {
    max_len: usize,
    queue: VecDeque<String>,
    states: BTreeMap<String, RecentIdState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecentIdState {
    Processing,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecentIdReservation {
    Accepted,
    InProgressDuplicate,
    CompletedDuplicate,
}

impl RecentIdCache {
    fn new(max_len: usize) -> Self {
        Self {
            max_len: max_len.max(1),
            queue: VecDeque::new(),
            states: BTreeMap::new(),
        }
    }

    fn begin_processing(&mut self, id: &str) -> RecentIdReservation {
        let id = id.trim();
        if id.is_empty() {
            return RecentIdReservation::CompletedDuplicate;
        }
        if let Some(state) = self.states.get(id) {
            return match state {
                RecentIdState::Processing => RecentIdReservation::InProgressDuplicate,
                RecentIdState::Completed => RecentIdReservation::CompletedDuplicate,
            };
        }

        self.queue.push_back(id.to_owned());
        self.states.insert(id.to_owned(), RecentIdState::Processing);
        self.trim_to_max();
        RecentIdReservation::Accepted
    }

    fn mark_completed(&mut self, id: &str) {
        let id = id.trim();
        if let Some(state) = self.states.get_mut(id) {
            *state = RecentIdState::Completed;
        }
    }

    fn release(&mut self, id: &str) {
        let id = id.trim();
        if self.states.remove(id).is_some() {
            self.queue.retain(|entry| entry != id);
        }
    }

    fn trim_to_max(&mut self) {
        while self.queue.len() > self.max_len {
            if let Some(removed) = self.queue.pop_front() {
                self.states.remove(&removed);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Webhook state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct WhatsappWebhookState {
    config: LoongClawConfig,
    resolved_path: Option<PathBuf>,
    configured_account_id: String,
    account_id: String,
    verify_token: Option<String>,
    app_secret: Option<String>,
    access_token: String,
    phone_number_id: String,
    api_base_url: String,
    allowed_phone_numbers: BTreeSet<String>,
    seen_messages: Arc<Mutex<RecentIdCache>>,
    kernel_ctx: Arc<KernelContext>,
    runtime: Arc<ChannelOperationRuntimeTracker>,
}

impl WhatsappWebhookState {
    pub(super) fn new(
        config: LoongClawConfig,
        resolved_path: PathBuf,
        resolved: &ResolvedWhatsappChannelConfig,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> CliResult<Self> {
        let access_token = resolved
            .access_token()
            .ok_or_else(|| "whatsapp access_token is required for serve".to_owned())?;
        let phone_number_id = resolved
            .phone_number_id()
            .ok_or_else(|| "whatsapp phone_number_id is required for serve".to_owned())?;

        Ok(Self {
            configured_account_id: resolved.configured_account_id.clone(),
            account_id: resolved.account.id.clone(),
            verify_token: resolved.verify_token(),
            app_secret: resolved.app_secret(),
            access_token,
            phone_number_id,
            api_base_url: resolved.resolved_api_base_url(),
            allowed_phone_numbers: BTreeSet::new(),
            seen_messages: Arc::new(Mutex::new(RecentIdCache::new(512))),
            config,
            resolved_path: Some(resolved_path),
            kernel_ctx: Arc::new(kernel_ctx),
            runtime,
        })
    }
}

// ---------------------------------------------------------------------------
// GET handler — Meta webhook verification challenge
// ---------------------------------------------------------------------------

fn extract_query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == key { Some(v) } else { None }
    })
}

pub(super) async fn whatsapp_verify_handler(
    State(state): State<WhatsappWebhookState>,
    uri: Uri,
) -> impl IntoResponse {
    let raw_query = uri.query().unwrap_or_default();
    let mode = extract_query_param(raw_query, "hub.mode").unwrap_or_default();
    if mode != "subscribe" {
        return (
            StatusCode::FORBIDDEN,
            format!("unexpected hub.mode: {mode}"),
        )
            .into_response();
    }

    let expected = state.verify_token.as_deref().unwrap_or_default();
    let provided = extract_query_param(raw_query, "hub.verify_token").unwrap_or_default();
    if expected.is_empty() || provided != expected {
        return (StatusCode::FORBIDDEN, "verify token mismatch".to_owned()).into_response();
    }

    let challenge = extract_query_param(raw_query, "hub.challenge").unwrap_or_default();
    (StatusCode::OK, challenge.to_owned()).into_response()
}

// ---------------------------------------------------------------------------
// POST handler — inbound webhook events
// ---------------------------------------------------------------------------

pub(super) async fn whatsapp_webhook_handler(
    State(state): State<WhatsappWebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Verify HMAC-SHA256 signature
    if let Some(app_secret) = state.app_secret.as_deref() {
        let signature = headers
            .get("x-hub-signature-256")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .unwrap_or_default();
        let hex = signature.strip_prefix("sha256=").unwrap_or(signature);
        if !crate::crypto::verify_hmac_sha256(app_secret.as_bytes(), &body, hex) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid signature"})),
            )
                .into_response();
        }
    }

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid JSON: {error}")})),
            )
                .into_response();
        }
    };

    // Process webhook entries
    tokio::spawn(async move {
        if let Err(error) = process_whatsapp_webhook(&state, &payload).await {
            log_whatsapp_warning("webhook processing error", &error);
        }
    });

    // Always return 200 quickly to Meta
    (StatusCode::OK, Json(json!({"status": "ok"}))).into_response()
}

// ---------------------------------------------------------------------------
// Webhook payload processing
// ---------------------------------------------------------------------------

async fn process_whatsapp_webhook(state: &WhatsappWebhookState, payload: &Value) -> CliResult<()> {
    let entries = payload
        .get("entry")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for entry in &entries {
        let changes = entry
            .get("changes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for change in &changes {
            let value = match change.get("value") {
                Some(v) => v,
                None => continue,
            };
            let messages = value
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for message in &messages {
                if let Err(error) = handle_whatsapp_inbound_message(state, value, message).await {
                    log_whatsapp_warning("inbound message error", &error);
                }
            }
        }
    }

    Ok(())
}

async fn handle_whatsapp_inbound_message(
    state: &WhatsappWebhookState,
    _value: &Value,
    message: &Value,
) -> CliResult<()> {
    let message_id = message
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or_default();
    if message_id.is_empty() {
        return Ok(());
    }

    // Dedup check
    {
        let mut cache = state.seen_messages.lock().await;
        match cache.begin_processing(message_id) {
            RecentIdReservation::Accepted => {}
            RecentIdReservation::InProgressDuplicate | RecentIdReservation::CompletedDuplicate => {
                return Ok(());
            }
        }
    }

    let result = async {
        let sender = message
            .get("from")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or_default();
        if sender.is_empty() {
            return Ok(());
        }

        // Check allowlist if configured
        if !state.allowed_phone_numbers.is_empty() && !state.allowed_phone_numbers.contains(sender)
        {
            return Ok(());
        }

        let msg_type = message
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = match msg_type {
            "text" => message
                .get("text")
                .and_then(|t| t.get("body"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            _ => {
                // Only handle text messages for now
                return Ok(());
            }
        };
        if text.trim().is_empty() {
            return Ok(());
        }

        state
            .runtime
            .mark_run_start()
            .await
            .map_err(|error| format!("whatsapp runtime start failed: {error}"))?;

        let process_result = async {
            let session = ChannelSession::with_account(
                ChannelPlatform::WhatsApp,
                state.account_id.as_str(),
                sender,
            )
            .with_configured_account_id(state.configured_account_id.as_str())
            .with_participant_id(sender);

            let reply_target = ChannelOutboundTarget::new(
                ChannelPlatform::WhatsApp,
                ChannelOutboundTargetKind::Address,
                sender,
            );

            let channel_message = ChannelInboundMessage {
                session,
                reply_target,
                text,
                delivery: ChannelDelivery {
                    ack_cursor: None,
                    source_message_id: Some(message_id.to_owned()),
                    sender_principal_key: Some(sender.to_owned()),
                    thread_root_id: None,
                    parent_message_id: None,
                    resources: Vec::new(),
                    feishu_callback: None,
                },
            };

            let reply = process_inbound_with_provider(
                &state.config,
                state.resolved_path.as_deref(),
                &channel_message,
                state.kernel_ctx.as_ref(),
                ChannelTurnFeedbackPolicy::final_trace_significant(),
            )
            .await?;

            // Send reply via WhatsApp Cloud API
            send_whatsapp_text_reply(state, sender, &reply).await?;
            Ok(())
        }
        .await;

        if let Err(error) = state.runtime.mark_run_end().await {
            log_whatsapp_warning("runtime end failed", &error);
        }

        process_result
    }
    .await;

    match &result {
        Ok(()) => {
            let mut cache = state.seen_messages.lock().await;
            cache.mark_completed(message_id);
        }
        Err(_) => {
            let mut cache = state.seen_messages.lock().await;
            cache.release(message_id);
        }
    }

    result
}

async fn send_whatsapp_text_reply(
    state: &WhatsappWebhookState,
    recipient: &str,
    text: &str,
) -> CliResult<()> {
    let url = format!(
        "{}/{}/messages",
        state.api_base_url.trim_end_matches('/'),
        state.phone_number_id.trim()
    );
    let body = json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient,
        "type": "text",
        "text": {
            "preview_url": false,
            "body": text,
        },
    });
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .bearer_auth(&state.access_token)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("whatsapp reply send failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let payload = response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_owned());
        return Err(format!(
            "whatsapp reply failed with status {}: {payload}",
            status.as_u16()
        ));
    }

    Ok(())
}

fn log_whatsapp_warning(context: &str, error: &str) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("warning: whatsapp {context}: {error}");
    }
}
