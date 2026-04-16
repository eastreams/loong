use serde_json::json;

use crate::{CliResult, config::ResolvedLineChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{
        ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_base_url,
        validate_outbound_http_target,
    },
};

pub(super) async fn run_line_send(
    resolved: &ResolvedLineChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Address {
        return Err(format!(
            "line send requires address target kind, got {}",
            target_kind.as_str()
        ));
    }

    let channel_access_token = resolved.channel_access_token().ok_or_else(|| {
        "line channel_access_token missing (set line.channel_access_token or env)".to_owned()
    })?;
    let recipient = target_id.trim();
    if recipient.is_empty() {
        return Err("line outbound target id is empty".to_owned());
    }

    let raw_api_base_url = resolved.resolved_api_base_url();
    let api_base_url =
        validate_outbound_http_base_url("line api_base_url", raw_api_base_url.as_str(), policy)?;
    let trimmed_api_base_url = api_base_url.as_str().trim_end_matches('/');
    let request_url = format!("{trimmed_api_base_url}/message/push");
    let request_url =
        validate_outbound_http_target("line api_base_url", request_url.as_str(), policy)?;
    let request_body = json!({
        "to": recipient,
        "messages": [
            {
                "type": "text",
                "text": text,
            }
        ],
    });

    let client = build_outbound_http_client("line send", policy)?;
    let request = client
        .post(request_url)
        .bearer_auth(channel_access_token)
        .json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("line send failed: {error}"))?;

    ensure_line_success(response).await
}

async fn ensure_line_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read line error response failed: {error}"))?;
    let trimmed_body = body.trim();
    let detail = if trimmed_body.is_empty() {
        "empty response body".to_owned()
    } else {
        trimmed_body.to_owned()
    };

    Err(format!(
        "line send failed with status {}: {detail}",
        status.as_u16()
    ))
}
