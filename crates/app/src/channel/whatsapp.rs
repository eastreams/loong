use serde_json::{Value, json};

use crate::{CliResult, config::ResolvedWhatsappChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_target},
};

pub(super) async fn run_whatsapp_send(
    resolved: &ResolvedWhatsappChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Address {
        return Err(format!(
            "whatsapp send requires address target kind, got {}",
            target_kind.as_str()
        ));
    }

    let access_token = resolved.access_token().ok_or_else(|| {
        "whatsapp access token missing (set whatsapp.access_token or env)".to_owned()
    })?;
    let phone_number_id = resolved.phone_number_id().ok_or_else(|| {
        "whatsapp phone_number_id missing (set whatsapp.phone_number_id or env)".to_owned()
    })?;
    let recipient = target_id.trim();
    if recipient.is_empty() {
        return Err("whatsapp outbound target id is empty".to_owned());
    }

    let api_base_url = resolved.resolved_api_base_url();
    let request_url = format!(
        "{}/{}/messages",
        api_base_url.trim_end_matches('/'),
        phone_number_id.trim()
    );
    let request_url =
        validate_outbound_http_target("whatsapp api_base_url", request_url.as_str(), policy)?;
    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient,
        "type": "text",
        "text": {
            "preview_url": false,
            "body": text,
        },
    });

    let client = build_outbound_http_client("whatsapp send", policy)?;
    let request = client
        .post(request_url)
        .bearer_auth(access_token)
        .json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("whatsapp send failed: {error}"))?;
    let payload = read_whatsapp_json_response(response).await?;

    let message_id = payload
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.first())
        .and_then(|message| message.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if message_id.is_none() {
        return Err(format!(
            "whatsapp send did not return a message id: {payload}"
        ));
    }

    Ok(())
}

async fn read_whatsapp_json_response(response: reqwest::Response) -> CliResult<Value> {
    let status = response.status();
    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("decode whatsapp send response failed: {error}"))?;

    if status.is_success() {
        return Ok(payload);
    }

    let detail = payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| payload.to_string());
    Err(format!(
        "whatsapp send failed with status {}: {detail}",
        status.as_u16()
    ))
}
