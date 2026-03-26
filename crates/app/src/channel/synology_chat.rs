use serde::Serialize;

use crate::{CliResult, config::ResolvedSynologyChatChannelConfig};

use super::{ChannelOutboundTargetKind, http::build_outbound_http_client};

#[derive(Debug, Serialize)]
struct SynologyChatWebhookPayload {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_ids: Option<Vec<u64>>,
}

pub(super) async fn run_synology_chat_send(
    resolved: &ResolvedSynologyChatChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: Option<&str>,
    text: &str,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Address {
        return Err(format!(
            "synology chat send requires address target kind, got {}",
            target_kind.as_str()
        ));
    }

    let incoming_url = resolved.incoming_url().ok_or_else(|| {
        "synology_chat incoming_url missing (set synology_chat.incoming_url or env)".to_owned()
    })?;
    let request_url = reqwest::Url::parse(incoming_url.as_str())
        .map_err(|error| format!("synology chat incoming_url is invalid: {error}"))?;
    let target_user_id = parse_synology_chat_target_user_id(target_id)?;
    let request_payload_json = build_synology_chat_payload_json(text, target_user_id)?;

    let client = build_outbound_http_client("synology chat send")?;
    let request = client
        .post(request_url)
        .form(&[("payload", request_payload_json)]);
    let response = request
        .send()
        .await
        .map_err(|error| format!("synology chat send failed: {error}"))?;

    ensure_synology_chat_success(response).await
}

fn parse_synology_chat_target_user_id(target_id: Option<&str>) -> CliResult<Option<u64>> {
    let target_id = target_id.map(str::trim).filter(|value| !value.is_empty());
    let Some(target_id) = target_id else {
        return Ok(None);
    };

    let user_id = target_id.parse::<u64>().map_err(|error| {
        format!(
            "synology chat target user id must be a numeric address, got `{target_id}`: {error}"
        )
    })?;
    Ok(Some(user_id))
}

fn build_synology_chat_payload_json(text: &str, target_user_id: Option<u64>) -> CliResult<String> {
    let user_ids = target_user_id.map(|user_id| vec![user_id]);
    let payload = SynologyChatWebhookPayload {
        text: text.to_owned(),
        user_ids,
    };
    serde_json::to_string(&payload)
        .map_err(|error| format!("serialize synology chat webhook payload failed: {error}"))
}

async fn ensure_synology_chat_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read synology chat error response failed: {error}"))?;
    let trimmed_body = body.trim();
    let detail = if trimmed_body.is_empty() {
        "empty response body".to_owned()
    } else {
        trimmed_body.to_owned()
    };

    Err(format!(
        "synology chat send failed with status {}: {detail}",
        status.as_u16()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_synology_chat_payload_json_omits_user_ids_without_target() {
        let payload_json = build_synology_chat_payload_json("hello synology", None)
            .expect("build synology chat payload json");

        assert_eq!(payload_json, "{\"text\":\"hello synology\"}");
    }

    #[test]
    fn build_synology_chat_payload_json_includes_target_user_id() {
        let payload_json = build_synology_chat_payload_json("hello synology", Some(42))
            .expect("build synology chat payload json");

        assert_eq!(
            payload_json,
            "{\"text\":\"hello synology\",\"user_ids\":[42]}"
        );
    }

    #[test]
    fn parse_synology_chat_target_user_id_rejects_non_numeric_target() {
        let error = parse_synology_chat_target_user_id(Some("user-abc"))
            .expect_err("non-numeric target should fail");

        assert!(
            error.contains("synology chat target user id must be a numeric address"),
            "unexpected error: {error}"
        );
    }
}
