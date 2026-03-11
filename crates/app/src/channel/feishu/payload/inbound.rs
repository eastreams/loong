use std::collections::BTreeSet;

use serde_json::Value;

use crate::CliResult;
use crate::channel::{ChannelOutboundTarget, ChannelPlatform, ChannelSession};

use super::crypto::decrypt_payload_if_needed;
use super::types::{FeishuInboundEvent, FeishuWebhookAction};

pub(in crate::channel::feishu) fn parse_feishu_webhook_payload(
    payload: &Value,
    verification_token: Option<&str>,
    encrypt_key: Option<&str>,
    allowed_chat_ids: &BTreeSet<String>,
    ignore_bot_messages: bool,
    account_id: &str,
) -> CliResult<FeishuWebhookAction> {
    let decrypted_payload = decrypt_payload_if_needed(payload, encrypt_key)?;
    let payload = decrypted_payload.as_ref().unwrap_or(payload);

    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        verify_feishu_token(payload, verification_token)?;
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
    if event_type != "im.message.receive_v1" {
        return Ok(FeishuWebhookAction::Ignore);
    }

    verify_feishu_token(payload, verification_token)?;

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
    if message_type != "text" {
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

    let message_id = message
        .get("message_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "feishu message event missing message.message_id".to_owned())?;

    let content = message
        .get("content")
        .ok_or_else(|| "feishu message event missing message.content".to_owned())?;
    let text = parse_feishu_text_content(content)
        .ok_or_else(|| "feishu message content is not a non-empty text payload".to_owned())?;

    let event_id = payload
        .get("header")
        .and_then(|header| header.get("event_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("message:{message_id}"));

    Ok(FeishuWebhookAction::Inbound(FeishuInboundEvent {
        event_id,
        session: ChannelSession::with_account(
            ChannelPlatform::Feishu,
            account_id,
            chat_id.to_owned(),
        ),
        reply_target: ChannelOutboundTarget::feishu_message_reply(message_id.to_owned()),
        text,
    }))
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
        .get("token")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if incoming.is_empty() {
        return Err("unauthorized: feishu payload missing token".to_owned());
    }
    if incoming != expected_token {
        return Err("unauthorized: feishu verification token mismatch".to_owned());
    }
    Ok(())
}

fn parse_feishu_text_content(content: &Value) -> Option<String> {
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
