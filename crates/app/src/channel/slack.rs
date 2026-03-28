use serde_json::{Value, json};

use crate::{CliResult, config::ResolvedSlackChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_target},
};

pub(super) async fn run_slack_send(
    resolved: &ResolvedSlackChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Conversation {
        return Err(format!(
            "slack send requires conversation target kind, got {}",
            target_kind.as_str()
        ));
    }

    let bot_token = resolved
        .bot_token()
        .ok_or_else(|| "slack bot token missing (set slack.bot_token or env)".to_owned())?;
    let channel_id = target_id.trim();
    if channel_id.is_empty() {
        return Err("slack outbound target id is empty".to_owned());
    }

    let api_base_url = resolved.resolved_api_base_url();
    let request_url = format!("{}/chat.postMessage", api_base_url.trim_end_matches('/'));
    let request_url =
        validate_outbound_http_target("slack api_base_url", request_url.as_str(), policy)?;
    let request_body = json!({
        "channel": channel_id,
        "text": text,
    });

    let client = build_outbound_http_client("slack send", policy)?;
    let request = client
        .post(request_url)
        .bearer_auth(bot_token)
        .json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("slack send failed: {error}"))?;
    let payload = read_slack_json_response(response).await?;

    let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        return Err(format!("slack send did not succeed: {payload}"));
    }

    Ok(())
}

async fn read_slack_json_response(response: reqwest::Response) -> CliResult<Value> {
    let status = response.status();
    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("decode slack send response failed: {error}"))?;

    if !status.is_success() {
        let detail = payload
            .get("error")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| payload.to_string());
        return Err(format!(
            "slack send failed with status {}: {detail}",
            status.as_u16()
        ));
    }

    Ok(payload)
}
