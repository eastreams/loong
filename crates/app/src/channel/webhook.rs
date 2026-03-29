use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use serde_json::{Map, Value};

use crate::{
    CliResult,
    config::{ResolvedWebhookChannelConfig, WebhookPayloadFormat},
};

use super::{
    ChannelOutboundTargetKind,
    http::{
        ChannelOutboundHttpPolicy, build_outbound_http_client, response_body_detail,
        validate_outbound_http_target,
    },
    webhook_auth::build_webhook_auth_header_from_parts,
};

const WEBHOOK_JSON_CONTENT_TYPE: &str = "application/json";
const WEBHOOK_TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

struct WebhookRequestBody {
    content_type: &'static str,
    body: Vec<u8>,
}

pub(super) async fn run_webhook_send(
    resolved: &ResolvedWebhookChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    endpoint_url: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    ensure_webhook_target_kind(target_kind)?;

    let request_url = parse_webhook_endpoint_url(endpoint_url, policy)?;
    let request_body = build_webhook_request_body(resolved, text)?;
    let auth_header = build_webhook_auth_header(resolved)?;

    let client = build_outbound_http_client("webhook send", policy)?;
    let mut request = client
        .post(request_url)
        .header(CONTENT_TYPE, request_body.content_type)
        .body(request_body.body);

    if let Some((header_name, header_value)) = auth_header {
        request = request.header(header_name, header_value);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("webhook send failed: {error}"))?;

    ensure_webhook_success(response).await
}

fn ensure_webhook_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Endpoint {
        return Ok(());
    }

    Err(format!(
        "webhook send requires endpoint target kind, got {}",
        target_kind.as_str()
    ))
}

fn parse_webhook_endpoint_url(
    endpoint_url: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<reqwest::Url> {
    validate_outbound_http_target("webhook outbound target endpoint", endpoint_url, policy)
}

fn build_webhook_request_body(
    resolved: &ResolvedWebhookChannelConfig,
    text: &str,
) -> CliResult<WebhookRequestBody> {
    match resolved.payload_format {
        WebhookPayloadFormat::JsonText => build_webhook_json_request_body(resolved, text),
        WebhookPayloadFormat::PlainText => build_webhook_plain_text_request_body(text),
    }
}

fn build_webhook_json_request_body(
    resolved: &ResolvedWebhookChannelConfig,
    text: &str,
) -> CliResult<WebhookRequestBody> {
    let request_json = build_webhook_json_payload(resolved, text)?;
    let request_bytes = serde_json::to_vec(&request_json)
        .map_err(|error| format!("serialize webhook json payload failed: {error}"))?;

    Ok(WebhookRequestBody {
        content_type: WEBHOOK_JSON_CONTENT_TYPE,
        body: request_bytes,
    })
}

fn build_webhook_plain_text_request_body(text: &str) -> CliResult<WebhookRequestBody> {
    let request_text = text.to_owned();
    let request_bytes = request_text.into_bytes();

    Ok(WebhookRequestBody {
        content_type: WEBHOOK_TEXT_CONTENT_TYPE,
        body: request_bytes,
    })
}

fn build_webhook_json_payload(
    resolved: &ResolvedWebhookChannelConfig,
    text: &str,
) -> CliResult<Value> {
    let field_name = resolved.payload_text_field.trim();
    if field_name.is_empty() {
        return Err("webhook payload_text_field is empty for json_text payload format".to_owned());
    }

    let mut payload = Map::new();
    let text_value = Value::String(text.to_owned());
    payload.insert(field_name.to_owned(), text_value);

    Ok(Value::Object(payload))
}

fn build_webhook_auth_header(
    resolved: &ResolvedWebhookChannelConfig,
) -> CliResult<Option<(HeaderName, HeaderValue)>> {
    let auth_token = resolved.auth_token();
    let auth_token = auth_token.as_deref();
    let auth_header_name = resolved.auth_header_name.as_str();
    let auth_token_prefix = resolved.auth_token_prefix.as_str();

    build_webhook_auth_header_from_parts(auth_token, auth_header_name, auth_token_prefix)
}

async fn ensure_webhook_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read webhook error response failed: {error}"))?;
    let detail = response_body_detail(body.as_str());

    Err(format!(
        "webhook send failed with status {}: {detail}",
        status.as_u16()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebhookChannelConfig;

    fn test_resolved_webhook_config(
        payload_format: WebhookPayloadFormat,
    ) -> ResolvedWebhookChannelConfig {
        let payload_format_raw = payload_format.as_str();
        let config: WebhookChannelConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "account_id": "Webhook Ops",
            "payload_format": payload_format_raw,
            "payload_text_field": "message"
        }))
        .expect("deserialize webhook config");

        config
            .resolve_account(None)
            .expect("resolve webhook config for tests")
    }

    #[test]
    fn build_webhook_json_payload_uses_custom_text_field() {
        let resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);

        let payload = build_webhook_json_payload(&resolved, "hello webhook")
            .expect("build webhook json payload");

        assert_eq!(payload["message"].as_str(), Some("hello webhook"));
    }

    #[test]
    fn build_webhook_plain_text_request_body_returns_raw_text() {
        let request_body = build_webhook_plain_text_request_body("hello webhook")
            .expect("build webhook plain text request body");

        assert_eq!(request_body.content_type, WEBHOOK_TEXT_CONTENT_TYPE);
        assert_eq!(request_body.body, b"hello webhook".to_vec());
    }

    #[test]
    fn build_webhook_auth_header_rejects_invalid_header_name() {
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        let auth_token =
            serde_json::from_value(serde_json::json!("token-123")).expect("deserialize auth token");
        resolved.auth_token = Some(auth_token);
        resolved.auth_header_name = "bad header".to_owned();

        let error =
            build_webhook_auth_header(&resolved).expect_err("invalid header name should fail");

        assert!(
            error.contains("webhook auth_header_name is invalid"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn build_webhook_auth_header_rejects_invalid_header_value() {
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        let auth_token =
            serde_json::from_value(serde_json::json!("token-123")).expect("deserialize auth token");
        resolved.auth_token = Some(auth_token);
        resolved.auth_token_prefix = "Bearer\n".to_owned();

        let error =
            build_webhook_auth_header(&resolved).expect_err("invalid header value should fail");

        assert!(
            error.contains("webhook auth header value is invalid"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn build_webhook_json_payload_rejects_empty_text_field() {
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        resolved.payload_text_field = "   ".to_owned();

        let error =
            build_webhook_json_payload(&resolved, "hello").expect_err("empty field should fail");

        assert!(
            error.contains("webhook payload_text_field is empty"),
            "unexpected error: {error}"
        );
    }
}
