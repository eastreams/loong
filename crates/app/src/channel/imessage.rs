use serde::Serialize;

use crate::{CliResult, config::ResolvedImessageChannelConfig};

use super::{ChannelOutboundTargetKind, http::build_outbound_http_client};

#[derive(Debug, Serialize)]
struct ImessageSendRequestBody {
    #[serde(rename = "chatGuid")]
    chat_guid: String,
    message: String,
}

pub(super) async fn run_imessage_send(
    resolved: &ResolvedImessageChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    ensure_imessage_target_kind(target_kind)?;

    let bridge_url = resolved
        .bridge_url()
        .ok_or_else(|| "imessage bridge_url missing (set imessage.bridge_url or env)".to_owned())?;
    let bridge_token = resolved.bridge_token().ok_or_else(|| {
        "imessage bridge_token missing (set imessage.bridge_token or env)".to_owned()
    })?;
    let request_url = build_imessage_request_url(bridge_url.as_str(), bridge_token.as_str())?;

    let chat_guid = target_id.trim();
    if chat_guid.is_empty() {
        return Err("imessage outbound target id is empty".to_owned());
    }

    let request_body = ImessageSendRequestBody {
        chat_guid: chat_guid.to_owned(),
        message: text.to_owned(),
    };

    let client = build_outbound_http_client("imessage send")?;
    let request = client.post(request_url).json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("imessage send failed: {error}"))?;

    ensure_imessage_success(response).await
}

fn ensure_imessage_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Conversation {
        return Ok(());
    }

    Err(format!(
        "imessage send requires conversation target kind, got {}",
        target_kind.as_str()
    ))
}

fn build_imessage_request_url(bridge_url: &str, bridge_token: &str) -> CliResult<reqwest::Url> {
    let trimmed_bridge_url = bridge_url.trim();
    if trimmed_bridge_url.is_empty() {
        return Err("imessage bridge_url is empty".to_owned());
    }

    let trimmed_bridge_token = bridge_token.trim();
    if trimmed_bridge_token.is_empty() {
        return Err("imessage bridge_token is empty".to_owned());
    }

    let mut request_url = reqwest::Url::parse(trimmed_bridge_url)
        .map_err(|error| format!("imessage bridge_url is invalid: {error}"))?;
    let mut path_segments = request_url.path_segments_mut().map_err(|_path_error| {
        "imessage bridge_url cannot be used as a hierarchical base url".to_owned()
    })?;
    path_segments.pop_if_empty();
    path_segments.push("api");
    path_segments.push("v1");
    path_segments.push("message");
    path_segments.push("text");
    drop(path_segments);
    request_url
        .query_pairs_mut()
        .append_pair("guid", trimmed_bridge_token);
    Ok(request_url)
}

async fn ensure_imessage_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read imessage error response failed: {error}"))?;
    let trimmed_body = body.trim();
    let detail = if trimmed_body.is_empty() {
        "empty response body".to_owned()
    } else {
        trimmed_body.to_owned()
    };

    Err(format!(
        "imessage send failed with status {}: {detail}",
        status.as_u16()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_imessage_request_url_preserves_base_path_and_guid_query() {
        let request_url =
            build_imessage_request_url("https://bluebubbles.example.test/base", "bridge-password")
                .expect("build imessage request url");

        assert_eq!(
            request_url.as_str(),
            "https://bluebubbles.example.test/base/api/v1/message/text?guid=bridge-password"
        );
    }

    #[test]
    fn ensure_imessage_target_kind_rejects_non_conversation_targets() {
        let error = ensure_imessage_target_kind(ChannelOutboundTargetKind::Address)
            .expect_err("address target kind should be rejected");

        assert!(
            error.contains("imessage send requires conversation target kind"),
            "unexpected error: {error}"
        );
    }
}
