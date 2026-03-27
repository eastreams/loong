use serde::Serialize;

use crate::{CliResult, config::ResolvedTeamsChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_target},
};

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
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    ensure_teams_target_kind(target_kind)?;
    let request_url = parse_teams_endpoint_url(endpoint_url, policy)?;
    let request_body = build_teams_webhook_payload(text);

    let client = build_outbound_http_client("teams send", policy)?;
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

fn parse_teams_endpoint_url(
    endpoint_url: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<reqwest::Url> {
    validate_outbound_http_target("teams outbound target endpoint", endpoint_url, policy)
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
    let body = response
        .text()
        .await
        .map_err(|error| format!("read teams response failed: {error}"))?;
    let trimmed_body = body.trim();
    if status.is_success() {
        if teams_response_body_indicates_failure(trimmed_body) {
            return Err(format!(
                "teams send returned an error payload with status {}: {}",
                status.as_u16(),
                trimmed_body
            ));
        }

        return Ok(());
    }

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

fn teams_response_body_indicates_failure(body: &str) -> bool {
    let normalized_body = body.trim().to_ascii_lowercase();
    if normalized_body.is_empty() {
        return false;
    }

    [
        "http error 429",
        "too many requests",
        "throttl",
        "webhook message delivery failed",
        "microsoft teams endpoint returned http error",
    ]
    .iter()
    .any(|indicator| normalized_body.contains(indicator))
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

    #[test]
    fn teams_response_body_failure_detection_matches_known_throttling_markers() {
        let throttled_body = "Webhook message delivery failed with error: Microsoft Teams endpoint returned HTTP error 429";
        let success_body = "1";

        assert!(teams_response_body_indicates_failure(throttled_body));
        assert!(!teams_response_body_indicates_failure(success_body));
    }
}
