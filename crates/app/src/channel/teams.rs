use serde::Serialize;

use crate::{CliResult, config::ResolvedTeamsChannelConfig};

use super::ChannelOutboundTargetKind;

const TEAMS_ADAPTIVE_CARD_SCHEMA: &str = "http://adaptivecards.io/schemas/adaptive-card.json";
const TEAMS_ADAPTIVE_CARD_CONTENT_TYPE: &str = "application/vnd.microsoft.card.adaptive";

#[derive(Debug, Serialize)]
struct TeamsWebhookPayload {
    #[serde(rename = "type")]
    message_type: &'static str,
    attachments: Vec<TeamsWebhookAttachment>,
}

#[derive(Debug, Serialize)]
struct TeamsWebhookAttachment {
    #[serde(rename = "contentType")]
    content_type: &'static str,
    content: TeamsAdaptiveCard,
}

#[derive(Debug, Serialize)]
struct TeamsAdaptiveCard {
    #[serde(rename = "$schema")]
    schema: &'static str,
    #[serde(rename = "type")]
    card_type: &'static str,
    version: &'static str,
    body: Vec<TeamsAdaptiveCardBodyBlock>,
}

#[derive(Debug, Serialize)]
struct TeamsAdaptiveCardBodyBlock {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: String,
    wrap: bool,
}

pub(super) async fn run_teams_send(
    _resolved: &ResolvedTeamsChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    endpoint_url: &str,
    text: &str,
) -> CliResult<()> {
    ensure_teams_target_kind(target_kind)?;
    let request_url = parse_teams_endpoint_url(endpoint_url)?;
    let request_body = build_teams_webhook_payload(text);

    let client = reqwest::Client::new();
    let request = client.post(request_url).json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("teams send failed: {error}"))?;

    ensure_teams_success(response).await
}

fn ensure_teams_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Endpoint {
        return Ok(());
    }

    Err(format!(
        "teams send requires endpoint target kind, got {}",
        target_kind.as_str()
    ))
}

fn parse_teams_endpoint_url(endpoint_url: &str) -> CliResult<reqwest::Url> {
    let trimmed_endpoint_url = endpoint_url.trim();
    if trimmed_endpoint_url.is_empty() {
        return Err("teams outbound target endpoint is empty".to_owned());
    }

    reqwest::Url::parse(trimmed_endpoint_url)
        .map_err(|error| format!("teams outbound target endpoint is invalid: {error}"))
}

fn build_teams_webhook_payload(text: &str) -> TeamsWebhookPayload {
    let text_block = TeamsAdaptiveCardBodyBlock {
        block_type: "TextBlock",
        text: text.to_owned(),
        wrap: true,
    };
    let body = vec![text_block];
    let adaptive_card = TeamsAdaptiveCard {
        schema: TEAMS_ADAPTIVE_CARD_SCHEMA,
        card_type: "AdaptiveCard",
        version: "1.2",
        body,
    };
    let attachment = TeamsWebhookAttachment {
        content_type: TEAMS_ADAPTIVE_CARD_CONTENT_TYPE,
        content: adaptive_card,
    };
    let attachments = vec![attachment];
    TeamsWebhookPayload {
        message_type: "message",
        attachments,
    }
}

async fn ensure_teams_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read teams error response failed: {error}"))?;
    let trimmed_body = body.trim();
    let detail = if trimmed_body.is_empty() {
        "empty response body".to_owned()
    } else {
        trimmed_body.to_owned()
    };

    Err(format!(
        "teams send failed with status {}: {detail}",
        status.as_u16()
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn build_teams_webhook_payload_wraps_text_in_adaptive_card() {
        let payload = build_teams_webhook_payload("hello teams");
        let payload_value = serde_json::to_value(payload).expect("serialize teams payload");

        assert_eq!(
            payload_value.get("type").and_then(Value::as_str),
            Some("message")
        );
        assert_eq!(
            payload_value["attachments"][0]["contentType"].as_str(),
            Some(TEAMS_ADAPTIVE_CARD_CONTENT_TYPE)
        );
        assert_eq!(
            payload_value["attachments"][0]["content"]["body"][0]["text"].as_str(),
            Some("hello teams")
        );
        assert_eq!(
            payload_value["attachments"][0]["content"]["body"][0]["wrap"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn ensure_teams_target_kind_rejects_non_endpoint_targets() {
        let error = ensure_teams_target_kind(ChannelOutboundTargetKind::Conversation)
            .expect_err("conversation target kind should be rejected");

        assert!(
            error.contains("teams send requires endpoint target kind"),
            "unexpected error: {error}"
        );
    }
}
