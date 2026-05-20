use std::{
    collections::VecDeque,
    convert::Infallible,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, OnceLock},
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
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::CliResult;
use crate::KernelContext;
use crate::channel::dispatch::process_inbound_with_provider_and_error_mode_and_retry_progress;
use crate::channel::feishu::api::{FeishuClient, resources::cards};
use crate::channel::traits::messaging::{MessageContent, MessageEditApi, MessageSendApi};
use crate::channel::{
    ChannelInboundMessage, ChannelOutboundTarget, ChannelTurnFeedbackPolicy,
    access_policy::ChannelInboundAccessPolicy, process_inbound_with_provider,
    runtime::state::ChannelOperationRuntimeTracker,
};
use crate::config::{LoongConfig, ResolvedFeishuChannelConfig};
use crate::crypto::timing_safe_eq;

use super::adapter::{FeishuAdapter, outbound_reply_message_from_text};
use super::payload::{FeishuCardCallbackEvent, FeishuWebhookAction};
use super::send::send_channel_message_via_message_send_api;

const FEISHU_CALLBACK_RESPONSE_MARKER: &str = "[feishu_callback_response]";
const PROVIDER_ERROR_REPLY_PREFIX: &str = "[provider_error] ";

#[derive(Clone)]
pub(in crate::channel) struct FeishuWebhookState {
    config: LoongConfig,
    resolved_path: Option<PathBuf>,
    adapter: Arc<Mutex<FeishuAdapter>>,
    configured_account_id: String,
    account_id: String,
    verification_token: Option<String>,
    encrypt_key: Option<String>,
    access_policy: ChannelInboundAccessPolicy<String>,
    ack_reactions: bool,
    ignore_bot_messages: bool,
    require_mention: bool,
    bot_id: Arc<OnceLock<String>>,
    seen_events: Arc<Mutex<RecentIdCache>>,
    seen_ack_reactions: Arc<Mutex<RecentIdCache>>,
    kernel_ctx: Arc<KernelContext>,
    runtime: Arc<ChannelOperationRuntimeTracker>,
}

impl FeishuWebhookState {
    #[cfg(test)]
    pub(super) fn new(
        config: LoongConfig,
        resolved: &ResolvedFeishuChannelConfig,
        adapter: FeishuAdapter,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> Self {
        Self::new_with_optional_resolved_path(config, None, resolved, adapter, kernel_ctx, runtime)
    }

    pub(super) fn new_with_resolved_path(
        config: LoongConfig,
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
        config: LoongConfig,
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
            require_mention: resolved.require_mention,
            bot_id: adapter.bot_open_id_handle(),
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
        super::payload::parse_feishu_inbound_payload_with_options(
            payload,
            super::payload::FeishuTransportAuth::websocket(),
            &self.access_policy,
            self.ignore_bot_messages,
            self.require_mention,
            self.bot_id.get().map(String::as_str),
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
    config: LoongConfig,
    deferred_updates: Vec<crate::tools::DeferredFeishuCardUpdate>,
}

struct FeishuRetryStatusHandle {
    tx: mpsc::UnboundedSender<FeishuRetryStatusCommand>,
    last_progress: Arc<std::sync::Mutex<Option<crate::provider::ProviderRetryProgress>>>,
}

enum FeishuRetryStatusCommand {
    Retry(crate::provider::ProviderRetryProgress),
    FinalSuccess {
        ack: oneshot::Sender<()>,
    },
    FinalFailure {
        message: String,
        retry_progress: Option<crate::provider::ProviderRetryProgress>,
        ack: oneshot::Sender<bool>,
    },
}

#[derive(Default)]
struct FeishuRetryStatusState {
    message_id: Option<String>,
    latest_attempt: Option<usize>,
    finalized: bool,
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
    fn from_parsed_response(response: FeishuParsedActionResponse, config: LoongConfig) -> Self {
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

impl FeishuRetryStatusHandle {
    fn new(adapter: Arc<Mutex<FeishuAdapter>>, reply_target: ChannelOutboundTarget) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let last_progress = Arc::new(std::sync::Mutex::new(None));
        tokio::spawn(async move {
            let mut state = FeishuRetryStatusState::default();
            while let Some(command) = rx.recv().await {
                match command {
                    FeishuRetryStatusCommand::Retry(progress) => {
                        if state.finalized || state.latest_attempt == Some(progress.next_attempt) {
                            continue;
                        }
                        if upsert_feishu_retry_status_message(
                            &adapter,
                            &reply_target,
                            &mut state,
                            render_feishu_retry_progress_message(&progress),
                        )
                        .await
                        {
                            state.latest_attempt = Some(progress.next_attempt);
                        }
                    }
                    FeishuRetryStatusCommand::FinalSuccess { ack } => {
                        state.finalized = true;
                        if state.message_id.is_some() {
                            upsert_feishu_retry_status_message(
                                &adapter,
                                &reply_target,
                                &mut state,
                                "Recovered after retrying. Final answer below.".to_owned(),
                            )
                            .await;
                            let _ = ack.send(());
                        } else {
                            let _ = ack.send(());
                        }
                    }
                    FeishuRetryStatusCommand::FinalFailure {
                        message,
                        retry_progress,
                        ack,
                    } => {
                        state.finalized = true;
                        if state.message_id.is_none()
                            && let Some(progress) = retry_progress.as_ref()
                            && state.latest_attempt != Some(progress.next_attempt)
                            && upsert_feishu_retry_status_message(
                                &adapter,
                                &reply_target,
                                &mut state,
                                render_feishu_retry_progress_message(progress),
                            )
                            .await
                        {
                            state.latest_attempt = Some(progress.next_attempt);
                        }
                        let handled = if state.message_id.is_some() {
                            upsert_feishu_retry_status_message(
                                &adapter,
                                &reply_target,
                                &mut state,
                                message,
                            )
                            .await
                        } else {
                            false
                        };
                        let _ = ack.send(handled);
                    }
                }
            }
        });
        Self { tx, last_progress }
    }

    fn callback(&self) -> crate::provider::ProviderRetryProgressCallback {
        let tx = self.tx.clone();
        let last_progress = Arc::clone(&self.last_progress);
        Some(Arc::new(move |progress| {
            let mut slot = match last_progress.lock() {
                Ok(slot) => slot,
                Err(poisoned) => poisoned.into_inner(),
            };
            *slot = Some(progress.clone());
            if tx.send(FeishuRetryStatusCommand::Retry(progress)).is_err() {
                tracing::debug!(
                    target: "loong.channel.feishu",
                    "feishu retry status worker already stopped before retry progress could be delivered"
                );
            }
        }))
    }

    async fn finalize_success(&self) {
        self.send_with_ack(
            |ack| FeishuRetryStatusCommand::FinalSuccess { ack },
            (),
            "final success",
        )
        .await;
    }

    async fn finalize_failure(&self, message: String) -> bool {
        let retry_progress = {
            let slot = match self.last_progress.lock() {
                Ok(slot) => slot,
                Err(poisoned) => poisoned.into_inner(),
            };
            slot.clone()
        };
        self.send_with_ack(
            |ack| FeishuRetryStatusCommand::FinalFailure {
                message,
                retry_progress,
                ack,
            },
            false,
            "final failure",
        )
        .await
    }

    async fn send_with_ack<T>(
        &self,
        command: impl FnOnce(oneshot::Sender<T>) -> FeishuRetryStatusCommand,
        fallback: T,
        label: &'static str,
    ) -> T
    where
        T: Send + 'static,
    {
        let (ack_tx, ack_rx) = oneshot::channel();
        if self.tx.send(command(ack_tx)).is_err() {
            tracing::debug!(
                target: "loong.channel.feishu",
                phase = label,
                "feishu retry status worker already stopped before finalization command could be delivered"
            );
            return fallback;
        }

        match ack_rx.await {
            Ok(value) => value,
            Err(_error) => {
                tracing::debug!(
                    target: "loong.channel.feishu",
                    phase = label,
                    "feishu retry status worker stopped before sending finalization acknowledgement"
                );
                fallback
            }
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
        target: "loong.channel.feishu",
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

    let parsed = super::payload::parse_feishu_webhook_payload_with_options(
        &payload,
        state.verification_token.as_deref(),
        state.encrypt_key.as_deref(),
        &state.access_policy,
        state.ignore_bot_messages,
        state.require_mention,
        state.bot_id.get().map(String::as_str),
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
                target: "loong.channel.feishu",
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
                target: "loong.channel.feishu",
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
                let reservation = dedupe.begin_processing(event.delivery_dedupe_key());
                if !matches!(reservation, RecentIdReservation::Accepted) {
                    tracing::debug!(
                        target: "loong.channel.feishu",
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
                target: "loong.channel.feishu",
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
                let reservation = dedupe.begin_processing(event.delivery_dedupe_key());
                if !matches!(reservation, RecentIdReservation::Accepted) {
                    tracing::debug!(
                        target: "loong.channel.feishu",
                        transport = "webhook",
                        action = "inbound",
                        configured_account_id = %state.configured_account_id,
                        event_id = %event.event_id,
                        message_id = %event.message_id,
                        dedupe_key = %event.delivery_dedupe_key(),
                        reservation = ?reservation,
                        "deduplicated feishu inbound event"
                    );
                    return Ok(FeishuParsedActionResponse::immediate(
                        json!({"code": 0, "msg": "duplicate_event"}),
                    ));
                }
            }

            let delivery_dedupe_key = event.delivery_dedupe_key().to_owned();
            let result = handle_feishu_inbound_event(state, event).await;

            {
                let mut dedupe = state.seen_events.lock().await;
                if result.is_ok() {
                    dedupe.mark_completed(&delivery_dedupe_key);
                } else {
                    dedupe.release(&delivery_dedupe_key);
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

    if let Err(error) = state.runtime.mark_run_start().await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("channel runtime start failed: {error}"),
        ));
    }

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
                acp_bootstrap_mcp_servers: state.config.feishu.acp.bootstrap_mcp_servers.clone(),
                acp_working_directory: state.config.feishu.acp.resolved_working_directory(),
            },
        };
        let reply_target = &channel_message.reply_target;
        let retry_status =
            FeishuRetryStatusHandle::new(state.adapter.clone(), reply_target.clone());
        let reply = process_inbound_with_provider_and_error_mode_and_retry_progress(
            &state.config,
            state.resolved_path.as_deref(),
            &channel_message,
            state.kernel_ctx.as_ref(),
            ChannelTurnFeedbackPolicy::final_trace_significant(),
            crate::conversation::ProviderErrorMode::InlineMessage,
            retry_status.callback(),
        )
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("provider processing failed: {error}"),
            )
        })?;
        if let Some(error) = provider_error_reply_body(reply.as_str()) {
            let rendered_error = render_feishu_user_facing_provider_error(error);
            if retry_status.finalize_failure(rendered_error.clone()).await {
                return Ok(FeishuParsedActionResponse::immediate(
                    json!({"code": 0, "msg": "ok"}),
                ));
            }
            {
                let outbound = outbound_reply_message_from_text(rendered_error);
                let mut adapter = state.adapter.lock().await;
                if let Err(first_error) = send_channel_message_via_message_send_api(
                    &*adapter,
                    reply_target,
                    outbound.clone(),
                )
                .await
                {
                    if let Err(error) = adapter.refresh_tenant_token().await {
                        return Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!(
                                "feishu token refresh failed after send error `{first_error}`: {error}"
                            ),
                        ));
                    }
                    send_channel_message_via_message_send_api(&*adapter, reply_target, outbound)
                        .await
                        .map_err(|error| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("feishu reply failed after token refresh: {error}"),
                            )
                        })?;
                }
            }
            return Ok(FeishuParsedActionResponse::immediate(
                json!({"code": 0, "msg": "ok"}),
            ));
        }

        {
            let outbound = outbound_reply_message_from_text(render_feishu_user_facing_reply(reply));
            let mut adapter = state.adapter.lock().await;
            if let Err(first_error) =
                send_channel_message_via_message_send_api(&*adapter, reply_target, outbound.clone())
                    .await
            {
                if let Err(error) = adapter.refresh_tenant_token().await {
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "feishu token refresh failed after send error `{first_error}`: {error}"
                        ),
                    ));
                }
                send_channel_message_via_message_send_api(&*adapter, reply_target, outbound)
                    .await
                    .map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("feishu reply failed after token refresh: {error}"),
                        )
                    })?;
            }
        }
        retry_status.finalize_success().await;
        Ok(FeishuParsedActionResponse::immediate(
            json!({"code": 0, "msg": "ok"}),
        ))
    }
    .await;

    if result.is_ok() {
        tracing::info!(
            target: "loong.channel.feishu",
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

fn render_feishu_user_facing_reply(reply: String) -> String {
    let Some(error) = provider_error_reply_body(reply.as_str()) else {
        return reply;
    };
    render_feishu_user_facing_provider_error(error)
}

fn provider_error_reply_body(reply: &str) -> Option<&str> {
    reply.strip_prefix(PROVIDER_ERROR_REPLY_PREFIX)
}

fn render_feishu_user_facing_provider_error(error: &str) -> String {
    if provider_error_mentions_timeout(error) {
        return "Sorry, I couldn't finish this request because the model timed out before a full reply was produced. Please try again in a moment.".to_owned();
    }

    let summary = summarize_feishu_user_facing_provider_error(error);
    if summary.is_empty() {
        return "Sorry, I couldn't finish this request. Please try again in a moment.".to_owned();
    }

    format!("Sorry, I couldn't finish this request.\n\nReason: {summary}")
}

fn summarize_feishu_user_facing_provider_error(error: &str) -> String {
    const MAX_LEN: usize = 220;
    let summary = error
        .split(" | provider_failover=")
        .next()
        .unwrap_or(error)
        .split(" if you're using a proxy/TUN/fake-ip setup")
        .next()
        .unwrap_or(error)
        .trim();
    if summary.is_empty() {
        return String::new();
    }
    if summary.chars().count() <= MAX_LEN {
        return summary.to_owned();
    }
    let truncated = summary.chars().take(MAX_LEN).collect::<String>();
    format!("{truncated}...")
}

fn provider_error_mentions_timeout(error: &str) -> bool {
    error.contains("timed out") || error.contains("timeout")
}

fn render_feishu_retry_progress_message(
    progress: &crate::provider::ProviderRetryProgress,
) -> String {
    let attempt_label = format!(
        "attempt {}/{}",
        progress.next_attempt, progress.max_attempts
    );
    let delay_suffix = render_retry_delay_suffix(progress.delay_ms);

    if progress.timeout {
        return format!("Model connection timed out. Retrying {attempt_label}{delay_suffix}...");
    }
    if progress.connect {
        return format!(
            "Connection to the model failed. Retrying {attempt_label}{delay_suffix}..."
        );
    }
    if let Some(status_code) = progress.status_code {
        if status_code == 429 {
            return format!(
                "The model provider is rate limiting requests. Retrying {attempt_label}{delay_suffix}..."
            );
        }
        return format!(
            "The model provider returned a transient HTTP {status_code} error. Retrying {attempt_label}{delay_suffix}..."
        );
    }

    format!("A transient model error occurred. Retrying {attempt_label}{delay_suffix}...")
}

fn render_retry_delay_suffix(delay_ms: u64) -> String {
    if delay_ms < 1_000 {
        return String::new();
    }
    let delay_s = delay_ms / 1_000;
    format!(" in {delay_s}s")
}

async fn upsert_feishu_retry_status_message(
    adapter: &Arc<Mutex<FeishuAdapter>>,
    reply_target: &ChannelOutboundTarget,
    state: &mut FeishuRetryStatusState,
    text: String,
) -> bool {
    let content = MessageContent::Text { text };

    if let Some(message_id) = state.message_id.as_deref() {
        let adapter = adapter.lock().await;
        match adapter.edit_message(message_id, &content).await {
            Ok(_) => return true,
            Err(error) => {
                tracing::warn!(
                    target: "loong.channel.feishu",
                    error = %error,
                    message_id = %message_id,
                    "failed to update feishu retry status message"
                );
                return false;
            }
        }
    }

    {
        let adapter = adapter.lock().await;
        match adapter.reply(reply_target, &content, None).await {
            Ok(message) => {
                state.message_id = Some(message.id);
                true
            }
            Err(error) => {
                tracing::warn!(
                    target: "loong.channel.feishu",
                    error = %error,
                    "failed to create feishu retry status message"
                );
                false
            }
        }
    }
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
            acp_bootstrap_mcp_servers: Vec::new(),
            acp_working_directory: None,
        },
    }
}

fn dispatch_deferred_feishu_card_updates(
    config: LoongConfig,
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
    config: LoongConfig,
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
mod tests;
