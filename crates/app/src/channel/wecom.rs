use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::CliResult;
use crate::KernelContext;
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedWecomChannelConfig,
};

use super::{
    CHANNEL_OPERATION_SERVE_ID, ChannelDelivery, ChannelDeliveryResource, ChannelInboundMessage,
    ChannelOperationRuntimeTracker, ChannelOutboundTarget, ChannelOutboundTargetKind,
    ChannelPlatform, ChannelServeStopHandle, ChannelSession, ChannelTurnFeedbackPolicy,
    process_inbound_with_provider, runtime_state,
};

const WECOM_SUBSCRIBE_CMD: &str = "aibot_subscribe";
const WECOM_PING_CMD: &str = "ping";
const WECOM_MESSAGE_CALLBACK_CMD: &str = "aibot_msg_callback";
const WECOM_EVENT_CALLBACK_CMD: &str = "aibot_event_callback";
const WECOM_RESPOND_MSG_CMD: &str = "aibot_respond_msg";
const WECOM_SEND_MSG_CMD: &str = "aibot_send_msg";
const WECOM_EVENT_DISCONNECTED: &str = "disconnected_event";
const WECOM_GROUP_CHAT_TYPE: u8 = 2;
const WECOM_CONNECTION_OWNER_OPERATION_ID: &str = "owner";

type WecomWebsocketStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WecomServeSessionOutcome {
    Stopped,
    Reconnect,
    ExclusiveDisconnect,
}

#[derive(Debug, Clone)]
struct WecomParsedInboundMessage {
    req_id: String,
    message: ChannelInboundMessage,
}

#[derive(Debug, Clone)]
struct WecomConnectionConfig {
    websocket_url: String,
    bot_id: String,
    secret: String,
}

struct WecomWebsocketClient {
    stream: WecomWebsocketStream,
    request_counter: u64,
}

impl WecomWebsocketClient {
    async fn connect(connection: &WecomConnectionConfig) -> CliResult<Self> {
        ensure_wecom_websocket_rustls_provider();
        let websocket_url = connection.websocket_url.as_str();
        let connect_result = connect_async(websocket_url).await;
        let (stream, _) =
            connect_result.map_err(|error| format!("connect WeCom websocket failed: {error}"))?;

        let mut client = Self {
            stream,
            request_counter: 0,
        };
        client.subscribe(connection).await?;
        Ok(client)
    }

    fn next_request_id(&mut self, scope: &str) -> String {
        self.request_counter = self.request_counter.saturating_add(1);
        let timestamp_ms = current_time_ms();
        let counter = self.request_counter;
        format!("loongclaw-{scope}-{timestamp_ms}-{counter}")
    }

    async fn subscribe(&mut self, connection: &WecomConnectionConfig) -> CliResult<()> {
        let bot_id = connection.bot_id.as_str();
        let secret = connection.secret.as_str();
        let req_id = self.next_request_id("subscribe");
        let headers = json!({ "req_id": req_id });
        let body = json!({
            "bot_id": bot_id,
            "secret": secret,
        });
        let envelope = json!({
            "cmd": WECOM_SUBSCRIBE_CMD,
            "headers": headers,
            "body": body,
        });
        self.send_json(envelope, "wecom subscribe").await?;
        self.wait_for_command_ok(req_id.as_str(), WECOM_SUBSCRIBE_CMD, "wecom subscribe")
            .await
    }

    async fn send_ping(&mut self) -> CliResult<()> {
        let req_id = self.next_request_id("ping");
        let headers = json!({ "req_id": req_id });
        let envelope = json!({
            "cmd": WECOM_PING_CMD,
            "headers": headers,
        });
        self.send_json(envelope, "wecom ping").await
    }

    async fn send_reply_text(&mut self, req_id: &str, text: &str) -> CliResult<()> {
        let trimmed_req_id = req_id.trim();
        if trimmed_req_id.is_empty() {
            return Err("wecom reply req_id is empty".to_owned());
        }

        let headers = json!({ "req_id": trimmed_req_id });
        let body = json!({
            "msgtype": "text",
            "text": {
                "content": text,
            },
        });
        let envelope = json!({
            "cmd": WECOM_RESPOND_MSG_CMD,
            "headers": headers,
            "body": body,
        });
        self.send_json(envelope, "wecom reply").await
    }

    async fn send_markdown(
        &mut self,
        conversation_id: &str,
        chat_type: Option<u8>,
        text: &str,
    ) -> CliResult<()> {
        let trimmed_conversation_id = conversation_id.trim();
        if trimmed_conversation_id.is_empty() {
            return Err("wecom conversation target is empty".to_owned());
        }

        let req_id = self.next_request_id("send");
        let headers = json!({ "req_id": req_id });
        let mut body = serde_json::Map::new();
        let chat_id = json!(trimmed_conversation_id);
        let message_type = json!("markdown");
        let markdown = json!({ "content": text });
        body.insert("chatid".to_owned(), chat_id);
        body.insert("msgtype".to_owned(), message_type);
        body.insert("markdown".to_owned(), markdown);
        if let Some(chat_type) = chat_type {
            let chat_type = json!(chat_type);
            body.insert("chat_type".to_owned(), chat_type);
        }
        let body = Value::Object(body);
        let envelope = json!({
            "cmd": WECOM_SEND_MSG_CMD,
            "headers": headers,
            "body": body,
        });
        self.send_json(envelope, "wecom proactive send").await?;
        self.wait_for_command_ok(req_id.as_str(), WECOM_SEND_MSG_CMD, "wecom proactive send")
            .await
    }

    async fn send_json(&mut self, value: Value, context: &str) -> CliResult<()> {
        let payload = serde_json::to_string(&value)
            .map_err(|error| format!("serialize {context} payload failed: {error}"))?;
        self.stream
            .send(Message::Text(payload.into()))
            .await
            .map_err(|error| format!("send {context} payload failed: {error}"))
    }

    async fn wait_for_command_ok(
        &mut self,
        expected_req_id: &str,
        expected_cmd: &str,
        context: &str,
    ) -> CliResult<()> {
        loop {
            let response = self.read_next_json(context).await?;
            let response_req_id = json_string_at(&response, &["headers", "req_id"]);
            if response_req_id.as_deref() != Some(expected_req_id) {
                continue;
            }

            let response_cmd = json_string_at(&response, &["cmd"]);
            if let Some(response_cmd) = response_cmd {
                let normalized_response_cmd = response_cmd.trim();
                if !normalized_response_cmd.is_empty() && normalized_response_cmd != expected_cmd {
                    continue;
                }
            }

            let errcode = extract_wecom_errcode(&response).unwrap_or(0);
            if errcode == 0 {
                return Ok(());
            }

            let errmsg =
                extract_wecom_errmsg(&response).unwrap_or_else(|| "unknown error".to_owned());
            return Err(format!("{context} failed: {errmsg} (errcode={errcode})"));
        }
    }

    async fn read_next_json(&mut self, context: &str) -> CliResult<Value> {
        loop {
            let next = self.stream.next().await;
            let message = match next {
                Some(Ok(message)) => message,
                Some(Err(error)) => {
                    return Err(format!("read {context} websocket frame failed: {error}"));
                }
                None => {
                    return Err(format!("{context} websocket closed by remote peer"));
                }
            };

            match message {
                Message::Text(text) => {
                    let value = serde_json::from_str::<Value>(text.as_ref()).map_err(|error| {
                        format!("decode {context} websocket json failed: {error}")
                    })?;
                    return Ok(value);
                }
                Message::Binary(bytes) => {
                    let text = std::str::from_utf8(bytes.as_ref()).map_err(|error| {
                        format!("decode {context} websocket binary frame as utf8 failed: {error}")
                    })?;
                    let value = serde_json::from_str::<Value>(text).map_err(|error| {
                        format!("decode {context} websocket json failed: {error}")
                    })?;
                    return Ok(value);
                }
                Message::Ping(payload) => {
                    self.stream
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| format!("send WeCom pong failed: {error}"))?;
                }
                Message::Pong(_) => {}
                Message::Frame(_) => {}
                Message::Close(frame) => {
                    let reason = frame
                        .as_ref()
                        .map(|value| value.reason.to_string())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "remote peer closed the socket".to_owned());
                    return Err(format!("{context} websocket closed: {reason}"));
                }
            }
        }
    }
}

pub(super) async fn run_wecom_send(
    resolved: &ResolvedWecomChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Conversation {
        return Err(format!(
            "wecom send requires conversation target kind, got {}",
            target_kind.as_str()
        ));
    }

    send_wecom_text(resolved, target_id, None, text).await
}

pub(super) async fn send_wecom_text(
    resolved: &ResolvedWecomChannelConfig,
    conversation_id: &str,
    chat_type: Option<u8>,
    text: &str,
) -> CliResult<()> {
    let connection = resolve_wecom_connection_config(resolved)?;
    ensure_wecom_send_runtime_is_exclusive(resolved)?;
    let _owner_guard = acquire_wecom_connection_owner(resolved).await?;
    let mut client = WecomWebsocketClient::connect(&connection).await?;
    client.send_markdown(conversation_id, chat_type, text).await
}

#[allow(clippy::print_stdout)]
pub(super) async fn run_wecom_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedWecomChannelConfig,
    resolved_path: &std::path::Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    let connection = resolve_wecom_connection_config(resolved)?;
    let _owner_guard = acquire_wecom_connection_owner(resolved).await?;

    println!(
        "wecom channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, websocket_url={}, ping_interval={}s)",
        resolved_path.display(),
        resolved.configured_account_id,
        resolved.account.label,
        selected_by_default,
        default_account_source.as_str(),
        resolved.resolved_websocket_url(),
        resolved.ping_interval_s
    );

    let reconnect_interval = Duration::from_secs(resolved.reconnect_interval_s.max(1));
    loop {
        let session_result = run_wecom_serve_session(
            config,
            resolved_path,
            resolved,
            &connection,
            kernel_ctx.clone(),
            runtime.clone(),
            stop.clone(),
        )
        .await;
        let outcome = match session_result {
            Ok(outcome) => outcome,
            Err(error) => {
                let reconnect_seconds = reconnect_interval.as_secs();
                emit_wecom_warning(format!(
                    "wecom serve session ended with transport error: {error}; reconnecting in {reconnect_seconds}s"
                ));
                if wait_for_wecom_reconnect_or_stop(&stop, reconnect_interval).await {
                    return Ok(());
                }
                continue;
            }
        };

        match outcome {
            WecomServeSessionOutcome::Stopped => return Ok(()),
            WecomServeSessionOutcome::ExclusiveDisconnect => {
                return Err(
                    "wecom long connection received disconnected_event; another active client likely took over the bot session"
                        .to_owned(),
                );
            }
            WecomServeSessionOutcome::Reconnect => {
                if wait_for_wecom_reconnect_or_stop(&stop, reconnect_interval).await {
                    return Ok(());
                }
            }
        }
    }
}

async fn run_wecom_serve_session(
    config: &LoongClawConfig,
    resolved_path: &std::path::Path,
    resolved: &ResolvedWecomChannelConfig,
    connection: &WecomConnectionConfig,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
) -> CliResult<WecomServeSessionOutcome> {
    let mut client = tokio::select! {
        _ = stop.wait() => return Ok(WecomServeSessionOutcome::Stopped),
        client = WecomWebsocketClient::connect(connection) => client?,
    };
    let ping_interval = Duration::from_secs(resolved.ping_interval_s.max(1));
    let mut ping_timer = tokio::time::interval(ping_interval);
    ping_timer.tick().await;
    let provider_ctx = Arc::new(KernelContext {
        kernel: kernel_ctx.kernel.clone(),
        token: kernel_ctx.token.clone(),
    });

    loop {
        let next_frame = tokio::select! {
            _ = stop.wait() => return Ok(WecomServeSessionOutcome::Stopped),
            _ = ping_timer.tick() => {
                client.send_ping().await?;
                continue;
            }
            frame = client.read_next_json("wecom serve") => frame?,
        };

        let command = json_string_at(&next_frame, &["cmd"]).unwrap_or_default();
        let command = command.trim().to_owned();
        if command.is_empty() {
            continue;
        }

        if command == WECOM_MESSAGE_CALLBACK_CMD {
            let parsed = match parse_wecom_inbound_message(&next_frame, resolved) {
                Ok(parsed) => parsed,
                Err(error) => {
                    emit_wecom_warning(format!("ignore malformed wecom inbound callback: {error}"));
                    continue;
                }
            };
            let Some(parsed) = parsed else {
                continue;
            };

            let conversation_id = parsed.message.session.conversation_id.as_str();
            if !is_wecom_allowed_conversation(resolved, conversation_id) {
                emit_wecom_warning(format!(
                    "ignore wecom inbound callback from non-allowlisted conversation `{conversation_id}`"
                ));
                continue;
            }

            let mark_run_start_result = runtime.mark_run_start().await;
            if let Err(error) = mark_run_start_result {
                emit_wecom_warning(format!(
                    "skip wecom inbound callback because runtime start tracking failed: {error}"
                ));
                continue;
            }

            let reply_result = process_inbound_with_provider(
                config,
                Some(resolved_path),
                &parsed.message,
                provider_ctx.as_ref(),
                ChannelTurnFeedbackPolicy::final_trace_significant(),
            )
            .await;

            let mark_run_end_result = runtime.mark_run_end().await;
            if let Err(error) = mark_run_end_result {
                emit_wecom_warning(format!(
                    "wecom runtime end tracking failed after inbound processing: {error}"
                ));
            }

            let reply = match reply_result {
                Ok(reply) => reply,
                Err(error) => {
                    emit_wecom_warning(format!(
                        "wecom inbound callback processing failed and will be skipped: {error}"
                    ));
                    continue;
                }
            };

            client
                .send_reply_text(parsed.req_id.as_str(), reply.as_str())
                .await?;
            continue;
        }

        if command == WECOM_EVENT_CALLBACK_CMD {
            if is_disconnected_event(&next_frame) {
                return Ok(WecomServeSessionOutcome::ExclusiveDisconnect);
            }
            continue;
        }

        if command == WECOM_PING_CMD
            || command == WECOM_SUBSCRIBE_CMD
            || command == WECOM_RESPOND_MSG_CMD
            || command == WECOM_SEND_MSG_CMD
        {
            continue;
        }

        if extract_wecom_errcode(&next_frame).is_some() {
            continue;
        }

        return Ok(WecomServeSessionOutcome::Reconnect);
    }
}

fn parse_wecom_inbound_message(
    value: &Value,
    resolved: &ResolvedWecomChannelConfig,
) -> CliResult<Option<WecomParsedInboundMessage>> {
    let req_id = json_string_at(value, &["headers", "req_id"])
        .ok_or_else(|| "wecom inbound callback missing headers.req_id".to_owned())?;
    let body = value
        .get("body")
        .ok_or_else(|| "wecom inbound callback missing body".to_owned())?;

    let conversation_id = resolve_wecom_conversation_id(body)
        .ok_or_else(|| "wecom inbound callback missing conversation identity".to_owned())?;
    let participant_id = resolve_wecom_participant_id(body, conversation_id.as_str());
    let text = extract_wecom_message_text(body)
        .ok_or_else(|| "wecom inbound callback missing supported message content".to_owned())?;
    let source_message_id = json_string_at(body, &["msgid"]);
    let sender_user_id = json_string_at(body, &["from", "userid"]);
    let sender_principal_key = sender_user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("wecom:user:{value}"));
    let resources = extract_wecom_resources(body);

    let session = ChannelSession::with_account(
        ChannelPlatform::Wecom,
        resolved.account.id.as_str(),
        conversation_id.as_str(),
    )
    .with_configured_account_id(resolved.configured_account_id.as_str());
    let session = match participant_id {
        Some(participant_id) => session.with_participant_id(participant_id),
        None => session,
    };

    let reply_target = ChannelOutboundTarget::new(
        ChannelPlatform::Wecom,
        ChannelOutboundTargetKind::MessageReply,
        req_id.clone(),
    );
    let delivery = ChannelDelivery {
        ack_cursor: None,
        source_message_id,
        sender_principal_key,
        thread_root_id: None,
        parent_message_id: None,
        resources,
        feishu_callback: None,
    };
    let message = ChannelInboundMessage {
        session,
        reply_target,
        text,
        delivery,
    };

    Ok(Some(WecomParsedInboundMessage { req_id, message }))
}

fn resolve_wecom_conversation_id(body: &Value) -> Option<String> {
    let chat_type = resolve_wecom_chat_type(body);
    let chat_id = json_string_at(body, &["chatid"]);
    let user_id = json_string_at(body, &["from", "userid"]);

    match chat_type {
        Some(WECOM_GROUP_CHAT_TYPE) => chat_id,
        Some(_) => user_id.or(chat_id),
        None => user_id.or(chat_id),
    }
}

fn resolve_wecom_participant_id(body: &Value, conversation_id: &str) -> Option<String> {
    let chat_type = resolve_wecom_chat_type(body);
    let user_id = json_string_at(body, &["from", "userid"])?;
    let trimmed_user_id = user_id.trim();
    if trimmed_user_id.is_empty() {
        return None;
    }

    if chat_type == Some(WECOM_GROUP_CHAT_TYPE) {
        return Some(trimmed_user_id.to_owned());
    }

    if trimmed_user_id == conversation_id.trim() {
        return None;
    }

    Some(trimmed_user_id.to_owned())
}

fn resolve_wecom_chat_type(body: &Value) -> Option<u8> {
    let raw = json_string_at(body, &["chattype"])?;
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if normalized == "group" {
        return Some(WECOM_GROUP_CHAT_TYPE);
    }
    if normalized == "single" || normalized == "private" {
        return Some(1);
    }
    normalized.parse::<u8>().ok()
}

fn extract_wecom_message_text(body: &Value) -> Option<String> {
    let msgtype = json_string_at(body, &["msgtype"])?;
    let normalized_msgtype = msgtype.trim().to_ascii_lowercase();
    if normalized_msgtype == "text" {
        return json_string_at(body, &["text", "content"]);
    }
    if normalized_msgtype == "voice" {
        return json_string_at(body, &["voice", "content"]);
    }
    if normalized_msgtype == "mixed" {
        return extract_wecom_mixed_message_text(body);
    }
    if normalized_msgtype == "image" {
        let image_url = json_string_at(body, &["image", "url"])?;
        return Some(format!("[image] {image_url}"));
    }
    if normalized_msgtype == "file" {
        let file_url = json_string_at(body, &["file", "url"])?;
        return Some(format!("[file] {file_url}"));
    }
    if normalized_msgtype == "video" {
        let video_url = json_string_at(body, &["video", "url"])?;
        return Some(format!("[video] {video_url}"));
    }
    if normalized_msgtype == "stream" {
        let stream_id = json_string_at(body, &["stream", "id"])?;
        return Some(format!("[stream] {stream_id}"));
    }
    None
}

fn extract_wecom_mixed_message_text(body: &Value) -> Option<String> {
    let items = json_array_at(body, &["mixed", "msg_item"])?;
    let mut segments = Vec::new();
    for item in items {
        let text_segment = json_string_at(item, &["text", "content"])
            .or_else(|| json_string_at(item, &["content"]));
        if let Some(text_segment) = text_segment {
            segments.push(text_segment);
            continue;
        }

        let image_url = json_string_at(item, &["image", "url"]);
        if let Some(image_url) = image_url {
            segments.push(format!("[image] {image_url}"));
            continue;
        }

        let file_url = json_string_at(item, &["file", "url"]);
        if let Some(file_url) = file_url {
            segments.push(format!("[file] {file_url}"));
            continue;
        }

        let video_url = json_string_at(item, &["video", "url"]);
        if let Some(video_url) = video_url {
            segments.push(format!("[video] {video_url}"));
            continue;
        }
    }

    let joined = segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>()
        .join("\n");
    let joined = joined.trim().to_owned();
    if joined.is_empty() {
        return None;
    }

    Some(joined)
}

fn extract_wecom_resources(body: &Value) -> Vec<ChannelDeliveryResource> {
    let mut resources = Vec::new();
    maybe_push_wecom_resource(
        &mut resources,
        "image",
        json_string_at(body, &["image", "url"]),
        None,
    );
    maybe_push_wecom_resource(
        &mut resources,
        "file",
        json_string_at(body, &["file", "url"]),
        None,
    );
    maybe_push_wecom_resource(
        &mut resources,
        "video",
        json_string_at(body, &["video", "url"]),
        None,
    );
    resources
}

fn maybe_push_wecom_resource(
    resources: &mut Vec<ChannelDeliveryResource>,
    resource_type: &str,
    file_key: Option<String>,
    file_name: Option<String>,
) {
    let Some(file_key) = file_key else {
        return;
    };
    let trimmed_file_key = file_key.trim();
    if trimmed_file_key.is_empty() {
        return;
    }

    resources.push(ChannelDeliveryResource {
        resource_type: resource_type.to_owned(),
        file_key: trimmed_file_key.to_owned(),
        file_name,
    });
}

fn resolve_wecom_connection_config(
    resolved: &ResolvedWecomChannelConfig,
) -> CliResult<WecomConnectionConfig> {
    let websocket_url = resolved.resolved_websocket_url();
    let websocket_url = websocket_url.trim().to_owned();
    if websocket_url.is_empty() {
        return Err("wecom websocket_url is empty".to_owned());
    }

    let bot_id = resolved
        .bot_id()
        .ok_or_else(|| "wecom bot_id is missing; configure wecom.bot_id or env".to_owned())?;
    let secret = resolved
        .secret()
        .ok_or_else(|| "wecom secret is missing; configure wecom.secret or env".to_owned())?;

    Ok(WecomConnectionConfig {
        websocket_url,
        bot_id,
        secret,
    })
}

async fn acquire_wecom_connection_owner(
    resolved: &ResolvedWecomChannelConfig,
) -> CliResult<runtime_state::ChannelOperationExclusiveGuard> {
    runtime_state::ChannelOperationExclusiveGuard::acquire(
        ChannelPlatform::Wecom,
        WECOM_CONNECTION_OWNER_OPERATION_ID,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
    )
    .await
}

fn ensure_wecom_send_runtime_is_exclusive(resolved: &ResolvedWecomChannelConfig) -> CliResult<()> {
    let runtime_dir = runtime_state::default_channel_runtime_state_dir();
    let now_ms = current_time_ms();
    runtime_state::prune_inactive_channel_operation_runtime_files_for_account_from_dir(
        runtime_dir.as_path(),
        ChannelPlatform::Wecom,
        CHANNEL_OPERATION_SERVE_ID,
        Some(resolved.account.id.as_str()),
        now_ms,
    )?;
    let runtime = runtime_state::load_channel_operation_runtime_for_account_from_dir(
        runtime_dir.as_path(),
        ChannelPlatform::Wecom,
        CHANNEL_OPERATION_SERVE_ID,
        resolved.account.id.as_str(),
        now_ms,
    );
    let Some(runtime) = runtime else {
        return Ok(());
    };
    if runtime.running_instances == 0 {
        return Ok(());
    }

    let process_id = runtime
        .pid
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    Err(format!(
        "wecom account `{}` already has an active serve runtime (pid={process_id}, running_instances={}); stop the existing `wecom-serve` session before using `wecom-send`",
        resolved.account.id, runtime.running_instances
    ))
}

fn is_wecom_allowed_conversation(
    resolved: &ResolvedWecomChannelConfig,
    conversation_id: &str,
) -> bool {
    resolved
        .allowed_conversation_ids
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .any(|allowed| allowed == conversation_id.trim())
}

async fn wait_for_wecom_reconnect_or_stop(
    stop: &ChannelServeStopHandle,
    reconnect_interval: Duration,
) -> bool {
    tokio::select! {
        _ = stop.wait() => true,
        _ = tokio::time::sleep(reconnect_interval) => false,
    }
}

#[allow(clippy::print_stderr)]
fn emit_wecom_warning(message: String) {
    eprintln!("warning: {message}");
}

fn is_disconnected_event(value: &Value) -> bool {
    let top_level_event = json_string_at(value, &["body", "event_type"]);
    let nested_event = json_string_at(value, &["body", "event", "type"]);
    let raw_event = top_level_event.or(nested_event).unwrap_or_default();
    raw_event.trim() == WECOM_EVENT_DISCONNECTED
}

fn extract_wecom_errcode(value: &Value) -> Option<i64> {
    json_i64_at(value, &["errcode"]).or_else(|| json_i64_at(value, &["body", "errcode"]))
}

fn extract_wecom_errmsg(value: &Value) -> Option<String> {
    json_string_at(value, &["errmsg"]).or_else(|| json_string_at(value, &["body", "errmsg"]))
}

fn json_string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    let string = current.as_str()?;
    let trimmed = string.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn json_i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_i64()
}

fn json_array_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Vec<Value>> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_array()
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn ensure_wecom_websocket_rustls_provider() {
    static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();

    RUSTLS_PROVIDER_INIT.get_or_init(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        routing::post,
    };
    use std::path::PathBuf;
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, Notify};
    use tokio_tungstenite::accept_async;

    use crate::channel::ChannelPlatform;
    use crate::config::ProviderConfig;
    use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_test_kernel_context};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockRequest {
        path: String,
        body: String,
    }

    #[derive(Clone, Default)]
    struct MockServerState {
        requests: Arc<Mutex<Vec<MockRequest>>>,
    }

    async fn record_request(State(state): State<MockServerState>, request: Request) {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, usize::MAX)
            .await
            .expect("read mock request body");
        let body = String::from_utf8(body.to_vec()).expect("mock request utf8");
        state.requests.lock().await.push(MockRequest {
            path: parts.uri.path().to_owned(),
            body,
        });
    }

    async fn spawn_mock_http_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock http server");
        let address = listener.local_addr().expect("mock http server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock http server");
        });
        (format!("http://{address}"), handle)
    }

    async fn spawn_mock_provider_server(
        requests: Arc<Mutex<Vec<MockRequest>>>,
        reply_text: &'static str,
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
                        Json(json!({
                            "choices": [{
                                "message": {
                                    "content": reply_text
                                }
                            }]
                        }))
                    }
                }
            }),
        );
        spawn_mock_http_server(router).await
    }

    fn temp_wecom_test_dir(label: &str) -> PathBuf {
        let timestamp_ms = current_time_ms();
        let process_id = std::process::id();
        let path = format!("loongclaw-wecom-{label}-{process_id}-{timestamp_ms}");
        std::env::temp_dir().join(path)
    }

    fn build_wecom_test_config(provider_base_url: &str, websocket_url: &str) -> LoongClawConfig {
        let temp_dir = temp_wecom_test_dir("runtime");
        std::fs::create_dir_all(&temp_dir).expect("create wecom temp dir");

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
        config.wecom.enabled = true;
        config.wecom.account_id = Some("wecom_main".to_owned());
        config.wecom.bot_id = Some(loongclaw_contracts::SecretRef::Inline(
            "bot_test".to_owned(),
        ));
        config.wecom.secret = Some(loongclaw_contracts::SecretRef::Inline(
            "secret_test".to_owned(),
        ));
        config.wecom.websocket_url = Some(websocket_url.to_owned());
        config.wecom.allowed_conversation_ids = vec!["group_demo".to_owned()];
        config
    }

    fn write_wecom_test_config_file(config: &LoongClawConfig, label: &str) -> PathBuf {
        let config_dir = temp_wecom_test_dir(label);
        std::fs::create_dir_all(&config_dir).expect("create wecom config dir");
        let config_path = config_dir.join("loongclaw.toml");
        let encoded = crate::config::render(config).expect("render wecom test config");
        std::fs::write(&config_path, encoded).expect("write wecom test config");
        config_path
    }

    async fn spawn_mock_wecom_send_server()
    -> (String, tokio::task::JoinHandle<CliResult<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock wecom send server");
        let address = listener.local_addr().expect("mock wecom send server addr");
        let handle = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut stream = accept_async(socket)
                .await
                .map_err(|error| format!("accept websocket failed: {error}"))?;
            let mut frames = Vec::new();

            let subscribe = read_text_frame(&mut stream).await?;
            let subscribe_req_id = json_string_at(&subscribe, &["headers", "req_id"])
                .ok_or_else(|| "missing subscribe req_id".to_owned())?;
            frames.push(subscribe);
            let subscribe_ack = json!({
                "cmd": WECOM_SUBSCRIBE_CMD,
                "headers": { "req_id": subscribe_req_id },
                "errcode": 0,
                "errmsg": "ok",
            });
            send_text_frame(&mut stream, &subscribe_ack).await?;

            let send = read_text_frame(&mut stream).await?;
            let send_req_id = json_string_at(&send, &["headers", "req_id"])
                .ok_or_else(|| "missing send req_id".to_owned())?;
            frames.push(send);
            let send_ack = json!({
                "cmd": WECOM_SEND_MSG_CMD,
                "headers": { "req_id": send_req_id },
                "errcode": 0,
                "errmsg": "ok",
            });
            send_text_frame(&mut stream, &send_ack).await?;
            stream
                .close(None)
                .await
                .map_err(|error| format!("close websocket failed: {error}"))?;
            Ok(frames)
        });
        (format!("ws://{address}/events"), handle)
    }

    async fn spawn_mock_wecom_inbound_server(
        reply_seen: Arc<Notify>,
        release_server: Arc<Notify>,
    ) -> (String, tokio::task::JoinHandle<CliResult<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock wecom inbound server");
        let address = listener
            .local_addr()
            .expect("mock wecom inbound server addr");
        let handle = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut stream = accept_async(socket)
                .await
                .map_err(|error| format!("accept websocket failed: {error}"))?;
            let mut frames = Vec::new();

            let subscribe = read_text_frame(&mut stream).await?;
            let subscribe_req_id = json_string_at(&subscribe, &["headers", "req_id"])
                .ok_or_else(|| "missing subscribe req_id".to_owned())?;
            frames.push(subscribe);
            let subscribe_ack = json!({
                "cmd": WECOM_SUBSCRIBE_CMD,
                "headers": { "req_id": subscribe_req_id },
                "errcode": 0,
                "errmsg": "ok",
            });
            send_text_frame(&mut stream, &subscribe_ack).await?;

            let inbound = json!({
                "cmd": WECOM_MESSAGE_CALLBACK_CMD,
                "headers": { "req_id": "req-inbound-1" },
                "body": {
                    "msgid": "msg-1",
                    "aibotid": "bot_test",
                    "chatid": "group_demo",
                    "chattype": "group",
                    "from": {
                        "userid": "user_demo"
                    },
                    "msgtype": "text",
                    "text": {
                        "content": "hello wecom"
                    }
                }
            });
            send_text_frame(&mut stream, &inbound).await?;

            loop {
                let frame = read_text_frame(&mut stream).await?;
                let command = json_string_at(&frame, &["cmd"]).unwrap_or_default();
                if command == WECOM_RESPOND_MSG_CMD {
                    frames.push(frame);
                    reply_seen.notify_waiters();
                    release_server.notified().await;
                    stream
                        .close(None)
                        .await
                        .map_err(|error| format!("close websocket failed: {error}"))?;
                    return Ok(frames);
                }
            }
        });
        (format!("ws://{address}/events"), handle)
    }

    async fn spawn_mock_wecom_non_allowlisted_inbound_server(
        release_server: Arc<Notify>,
        observation_done: Arc<Notify>,
    ) -> (String, tokio::task::JoinHandle<CliResult<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock wecom filtered inbound server");
        let address = listener
            .local_addr()
            .expect("mock wecom filtered inbound server addr");
        let handle = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut stream = accept_async(socket)
                .await
                .map_err(|error| format!("accept websocket failed: {error}"))?;
            let mut frames = Vec::new();

            let subscribe = read_text_frame(&mut stream).await?;
            let subscribe_req_id = json_string_at(&subscribe, &["headers", "req_id"])
                .ok_or_else(|| "missing subscribe req_id".to_owned())?;
            frames.push(subscribe);
            let subscribe_ack = json!({
                "cmd": WECOM_SUBSCRIBE_CMD,
                "headers": { "req_id": subscribe_req_id },
                "errcode": 0,
                "errmsg": "ok",
            });
            send_text_frame(&mut stream, &subscribe_ack).await?;

            let inbound = json!({
                "cmd": WECOM_MESSAGE_CALLBACK_CMD,
                "headers": { "req_id": "req-inbound-denied" },
                "body": {
                    "msgid": "msg-denied",
                    "aibotid": "bot_test",
                    "chatid": "group_denied",
                    "chattype": "group",
                    "from": {
                        "userid": "user_denied"
                    },
                    "msgtype": "text",
                    "text": {
                        "content": "ignore me"
                    }
                }
            });
            send_text_frame(&mut stream, &inbound).await?;

            let unexpected_frame_result =
                tokio::time::timeout(Duration::from_millis(300), read_text_frame(&mut stream))
                    .await;
            if let Ok(Ok(unexpected_frame)) = unexpected_frame_result {
                frames.push(unexpected_frame);
            }

            observation_done.notify_waiters();
            release_server.notified().await;
            stream
                .close(None)
                .await
                .map_err(|error| format!("close websocket failed: {error}"))?;
            Ok(frames)
        });
        (format!("ws://{address}/events"), handle)
    }

    async fn spawn_mock_wecom_reconnect_server(
        reply_seen: Arc<Notify>,
        release_server: Arc<Notify>,
    ) -> (String, tokio::task::JoinHandle<CliResult<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock wecom reconnect server");
        let address = listener
            .local_addr()
            .expect("mock wecom reconnect server addr");
        let handle = tokio::spawn(async move {
            let mut frames = Vec::new();

            let (first_socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut first_stream = accept_async(first_socket)
                .await
                .map_err(|error| format!("accept first websocket failed: {error}"))?;
            let first_subscribe = read_text_frame(&mut first_stream).await?;
            let first_req_id = json_string_at(&first_subscribe, &["headers", "req_id"])
                .ok_or_else(|| "missing first subscribe req_id".to_owned())?;
            frames.push(first_subscribe);
            let first_ack = json!({
                "cmd": WECOM_SUBSCRIBE_CMD,
                "headers": { "req_id": first_req_id },
                "errcode": 0,
                "errmsg": "ok",
            });
            send_text_frame(&mut first_stream, &first_ack).await?;
            first_stream
                .close(None)
                .await
                .map_err(|error| format!("close first websocket failed: {error}"))?;

            let (second_socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut second_stream = accept_async(second_socket)
                .await
                .map_err(|error| format!("accept second websocket failed: {error}"))?;
            let second_subscribe = read_text_frame(&mut second_stream).await?;
            let second_req_id = json_string_at(&second_subscribe, &["headers", "req_id"])
                .ok_or_else(|| "missing second subscribe req_id".to_owned())?;
            frames.push(second_subscribe);
            let second_ack = json!({
                "cmd": WECOM_SUBSCRIBE_CMD,
                "headers": { "req_id": second_req_id },
                "errcode": 0,
                "errmsg": "ok",
            });
            send_text_frame(&mut second_stream, &second_ack).await?;

            let inbound = json!({
                "cmd": WECOM_MESSAGE_CALLBACK_CMD,
                "headers": { "req_id": "req-reconnect-1" },
                "body": {
                    "msgid": "msg-reconnect-1",
                    "aibotid": "bot_test",
                    "chatid": "group_demo",
                    "chattype": "group",
                    "from": {
                        "userid": "user_demo"
                    },
                    "msgtype": "text",
                    "text": {
                        "content": "hello after reconnect"
                    }
                }
            });
            send_text_frame(&mut second_stream, &inbound).await?;

            loop {
                let frame = read_text_frame(&mut second_stream).await?;
                let command = json_string_at(&frame, &["cmd"]).unwrap_or_default();
                if command == WECOM_RESPOND_MSG_CMD {
                    frames.push(frame);
                    reply_seen.notify_waiters();
                    release_server.notified().await;
                    second_stream
                        .close(None)
                        .await
                        .map_err(|error| format!("close second websocket failed: {error}"))?;
                    return Ok(frames);
                }
            }
        });
        (format!("ws://{address}/events"), handle)
    }

    async fn read_text_frame<S>(
        stream: &mut tokio_tungstenite::WebSocketStream<S>,
    ) -> CliResult<Value>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            let next = stream.next().await;
            let message = match next {
                Some(Ok(message)) => message,
                Some(Err(error)) => return Err(format!("read websocket frame failed: {error}")),
                None => return Err("websocket peer disconnected".to_owned()),
            };

            match message {
                Message::Text(text) => {
                    let value = serde_json::from_str::<Value>(text.as_ref())
                        .map_err(|error| format!("decode websocket text frame failed: {error}"))?;
                    return Ok(value);
                }
                Message::Binary(bytes) => {
                    let text = std::str::from_utf8(bytes.as_ref()).map_err(|error| {
                        format!("decode websocket binary frame failed: {error}")
                    })?;
                    let value = serde_json::from_str::<Value>(text).map_err(|error| {
                        format!("decode websocket binary frame json failed: {error}")
                    })?;
                    return Ok(value);
                }
                Message::Ping(payload) => {
                    stream
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| format!("send websocket pong failed: {error}"))?;
                }
                Message::Pong(_) | Message::Frame(_) => {}
                Message::Close(_) => {
                    return Err("websocket closed before expected frame".to_owned());
                }
            }
        }
    }

    async fn send_text_frame<S>(
        stream: &mut tokio_tungstenite::WebSocketStream<S>,
        value: &Value,
    ) -> CliResult<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let payload = serde_json::to_string(value)
            .map_err(|error| format!("serialize websocket test frame failed: {error}"))?;
        stream
            .send(Message::Text(payload.into()))
            .await
            .map_err(|error| format!("send websocket test frame failed: {error}"))
    }

    #[test]
    fn parse_wecom_inbound_message_uses_group_chatid_and_participant() {
        let config = build_wecom_test_config("http://127.0.0.1:9", "ws://127.0.0.1:9");
        let resolved = config
            .wecom
            .resolve_account(None)
            .expect("resolve wecom config");
        let payload = json!({
            "cmd": WECOM_MESSAGE_CALLBACK_CMD,
            "headers": {
                "req_id": "req-123"
            },
            "body": {
                "msgid": "msg-123",
                "chatid": "group_demo",
                "chattype": "group",
                "from": {
                    "userid": "user_demo"
                },
                "msgtype": "text",
                "text": {
                    "content": "hello group"
                }
            }
        });

        let parsed = parse_wecom_inbound_message(&payload, &resolved)
            .expect("parse inbound message")
            .expect("inbound message should exist");

        assert_eq!(parsed.req_id, "req-123");
        assert_eq!(parsed.message.session.platform, ChannelPlatform::Wecom);
        assert_eq!(parsed.message.session.conversation_id, "group_demo");
        assert_eq!(
            parsed.message.session.participant_id.as_deref(),
            Some("user_demo")
        );
        assert_eq!(
            parsed.message.session.configured_account_id.as_deref(),
            Some("wecom_main")
        );
        assert_eq!(parsed.message.text, "hello group");
        assert_eq!(
            parsed.message.delivery.sender_principal_key.as_deref(),
            Some("wecom:user:user_demo")
        );
    }

    #[tokio::test]
    async fn run_wecom_send_subscribes_and_sends_markdown_message() {
        let (websocket_url, websocket_server) = spawn_mock_wecom_send_server().await;
        let mut config = build_wecom_test_config("http://127.0.0.1:9", websocket_url.as_str());
        config.wecom.account_id = Some("wecom_send_runtime_test".to_owned());
        let resolved = config
            .wecom
            .resolve_account(None)
            .expect("resolve wecom account");

        run_wecom_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "group_demo",
            "hello proactive wecom",
        )
        .await
        .expect("wecom send should succeed");

        let frames = websocket_server
            .await
            .expect("join websocket send server")
            .expect("mock websocket send server result");

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["cmd"], json!(WECOM_SUBSCRIBE_CMD));
        assert_eq!(frames[0]["body"]["bot_id"], json!("bot_test"));
        assert_eq!(frames[1]["cmd"], json!(WECOM_SEND_MSG_CMD));
        assert_eq!(frames[1]["body"]["chatid"], json!("group_demo"));
        assert_eq!(frames[1]["body"]["msgtype"], json!("markdown"));
        assert_eq!(
            frames[1]["body"]["markdown"]["content"],
            json!("hello proactive wecom")
        );
    }

    #[tokio::test]
    async fn send_wecom_text_rejects_active_serve_runtime_before_owner_conflict() {
        let runtime_dir = runtime_state::default_channel_runtime_state_dir();
        let now_ms = current_time_ms();
        let mut config = build_wecom_test_config("http://127.0.0.1:9", "ws://127.0.0.1:9");
        config.wecom.account_id = Some("wecom_runtime_block_test".to_owned());
        let resolved = config
            .wecom
            .resolve_account(None)
            .expect("resolve wecom account");

        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Wecom,
            CHANNEL_OPERATION_SERVE_ID,
            resolved.account.id.as_str(),
            7001,
            true,
            false,
            0,
            Some(now_ms),
            Some(now_ms),
            Some(7001),
        )
        .expect("seed wecom serve runtime");

        let _owner_guard = runtime_state::ChannelOperationExclusiveGuard::acquire(
            ChannelPlatform::Wecom,
            WECOM_CONNECTION_OWNER_OPERATION_ID,
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("seed wecom owner guard");

        let error = send_wecom_text(&resolved, "group_demo", None, "hello active runtime")
            .await
            .expect_err("active serve runtime should block proactive send");

        assert!(error.contains("already has an active serve runtime"));
        assert!(error.contains("wecom-send"));
    }

    #[tokio::test]
    async fn run_wecom_serve_session_reaches_provider_and_replies() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone(), "wecom inbound ack").await;
        let reply_seen = Arc::new(Notify::new());
        let release_server = Arc::new(Notify::new());
        let (websocket_url, websocket_server) =
            spawn_mock_wecom_inbound_server(reply_seen.clone(), release_server.clone()).await;

        let config = build_wecom_test_config(provider_base_url.as_str(), websocket_url.as_str());
        let connection = resolve_wecom_connection_config(
            &config
                .wecom
                .resolve_account(None)
                .expect("resolve wecom connection config"),
        )
        .expect("resolve wecom connection details");
        let resolved_path = write_wecom_test_config_file(&config, "config");
        let resolved = config
            .wecom
            .resolve_account(None)
            .expect("resolve wecom account");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Wecom,
                CHANNEL_OPERATION_SERVE_ID,
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let kernel_ctx = bootstrap_test_kernel_context("wecom-channel-test", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
        let stop = ChannelServeStopHandle::new();
        let stop_for_task = stop.clone();
        let config_for_task = config.clone();
        let resolved_for_task = resolved.clone();
        let runtime_for_task = runtime.clone();
        let session_task = tokio::spawn(async move {
            run_wecom_serve_session(
                &config_for_task,
                resolved_path.as_path(),
                &resolved_for_task,
                &connection,
                kernel_ctx,
                runtime_for_task,
                stop_for_task,
            )
            .await
        });

        reply_seen.notified().await;
        stop.request_stop();
        release_server.notify_waiters();

        let outcome = session_task
            .await
            .expect("join wecom serve session")
            .expect("wecom serve session result");
        assert_eq!(outcome, WecomServeSessionOutcome::Stopped);
        runtime.shutdown().await.expect("shutdown runtime tracker");

        let websocket_frames = websocket_server
            .await
            .expect("join websocket inbound server")
            .expect("mock websocket inbound server result");
        assert_eq!(websocket_frames.len(), 2);
        assert_eq!(websocket_frames[1]["cmd"], json!(WECOM_RESPOND_MSG_CMD));
        assert_eq!(
            websocket_frames[1]["headers"]["req_id"],
            json!("req-inbound-1")
        );
        assert_eq!(websocket_frames[1]["body"]["msgtype"], json!("text"));
        assert_eq!(
            websocket_frames[1]["body"]["text"]["content"],
            json!("wecom inbound ack")
        );

        let provider_requests = provider_requests.lock().await;
        assert_eq!(provider_requests.len(), 1);
        assert_eq!(provider_requests[0].path, "/v1/chat/completions");

        provider_server.abort();
    }

    #[tokio::test]
    async fn run_wecom_serve_session_skips_non_allowlisted_conversations() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone(), "should not be used").await;
        let release_server = Arc::new(Notify::new());
        let observation_done = Arc::new(Notify::new());
        let (websocket_url, websocket_server) = spawn_mock_wecom_non_allowlisted_inbound_server(
            release_server.clone(),
            observation_done.clone(),
        )
        .await;

        let config = build_wecom_test_config(provider_base_url.as_str(), websocket_url.as_str());
        let connection = resolve_wecom_connection_config(
            &config
                .wecom
                .resolve_account(None)
                .expect("resolve wecom connection config"),
        )
        .expect("resolve wecom connection details");
        let resolved_path = write_wecom_test_config_file(&config, "config-denied");
        let resolved = config
            .wecom
            .resolve_account(None)
            .expect("resolve wecom account");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Wecom,
                CHANNEL_OPERATION_SERVE_ID,
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start runtime tracker"),
        );
        let kernel_ctx =
            bootstrap_test_kernel_context("wecom-channel-test-denied", DEFAULT_TOKEN_TTL_S)
                .expect("bootstrap kernel context");
        let stop = ChannelServeStopHandle::new();
        let stop_for_task = stop.clone();
        let config_for_task = config.clone();
        let resolved_for_task = resolved.clone();
        let runtime_for_task = runtime.clone();
        let session_task = tokio::spawn(async move {
            run_wecom_serve_session(
                &config_for_task,
                resolved_path.as_path(),
                &resolved_for_task,
                &connection,
                kernel_ctx,
                runtime_for_task,
                stop_for_task,
            )
            .await
        });

        observation_done.notified().await;
        stop.request_stop();
        release_server.notify_waiters();

        let outcome = session_task
            .await
            .expect("join denied serve session")
            .expect("denied serve session result");
        assert_eq!(outcome, WecomServeSessionOutcome::Stopped);
        runtime.shutdown().await.expect("shutdown runtime tracker");

        let websocket_frames = websocket_server
            .await
            .expect("join denied websocket server")
            .expect("denied websocket server result");
        assert_eq!(websocket_frames.len(), 1);
        assert_eq!(websocket_frames[0]["cmd"], json!(WECOM_SUBSCRIBE_CMD));

        let provider_requests = provider_requests.lock().await;
        assert!(provider_requests.is_empty());

        provider_server.abort();
    }

    #[tokio::test]
    async fn run_wecom_channel_reconnects_after_transport_drop() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone(), "reconnect ack").await;
        let reply_seen = Arc::new(Notify::new());
        let release_server = Arc::new(Notify::new());
        let (websocket_url, websocket_server) =
            spawn_mock_wecom_reconnect_server(reply_seen.clone(), release_server.clone()).await;

        let mut config =
            build_wecom_test_config(provider_base_url.as_str(), websocket_url.as_str());
        config.wecom.reconnect_interval_s = 1;
        let resolved_path = write_wecom_test_config_file(&config, "config-reconnect");
        let resolved = config
            .wecom
            .resolve_account(None)
            .expect("resolve reconnect wecom account");
        let runtime = Arc::new(
            ChannelOperationRuntimeTracker::start(
                ChannelPlatform::Wecom,
                CHANNEL_OPERATION_SERVE_ID,
                resolved.account.id.as_str(),
                resolved.account.label.as_str(),
            )
            .await
            .expect("start reconnect runtime tracker"),
        );
        let kernel_ctx =
            bootstrap_test_kernel_context("wecom-channel-test-reconnect", DEFAULT_TOKEN_TTL_S)
                .expect("bootstrap reconnect kernel context");
        let stop = ChannelServeStopHandle::new();
        let stop_for_task = stop.clone();
        let config_for_task = config.clone();
        let resolved_for_task = resolved.clone();
        let runtime_for_task = runtime.clone();
        let channel_task = tokio::spawn(async move {
            run_wecom_channel(
                &config_for_task,
                &resolved_for_task,
                resolved_path.as_path(),
                true,
                ChannelDefaultAccountSelectionSource::ExplicitDefault,
                kernel_ctx,
                runtime_for_task,
                stop_for_task,
            )
            .await
        });

        reply_seen.notified().await;
        stop.request_stop();
        release_server.notify_waiters();

        channel_task
            .await
            .expect("join reconnect channel task")
            .expect("reconnect channel result");
        runtime
            .shutdown()
            .await
            .expect("shutdown reconnect runtime");

        let websocket_frames = websocket_server
            .await
            .expect("join reconnect websocket server")
            .expect("reconnect websocket server result");
        assert_eq!(websocket_frames.len(), 3);
        assert_eq!(websocket_frames[0]["cmd"], json!(WECOM_SUBSCRIBE_CMD));
        assert_eq!(websocket_frames[1]["cmd"], json!(WECOM_SUBSCRIBE_CMD));
        assert_eq!(websocket_frames[2]["cmd"], json!(WECOM_RESPOND_MSG_CMD));

        let provider_requests = provider_requests.lock().await;
        assert_eq!(provider_requests.len(), 1);
        assert_eq!(provider_requests[0].path, "/v1/chat/completions");

        provider_server.abort();
    }
}
