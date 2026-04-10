use std::{
    collections::VecDeque,
    convert::Infallible,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use http_body::{Body as HttpBody, Frame, SizeHint};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::CliResult;
use crate::KernelContext;
use crate::channel::feishu::api::{FeishuClient, resources::cards};
use crate::channel::{
    ChannelInboundMessage, ChannelOutboundTarget, ChannelTurnFeedbackPolicy,
    access_policy::ChannelInboundAccessPolicy, process_inbound_with_provider,
    runtime::state::ChannelOperationRuntimeTracker,
};
use crate::config::{LoongClawConfig, ResolvedFeishuChannelConfig};
use crate::crypto::timing_safe_eq;

use super::adapter::{FeishuAdapter, outbound_reply_message_from_text};
use super::payload::{FeishuCardCallbackEvent, FeishuWebhookAction};
use super::send::send_channel_message_via_message_send_api;

const FEISHU_CALLBACK_RESPONSE_MARKER: &str = "[feishu_callback_response]";

#[derive(Clone)]
pub(super) struct FeishuWebhookState {
    config: LoongClawConfig,
    resolved_path: Option<PathBuf>,
    adapter: Arc<Mutex<FeishuAdapter>>,
    configured_account_id: String,
    account_id: String,
    verification_token: Option<String>,
    encrypt_key: Option<String>,
    access_policy: ChannelInboundAccessPolicy<String>,
    ack_reactions: bool,
    ignore_bot_messages: bool,
    seen_events: Arc<Mutex<RecentIdCache>>,
    seen_ack_reactions: Arc<Mutex<RecentIdCache>>,
    kernel_ctx: Arc<KernelContext>,
    runtime: Arc<ChannelOperationRuntimeTracker>,
}

impl FeishuWebhookState {
    #[cfg(test)]
    pub(super) fn new(
        config: LoongClawConfig,
        resolved: &ResolvedFeishuChannelConfig,
        adapter: FeishuAdapter,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> Self {
        Self::new_with_optional_resolved_path(config, None, resolved, adapter, kernel_ctx, runtime)
    }

    pub(super) fn new_with_resolved_path(
        config: LoongClawConfig,
        resolved_path: PathBuf,
        resolved: &ResolvedFeishuChannelConfig,
        adapter: FeishuAdapter,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> Self {
        Self::new_with_optional_resolved_path(
            config,
            Some(resolved_path),
            resolved,
            adapter,
            kernel_ctx,
            runtime,
        )
    }

    fn new_with_optional_resolved_path(
        config: LoongClawConfig,
        resolved_path: Option<PathBuf>,
        resolved: &ResolvedFeishuChannelConfig,
        adapter: FeishuAdapter,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> Self {
        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            resolved.allowed_chat_ids.as_slice(),
            resolved.allowed_sender_ids.as_slice(),
            true,
        );

        Self {
            configured_account_id: resolved.configured_account_id.clone(),
            account_id: resolved.account.id.clone(),
            verification_token: resolved.verification_token(),
            encrypt_key: resolved.encrypt_key(),
            access_policy,
            ack_reactions: resolved.ack_reactions,
            ignore_bot_messages: resolved.ignore_bot_messages,
            config,
            resolved_path,
            adapter: Arc::new(Mutex::new(adapter)),
            seen_events: Arc::new(Mutex::new(RecentIdCache::new(2_048))),
            seen_ack_reactions: Arc::new(Mutex::new(RecentIdCache::new(4_096))),
            kernel_ctx: Arc::new(kernel_ctx),
            runtime,
        }
    }

    pub(super) fn parse_websocket_payload(
        &self,
        payload: &Value,
    ) -> CliResult<FeishuWebhookAction> {
        super::payload::parse_feishu_inbound_payload_with_access_policy(
            payload,
            super::payload::FeishuTransportAuth::websocket(),
            &self.access_policy,
            self.ignore_bot_messages,
            self.configured_account_id.as_str(),
            self.account_id.as_str(),
        )
    }

    pub(super) fn dispatch_deferred_updates(
        &self,
        updates: Vec<crate::tools::DeferredFeishuCardUpdate>,
    ) {
        dispatch_deferred_feishu_card_updates(self.config.clone(), updates);
    }

    pub(super) fn configured_account_id(&self) -> &str {
        self.configured_account_id.as_str()
    }

    pub(super) fn account_id(&self) -> &str {
        self.account_id.as_str()
    }
}

struct RecentIdCache {
    max_len: usize,
    queue: VecDeque<String>,
    states: std::collections::BTreeMap<String, RecentIdState>,
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

#[allow(dead_code)]
enum FeishuCallbackResponse {
    Noop,
    Toast {
        kind: &'static str,
        content: String,
    },
    Card {
        toast: Option<FeishuCallbackToast>,
        card: Value,
    },
}

#[allow(dead_code)]
struct FeishuCallbackToast {
    kind: &'static str,
    content: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuStructuredCallbackResponse {
    mode: String,
    kind: Option<String>,
    content: Option<String>,
    toast: Option<FeishuStructuredCallbackToast>,
    card: Option<Value>,
    markdown: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeishuStructuredCallbackToast {
    kind: String,
    content: String,
}

#[derive(Debug)]
pub(super) struct FeishuParsedActionResponse {
    pub(super) body: Value,
    pub(super) websocket_body: Option<Value>,
    pub(super) deferred_updates: Vec<crate::tools::DeferredFeishuCardUpdate>,
}

#[derive(Debug)]
struct FeishuWebhookSuccessResponse {
    body: Value,
    post_response_dispatch: Option<FeishuWebhookPostResponseDispatch>,
}

#[derive(Debug)]
struct FeishuWebhookPostResponseDispatch {
    config: LoongClawConfig,
    deferred_updates: Vec<crate::tools::DeferredFeishuCardUpdate>,
}

struct FeishuPostResponseJsonBody {
    bytes: Option<Bytes>,
    post_response_dispatch: Option<FeishuWebhookPostResponseDispatch>,
}

impl FeishuCallbackResponse {
    fn as_json(&self) -> Value {
        match self {
            Self::Noop => json!({}),
            Self::Toast { kind, content } => json!({
                "toast": {
                    "type": kind,
                    "content": content,
                }
            }),
            Self::Card { toast, card } => {
                let mut body = serde_json::Map::new();
                if let Some(toast) = toast {
                    body.insert(
                        "toast".to_owned(),
                        json!({
                            "type": toast.kind,
                            "content": toast.content,
                        }),
                    );
                }
                body.insert("card".to_owned(), card.clone());
                Value::Object(body)
            }
        }
    }
}

impl FeishuWebhookSuccessResponse {
    fn from_parsed_response(response: FeishuParsedActionResponse, config: LoongClawConfig) -> Self {
        Self {
            body: response.body,
            post_response_dispatch: (!response.deferred_updates.is_empty()).then_some(
                FeishuWebhookPostResponseDispatch {
                    config,
                    deferred_updates: response.deferred_updates,
                },
            ),
        }
    }

    #[cfg(test)]
    fn body(&self) -> &Value {
        &self.body
    }
}

impl FeishuParsedActionResponse {
    fn immediate(body: Value) -> Self {
        Self {
            body,
            websocket_body: None,
            deferred_updates: Vec::new(),
        }
    }

    fn with_deferred_card_updates(
        body: Value,
        deferred_updates: Vec<crate::tools::DeferredFeishuCardUpdate>,
    ) -> Self {
        Self {
            websocket_body: Some(body.clone()),
            body,
            deferred_updates,
        }
    }
}

impl IntoResponse for FeishuWebhookSuccessResponse {
    fn into_response(self) -> Response {
        let body_bytes = match serde_json::to_vec(&self.body) {
            Ok(body) => body,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "code": StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                        "msg": format!("serialize feishu webhook response failed: {error}"),
                    })),
                )
                    .into_response();
            }
        };

        let mut response = Response::new(Body::new(FeishuPostResponseJsonBody {
            bytes: Some(Bytes::from(body_bytes)),
            post_response_dispatch: self.post_response_dispatch,
        }));
        *response.status_mut() = StatusCode::OK;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        response
    }
}

impl FeishuWebhookPostResponseDispatch {
    fn spawn(self) {
        dispatch_deferred_feishu_card_updates(self.config, self.deferred_updates);
    }
}

impl HttpBody for FeishuPostResponseJsonBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if let Some(bytes) = this.bytes.take() {
            return Poll::Ready(Some(Ok(Frame::data(bytes))));
        }
        if let Some(dispatch) = this.post_response_dispatch.take() {
            dispatch.spawn();
        }
        Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        self.bytes.is_none() && self.post_response_dispatch.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::new();
        hint.set_exact(self.bytes.as_ref().map_or(0, |bytes| bytes.len() as u64));
        hint
    }
}

impl Drop for FeishuPostResponseJsonBody {
    fn drop(&mut self) {
        if let Some(dispatch) = self.post_response_dispatch.take() {
            dispatch.spawn();
        }
    }
}

impl RecentIdCache {
    fn new(max_len: usize) -> Self {
        Self {
            max_len: max_len.max(1),
            queue: VecDeque::new(),
            states: std::collections::BTreeMap::new(),
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

pub(super) async fn feishu_webhook_handler(
    State(state): State<FeishuWebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    tracing::debug!(
        target: "loongclaw.channel.feishu",
        transport = "webhook",
        configured_account_id = %state.configured_account_id,
        content_length = body.len(),
        has_signature = headers.contains_key("X-Lark-Signature"),
        "received feishu webhook request"
    );

    let body_text = match std::str::from_utf8(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "code": StatusCode::BAD_REQUEST.as_u16(),
                    "msg": format!("invalid utf-8 request body: {error}"),
                })),
            )
                .into_response();
        }
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "code": StatusCode::BAD_REQUEST.as_u16(),
                    "msg": format!("invalid JSON request body: {error}"),
                })),
            )
                .into_response();
        }
    };

    match handle_feishu_webhook_payload(state, &headers, body_text, payload).await {
        Ok(reply) => reply.into_response(),
        Err((status, message)) => (
            status,
            Json(json!({
                "code": status.as_u16(),
                "msg": message,
            })),
        )
            .into_response(),
    }
}

async fn handle_feishu_webhook_payload(
    state: FeishuWebhookState,
    headers: &HeaderMap,
    raw_body: &str,
    payload: Value,
) -> Result<FeishuWebhookSuccessResponse, (StatusCode, String)> {
    verify_feishu_signature(headers, raw_body, &payload, state.encrypt_key.as_deref())?;

    let parsed = super::payload::parse_feishu_webhook_payload_with_access_policy(
        &payload,
        state.verification_token.as_deref(),
        state.encrypt_key.as_deref(),
        &state.access_policy,
        state.ignore_bot_messages,
        state.configured_account_id.as_str(),
        state.account_id.as_str(),
    )
    .map_err(map_feishu_parse_error)?;

    let response = handle_feishu_parsed_action(&state, parsed).await?;
    Ok(FeishuWebhookSuccessResponse::from_parsed_response(
        response,
        state.config.clone(),
    ))
}

pub(super) async fn handle_feishu_parsed_action(
    state: &FeishuWebhookState,
    parsed: FeishuWebhookAction,
) -> Result<FeishuParsedActionResponse, (StatusCode, String)> {
    match parsed {
        FeishuWebhookAction::UrlVerification { challenge } => {
            tracing::debug!(
                target: "loongclaw.channel.feishu",
                transport = "webhook",
                configured_account_id = %state.configured_account_id,
                "accepted feishu url verification request"
            );
            Ok(FeishuParsedActionResponse::immediate(
                json!({ "challenge": challenge }),
            ))
        }
        FeishuWebhookAction::Ignore => Ok(FeishuParsedActionResponse::immediate(
            json!({"code": 0, "msg": "ignored"}),
        )),
        FeishuWebhookAction::CardCallback(event) => {
            tracing::info!(
                target: "loongclaw.channel.feishu",
                transport = "webhook",
                action = "card_callback",
                configured_account_id = %state.configured_account_id,
                event_id = %event.event_id,
                conversation_id = %event.session.conversation_id,
                has_open_message_id = event.context.open_message_id.is_some(),
                has_open_chat_id = event.context.open_chat_id.is_some(),
                has_principal = event.principal.is_some(),
                "accepted feishu card callback event"
            );
            {
                let mut dedupe = state.seen_events.lock().await;
                let reservation = dedupe.begin_processing(&event.event_id);
                if !matches!(reservation, RecentIdReservation::Accepted) {
                    tracing::debug!(
                        target: "loongclaw.channel.feishu",
                        transport = "webhook",
                        action = "card_callback",
                        configured_account_id = %state.configured_account_id,
                        event_id = %event.event_id,
                        reservation = ?reservation,
                        "deduplicated feishu card callback event"
                    );
                    return Ok(FeishuParsedActionResponse::immediate(
                        FeishuCallbackResponse::Noop.as_json(),
                    ));
                }
            }

            let event_id = event.event_id.clone();
            let response = handle_feishu_card_callback_event(state, &event).await;

            {
                let mut dedupe = state.seen_events.lock().await;
                dedupe.mark_completed(&event_id);
            }

            Ok(response)
        }
        FeishuWebhookAction::Inbound(event) => {
            tracing::info!(
                target: "loongclaw.channel.feishu",
                transport = "webhook",
                action = "inbound",
                configured_account_id = %state.configured_account_id,
                event_id = %event.event_id,
                message_id = %event.message_id,
                conversation_id = %event.session.conversation_id,
                has_thread = event.session.thread_id.is_some(),
                has_principal = event.principal.is_some(),
                resource_count = event.resources.len(),
                "accepted feishu inbound event"
            );
            {
                let mut dedupe = state.seen_events.lock().await;
                let reservation = dedupe.begin_processing(&event.event_id);
                if !matches!(reservation, RecentIdReservation::Accepted) {
                    tracing::debug!(
                        target: "loongclaw.channel.feishu",
                        transport = "webhook",
                        action = "inbound",
                        configured_account_id = %state.configured_account_id,
                        event_id = %event.event_id,
                        reservation = ?reservation,
                        "deduplicated feishu inbound event"
                    );
                    return Ok(FeishuParsedActionResponse::immediate(
                        json!({"code": 0, "msg": "duplicate_event"}),
                    ));
                }
            }

            let event_id = event.event_id.clone();
            let result = handle_feishu_inbound_event(state, event).await;

            {
                let mut dedupe = state.seen_events.lock().await;
                if result.is_ok() {
                    dedupe.mark_completed(&event_id);
                } else {
                    dedupe.release(&event_id);
                }
            }

            result
        }
    }
}

async fn handle_feishu_card_callback_event(
    state: &FeishuWebhookState,
    event: &FeishuCardCallbackEvent,
) -> FeishuParsedActionResponse {
    if let Err(error) = state.runtime.mark_run_start().await {
        log_feishu_callback_warning("runtime start failed", &error);
        return FeishuParsedActionResponse::immediate(FeishuCallbackResponse::Noop.as_json());
    }

    let inbound = build_feishu_card_callback_inbound_message(event);
    let mut callback_response = FeishuCallbackResponse::Noop.as_json();
    if let Err(error) = process_inbound_with_provider(
        &state.config,
        state.resolved_path.as_deref(),
        &inbound,
        state.kernel_ctx.as_ref(),
        ChannelTurnFeedbackPolicy::disabled(),
    )
    .await
    .map(|reply| {
        if let Some(response) = parse_feishu_structured_callback_response(&reply) {
            callback_response = response.as_json();
        }
    }) {
        log_feishu_callback_warning("provider processing failed", &error);
    }
    let deferred_updates =
        crate::tools::drain_deferred_feishu_card_updates(event.event_id.as_str());

    if let Err(error) = state.runtime.mark_run_end().await {
        log_feishu_callback_warning("runtime end failed", &error);
    }

    FeishuParsedActionResponse::with_deferred_card_updates(callback_response, deferred_updates)
}

async fn handle_feishu_inbound_event(
    state: &FeishuWebhookState,
    event: super::payload::FeishuInboundEvent,
) -> Result<FeishuParsedActionResponse, (StatusCode, String)> {
    let inbound_event_id = event.event_id.clone();
    let inbound_message_id = event.message_id.clone();
    let inbound_conversation_id = event.session.conversation_id.clone();

    state.runtime.mark_run_start().await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("channel runtime start failed: {error}"),
        )
    })?;

    let result = async {
        let inbound_message_id = event.message_id.clone();
        maybe_send_feishu_ack_reaction_nonblocking(state, inbound_message_id.as_str()).await;
        let channel_message = ChannelInboundMessage {
            session: event.session,
            reply_target: event.reply_target,
            text: event.text,
            delivery: crate::channel::ChannelDelivery {
                ack_cursor: None,
                source_message_id: Some(inbound_message_id.clone()),
                sender_principal_key: event.principal.as_ref().map(|value| value.storage_key()),
                thread_root_id: event.root_id,
                parent_message_id: event.parent_id,
                resources: event.resources,
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
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("provider processing failed: {error}"),
            )
        })?;
        let reply_target = channel_message.reply_target.clone();
        let outbound = outbound_reply_message_from_text(reply);

        let mut adapter = state.adapter.lock().await;
        if let Err(first_error) =
            send_channel_message_via_message_send_api(&*adapter, &reply_target, outbound.clone())
                .await
        {
            adapter.refresh_tenant_token().await.map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "feishu token refresh failed after send error `{first_error}`: {error}"
                    ),
                )
            })?;
            send_channel_message_via_message_send_api(&*adapter, &reply_target, outbound)
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("feishu reply failed after token refresh: {error}"),
                    )
                })?;
        }
        Ok(FeishuParsedActionResponse::immediate(
            json!({"code": 0, "msg": "ok"}),
        ))
    }
    .await;

    if result.is_ok() {
        tracing::info!(
            target: "loongclaw.channel.feishu",
            transport = "webhook",
            action = "inbound",
            configured_account_id = %state.configured_account_id,
            event_id = %inbound_event_id,
            message_id = %inbound_message_id,
            conversation_id = %inbound_conversation_id,
            "feishu inbound event processed successfully"
        );
    }

    if let Err(error) = state.runtime.mark_run_end().await {
        log_feishu_inbound_warning("runtime end failed", &error);
    }

    result
}

async fn maybe_send_feishu_ack_reaction_nonblocking(state: &FeishuWebhookState, message_id: &str) {
    if !state.ack_reactions {
        return;
    }
    let message_id = message_id.trim();
    if message_id.is_empty() {
        return;
    }

    {
        let mut dedupe = state.seen_ack_reactions.lock().await;
        if !matches!(
            dedupe.begin_processing(message_id),
            RecentIdReservation::Accepted
        ) {
            return;
        }
    }

    let message_id = message_id.to_owned();
    let adapter = Arc::clone(&state.adapter);
    let seen_ack_reactions = Arc::clone(&state.seen_ack_reactions);
    tokio::spawn(async move {
        let result = {
            let adapter = adapter.lock().await;
            adapter.add_ack_reaction(message_id.as_str()).await
        };

        let mut dedupe = seen_ack_reactions.lock().await;
        match result {
            Ok(()) => dedupe.mark_completed(message_id.as_str()),
            Err(error) => {
                dedupe.release(message_id.as_str());
                log_feishu_inbound_warning("ack reaction failed", &error);
            }
        }
    });
}

fn parse_feishu_structured_callback_response(text: &str) -> Option<FeishuCallbackResponse> {
    let payload = text
        .trim()
        .strip_prefix(FEISHU_CALLBACK_RESPONSE_MARKER)?
        .trim();
    let response = serde_json::from_str::<FeishuStructuredCallbackResponse>(payload).ok()?;
    match response.mode.trim().to_ascii_lowercase().as_str() {
        "toast" => {
            if response.toast.is_some() || response.card.is_some() {
                return None;
            }
            let toast = parse_feishu_callback_toast(
                response.kind.as_deref()?,
                response.content.as_deref()?,
            )?;
            Some(FeishuCallbackResponse::Toast {
                kind: toast.kind,
                content: toast.content,
            })
        }
        "card" => {
            if response.kind.is_some() || response.content.is_some() {
                return None;
            }
            let card = match (response.card, response.markdown) {
                (Some(Value::Object(map)), None) => Value::Object(map),
                (None, Some(markdown)) => {
                    let markdown = markdown.trim();
                    if markdown.is_empty() {
                        return None;
                    }
                    cards::build_markdown_card(markdown)
                }
                _ => return None,
            };
            let toast = match response.toast {
                Some(FeishuStructuredCallbackToast { kind, content }) => {
                    Some(parse_feishu_callback_toast(&kind, &content)?)
                }
                None => None,
            };

            Some(FeishuCallbackResponse::Card { toast, card })
        }
        _ => None,
    }
}

fn parse_feishu_callback_toast(kind: &str, content: &str) -> Option<FeishuCallbackToast> {
    let kind = match kind.trim().to_ascii_lowercase().as_str() {
        "success" => "success",
        "info" => "info",
        "warning" => "warning",
        "error" => "error",
        _ => return None,
    };
    let content = content.trim();
    if content.is_empty() {
        return None;
    }

    Some(FeishuCallbackToast {
        kind,
        content: content.to_owned(),
    })
}

fn build_feishu_card_callback_inbound_message(
    event: &FeishuCardCallbackEvent,
) -> ChannelInboundMessage {
    let reply_target = if let Some(message_id) = event.context.open_message_id.as_deref() {
        let mut target = ChannelOutboundTarget::feishu_message_reply(message_id.to_owned())
            .with_feishu_reply_in_thread(true);
        if let Some(chat_id) = event.context.open_chat_id.as_deref() {
            target = target.with_feishu_reply_chat_id(chat_id.to_owned());
        } else {
            target = target.with_feishu_reply_chat_id(event.session.conversation_id.clone());
        }
        target
    } else if let Some(chat_id) = event.context.open_chat_id.as_deref() {
        ChannelOutboundTarget::feishu_receive_id(chat_id.to_owned())
            .with_feishu_receive_id_type("chat_id")
    } else {
        ChannelOutboundTarget::feishu_receive_id(event.session.conversation_id.clone())
    };

    ChannelInboundMessage {
        session: event.session.clone(),
        reply_target,
        text: event.text.clone(),
        delivery: crate::channel::ChannelDelivery {
            ack_cursor: None,
            source_message_id: event.context.open_message_id.clone(),
            sender_principal_key: event.principal.as_ref().map(|value| value.storage_key()),
            thread_root_id: event.context.open_message_id.clone(),
            parent_message_id: None,
            resources: Vec::new(),
            feishu_callback: Some(crate::channel::ChannelDeliveryFeishuCallback {
                callback_token: event.callback_token.clone(),
                open_message_id: event.context.open_message_id.clone(),
                open_chat_id: event.context.open_chat_id.clone(),
                operator_open_id: event.principal.as_ref().map(|value| value.open_id.clone()),
                deferred_context_id: Some(event.event_id.clone()),
            }),
        },
    }
}

fn dispatch_deferred_feishu_card_updates(
    config: LoongClawConfig,
    updates: Vec<crate::tools::DeferredFeishuCardUpdate>,
) {
    if updates.is_empty() {
        return;
    }

    for update in updates {
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(error) = execute_deferred_feishu_card_update(config, update).await {
                log_feishu_callback_warning("deferred card update failed", &error);
            }
        });
    }
}

async fn execute_deferred_feishu_card_update(
    config: LoongClawConfig,
    update: crate::tools::DeferredFeishuCardUpdate,
) -> crate::CliResult<()> {
    let resolved = config
        .feishu
        .resolve_account(Some(update.configured_account_id.as_str()))?;
    let client = FeishuClient::from_configs(&resolved, &config.feishu_integration)?;
    let tenant_access_token = client.get_tenant_access_token().await?;
    cards::delay_update_message_card(
        &client,
        &tenant_access_token,
        &cards::FeishuCardUpdateRequest {
            token: update.token,
            card: update.card,
            open_ids: update.open_ids,
        },
    )
    .await?;
    Ok(())
}

fn log_feishu_callback_warning(context: &str, error: &str) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("warning: feishu card callback {context}: {error}");
    }
}

fn log_feishu_inbound_warning(context: &str, error: &str) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("warning: feishu inbound {context}: {error}");
    }
}

fn map_feishu_parse_error(error: String) -> (StatusCode, String) {
    if let Some(message) = error.strip_prefix("unauthorized:") {
        return (StatusCode::UNAUTHORIZED, message.trim().to_owned());
    }
    (StatusCode::BAD_REQUEST, error)
}

fn verify_feishu_signature(
    headers: &HeaderMap,
    raw_body: &str,
    payload: &Value,
    encrypt_key: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        return Ok(());
    }

    let Some(encrypt_key) = encrypt_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "unauthorized: feishu encrypt key is not configured".to_owned(),
        ));
    };

    let timestamp = read_header_required(headers, "X-Lark-Request-Timestamp")?;
    let nonce = read_header_required(headers, "X-Lark-Request-Nonce")?;
    let signature = read_header_required(headers, "X-Lark-Signature")?;

    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(encrypt_key.as_bytes());
    hasher.update(raw_body.as_bytes());
    let expected = hex::encode(hasher.finalize());

    if !timing_safe_eq(expected.as_bytes(), signature.as_bytes()) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "unauthorized: feishu signature mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn read_header_required<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, (StatusCode, String)> {
    let value = headers
        .get(name)
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                format!("unauthorized: missing required header `{name}`"),
            )
        })?
        .to_str()
        .map_err(|error| {
            (
                StatusCode::UNAUTHORIZED,
                format!("unauthorized: invalid header `{name}`: {error}"),
            )
        })?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!("unauthorized: empty required header `{name}`"),
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelPlatform;
    use crate::channel::runtime::state::start_channel_operation_runtime_tracker_for_test;
    use crate::config::{LoongClawConfig, ProviderConfig};
    use crate::context::{DEFAULT_TOKEN_TTL_S, KernelContext, bootstrap_test_kernel_context};
    use crate::tools::runtime_config::ToolRuntimeConfig;
    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        response::IntoResponse,
        routing::post,
    };
    use loongclaw_contracts::Capability;
    use loongclaw_kernel::{
        ExecutionRoute, HarnessKind, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine,
        SystemClock, VerticalPackManifest,
    };
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::sync::Mutex;

    const MOCK_PROVIDER_MARKDOWN_REPLY: &str = "## structured inbound ack\n\n- rendered";
    const FEISHU_WEBHOOK_TEST_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockRequest {
        path: String,
        query: Option<String>,
        authorization: Option<String>,
        body: String,
    }

    #[derive(Clone, Default)]
    struct MockServerState {
        requests: Arc<Mutex<Vec<MockRequest>>>,
    }

    fn temp_webhook_test_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-feishu-webhook-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn run_feishu_webhook_test_on_large_stack<F, Fut>(thread_name: &str, operation: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let join_handle = std::thread::Builder::new()
            .name(thread_name.to_owned())
            .stack_size(FEISHU_WEBHOOK_TEST_STACK_SIZE_BYTES)
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build feishu webhook test runtime");
                runtime.block_on(operation());
            })
            .expect("spawn feishu webhook large-stack test thread");
        match join_handle.join() {
            Ok(()) => {}
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    fn webhook_tool_runtime_config(config: &LoongClawConfig) -> ToolRuntimeConfig {
        ToolRuntimeConfig::from_loongclaw_config(config, None)
    }

    fn bootstrap_webhook_kernel_context(
        agent_id: &str,
        ttl_s: u64,
        config: &LoongClawConfig,
    ) -> Result<KernelContext, String> {
        let mut kernel = LoongClawKernel::with_runtime(
            StaticPolicyEngine::default(),
            Arc::new(SystemClock),
            Arc::new(InMemoryAuditSink::default()),
        );
        kernel
            .register_pack(VerticalPackManifest {
                pack_id: "dev-automation".to_owned(),
                domain: "mvp".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: None,
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([
                    Capability::InvokeTool,
                    Capability::MemoryRead,
                    Capability::MemoryWrite,
                ]),
                metadata: BTreeMap::new(),
            })
            .map_err(|error| format!("kernel pack registration failed: {error}"))?;

        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config =
                crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                    &config.memory,
                );
            kernel.register_core_memory_adapter(crate::memory::MvpMemoryAdapter::with_config(
                memory_config,
            ));
            kernel
                .set_default_core_memory_adapter("mvp-memory")
                .map_err(|error| format!("set default memory adapter failed: {error}"))?;
        }

        kernel.register_core_tool_adapter(crate::tools::MvpToolAdapter::with_config(
            webhook_tool_runtime_config(config),
        ));
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .map_err(|error| format!("set default tool adapter failed: {error}"))?;

        let token = kernel
            .issue_token("dev-automation", agent_id, ttl_s)
            .map_err(|error| format!("kernel token issue failed: {error}"))?;

        Ok(KernelContext {
            kernel: Arc::new(kernel),
            token,
        })
    }

    async fn spawn_mock_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let address = listener.local_addr().expect("mock server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock server");
        });
        (format!("http://{address}"), handle)
    }

    async fn record_request(State(state): State<MockServerState>, request: Request) -> String {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, usize::MAX)
            .await
            .expect("read mock request body");
        let body_text = String::from_utf8(body.to_vec()).expect("mock request body utf8");
        state.requests.lock().await.push(MockRequest {
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(ToOwned::to_owned),
            authorization: parts
                .headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body: body_text.clone(),
        });
        body_text
    }

    fn mock_provider_stream_enabled(body: &str) -> bool {
        serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|payload| payload.get("stream").and_then(Value::as_bool))
            .unwrap_or(false)
    }

    fn mock_provider_stream_response_body(response_text: &str) -> String {
        format!(
            "data: {}\n\n\
data: {}\n\n\
data: [DONE]\n\n",
            json!({
                "choices": [{
                    "delta": {
                        "content": response_text
                    }
                }]
            }),
            json!({
                "choices": [{
                    "delta": {},
                    "finish_reason": "stop"
                }]
            }),
        )
    }

    fn mock_provider_success_response(
        request_body: &str,
        response_text: &str,
    ) -> axum::response::Response {
        if mock_provider_stream_enabled(request_body) {
            return (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                mock_provider_stream_response_body(response_text),
            )
                .into_response();
        }

        Json(json!({
            "choices": [{
                "message": {
                    "content": response_text
                }
            }]
        }))
        .into_response()
    }

    async fn wait_for_request_count(
        requests: &Arc<Mutex<Vec<MockRequest>>>,
        expected_len: usize,
    ) -> Vec<MockRequest> {
        for _ in 0..50 {
            let snapshot = requests.lock().await.clone();
            if snapshot.len() >= expected_len {
                return snapshot;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        requests.lock().await.clone()
    }

    async fn spawn_mock_provider_server(
        requests: Arc<Mutex<Vec<MockRequest>>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = MockServerState { requests };
        let router = Router::new().route(
            "/v1/chat/completions",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        let request_body = record_request(State(state), request).await;
                        mock_provider_success_response(
                            request_body.as_str(),
                            MOCK_PROVIDER_MARKDOWN_REPLY,
                        )
                    }
                }
            }),
        );
        spawn_mock_server(router).await
    }

    async fn spawn_mock_provider_callback_toast_server(
        requests: Arc<Mutex<Vec<MockRequest>>>,
        response_text: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = MockServerState { requests };
        let router = Router::new().route(
            "/v1/chat/completions",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        let request_body = record_request(State(state), request).await;
                        mock_provider_success_response(request_body.as_str(), response_text)
                    }
                }
            }),
        );
        spawn_mock_server(router).await
    }

    async fn spawn_mock_provider_failure_server(
        requests: Arc<Mutex<Vec<MockRequest>>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = MockServerState { requests };
        let router = Router::new().route(
            "/v1/chat/completions",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "error": {
                                    "message": "provider offline"
                                }
                            })),
                        )
                    }
                }
            }),
        );
        spawn_mock_server(router).await
    }

    async fn spawn_mock_provider_delayed_success_server(
        requests: Arc<Mutex<Vec<MockRequest>>>,
        delay: std::time::Duration,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = MockServerState { requests };
        let router = Router::new().route(
            "/v1/chat/completions",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        let request_body = record_request(State(state), request).await;
                        tokio::time::sleep(delay).await;
                        mock_provider_success_response(
                            request_body.as_str(),
                            MOCK_PROVIDER_MARKDOWN_REPLY,
                        )
                    }
                }
            }),
        );
        spawn_mock_server(router).await
    }

    async fn spawn_mock_provider_card_update_server(
        requests: Arc<Mutex<Vec<MockRequest>>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = MockServerState { requests };
        let turn_index = Arc::new(Mutex::new(0usize));
        let router = Router::new().route(
            "/v1/chat/completions",
            post({
                let state = state.clone();
                let turn_index = turn_index.clone();
                move |request: Request| {
                    let state = state.clone();
                    let turn_index = turn_index.clone();
                    async move {
                        let (parts, body) = request.into_parts();
                        let body_bytes = to_bytes(body, usize::MAX)
                            .await
                            .expect("read mock request body");
                        let body_text = String::from_utf8(body_bytes.to_vec())
                            .expect("mock request body utf8");
                        state.requests.lock().await.push(MockRequest {
                            path: parts.uri.path().to_owned(),
                            query: parts.uri.query().map(ToOwned::to_owned),
                            authorization: parts
                                .headers
                                .get(axum::http::header::AUTHORIZATION)
                                .and_then(|value: &axum::http::HeaderValue| value.to_str().ok())
                                .map(ToOwned::to_owned),
                            body: body_text.clone(),
                        });
                        let mut turn_index = turn_index.lock().await;
                        *turn_index += 1;
                        if *turn_index == 1 {
                            Json(json!({
                                "choices": [{
                                    "message": {
                                        "content": "looking up the right callback card tool",
                                        "tool_calls": [{
                                            "id": "call_tool_search_1",
                                            "type": "function",
                                            "function": {
                                                "name": "tool_search",
                                                "arguments": "{\"query\":\"feishu card update callback token markdown\",\"limit\":1}"
                                            }
                                        }]
                                    }
                                }]
                            }))
                        } else if *turn_index == 2 {
                            // In the discovery-first model, the provider must call
                            // tool_invoke with the lease obtained from tool_search.
                            // Extract the lease from the tool_search result in the
                            // request body.
                            let lease = extract_lease_from_provider_request_body(
                                &body_text,
                                "feishu.card.update",
                            );
                            let arguments = json!({
                                "tool_id": "feishu.card.update",
                                "lease": lease,
                                "arguments": {
                                    "card": {
                                        "config": {"wide_screen_mode": true},
                                        "elements": [{"tag": "markdown", "content": "callback updated"}]
                                    }
                                }
                            });
                            Json(json!({
                                "choices": [{
                                    "message": {
                                        "content": "updating card",
                                        "tool_calls": [{
                                            "id": "call_feishu_card_update_1",
                                            "type": "function",
                                            "function": {
                                                "name": "tool_invoke",
                                                "arguments": serde_json::to_string(&arguments).expect("serialize tool_invoke arguments")
                                            }
                                        }]
                                    }
                                }]
                            }))
                        } else {
                            Json(json!({
                                "choices": [{
                                    "message": {
                                        "content": "card updated"
                                    }
                                }]
                            }))
                        }
                    }
                }
            }),
        );
        spawn_mock_server(router).await
    }

    /// Extract a tool lease for the given tool_id from a provider request body
    /// that contains a previous tool_search result in the conversation messages.
    fn extract_lease_from_provider_request_body(body: &str, tool_id: &str) -> String {
        let body_json: Value = serde_json::from_str(body).expect("parse provider request body");
        let messages = body_json["messages"]
            .as_array()
            .expect("messages array in provider request body");
        for message in messages {
            let content = message["content"].as_str().unwrap_or("");
            if !content.contains("tool.search") || !content.contains(tool_id) {
                continue;
            }
            for line in content.lines() {
                let Some(payload) = line.trim().strip_prefix("[ok] ") else {
                    continue;
                };
                let Ok(envelope) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                if envelope["tool"].as_str() != Some("tool.search") {
                    continue;
                }
                if envelope["payload_truncated"].as_bool().unwrap_or(false) {
                    continue;
                }
                let summary_str = envelope["payload_summary"].as_str().unwrap_or("");
                let Ok(summary) = serde_json::from_str::<Value>(summary_str) else {
                    continue;
                };
                let Some(results) = summary["results"].as_array() else {
                    continue;
                };
                for result in results {
                    if result["tool_id"].as_str() == Some(tool_id)
                        && let Some(lease) = result["lease"].as_str()
                    {
                        return lease.to_owned();
                    }
                }
            }
        }
        panic!("could not extract lease for {tool_id} from provider request body");
    }

    #[test]
    fn extract_lease_from_provider_request_body_accepts_reordered_search_envelope_fields() {
        let payload_summary = serde_json::to_string(&json!({
            "query": "feishu card update callback token markdown",
            "results": [{
                "tool_id": "feishu.card.update",
                "summary": "Update a Feishu interactive card after a card callback.",
                "argument_hint": "callback_token?:string,card?:object,markdown?:string",
                "lease": "lease-feishu-card-update"
            }]
        }))
        .expect("encode payload summary");
        let body = serde_json::to_string(&json!({
            "messages": [{
                "role": "assistant",
                "content": format!(
                    "[tool_result]\n[ok] {}",
                    json!({
                        "payload_chars": 256,
                        "payload_summary": payload_summary,
                        "payload_truncated": false,
                        "status": "ok",
                        "tool": "tool.search",
                        "tool_call_id": "call_tool_search_1",
                    })
                )
            }]
        }))
        .expect("encode provider request body");

        assert_eq!(
            extract_lease_from_provider_request_body(body.as_str(), "feishu.card.update"),
            "lease-feishu-card-update"
        );
    }

    #[test]
    fn extract_lease_from_provider_request_body_ignores_non_search_envelopes() {
        let misleading_summary = serde_json::to_string(&json!({
            "results": [{
                "tool_id": "feishu.card.update",
                "lease": "lease-from-non-search"
            }]
        }))
        .expect("encode misleading payload summary");
        let search_summary = serde_json::to_string(&json!({
            "query": "feishu card update callback token markdown",
            "results": [{
                "tool_id": "feishu.card.update",
                "summary": "Update a Feishu interactive card after a card callback.",
                "argument_hint": "callback_token?:string,card?:object,markdown?:string",
                "lease": "lease-from-search"
            }]
        }))
        .expect("encode search payload summary");
        let body = serde_json::to_string(&json!({
            "messages": [{
                "role": "assistant",
                "content": format!(
                    "[tool_result]\n[ok] {}\n[ok] {}",
                    json!({
                        "status": "ok",
                        "tool": "file.read",
                        "tool_call_id": "call_file_read_1",
                        "payload_summary": misleading_summary,
                        "payload_chars": 64,
                        "payload_truncated": false,
                    }),
                    json!({
                        "status": "ok",
                        "tool": "tool.search",
                        "tool_call_id": "call_tool_search_1",
                        "payload_summary": search_summary,
                        "payload_chars": 256,
                        "payload_truncated": false,
                    })
                )
            }]
        }))
        .expect("encode provider request body");

        assert_eq!(
            extract_lease_from_provider_request_body(body.as_str(), "feishu.card.update"),
            "lease-from-search"
        );
    }

    async fn spawn_mock_feishu_api_server(
        requests: Arc<Mutex<Vec<MockRequest>>>,
        reply_message_id: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = MockServerState { requests };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "tenant_access_token": "t-token-webhook"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/interactive/v1/card/update",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "msg": "ok"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/{message_id}/reply",
                post({
                    let state = state.clone();
                    move |axum::extract::Path(message_id): axum::extract::Path<String>, request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "message_id": reply_message_id,
                                    "root_id": message_id
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/{message_id}/reactions",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "reaction_id": "reaction_webhook_1"
                                }
                            }))
                        }
                    }
                }),
            );
        spawn_mock_server(router).await
    }

    async fn spawn_mock_feishu_api_server_with_failing_reactions(
        requests: Arc<Mutex<Vec<MockRequest>>>,
        reply_message_id: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = MockServerState { requests };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "tenant_access_token": "t-token-webhook"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/interactive/v1/card/update",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "msg": "ok"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/{message_id}/reply",
                post({
                    let state = state.clone();
                    move |axum::extract::Path(message_id): axum::extract::Path<String>, request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "message_id": reply_message_id,
                                    "root_id": message_id
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/{message_id}/reactions",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 99991663,
                                "msg": "reaction failed"
                            }))
                        }
                    }
                }),
            );
        spawn_mock_server(router).await
    }

    fn test_webhook_config(provider_base_url: &str, feishu_base_url: &str) -> LoongClawConfig {
        let temp_dir = temp_webhook_test_dir("runtime");
        std::fs::create_dir_all(&temp_dir).expect("create webhook temp dir");

        let mut config = LoongClawConfig {
            provider: ProviderConfig {
                base_url: provider_base_url.to_owned(),
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "test-provider-key".to_owned(),
                )),
                model: "test-model".to_owned(),
                ..ProviderConfig::default()
            },
            ..LoongClawConfig::default()
        };
        config.memory.sqlite_path = temp_dir.join("memory.sqlite3").display().to_string();
        config.tools.file_root = Some(temp_dir.join("tool-root").display().to_string());
        config.feishu.enabled = true;
        config.feishu.account_id = Some("feishu_main".to_owned());
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
            "cli_a1b2c3".to_owned(),
        ));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "secret-123".to_owned(),
        ));
        config.feishu.base_url = Some(feishu_base_url.to_owned());
        config.feishu.receive_id_type = "chat_id".to_owned();
        config.feishu.allowed_chat_ids = vec!["oc_demo".to_owned()];
        config.feishu.verification_token = Some(loongclaw_contracts::SecretRef::Inline(
            "verify-token".to_owned(),
        ));
        config.feishu.encrypt_key = Some(loongclaw_contracts::SecretRef::Inline(
            "encrypt-key".to_owned(),
        ));
        config
    }

    fn signed_headers(body: &str, encrypt_key: &str) -> HeaderMap {
        let timestamp = "1736480000";
        let nonce = "nonce-1";
        let mut hasher = Sha256::new();
        hasher.update(timestamp.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(encrypt_key.as_bytes());
        hasher.update(body.as_bytes());
        let signature = hex::encode(hasher.finalize());

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Lark-Request-Timestamp",
            timestamp.parse().expect("timestamp header"),
        );
        headers.insert("X-Lark-Request-Nonce", nonce.parse().expect("nonce header"));
        headers.insert(
            "X-Lark-Signature",
            signature.parse().expect("signature header"),
        );
        headers
    }

    #[test]
    fn recent_cache_deduplicates_and_rolls_window() {
        let mut cache = RecentIdCache::new(2);
        assert!(matches!(
            cache.begin_processing("a"),
            RecentIdReservation::Accepted
        ));
        cache.mark_completed("a");
        assert!(matches!(
            cache.begin_processing("a"),
            RecentIdReservation::CompletedDuplicate
        ));
        assert!(matches!(
            cache.begin_processing("b"),
            RecentIdReservation::Accepted
        ));
        cache.mark_completed("b");
        assert!(matches!(
            cache.begin_processing("c"),
            RecentIdReservation::Accepted
        ));
        cache.mark_completed("c");
        assert!(matches!(
            cache.begin_processing("a"),
            RecentIdReservation::Accepted
        ));
    }

    #[test]
    fn recent_cache_releases_failed_events_for_retry() {
        let mut cache = RecentIdCache::new(4);

        assert!(matches!(
            cache.begin_processing("evt-1"),
            RecentIdReservation::Accepted
        ));
        assert!(matches!(
            cache.begin_processing("evt-1"),
            RecentIdReservation::InProgressDuplicate
        ));

        cache.release("evt-1");

        assert!(matches!(
            cache.begin_processing("evt-1"),
            RecentIdReservation::Accepted
        ));
    }

    #[test]
    fn signature_verification_passes_with_valid_headers() {
        let body = r#"{"encrypt":"opaque"}"#;
        let encrypt_key = "test-encrypt-key";
        let timestamp = "1736480000";
        let nonce = "nonce-1";

        let mut hasher = Sha256::new();
        hasher.update(timestamp.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(encrypt_key.as_bytes());
        hasher.update(body.as_bytes());
        let signature = hex::encode(hasher.finalize());

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Lark-Request-Timestamp",
            timestamp.parse().expect("header"),
        );
        headers.insert("X-Lark-Request-Nonce", nonce.parse().expect("header"));
        headers.insert("X-Lark-Signature", signature.parse().expect("header"));

        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let result = verify_feishu_signature(&headers, body, &payload, Some(encrypt_key));
        assert!(result.is_ok());
    }

    #[test]
    fn signature_verification_rejects_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Lark-Request-Timestamp", "1".parse().expect("header"));
        headers.insert("X-Lark-Request-Nonce", "n".parse().expect("header"));
        headers.insert("X-Lark-Signature", "deadbeef".parse().expect("header"));

        let body = "{\"encrypt\":\"x\"}";
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let error =
            verify_feishu_signature(&headers, body, &payload, Some("key")).expect_err("mismatch");
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn signature_verification_requires_encrypt_key_for_event_payloads() {
        let headers = HeaderMap::new();
        let body = "{\"header\":{\"event_type\":\"im.message.receive_v1\"}}";
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let error = verify_feishu_signature(&headers, body, &payload, None)
            .expect_err("missing encrypt key should fail");
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
        assert!(error.1.contains("encrypt key is not configured"));
    }

    #[test]
    fn signature_verification_skips_url_verification_payload() {
        let headers = HeaderMap::new();
        let body = r#"{"type":"url_verification","token":"token","challenge":"ok"}"#;
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let result = verify_feishu_signature(&headers, body, &payload, Some("encrypt-key"));
        assert!(result.is_ok());
    }

    #[test]
    fn feishu_webhook_file_event_reaches_provider_as_structured_text_and_replies() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-file-event", || async move {
            feishu_webhook_file_event_reaches_provider_as_structured_text_and_replies_impl().await;
        });
    }

    async fn feishu_webhook_file_event_reaches_provider_as_structured_text_and_replies_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_1").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before webhook test");
        let kernel_ctx = bootstrap_test_kernel_context("feishu-webhook-test", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "token": "verify-token",
            "header": {
                "event_id": "evt_file_end_to_end",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": "ou_sender_1"
                    }
                },
                "message": {
                    "chat_id": "oc_demo",
                    "message_id": "om_inbound_file_1",
                    "message_type": "file",
                    "content": "{\"file_key\":\"file_v2_demo\",\"file_name\":\"report.pdf\"}"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("webhook should succeed");

        assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

        let provider_requests = provider_requests.lock().await.clone();
        assert_eq!(provider_requests.len(), 1);
        assert_eq!(provider_requests[0].path, "/v1/chat/completions");
        let provider_body =
            serde_json::from_str::<Value>(&provider_requests[0].body).expect("provider body json");
        assert_eq!(provider_body["stream"], json!(true));
        let provider_user_content = provider_body
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            })
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("provider user content");
        assert!(
            provider_user_content.contains("[feishu_inbound_message]"),
            "provider should receive the structured feishu marker"
        );
        assert!(
            provider_user_content.contains("\"message_type\":\"file\""),
            "provider should receive the structured file message type"
        );
        assert!(
            provider_user_content.contains("\"file_key\":\"file_v2_demo\""),
            "provider should receive the feishu file key"
        );
        assert!(
            provider_user_content.contains("Binary file content is not fetched automatically."),
            "provider should receive the binary fetch note"
        );

        let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
        assert_eq!(feishu_requests.len(), 3);
        let reaction_request = feishu_requests
            .iter()
            .find(|request| request.path == "/open-apis/im/v1/messages/om_inbound_file_1/reactions")
            .expect("webhook flow should add ack reaction");
        assert_eq!(
            reaction_request.authorization.as_deref(),
            Some("Bearer t-token-webhook")
        );
        assert!(
            reaction_request.body.contains("\"emoji_type\""),
            "reaction request should include a Feishu emoji type"
        );
        let reply_request = feishu_requests
            .iter()
            .find(|request| request.path == "/open-apis/im/v1/messages/om_inbound_file_1/reply")
            .expect("webhook flow should still send a reply");
        assert!(
            reply_request.body.contains("\"msg_type\":\"interactive\""),
            "webhook reply should send markdown-capable interactive cards"
        );
        assert!(
            reply_request.body.contains("\\\"tag\\\":\\\"markdown\\\""),
            "reply body should wrap the provider reply in a markdown card"
        );
        assert!(
            reply_request
                .body
                .contains("\\\"content\\\":\\\"## structured inbound ack\\\\n\\\\n- rendered\\\""),
            "reply body should preserve provider markdown content"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_skips_ack_reaction_when_disabled() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-no-ack", || async move {
            feishu_webhook_skips_ack_reaction_when_disabled_impl().await;
        });
    }

    async fn feishu_webhook_skips_ack_reaction_when_disabled_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_disabled_1").await;

        let mut config = test_webhook_config(&provider_base_url, &feishu_base_url);
        config.feishu.ack_reactions = false;
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before webhook test");
        let kernel_ctx =
            bootstrap_test_kernel_context("feishu-webhook-no-ack-test", DEFAULT_TOKEN_TTL_S)
                .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "token": "verify-token",
            "header": {
                "event_id": "evt_ack_disabled",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": "ou_sender_disabled"
                    }
                },
                "message": {
                    "chat_id": "oc_demo",
                    "message_id": "om_inbound_no_ack_1",
                    "message_type": "text",
                    "content": "{\"text\":\"hello without ack\"}"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("webhook should succeed");

        assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

        let feishu_requests = wait_for_request_count(&feishu_requests, 2).await;
        assert_eq!(feishu_requests.len(), 2);
        assert!(
            feishu_requests
                .iter()
                .all(|request| request.path
                    != "/open-apis/im/v1/messages/om_inbound_no_ack_1/reactions"),
            "disabled ack_reactions should skip the reaction API call"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_provider_failure_retry_does_not_duplicate_ack_reaction() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-ack-retry", || async move {
            feishu_webhook_provider_failure_retry_does_not_duplicate_ack_reaction_impl().await;
        });
    }

    async fn feishu_webhook_provider_failure_retry_does_not_duplicate_ack_reaction_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_failure_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before webhook test");
        let kernel_ctx =
            bootstrap_test_kernel_context("feishu-webhook-provider-failure", DEFAULT_TOKEN_TTL_S)
                .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "token": "verify-token",
            "header": {
                "event_id": "evt_provider_failure_no_ack",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": "ou_sender_provider_failure"
                    }
                },
                "message": {
                    "chat_id": "oc_demo",
                    "message_id": "om_inbound_failure_no_ack_1",
                    "message_type": "text",
                    "content": "{\"text\":\"provider failure should not ack\"}"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let error = handle_feishu_webhook_payload(
            state.clone(),
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect_err("webhook should surface provider failure");

        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(error.1.contains("provider processing failed"));

        let error_retry = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect_err("webhook retry should still surface provider failure");
        assert_eq!(error_retry.0, StatusCode::INTERNAL_SERVER_ERROR);

        let feishu_requests = wait_for_request_count(&feishu_requests, 2).await;
        assert_eq!(feishu_requests.len(), 2);
        assert_eq!(
            feishu_requests
                .iter()
                .filter(|request| request.path
                    == "/open-apis/im/v1/messages/om_inbound_failure_no_ack_1/reactions")
                .count(),
            1,
            "retrying a failed inbound turn must not duplicate ack reactions"
        );
        assert!(
            feishu_requests.iter().all(|request| request.path
                != "/open-apis/im/v1/messages/om_inbound_failure_no_ack_1/reply"),
            "failed inbound handling must not send a reply"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_reaction_failure_stays_best_effort_after_reply() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-ack-best-effort", || async move {
            feishu_webhook_reaction_failure_stays_best_effort_after_reply_impl().await;
        });
    }

    async fn feishu_webhook_reaction_failure_stays_best_effort_after_reply_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) = spawn_mock_feishu_api_server_with_failing_reactions(
            feishu_requests.clone(),
            "om_reply_reaction_failure_1",
        )
        .await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before webhook test");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-reaction-failure-best-effort",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "token": "verify-token",
            "header": {
                "event_id": "evt_reaction_failure_best_effort",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": "ou_sender_reaction_failure"
                    }
                },
                "message": {
                    "chat_id": "oc_demo",
                    "message_id": "om_inbound_reaction_failure_1",
                    "message_type": "text",
                    "content": "{\"text\":\"reaction failure should not fail webhook\"}"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("reaction failure should stay best-effort");

        assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

        let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
        assert_eq!(feishu_requests.len(), 3);
        assert!(
            feishu_requests.iter().any(|request| request.path
                == "/open-apis/im/v1/messages/om_inbound_reaction_failure_1/reply"),
            "reply should still be sent even when reaction fails"
        );
        assert!(
            feishu_requests.iter().any(|request| request.path
                == "/open-apis/im/v1/messages/om_inbound_reaction_failure_1/reactions"),
            "reaction attempt should still be issued"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_inbound_reply_stays_successful_when_runtime_end_write_fails() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-runtime-end", || async move {
            feishu_webhook_inbound_reply_stays_successful_when_runtime_end_write_fails_impl().await;
        });
    }

    async fn feishu_webhook_inbound_reply_stays_successful_when_runtime_end_write_fails_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_delayed_success_server(
            provider_requests.clone(),
            std::time::Duration::from_millis(50),
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_runtime_end").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before webhook test");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-runtime-end-failure",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime_dir = temp_webhook_test_dir("runtime-end-failure");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
        let runtime = Arc::new(
            start_channel_operation_runtime_tracker_for_test(
                &runtime_dir,
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
                424242,
            )
            .await
            .expect("start test runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let runtime_dir_for_delete = runtime_dir.clone();
        let runtime_delete = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            std::fs::remove_dir_all(&runtime_dir_for_delete).expect("remove runtime dir");
            std::fs::write(&runtime_dir_for_delete, "blocked")
                .expect("replace runtime dir with file");
        });

        let payload = json!({
            "token": "verify-token",
            "header": {
                "event_id": "evt_runtime_end_failure",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": "ou_sender_runtime_end"
                    }
                },
                "message": {
                    "chat_id": "oc_demo",
                    "message_id": "om_runtime_end_1",
                    "message_type": "text",
                    "content": "{\"text\":\"runtime end failure should stay acknowledged\"}"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("reply should stay successful even if runtime end bookkeeping fails");

        runtime_delete.await.expect("join runtime file deletion");

        assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

        let provider_requests = provider_requests.lock().await.clone();
        assert_eq!(provider_requests.len(), 1);
        let provider_body =
            serde_json::from_str::<Value>(&provider_requests[0].body).expect("provider body json");
        assert_eq!(provider_body["stream"], json!(true));

        let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
        assert_eq!(feishu_requests.len(), 3);
        assert!(
            feishu_requests
                .iter()
                .any(|request| request.path == "/open-apis/im/v1/messages/om_runtime_end_1/reply"),
            "reply should still be sent when runtime end bookkeeping fails"
        );
        assert!(
            feishu_requests
                .iter()
                .any(|request| request.path
                    == "/open-apis/im/v1/messages/om_runtime_end_1/reactions"),
            "ack reaction should still be attempted when runtime end bookkeeping fails"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_reaches_provider_and_returns_safe_noop_body() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-noop", || async move {
            feishu_webhook_card_callback_reaches_provider_and_returns_safe_noop_body_impl().await;
        });
    }

    async fn feishu_webhook_card_callback_reaches_provider_and_returns_safe_noop_body_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx =
            bootstrap_test_kernel_context("feishu-webhook-card-callback", DEFAULT_TOKEN_TTL_S)
                .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1",
                        "user_id": "u_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request",
                    "value": {
                        "ticket_id": "T-500"
                    }
                },
                "context": {
                    "open_message_id": "om_card_source_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(response.body(), &json!({}));

        let provider_requests = provider_requests.lock().await.clone();
        assert_eq!(provider_requests.len(), 1);
        let provider_body =
            serde_json::from_str::<Value>(&provider_requests[0].body).expect("provider body json");
        let provider_user_content = provider_body
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            })
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("provider user content");
        assert!(provider_user_content.contains("[feishu_card_callback]"));
        assert!(provider_user_content.contains("\"name\":\"approve_request\""));
        assert!(
            !provider_requests[0].body.contains("callback-token-1"),
            "callback token must stay out of provider-visible prompt state"
        );

        let feishu_requests = feishu_requests.lock().await.clone();
        assert_eq!(
            feishu_requests.len(),
            0,
            "callback flow should not send a normal Feishu reply"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_structured_toast_response_is_returned() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-toast", || async move {
            feishu_webhook_card_callback_structured_toast_response_is_returned_impl().await;
        });
    }

    async fn feishu_webhook_card_callback_structured_toast_response_is_returned_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"toast\",\"kind\":\"success\",\"content\":\"Approved\"}",
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-toast",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_toast_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-toast-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1",
                        "user_id": "u_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_toast_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(
            response.body(),
            &json!({
                "toast": {
                    "type": "success",
                    "content": "Approved"
                }
            })
        );
        assert_eq!(provider_requests.lock().await.len(), 1);
        assert_eq!(
            feishu_requests.lock().await.len(),
            0,
            "toast callback flow should not send a normal Feishu reply"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_structured_card_response_is_returned() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-card", || async move {
            feishu_webhook_card_callback_structured_card_response_is_returned_impl().await;
        });
    }

    async fn feishu_webhook_card_callback_structured_card_response_is_returned_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"card\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"Approved inline\"}]}}",
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx =
            bootstrap_test_kernel_context("feishu-webhook-card-callback-card", DEFAULT_TOKEN_TTL_S)
                .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_card_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-card-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1",
                        "user_id": "u_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_card_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(
            response.body(),
            &json!({
                "card": {
                    "elements": [
                        {
                            "tag": "markdown",
                            "content": "Approved inline"
                        }
                    ]
                }
            })
        );
        assert_eq!(provider_requests.lock().await.len(), 1);
        assert_eq!(
            feishu_requests.lock().await.len(),
            0,
            "card callback response should not send a normal Feishu reply"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_structured_card_markdown_response_is_returned() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-card-md", || async move {
            feishu_webhook_card_callback_structured_card_markdown_response_is_returned_impl().await;
        });
    }

    async fn feishu_webhook_card_callback_structured_card_markdown_response_is_returned_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"Approved inline\"}",
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-card-markdown",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_card_markdown_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-card-markdown-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1",
                        "user_id": "u_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_card_markdown_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(
            response.body(),
            &json!({
                "card": {
                    "schema": "2.0",
                    "config": {
                        "wide_screen_mode": true
                    },
                    "body": {
                        "elements": [
                            {
                                "tag": "markdown",
                                "content": "Approved inline"
                            }
                        ]
                    }
                }
            })
        );
        assert_eq!(provider_requests.lock().await.len(), 1);
        assert_eq!(
            feishu_requests.lock().await.len(),
            0,
            "card callback response should not send a normal Feishu reply"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_structured_card_response_with_toast_is_returned() {
        run_feishu_webhook_test_on_large_stack(
            "feishu-webhook-callback-card-toast",
            || async move {
                feishu_webhook_card_callback_structured_card_response_with_toast_is_returned_impl()
                    .await;
            },
        );
    }

    async fn feishu_webhook_card_callback_structured_card_response_with_toast_is_returned_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"toast\":{\"kind\":\"success\",\"content\":\"Approved\"},\"card\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"Approved inline\"}]}}",
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-card-with-toast",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_card_toast_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-card-toast-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1",
                        "user_id": "u_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_card_toast_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(
            response.body(),
            &json!({
                "toast": {
                    "type": "success",
                    "content": "Approved"
                },
                "card": {
                    "elements": [
                        {
                            "tag": "markdown",
                            "content": "Approved inline"
                        }
                    ]
                }
            })
        );
        assert_eq!(provider_requests.lock().await.len(), 1);
        assert_eq!(
            feishu_requests.lock().await.len(),
            0,
            "card callback response should not send a normal Feishu reply"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_structured_card_markdown_response_with_toast_is_returned() {
        run_feishu_webhook_test_on_large_stack(
            "feishu-webhook-callback-card-md-toast",
            || async move {
                feishu_webhook_card_callback_structured_card_markdown_response_with_toast_is_returned_impl().await;
            },
        );
    }

    async fn feishu_webhook_card_callback_structured_card_markdown_response_with_toast_is_returned_impl()
     {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"Approved inline\",\"toast\":{\"kind\":\"success\",\"content\":\"Approved\"}}",
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-card-markdown-with-toast",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_card_markdown_toast_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-card-markdown-toast-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1",
                        "user_id": "u_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_card_markdown_toast_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(
            response.body(),
            &json!({
                "toast": {
                    "type": "success",
                    "content": "Approved"
                },
                "card": {
                    "schema": "2.0",
                    "config": {
                        "wide_screen_mode": true
                    },
                    "body": {
                        "elements": [
                            {
                                "tag": "markdown",
                                "content": "Approved inline"
                            }
                        ]
                    }
                }
            })
        );
        assert_eq!(provider_requests.lock().await.len(), 1);
        assert_eq!(
            feishu_requests.lock().await.len(),
            0,
            "card callback response should not send a normal Feishu reply"
        );

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn parse_feishu_structured_callback_response_rejects_card_markdown_conflict() {
        let response = parse_feishu_structured_callback_response(
            "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"Approved inline\",\"card\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"raw\"}]}}",
        );

        assert!(response.is_none());
    }

    #[test]
    fn parse_feishu_structured_callback_response_rejects_empty_card_markdown() {
        let response = parse_feishu_structured_callback_response(
            "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"   \"}",
        );

        assert!(response.is_none());
    }

    #[test]
    fn feishu_webhook_card_callback_invalid_structured_response_falls_back_to_safe_noop_body() {
        run_feishu_webhook_test_on_large_stack(
            "feishu-webhook-callback-invalid-toast",
            || async move {
                feishu_webhook_card_callback_invalid_structured_response_falls_back_to_safe_noop_body_impl().await;
            },
        );
    }

    async fn feishu_webhook_card_callback_invalid_structured_response_falls_back_to_safe_noop_body_impl()
     {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"toast\",\"kind\":\"danger\",\"content\":\"nope\"}",
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-invalid-toast",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_invalid_toast_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-invalid-toast-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_invalid_toast_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(response.body(), &json!({}));
        assert_eq!(provider_requests.lock().await.len(), 1);
        assert_eq!(feishu_requests.lock().await.len(), 0);

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_invalid_structured_card_response_falls_back_to_safe_noop_body()
    {
        run_feishu_webhook_test_on_large_stack(
            "feishu-webhook-callback-invalid-card",
            || async move {
                feishu_webhook_card_callback_invalid_structured_card_response_falls_back_to_safe_noop_body_impl().await;
            },
        );
    }

    async fn feishu_webhook_card_callback_invalid_structured_card_response_falls_back_to_safe_noop_body_impl()
     {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"toast\":{\"kind\":\"danger\",\"content\":\"nope\"},\"card\":true}",
        )
        .await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-invalid-card",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_invalid_card_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-invalid-card-1",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_invalid_card_1",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");
        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback webhook should succeed");

        assert_eq!(response.body(), &json!({}));
        assert_eq!(provider_requests.lock().await.len(), 1);
        assert_eq!(feishu_requests.lock().await.len(), 0);

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_duplicate_is_deduped_safely() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-dedupe", || async move {
            feishu_webhook_card_callback_duplicate_is_deduped_safely_impl().await;
        });
    }

    async fn feishu_webhook_card_callback_duplicate_is_deduped_safely_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-dedupe",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_dedupe_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-dedupe",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_dedupe",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");

        let first = handle_feishu_webhook_payload(
            state.clone(),
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("first callback should succeed");
        let second = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("second callback should succeed");

        assert_eq!(first.body(), &json!({}));
        assert_eq!(second.body(), &json!({}));
        assert!(
            !provider_requests.lock().await.is_empty(),
            "callback failure path should still attempt provider processing"
        );
        assert_eq!(feishu_requests.lock().await.len(), 0);

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_webhook_card_callback_delayed_update_waits_for_response_body_consumption() {
        run_feishu_webhook_test_on_large_stack(
            "feishu-webhook-delayed-update-response-order",
            || async move {
                let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
                let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
                let (provider_base_url, provider_server) =
                    spawn_mock_provider_card_update_server(provider_requests.clone()).await;
                let (feishu_base_url, feishu_server) =
                    spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

                let config = test_webhook_config(&provider_base_url, &feishu_base_url);
                let resolved = config
                    .feishu
                    .resolve_account(None)
                    .expect("resolve feishu account");
                let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
                let kernel_ctx = bootstrap_webhook_kernel_context(
                    "feishu-webhook-card-callback-delayed-update-response-order",
                    DEFAULT_TOKEN_TTL_S,
                    &config,
                )
                .expect("bootstrap kernel context");
                let runtime = Arc::new(
                    ChannelOperationRuntimeTracker::start(
                        ChannelPlatform::Feishu,
                        "serve",
                        resolved.account.id.as_str(),
                        resolved.account.label.as_str(),
                    )
                    .await
                    .expect("start runtime tracker"),
                );
                let state =
                    FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

                let payload = json!({
                    "header": {
                        "event_id": "evt_card_webhook_response_order_1",
                        "event_type": "card.action.trigger",
                        "token": "verify-token"
                    },
                    "event": {
                        "token": "callback-token-response-order",
                        "operator": {
                            "operator_id": {
                                "open_id": "ou_sender_1"
                            }
                        },
                        "action": {
                            "tag": "button",
                            "name": "approve_request"
                        },
                        "context": {
                            "open_message_id": "om_card_source_response_order",
                            "open_chat_id": "oc_demo"
                        }
                    }
                });
                let raw_body = serde_json::to_string(&payload).expect("serialize payload");
                let headers = signed_headers(&raw_body, "encrypt-key");

                let response =
                    feishu_webhook_handler(State(state), headers, Bytes::from(raw_body.clone()))
                        .await
                        .into_response();

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                assert_eq!(
                    feishu_requests
                        .lock()
                        .await
                        .iter()
                        .filter(|request| request.path == "/open-apis/interactive/v1/card/update")
                        .count(),
                    0,
                    "delayed update must wait until the callback HTTP response body is consumed"
                );

                let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("read callback response body");
                assert_eq!(
                    serde_json::from_slice::<Value>(&response_body).expect("response body json"),
                    json!({})
                );

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let feishu_requests = feishu_requests.lock().await.clone();
                let provider_requests = provider_requests.lock().await.clone();
                let delayed_update = feishu_requests
                    .iter()
                    .find(|request| request.path == "/open-apis/interactive/v1/card/update")
                    .unwrap_or_else(|| {
                        panic!(
                            "delayed update request after body consumption; feishu_requests={feishu_requests:?}; provider_requests={provider_requests:?}"
                        )
                    });
                assert_eq!(
                    delayed_update.authorization.as_deref(),
                    Some("Bearer t-token-webhook")
                );
                assert!(
                    delayed_update
                        .body
                        .contains("\"token\":\"callback-token-response-order\"")
                );
                assert!(
                    delayed_update
                        .body
                        .contains("\"content\":\"callback updated\"")
                );
                assert!(
                    !provider_requests.is_empty(),
                    "callback processing should still reach the provider before deferred dispatch"
                );

                provider_server.abort();
                feishu_server.abort();
            },
        );
    }

    #[test]
    fn feishu_webhook_card_callback_delayed_update_dispatches_when_response_body_is_dropped() {
        run_feishu_webhook_test_on_large_stack(
            "feishu-webhook-delayed-update-response-drop",
            || async move {
                let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
                let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
                let (provider_base_url, provider_server) =
                    spawn_mock_provider_card_update_server(provider_requests.clone()).await;
                let (feishu_base_url, feishu_server) =
                    spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

                let config = test_webhook_config(&provider_base_url, &feishu_base_url);
                let resolved = config
                    .feishu
                    .resolve_account(None)
                    .expect("resolve feishu account");
                let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
                let kernel_ctx = bootstrap_webhook_kernel_context(
                    "feishu-webhook-card-callback-delayed-update-response-drop",
                    DEFAULT_TOKEN_TTL_S,
                    &config,
                )
                .expect("bootstrap kernel context");
                let runtime = Arc::new(
                    ChannelOperationRuntimeTracker::start(
                        ChannelPlatform::Feishu,
                        "serve",
                        resolved.account.id.as_str(),
                        resolved.account.label.as_str(),
                    )
                    .await
                    .expect("start runtime tracker"),
                );
                let state =
                    FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

                let payload = json!({
                    "header": {
                        "event_id": "evt_card_webhook_response_drop_1",
                        "event_type": "card.action.trigger",
                        "token": "verify-token"
                    },
                    "event": {
                        "token": "callback-token-response-drop",
                        "operator": {
                            "operator_id": {
                                "open_id": "ou_sender_1"
                            }
                        },
                        "action": {
                            "tag": "button",
                            "name": "approve_request"
                        },
                        "context": {
                            "open_message_id": "om_card_source_response_drop",
                            "open_chat_id": "oc_demo"
                        }
                    }
                });
                let raw_body = serde_json::to_string(&payload).expect("serialize payload");
                let headers = signed_headers(&raw_body, "encrypt-key");

                let response =
                    feishu_webhook_handler(State(state), headers, Bytes::from(raw_body.clone()))
                        .await
                        .into_response();
                drop(response);

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let feishu_requests = feishu_requests.lock().await.clone();
                let provider_requests = provider_requests.lock().await.clone();
                let delayed_update = feishu_requests
                    .iter()
                    .find(|request| request.path == "/open-apis/interactive/v1/card/update")
                    .unwrap_or_else(|| {
                        panic!(
                            "delayed update request after response drop; feishu_requests={feishu_requests:?}; provider_requests={provider_requests:?}"
                        )
                    });
                assert_eq!(
                    delayed_update.authorization.as_deref(),
                    Some("Bearer t-token-webhook")
                );
                assert!(
                    delayed_update
                        .body
                        .contains("\"token\":\"callback-token-response-drop\"")
                );
                assert!(
                    delayed_update
                        .body
                        .contains("\"content\":\"callback updated\"")
                );
                assert!(
                    !provider_requests.is_empty(),
                    "callback processing should still reach the provider before deferred dispatch"
                );

                provider_server.abort();
                feishu_server.abort();
            },
        );
    }

    #[test]
    fn feishu_webhook_card_callback_provider_failure_still_returns_safe_noop_body() {
        run_feishu_webhook_test_on_large_stack(
            "feishu-webhook-callback-provider-failure",
            || async move {
                feishu_webhook_card_callback_provider_failure_still_returns_safe_noop_body_impl()
                    .await;
            },
        );
    }

    async fn feishu_webhook_card_callback_provider_failure_still_returns_safe_noop_body_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_failure_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

        let config = test_webhook_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        let kernel_ctx = bootstrap_test_kernel_context(
            "feishu-webhook-card-callback-failure",
            DEFAULT_TOKEN_TTL_S,
        )
        .expect("bootstrap kernel context");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Feishu,
                "serve",
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

        let payload = json!({
            "header": {
                "event_id": "evt_card_webhook_failure_1",
                "event_type": "card.action.trigger",
                "token": "verify-token"
            },
            "event": {
                "token": "callback-token-failure",
                "operator": {
                    "operator_id": {
                        "open_id": "ou_sender_1"
                    }
                },
                "action": {
                    "tag": "button",
                    "name": "approve_request"
                },
                "context": {
                    "open_message_id": "om_card_source_failure",
                    "open_chat_id": "oc_demo"
                }
            }
        });
        let raw_body = serde_json::to_string(&payload).expect("serialize payload");
        let headers = signed_headers(&raw_body, "encrypt-key");

        let response = handle_feishu_webhook_payload(
            state,
            &headers,
            raw_body.as_str(),
            serde_json::from_str(raw_body.as_str()).expect("payload value"),
        )
        .await
        .expect("callback failure should still produce a safe Feishu body");

        assert_eq!(response.body(), &json!({}));
        assert!(
            !provider_requests.lock().await.is_empty(),
            "callback failure path should still attempt provider processing"
        );
        assert_eq!(feishu_requests.lock().await.len(), 0);

        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn execute_deferred_feishu_card_update_uses_delayed_update_api() {
        run_feishu_webhook_test_on_large_stack("feishu-webhook-deferred-update", || async move {
            execute_deferred_feishu_card_update_uses_delayed_update_api_impl().await;
        });
    }

    async fn execute_deferred_feishu_card_update_uses_delayed_update_api_impl() {
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let state = MockServerState {
            requests: feishu_requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "tenant_access_token": "t-token-deferred"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/interactive/v1/card/update",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "msg": "ok"
                            }))
                        }
                    }
                }),
            );
        let (feishu_base_url, feishu_server) = spawn_mock_server(router).await;

        let config = test_webhook_config("http://127.0.0.1:9", &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");

        execute_deferred_feishu_card_update(
            config,
            crate::tools::DeferredFeishuCardUpdate {
                configured_account_id: resolved.configured_account_id,
                token: "callback-token-deferred".to_owned(),
                card: json!({
                    "elements": [{
                        "tag": "markdown",
                        "content": "deferred update"
                    }]
                }),
                open_ids: vec!["ou_operator_1".to_owned()],
            },
        )
        .await
        .expect("deferred callback update should succeed");

        let feishu_requests = feishu_requests.lock().await.clone();
        assert_eq!(feishu_requests.len(), 2);
        assert_eq!(
            feishu_requests[0].path,
            "/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(
            feishu_requests[1].path,
            "/open-apis/interactive/v1/card/update"
        );
        assert_eq!(
            feishu_requests[1].authorization.as_deref(),
            Some("Bearer t-token-deferred")
        );
        assert!(
            feishu_requests[1]
                .body
                .contains("\"token\":\"callback-token-deferred\"")
        );
        assert!(
            feishu_requests[1]
                .body
                .contains("\"open_ids\":[\"ou_operator_1\"]")
        );
        assert!(
            feishu_requests[1]
                .body
                .contains("\"content\":\"deferred update\"")
        );

        feishu_server.abort();
    }
}
