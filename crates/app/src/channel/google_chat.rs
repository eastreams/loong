use serde_json::{Value, json};

use crate::{CliResult, config::ResolvedGoogleChatChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{
        ChannelOutboundHttpPolicy, build_outbound_http_client, read_json_or_text_response,
        response_body_detail, validate_outbound_http_target,
    },
};

pub(super) async fn run_google_chat_send(
    _resolved: &ResolvedGoogleChatChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    endpoint_url: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Endpoint {
        return Err(format!(
            "google chat send requires endpoint target kind, got {}",
            target_kind.as_str()
        ));
    }

    let request_url = validate_outbound_http_target(
        "google chat outbound target endpoint",
        endpoint_url,
        policy,
    )?;

    let request_body = json!({
        "text": text,
    });

    let client = build_outbound_http_client("google chat send", policy)?;
    let request = client.post(request_url).json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("google chat send failed: {error}"))?;
    let payload = read_google_chat_json_response(response).await?;

    let message_name = payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if message_name.is_none() {
        return Err(format!(
            "google chat send did not return a message name: {payload}"
        ));
    }

    Ok(())
}

async fn read_google_chat_json_response(response: reqwest::Response) -> CliResult<Value> {
    let (status, body, payload) = read_json_or_text_response(response, "google chat send").await?;

    if status.is_success() {
        if payload.is_object() {
            return Ok(payload);
        }

        let detail = response_body_detail(body.as_str());
        return Err(format!(
            "google chat send returned a non-json success payload: {detail}"
        ));
    }

    let detail = payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| response_body_detail(body.as_str()));
    Err(format!(
        "google chat send failed with status {}: {detail}",
        status.as_u16()
    ))
}
