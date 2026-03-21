use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::CliResult;
use crate::channel::feishu::FeishuUserPrincipal;
use crate::channel::{
    ChannelDeliveryResource, ChannelOutboundTarget, ChannelPlatform, ChannelSession,
};

use super::crypto::decrypt_payload_if_needed;
use super::types::{
    FeishuCardCallbackAction, FeishuCardCallbackContext, FeishuCardCallbackEvent,
    FeishuCardCallbackVersion, FeishuInboundEvent, FeishuWebhookAction,
};

const FEISHU_STRUCTURED_ONLY_MESSAGE_TYPES: &[&str] = &[
    "folder",
    "sticker",
    "interactive",
    "share_chat",
    "share_user",
    "system",
    "location",
    "video_chat",
    "todo",
    "vote",
    "merge_forward",
    "share_calendar_event",
    "calendar",
    "general_calendar",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::channel::feishu) enum FeishuTransportAuth<'a> {
    Webhook {
        verification_token: Option<&'a str>,
        encrypt_key: Option<&'a str>,
    },
    Websocket,
}

impl<'a> FeishuTransportAuth<'a> {
    pub(in crate::channel::feishu) fn webhook(
        verification_token: Option<&'a str>,
        encrypt_key: Option<&'a str>,
    ) -> Self {
        Self::Webhook {
            verification_token,
            encrypt_key,
        }
    }

    pub(in crate::channel::feishu) fn websocket() -> Self {
        Self::Websocket
    }

    fn verification_token(self) -> Option<&'a str> {
        match self {
            Self::Webhook {
                verification_token, ..
            } => verification_token,
            Self::Websocket => None,
        }
    }

    fn encrypt_key(self) -> Option<&'a str> {
        match self {
            Self::Webhook { encrypt_key, .. } => encrypt_key,
            Self::Websocket => None,
        }
    }

    fn should_verify_token(self) -> bool {
        matches!(self, Self::Webhook { .. })
    }

    fn should_decrypt(self) -> bool {
        matches!(self, Self::Webhook { .. })
    }
}

pub(in crate::channel::feishu) fn parse_feishu_inbound_payload(
    payload: &Value,
    transport_auth: FeishuTransportAuth<'_>,
    allowed_chat_ids: &BTreeSet<String>,
    ignore_bot_messages: bool,
    configured_account_id: &str,
    account_id: &str,
) -> CliResult<FeishuWebhookAction> {
    let decrypted_payload = if transport_auth.should_decrypt() {
        decrypt_payload_if_needed(payload, transport_auth.encrypt_key())?
    } else {
        None
    };
    let payload = decrypted_payload.as_ref().unwrap_or(payload);

    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        if transport_auth.should_verify_token() {
            verify_feishu_token(payload, transport_auth.verification_token())?;
        }
        let challenge = payload
            .get("challenge")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "feishu url_verification payload missing challenge".to_owned())?;
        return Ok(FeishuWebhookAction::UrlVerification {
            challenge: challenge.to_owned(),
        });
    }

    let event_type = payload
        .get("header")
        .and_then(|header| header.get("event_type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if event_type == "card.action.trigger" {
        if transport_auth.should_verify_token() {
            verify_feishu_token(payload, transport_auth.verification_token())?;
        }
        return parse_feishu_card_callback_v2(
            payload,
            allowed_chat_ids,
            configured_account_id,
            account_id,
        );
    }
    if event_type == "card.action.trigger_v1" || looks_like_feishu_legacy_card_callback(payload) {
        if transport_auth.should_verify_token() {
            verify_feishu_token(payload, transport_auth.verification_token())?;
        }
        return parse_feishu_card_callback_v1(
            payload,
            allowed_chat_ids,
            configured_account_id,
            account_id,
        );
    }
    if event_type != "im.message.receive_v1" {
        return Ok(FeishuWebhookAction::Ignore);
    }

    if transport_auth.should_verify_token() {
        verify_feishu_token(payload, transport_auth.verification_token())?;
    }

    let event = payload
        .get("event")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu message event payload missing event object".to_owned())?;

    let sender_type = event
        .get("sender")
        .and_then(|sender| sender.get("sender_type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if ignore_bot_messages && matches!(sender_type, "app" | "bot") {
        return Ok(FeishuWebhookAction::Ignore);
    }

    let message = event
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu message event payload missing message object".to_owned())?;

    let message_type = message
        .get("message_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_supported_feishu_inbound_message_type(message_type) {
        return Ok(FeishuWebhookAction::Ignore);
    }

    let chat_id = message
        .get("chat_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "feishu message event missing message.chat_id".to_owned())?;
    if !allowed_chat_ids.contains(chat_id) {
        return Ok(FeishuWebhookAction::Ignore);
    }

    let message_id = required_string_field(message, "message_id")
        .ok_or_else(|| "feishu message event missing message.message_id".to_owned())?;
    let root_id = optional_string_field(message, "root_id");
    let parent_id = optional_string_field(message, "parent_id");
    let thread_id = optional_string_field(message, "thread_id")
        .or_else(|| root_id.clone())
        .or_else(|| parent_id.clone());
    let principal = parse_sender_principal(event, account_id);

    let content = message
        .get("content")
        .ok_or_else(|| "feishu message event missing message.content".to_owned())?;
    let text = parse_feishu_inbound_message_text(message_type, content).ok_or_else(|| {
        format!("feishu message content is not a supported non-empty {message_type} payload")
    })?;
    let resources = parse_feishu_inbound_message_resources(message_type, content);

    let event_id = payload
        .get("header")
        .and_then(|header| header.get("event_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("message:{message_id}"));

    let mut reply_target = ChannelOutboundTarget::feishu_message_reply(message_id.to_owned());
    if thread_id.is_some() {
        reply_target = reply_target.with_feishu_reply_in_thread(true);
    }

    Ok(FeishuWebhookAction::Inbound(FeishuInboundEvent {
        event_id,
        message_id,
        root_id,
        parent_id,
        session: {
            let mut session = ChannelSession::with_account(
                ChannelPlatform::Feishu,
                account_id,
                chat_id.to_owned(),
            )
            .with_configured_account_id(configured_account_id);
            if let Some(principal) = principal.as_ref() {
                session = session.with_participant_id(principal.open_id.clone());
            }
            if let Some(thread_id) = thread_id {
                session = session.with_thread_id(thread_id);
            }
            session
        },
        principal,
        reply_target,
        text,
        resources,
    }))
}

pub(in crate::channel::feishu) fn parse_feishu_webhook_payload(
    payload: &Value,
    verification_token: Option<&str>,
    encrypt_key: Option<&str>,
    allowed_chat_ids: &BTreeSet<String>,
    ignore_bot_messages: bool,
    configured_account_id: &str,
    account_id: &str,
) -> CliResult<FeishuWebhookAction> {
    parse_feishu_inbound_payload(
        payload,
        FeishuTransportAuth::webhook(verification_token, encrypt_key),
        allowed_chat_ids,
        ignore_bot_messages,
        configured_account_id,
        account_id,
    )
}

pub(in crate::channel::feishu) fn normalize_webhook_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "/feishu/events".to_owned();
    }
    if trimmed.starts_with('/') {
        return trimmed.to_owned();
    }
    format!("/{trimmed}")
}

fn verify_feishu_token(payload: &Value, verification_token: Option<&str>) -> CliResult<()> {
    let Some(expected_token) = verification_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err("unauthorized: feishu verification token is not configured".to_owned());
    };

    let incoming = payload
        .get("header")
        .and_then(|header| header.get("token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            payload
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    if incoming.is_empty() {
        return Err("unauthorized: feishu payload missing token".to_owned());
    }
    if incoming != expected_token {
        return Err("unauthorized: feishu verification token mismatch".to_owned());
    }
    Ok(())
}

fn looks_like_feishu_legacy_card_callback(payload: &Value) -> bool {
    let Some(object) = payload.as_object() else {
        return false;
    };
    object.get("action").and_then(Value::as_object).is_some()
        && required_string_field(object, "open_message_id").is_some()
}

fn parse_feishu_card_callback_v2(
    payload: &Value,
    allowed_chat_ids: &BTreeSet<String>,
    configured_account_id: &str,
    account_id: &str,
) -> CliResult<FeishuWebhookAction> {
    let event = payload
        .get("event")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu card callback payload missing event object".to_owned())?;
    let context = event.get("context").and_then(Value::as_object);
    let open_chat_id = context.and_then(|value| optional_string_field(value, "open_chat_id"));
    if !is_allowed_feishu_card_callback_chat(open_chat_id.as_deref(), allowed_chat_ids) {
        return Ok(FeishuWebhookAction::Ignore);
    }

    let action = parse_feishu_card_callback_action(event, "action")?;
    let principal = parse_feishu_card_callback_principal_v2(event, account_id);
    let callback = build_feishu_card_callback_event(
        payload,
        FeishuCardCallbackVersion::V2,
        configured_account_id,
        account_id,
        principal,
        optional_string_field(event, "token"),
        action,
        FeishuCardCallbackContext {
            open_message_id: context
                .and_then(|value| optional_string_field(value, "open_message_id")),
            open_chat_id,
            url: context.and_then(|value| optional_string_field(value, "url")),
            preview_token: context.and_then(|value| optional_string_field(value, "preview_token")),
        },
    )?;
    Ok(FeishuWebhookAction::CardCallback(callback))
}

fn parse_feishu_card_callback_v1(
    payload: &Value,
    allowed_chat_ids: &BTreeSet<String>,
    configured_account_id: &str,
    account_id: &str,
) -> CliResult<FeishuWebhookAction> {
    let object = payload
        .as_object()
        .ok_or_else(|| "feishu legacy card callback payload must be a JSON object".to_owned())?;
    let open_chat_id = optional_string_field(object, "open_chat_id");
    if !is_allowed_feishu_card_callback_chat(open_chat_id.as_deref(), allowed_chat_ids) {
        return Ok(FeishuWebhookAction::Ignore);
    }

    let action = parse_feishu_card_callback_action(object, "action")?;
    let callback = build_feishu_card_callback_event(
        payload,
        FeishuCardCallbackVersion::V1,
        configured_account_id,
        account_id,
        parse_feishu_card_callback_principal_v1(object, account_id),
        None,
        action,
        FeishuCardCallbackContext {
            open_message_id: optional_string_field(object, "open_message_id"),
            open_chat_id,
            url: None,
            preview_token: None,
        },
    )?;
    Ok(FeishuWebhookAction::CardCallback(callback))
}

fn build_feishu_card_callback_event(
    payload: &Value,
    version: FeishuCardCallbackVersion,
    configured_account_id: &str,
    account_id: &str,
    principal: Option<FeishuUserPrincipal>,
    callback_token: Option<String>,
    action: FeishuCardCallbackAction,
    context: FeishuCardCallbackContext,
) -> CliResult<FeishuCardCallbackEvent> {
    let conversation_id = context
        .open_chat_id
        .as_deref()
        .or(context.open_message_id.as_deref())
        .or(context.preview_token.as_deref())
        .or(context.url.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "feishu card callback missing routable conversation context".to_owned())?
        .to_owned();
    let mut session =
        ChannelSession::with_account(ChannelPlatform::Feishu, account_id, conversation_id)
            .with_configured_account_id(configured_account_id);
    if let Some(principal) = principal.as_ref() {
        session = session.with_participant_id(principal.open_id.clone());
    }
    if let Some(open_message_id) = context.open_message_id.as_deref() {
        session = session.with_thread_id(open_message_id.to_owned());
    }

    let event_id = payload
        .get("header")
        .and_then(Value::as_object)
        .and_then(|header| required_string_field(header, "event_id"))
        .unwrap_or_else(|| {
            build_feishu_card_callback_event_id(version, &principal, &context, &action)
        });

    let text = summarize_feishu_card_callback(version, principal.as_ref(), &context, &action);

    Ok(FeishuCardCallbackEvent {
        event_id,
        version,
        session,
        principal,
        callback_token,
        action,
        context,
        text,
    })
}

fn is_allowed_feishu_card_callback_chat(
    open_chat_id: Option<&str>,
    allowed_chat_ids: &BTreeSet<String>,
) -> bool {
    if allowed_chat_ids.is_empty() {
        return true;
    }
    let Some(open_chat_id) = open_chat_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    allowed_chat_ids.contains(open_chat_id)
}

fn parse_feishu_card_callback_action(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> CliResult<FeishuCardCallbackAction> {
    let action = object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("feishu card callback payload missing {key} object"))?;
    let tag = required_string_field(action, "tag")
        .ok_or_else(|| "feishu card callback action missing tag".to_owned())?;
    Ok(FeishuCardCallbackAction {
        tag,
        name: optional_string_field(action, "name"),
        value: optional_json_field(action, "value"),
        form_value: optional_json_field(action, "form_value"),
        timezone: optional_string_field(action, "timezone"),
    })
}

fn parse_feishu_card_callback_principal_v2(
    event: &serde_json::Map<String, Value>,
    account_id: &str,
) -> Option<FeishuUserPrincipal> {
    let operator = event.get("operator")?.as_object()?;
    let operator_id = operator
        .get("operator_id")
        .and_then(Value::as_object)
        .unwrap_or(operator);
    let open_id = required_string_field(operator_id, "open_id")?;
    Some(FeishuUserPrincipal {
        account_id: account_id.trim().to_owned(),
        open_id,
        union_id: optional_string_field(operator_id, "union_id"),
        user_id: optional_string_field(operator_id, "user_id"),
        name: None,
        tenant_key: None,
        avatar_url: None,
        email: None,
        enterprise_email: None,
    })
}

fn parse_feishu_card_callback_principal_v1(
    payload: &serde_json::Map<String, Value>,
    account_id: &str,
) -> Option<FeishuUserPrincipal> {
    let open_id = required_string_field(payload, "open_id")?;
    Some(FeishuUserPrincipal {
        account_id: account_id.trim().to_owned(),
        open_id,
        union_id: optional_string_field(payload, "union_id"),
        user_id: optional_string_field(payload, "user_id"),
        name: None,
        tenant_key: optional_string_field(payload, "tenant_key"),
        avatar_url: None,
        email: None,
        enterprise_email: None,
    })
}

fn build_feishu_card_callback_event_id(
    version: FeishuCardCallbackVersion,
    principal: &Option<FeishuUserPrincipal>,
    context: &FeishuCardCallbackContext,
    action: &FeishuCardCallbackAction,
) -> String {
    let operator = principal
        .as_ref()
        .map(|value| value.open_id.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown_operator");
    let message = context
        .open_message_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown_message");
    let action_fingerprint = serde_json::to_string(&Value::Object({
        let mut body = Map::new();
        body.insert("tag".to_owned(), Value::String(action.tag.clone()));
        if let Some(name) = action.name.as_ref() {
            body.insert("name".to_owned(), Value::String(name.clone()));
        }
        if let Some(value) = action.value.as_ref() {
            body.insert("value".to_owned(), value.clone());
        }
        if let Some(form_value) = action.form_value.as_ref() {
            body.insert("form_value".to_owned(), form_value.clone());
        }
        if let Some(timezone) = action.timezone.as_ref() {
            body.insert("timezone".to_owned(), Value::String(timezone.clone()));
        }
        body
    }))
    .unwrap_or_else(|_| action.tag.clone());
    format!(
        "card_callback:{}:{}:{}:{}",
        feishu_card_callback_version_label(version),
        message,
        operator,
        action_fingerprint
    )
}

fn summarize_feishu_card_callback(
    version: FeishuCardCallbackVersion,
    principal: Option<&FeishuUserPrincipal>,
    context: &FeishuCardCallbackContext,
    action: &FeishuCardCallbackAction,
) -> String {
    let mut summary = Map::new();
    summary.insert(
        "callback_version".to_owned(),
        Value::String(feishu_card_callback_version_label(version).to_owned()),
    );
    summary.insert("tag".to_owned(), Value::String(action.tag.clone()));
    if let Some(name) = action.name.as_ref() {
        summary.insert("name".to_owned(), Value::String(name.clone()));
    }
    if let Some(value) = action.value.as_ref() {
        summary.insert("value".to_owned(), value.clone());
    }
    if let Some(form_value) = action.form_value.as_ref() {
        summary.insert("form_value".to_owned(), form_value.clone());
    }
    if let Some(timezone) = action.timezone.as_ref() {
        summary.insert("timezone".to_owned(), Value::String(timezone.clone()));
    }
    if let Some(principal) = principal {
        summary.insert(
            "operator_open_id".to_owned(),
            Value::String(principal.open_id.clone()),
        );
        if let Some(user_id) = principal.user_id.as_ref() {
            summary.insert(
                "operator_user_id".to_owned(),
                Value::String(user_id.clone()),
            );
        }
    }
    insert_summary_string(
        &mut summary,
        "open_message_id",
        context.open_message_id.clone(),
    );
    insert_summary_string(&mut summary, "open_chat_id", context.open_chat_id.clone());
    insert_summary_string(&mut summary, "url", context.url.clone());
    insert_summary_string(&mut summary, "preview_token", context.preview_token.clone());
    summary.insert(
        "card_update_hint".to_owned(),
        Value::String(
            "Use feishu.card.update for delayed updates within 30 minutes: callback tokens can be used at most twice; pass markdown for a standard markdown card or card for full Feishu card JSON; set shared=true for shared cards and keep open_ids empty; for non-shared cards, pass explicit open_ids or omit open_ids in callback turns to target the callback operator by default.".to_owned(),
        ),
    );
    summary.insert(
        "callback_response_hint".to_owned(),
        Value::String(
            "To show an immediate callback toast, respond exactly with [feishu_callback_response] followed by {\"mode\":\"toast\",\"kind\":\"success|info|warning|error\",\"content\":\"your message\"}; to return an immediate callback card body, use {\"mode\":\"card\",\"markdown\":\"your markdown\"} for a standard markdown card or {\"mode\":\"card\",\"card\":{...}} for full card JSON, and optionally include \"toast\":{\"kind\":\"success|info|warning|error\",\"content\":\"your message\"}.".to_owned(),
        ),
    );
    summary.insert(
        "note".to_owned(),
        Value::String("Feishu card callback preserved with action and host context.".to_owned()),
    );
    let body = serde_json::to_string(&Value::Object(summary))
        .unwrap_or_else(|_| "{\"callback_version\":\"unknown\"}".to_owned());
    format!("[feishu_card_callback]\n{body}")
}

fn feishu_card_callback_version_label(version: FeishuCardCallbackVersion) -> &'static str {
    match version {
        FeishuCardCallbackVersion::V1 => "v1",
        FeishuCardCallbackVersion::V2 => "v2",
    }
}

fn optional_json_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<Value> {
    match object.get(key) {
        Some(Value::Null) | None => None,
        Some(value) => Some(value.clone()),
    }
}

fn parse_feishu_inbound_message_text(message_type: &str, content: &Value) -> Option<String> {
    match message_type {
        "text" => parse_feishu_text_content(content),
        "image" | "file" | "post" | "audio" | "media" => {
            summarize_feishu_rich_content(message_type, content)
        }
        message_type if FEISHU_STRUCTURED_ONLY_MESSAGE_TYPES.contains(&message_type) => {
            summarize_feishu_structured_only_content(message_type, content)
        }
        _ => None,
    }
}

fn parse_feishu_inbound_message_resources(
    message_type: &str,
    content: &Value,
) -> Vec<ChannelDeliveryResource> {
    let Some(normalized) = normalize_feishu_content(content) else {
        return Vec::new();
    };

    match message_type {
        "image" => {
            let mut resources = Vec::new();
            push_channel_delivery_resource(
                &mut resources,
                "image",
                find_first_string_field(&normalized, "image_key"),
                None,
            );
            resources
        }
        "file" | "audio" => {
            let mut resources = Vec::new();
            push_channel_delivery_resource(
                &mut resources,
                "file",
                find_first_string_field(&normalized, "file_key"),
                find_first_string_field(&normalized, "file_name"),
            );
            resources
        }
        "media" => {
            let mut resources = Vec::new();
            push_channel_delivery_resource(
                &mut resources,
                "file",
                find_first_string_field(&normalized, "file_key"),
                find_first_string_field(&normalized, "file_name"),
            );
            push_channel_delivery_resource(
                &mut resources,
                "image",
                find_first_string_field(&normalized, "image_key"),
                None,
            );
            resources
        }
        "post" => collect_post_delivery_resources(&normalized),
        _ => Vec::new(),
    }
}

fn parse_feishu_text_content(content: &Value) -> Option<String> {
    #[allow(clippy::wildcard_enum_match_arm)]
    match content {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                return parse_feishu_text_content(&parsed);
            }
            Some(trimmed.to_owned())
        }
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        _ => None,
    }
}

fn is_supported_feishu_inbound_message_type(message_type: &str) -> bool {
    matches!(
        message_type,
        "text" | "image" | "file" | "post" | "audio" | "media"
    ) || FEISHU_STRUCTURED_ONLY_MESSAGE_TYPES.contains(&message_type)
}

fn summarize_feishu_rich_content(message_type: &str, content: &Value) -> Option<String> {
    let normalized = normalize_feishu_content(content)?;
    let mut summary = Map::new();
    summary.insert(
        "message_type".to_owned(),
        Value::String(message_type.to_owned()),
    );
    match message_type {
        "image" => {
            insert_summary_string(
                &mut summary,
                "image_key",
                find_first_string_field(&normalized, "image_key"),
            );
            summary.insert(
                "note".to_owned(),
                Value::String("Binary image content is not fetched automatically.".to_owned()),
            );
        }
        "file" => {
            insert_summary_string(
                &mut summary,
                "file_key",
                find_first_string_field(&normalized, "file_key"),
            );
            insert_summary_string(
                &mut summary,
                "file_name",
                find_first_string_field(&normalized, "file_name"),
            );
            summary.insert(
                "note".to_owned(),
                Value::String("Binary file content is not fetched automatically.".to_owned()),
            );
        }
        "audio" => {
            insert_summary_string(
                &mut summary,
                "file_key",
                find_first_string_field(&normalized, "file_key"),
            );
            summary.insert(
                "note".to_owned(),
                Value::String("Binary audio content is not fetched automatically.".to_owned()),
            );
        }
        "media" => {
            insert_summary_string(
                &mut summary,
                "file_key",
                find_first_string_field(&normalized, "file_key"),
            );
            insert_summary_string(
                &mut summary,
                "image_key",
                find_first_string_field(&normalized, "image_key"),
            );
            insert_summary_string(
                &mut summary,
                "file_name",
                find_first_string_field(&normalized, "file_name"),
            );
            summary.insert(
                "note".to_owned(),
                Value::String("Binary media content is not fetched automatically.".to_owned()),
            );
        }
        "post" => {
            insert_summary_string(
                &mut summary,
                "title",
                find_first_string_field(&normalized, "title"),
            );
            let text_fragments = collect_named_string_fields(&normalized, "text");
            if !text_fragments.is_empty() {
                summary.insert("text".to_owned(), Value::String(text_fragments.join(" ")));
            }
        }
        _ => return None,
    }
    insert_resource_download_hints(&mut summary, message_type, &normalized);
    insert_resource_inventory(&mut summary, message_type, &normalized);
    summary.insert("content".to_owned(), normalized);
    serde_json::to_string(&Value::Object(summary))
        .ok()
        .map(|body| format!("[feishu_inbound_message]\n{body}"))
}

fn insert_resource_download_hints(
    summary: &mut Map<String, Value>,
    message_type: &str,
    normalized: &Value,
) {
    let hints = build_resource_download_hints(message_type, normalized);
    if hints.is_empty() {
        return;
    }
    summary.insert("resource_download_hints".to_owned(), Value::Array(hints));
}

fn insert_resource_inventory(
    summary: &mut Map<String, Value>,
    message_type: &str,
    normalized: &Value,
) {
    let mut inventory = build_resource_inventory(message_type, normalized);
    if inventory.is_empty() {
        return;
    }

    let selection_required = inventory.len() > 1;
    for item in &mut inventory {
        if let Value::Object(entry) = item {
            entry.insert(
                "selection_required".to_owned(),
                Value::Bool(selection_required),
            );
        }
    }

    summary.insert("resource_inventory".to_owned(), Value::Array(inventory));
    summary.insert(
        "resource_selection_hint".to_owned(),
        Value::String(build_resource_selection_hint(selection_required)),
    );
}

fn build_resource_inventory(message_type: &str, normalized: &Value) -> Vec<Value> {
    match message_type {
        "image" => build_resource_inventory_entry(
            "image",
            "image",
            "image",
            "image_key",
            find_first_string_field(normalized, "image_key"),
            None,
        )
        .into_iter()
        .collect(),
        "file" => build_resource_inventory_entry(
            "file",
            "file",
            "file",
            "file_key",
            find_first_string_field(normalized, "file_key"),
            find_first_string_field(normalized, "file_name"),
        )
        .into_iter()
        .collect(),
        "audio" => build_resource_inventory_entry(
            "audio_file",
            "audio",
            "file",
            "file_key",
            find_first_string_field(normalized, "file_key"),
            find_first_string_field(normalized, "file_name"),
        )
        .into_iter()
        .collect(),
        "media" => [
            build_resource_inventory_entry(
                "media_file",
                "media",
                "file",
                "file_key",
                find_first_string_field(normalized, "file_key"),
                find_first_string_field(normalized, "file_name"),
            ),
            build_resource_inventory_entry(
                "media_preview_image",
                "image",
                "image",
                "image_key",
                find_first_string_field(normalized, "image_key"),
                None,
            ),
        ]
        .into_iter()
        .flatten()
        .collect(),
        "post" => collect_post_delivery_resources(normalized)
            .into_iter()
            .filter_map(|resource| match resource.resource_type.as_str() {
                "file" => build_resource_inventory_entry(
                    "post_file",
                    "file",
                    "file",
                    "file_key",
                    Some(resource.file_key),
                    resource.file_name,
                ),
                "image" => build_resource_inventory_entry(
                    "post_image",
                    "image",
                    "image",
                    "image_key",
                    Some(resource.file_key),
                    resource.file_name,
                ),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn build_resource_inventory_entry(
    role: &str,
    payload_type: &str,
    canonical_type: &str,
    source_key_field: &str,
    file_key: Option<String>,
    file_name: Option<String>,
) -> Option<Value> {
    let file_key = file_key
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())?;

    let mut item = Map::new();
    item.insert("role".to_owned(), Value::String(role.to_owned()));
    item.insert(
        "payload_type".to_owned(),
        Value::String(payload_type.to_owned()),
    );
    item.insert(
        "canonical_type".to_owned(),
        Value::String(canonical_type.to_owned()),
    );
    item.insert(
        "source_key_field".to_owned(),
        Value::String(source_key_field.to_owned()),
    );
    item.insert("file_key".to_owned(), Value::String(file_key));
    if let Some(file_name) = file_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        item.insert("file_name".to_owned(), Value::String(file_name));
    }
    Some(Value::Object(item))
}

fn build_resource_selection_hint(selection_required: bool) -> String {
    if selection_required {
        return "Current Feishu ingress carries multiple downloadable resources; if payload.file_key or payload.type uniquely identifies one entry in resource_inventory for this message, the other field may default in this turn. If you override payload.message_id to another message, current ingress defaults no longer apply; provide payload.file_key and payload.type explicitly. Otherwise choose one entry from resource_inventory and pass its file_key as payload.file_key plus its payload_type as payload.type to feishu.messages.resource.get. save_as remains required.".to_owned();
    }

    "Current Feishu ingress carries exactly one downloadable resource; payload.file_key and payload.type may default from ingress in this turn, or copy them from the single resource_inventory entry when making an explicit feishu.messages.resource.get call. If you override payload.message_id to another message, current ingress defaults no longer apply; provide payload.file_key and payload.type explicitly. save_as remains required.".to_owned()
}

fn build_resource_download_hints(message_type: &str, normalized: &Value) -> Vec<Value> {
    match message_type {
        "image" => vec![build_resource_download_hint(
            "image",
            "image",
            "image_key",
            true,
            note_with_cross_turn_message_id(
                "Current Feishu ingress carries exactly one downloadable image resource; outside the current ingress turn, pass the image_key value as payload.file_key.",
            ),
        )],
        "file" => vec![build_resource_download_hint(
            "file",
            "file",
            "file_key",
            true,
            note_with_cross_turn_message_id(
                "Current Feishu ingress carries exactly one downloadable file resource; outside the current ingress turn, pass the file_key value as payload.file_key.",
            ),
        )],
        "audio" => vec![build_resource_download_hint(
            "audio",
            "file",
            "file_key",
            true,
            note_with_cross_turn_message_id(
                "Current Feishu ingress carries exactly one downloadable audio file resource; outside the current ingress turn, pass the file_key value as payload.file_key. payload.type=\"file\" is also accepted.",
            ),
        )],
        "media" => vec![
            build_resource_download_hint(
                "media",
                "file",
                "file_key",
                false,
                note_with_cross_turn_message_id(
                    "Use the media message's file_key value as payload.file_key to download the binary file. Media messages also carry a preview image, so multiple resources are present and explicit selection is required.",
                ),
            ),
            build_resource_download_hint(
                "image",
                "image",
                "image_key",
                false,
                note_with_cross_turn_message_id(
                    "Use the media message's image_key value as payload.file_key to download the preview image. Media messages also carry the binary file, so multiple resources are present and explicit selection is required.",
                ),
            ),
        ],
        "post" => build_post_resource_download_hints(normalized),
        _ => Vec::new(),
    }
}

fn build_post_resource_download_hints(normalized: &Value) -> Vec<Value> {
    let resources = collect_post_delivery_resources(normalized);
    if resources.is_empty() {
        return Vec::new();
    }

    let multiple_resources = resources.len() > 1;
    let has_file = resources
        .iter()
        .any(|resource| resource.resource_type == "file");
    let has_image = resources
        .iter()
        .any(|resource| resource.resource_type == "image");
    let mut hints = Vec::new();
    if has_file {
        hints.push(build_resource_download_hint(
            "file",
            "file",
            "file_key",
            !multiple_resources,
            note_with_cross_turn_message_id(if multiple_resources {
                "Use the embedded file_key value as payload.file_key to download a file attachment from this post. Post messages can carry multiple resources, so explicit selection is required."
            } else {
                "Current Feishu ingress carries exactly one downloadable file attachment from this post; outside the current ingress turn, pass the embedded file_key value as payload.file_key."
            }),
        ));
    }
    if has_image {
        hints.push(build_resource_download_hint(
            "image",
            "image",
            "image_key",
            !multiple_resources,
            note_with_cross_turn_message_id(if multiple_resources {
                "Use the embedded image_key value as payload.file_key to download an image attachment from this post. Post messages can carry multiple resources, so explicit selection is required."
            } else {
                "Current Feishu ingress carries exactly one downloadable image attachment from this post; outside the current ingress turn, pass the embedded image_key value as payload.file_key."
            }),
        ));
    }
    hints
}

fn note_with_cross_turn_message_id(note: &str) -> String {
    format!(
        "{note} Outside the current ingress turn, also pass this message's message_id as payload.message_id."
    )
}

fn build_resource_download_hint(
    payload_type: &str,
    canonical_type: &str,
    use_key_from: &str,
    payload_file_key_can_default_from_ingress: bool,
    note: impl Into<String>,
) -> Value {
    let note = note.into();
    let mut hint = Map::new();
    hint.insert(
        "tool".to_owned(),
        Value::String("feishu.messages.resource.get".to_owned()),
    );
    hint.insert(
        "payload_type".to_owned(),
        Value::String(payload_type.to_owned()),
    );
    hint.insert(
        "canonical_type".to_owned(),
        Value::String(canonical_type.to_owned()),
    );
    hint.insert(
        "use_key_from".to_owned(),
        Value::String(use_key_from.to_owned()),
    );
    hint.insert(
        "payload_message_id_can_default_from_ingress".to_owned(),
        Value::Bool(true),
    );
    hint.insert(
        "payload_file_key_can_default_from_ingress".to_owned(),
        Value::Bool(payload_file_key_can_default_from_ingress),
    );
    hint.insert("save_as_required".to_owned(), Value::Bool(true));
    hint.insert("note".to_owned(), Value::String(note));
    Value::Object(hint)
}

fn summarize_feishu_structured_only_content(message_type: &str, content: &Value) -> Option<String> {
    let normalized = normalize_feishu_content(content)?;
    let mut summary = Map::new();
    summary.insert(
        "message_type".to_owned(),
        Value::String(message_type.to_owned()),
    );
    insert_summary_string(
        &mut summary,
        "title",
        find_first_string_field(&normalized, "title"),
    );
    insert_summary_string(
        &mut summary,
        "name",
        find_first_string_field(&normalized, "name"),
    );
    insert_summary_string(
        &mut summary,
        "address",
        find_first_string_field(&normalized, "address"),
    );
    insert_summary_string(
        &mut summary,
        "file_key",
        find_first_string_field(&normalized, "file_key"),
    );
    insert_summary_string(
        &mut summary,
        "file_name",
        find_first_string_field(&normalized, "file_name"),
    );
    apply_structured_only_summary_fields(message_type, &normalized, &mut summary);
    let text_fragments = collect_named_string_fields(&normalized, "text");
    if !text_fragments.is_empty() {
        summary.insert("text".to_owned(), Value::String(text_fragments.join(" ")));
    }
    summary.insert(
        "note".to_owned(),
        Value::String(structured_only_message_note(message_type).to_owned()),
    );
    summary.insert("content".to_owned(), normalized);
    serde_json::to_string(&Value::Object(summary))
        .ok()
        .map(|body| format!("[feishu_inbound_message]\n{body}"))
}

fn apply_structured_only_summary_fields(
    message_type: &str,
    normalized: &Value,
    summary: &mut Map<String, Value>,
) {
    match message_type {
        "interactive" => {
            insert_summary_string(summary, "type", find_first_string_field(normalized, "type"));
            insert_summary_string(
                summary,
                "action",
                find_first_string_field(normalized, "action"),
            );
            insert_summary_string(
                summary,
                "block_id",
                find_first_string_field(normalized, "block_id"),
            );
            insert_summary_string(
                summary,
                "text",
                find_first_string_field(normalized, "content"),
            );
        }
        "share_chat" => {
            insert_summary_string(
                summary,
                "chat_id",
                find_first_string_field(normalized, "chat_id"),
            );
            insert_summary_string(
                summary,
                "user_id",
                find_first_string_field(normalized, "user_id"),
            );
        }
        "share_user" => {
            insert_summary_string(
                summary,
                "user_id",
                find_first_string_field(normalized, "user_id"),
            );
        }
        "share_calendar_event" | "calendar" | "general_calendar" => {
            insert_summary_scalar_string(
                summary,
                "event_id",
                find_first_scalar_field_string(normalized, "event_id"),
            );
            insert_summary_scalar_string(
                summary,
                "start_time",
                find_first_scalar_field_string(normalized, "start_time"),
            );
            insert_summary_scalar_string(
                summary,
                "end_time",
                find_first_scalar_field_string(normalized, "end_time"),
            );
            insert_summary_scalar_string(
                summary,
                "summary",
                find_first_scalar_field_string(normalized, "summary"),
            );
        }
        "system" => {
            insert_summary_string(
                summary,
                "template",
                find_first_string_field(normalized, "template"),
            );
            insert_summary_string_list(
                summary,
                "from_user",
                find_first_string_list_field(normalized, "from_user"),
            );
            insert_summary_string_list(
                summary,
                "to_chatters",
                find_first_string_list_field(normalized, "to_chatters"),
            );
            insert_summary_string(
                summary,
                "divider_text",
                find_first_string_field(normalized, "text"),
            );
        }
        "video_chat" => {
            insert_summary_string(
                summary,
                "topic",
                find_first_string_field(normalized, "topic"),
            );
            insert_summary_scalar_string(
                summary,
                "start_time",
                find_first_scalar_field_string(normalized, "start_time"),
            );
        }
        "todo" => {
            insert_summary_string(
                summary,
                "task_id",
                find_first_string_field(normalized, "task_id"),
            );
            insert_summary_scalar_string(
                summary,
                "due_time",
                find_first_scalar_field_string(normalized, "due_time"),
            );
        }
        "vote" => {
            insert_summary_string(
                summary,
                "topic",
                find_first_string_field(normalized, "topic"),
            );
            insert_summary_string_list(
                summary,
                "options",
                find_first_string_list_field(normalized, "options"),
            );
        }
        "merge_forward" => {
            insert_summary_scalar_string(
                summary,
                "merged_content",
                find_first_scalar_field_string(normalized, "content"),
            );
        }
        _ => {}
    }
}

fn structured_only_message_note(message_type: &str) -> &'static str {
    match message_type {
        "folder" => {
            "Feishu folder content is preserved structurally and cannot be downloaded via message resource API."
        }
        "sticker" => {
            "Feishu sticker content is preserved structurally and cannot be downloaded via message resource API."
        }
        "interactive" => "Interactive Feishu card content preserved with action metadata.",
        "share_chat" => "Shared Feishu chat content preserved with chat metadata.",
        "share_user" => "Shared Feishu user content preserved with user metadata.",
        "share_calendar_event" | "calendar" | "general_calendar" => {
            "Feishu calendar event content preserved with event metadata."
        }
        "system" => "Feishu system message content preserved with template variables.",
        "location" => "Structured Feishu location content preserved verbatim.",
        "video_chat" => "Feishu video chat content preserved with call metadata.",
        "todo" => "Feishu todo content preserved with task metadata.",
        "vote" => "Feishu vote content preserved with topic and options.",
        "merge_forward" => {
            "Feishu merged-forward content preserved; fetch the message body explicitly to inspect sub-messages."
        }
        _ => "Structured Feishu content preserved verbatim.",
    }
}

fn normalize_feishu_content(content: &Value) -> Option<Value> {
    match content {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                return Some(parsed);
            }
            Some(Value::String(trimmed.to_owned()))
        }
        Value::Null => None,
        other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::Array(_)
        | other @ Value::Object(_) => Some(other.clone()),
    }
}

fn insert_summary_string(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_summary_scalar_string(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_summary_string_list(
    map: &mut Map<String, Value>,
    key: &str,
    values: Option<Vec<String>>,
) {
    if let Some(values) = values {
        map.insert(
            key.to_owned(),
            Value::Array(values.into_iter().map(Value::String).collect()),
        );
    }
}

fn push_channel_delivery_resource(
    resources: &mut Vec<ChannelDeliveryResource>,
    resource_type: &str,
    file_key: Option<String>,
    file_name: Option<String>,
) {
    let Some(file_key) = file_key else {
        return;
    };
    if resources
        .iter()
        .any(|existing| existing.resource_type == resource_type && existing.file_key == file_key)
    {
        return;
    }
    resources.push(ChannelDeliveryResource {
        resource_type: resource_type.to_owned(),
        file_key,
        file_name,
    });
}

fn collect_post_delivery_resources(content: &Value) -> Vec<ChannelDeliveryResource> {
    let mut resources = Vec::new();
    collect_post_delivery_resources_into(content, &mut resources);
    resources
}

fn collect_post_delivery_resources_into(
    value: &Value,
    resources: &mut Vec<ChannelDeliveryResource>,
) {
    match value {
        Value::Object(map) => {
            push_channel_delivery_resource(
                resources,
                "file",
                required_string_field(map, "file_key"),
                required_string_field(map, "file_name"),
            );
            push_channel_delivery_resource(
                resources,
                "image",
                required_string_field(map, "image_key"),
                None,
            );
            for child in map.values() {
                collect_post_delivery_resources_into(child, resources);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_post_delivery_resources_into(item, resources);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn find_first_string_field(value: &Value, field: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(current) = map
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(current.to_owned());
            }
            map.values()
                .find_map(|value| find_first_string_field(value, field))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|value| find_first_string_field(value, field)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn find_first_scalar_field_string(value: &Value, field: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(current) = map.get(field).and_then(value_to_scalar_string) {
                return Some(current);
            }
            map.values()
                .find_map(|value| find_first_scalar_field_string(value, field))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|value| find_first_scalar_field_string(value, field)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn find_first_string_list_field(value: &Value, field: &str) -> Option<Vec<String>> {
    match value {
        Value::Object(map) => {
            if let Some(current) = map.get(field).and_then(value_to_string_list) {
                return Some(current);
            }
            map.values()
                .find_map(|value| find_first_string_list_field(value, field))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|value| find_first_string_list_field(value, field)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn value_to_scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn value_to_string_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(items) => {
            let values: Vec<String> = items.iter().filter_map(value_to_scalar_string).collect();
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            None
        }
    }
}

fn collect_named_string_fields(value: &Value, field: &str) -> Vec<String> {
    let mut values = Vec::new();
    collect_named_string_fields_into(value, field, &mut values);
    values
}

fn collect_named_string_fields_into(value: &Value, field: &str, values: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(current) = map
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                && !values.iter().any(|existing| existing == current)
            {
                values.push(current.to_owned());
            }
            for value in map.values() {
                collect_named_string_fields_into(value, field, values);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_named_string_fields_into(value, field, values);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn parse_sender_principal(
    event: &serde_json::Map<String, Value>,
    account_id: &str,
) -> Option<FeishuUserPrincipal> {
    let sender_id = event.get("sender")?.get("sender_id")?.as_object()?;
    let open_id = required_string_field(sender_id, "open_id")?;
    Some(FeishuUserPrincipal {
        account_id: account_id.trim().to_owned(),
        open_id,
        union_id: optional_string_field(sender_id, "union_id"),
        user_id: optional_string_field(sender_id, "user_id"),
        name: None,
        tenant_key: None,
        avatar_url: None,
        email: None,
        enterprise_email: None,
    })
}

fn required_string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    required_string_field(object, key)
}
