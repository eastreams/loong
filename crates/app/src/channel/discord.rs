use serde_json::{Value, json};

use crate::{CliResult, config::ResolvedDiscordChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_target},
};

pub(super) async fn run_discord_send(
    resolved: &ResolvedDiscordChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Conversation {
        return Err(format!(
            "discord send requires conversation target kind, got {}",
            target_kind.as_str()
        ));
    }

    let bot_token = resolved
        .bot_token()
        .ok_or_else(|| "discord bot token missing (set discord.bot_token or env)".to_owned())?;
    let channel_id = target_id.trim();
    if channel_id.is_empty() {
        return Err("discord outbound target id is empty".to_owned());
    }

    let api_base_url = resolved.resolved_api_base_url();
    let request_url = format!(
        "{}/channels/{channel_id}/messages",
        api_base_url.trim_end_matches('/')
    );
    let request_url =
        validate_outbound_http_target("discord api_base_url", request_url.as_str(), policy)?;
    let request_body = json!({
        "content": text,
    });

    let client = build_outbound_http_client("discord send", policy)?;
    let request = client
        .post(request_url)
        .header(reqwest::header::AUTHORIZATION, format!("Bot {bot_token}"))
        .json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("discord send failed: {error}"))?;
    let payload = read_discord_json_response(response).await?;

    let message_id = payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if message_id.is_none() {
        return Err(format!(
            "discord send did not return a message id: {payload}"
        ));
    }

    Ok(())
}

async fn read_discord_json_response(response: reqwest::Response) -> CliResult<Value> {
    let status = response.status();
    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("decode discord send response failed: {error}"))?;

    if status.is_success() {
        return Ok(payload);
    }

    let detail = payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| payload.to_string());
    Err(format!(
        "discord send failed with status {}: {detail}",
        status.as_u16()
    ))
}
