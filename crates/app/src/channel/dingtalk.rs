use base64::Engine;
use hmac::{KeyInit, Mac};
use serde_json::{Value, json};

use crate::{CliResult, config::ResolvedDingtalkChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{
        ChannelOutboundHttpPolicy, build_outbound_http_client, read_json_or_text_response,
        response_body_detail, validate_outbound_http_target,
    },
};

type DingtalkHmacSha256 = hmac::Hmac<sha2::Sha256>;

pub(super) async fn run_dingtalk_send(
    resolved: &ResolvedDingtalkChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    endpoint_url: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Endpoint {
        return Err(format!(
            "dingtalk send requires endpoint target kind, got {}",
            target_kind.as_str()
        ));
    }

    let secret = resolved.secret();
    let request_url = build_dingtalk_request_url(endpoint_url, secret.as_deref(), policy)?;
    let request_body = json!({
        "msgtype": "text",
        "text": {
            "content": text,
        },
    });

    let client = build_outbound_http_client("dingtalk send", policy)?;
    let request = client.post(request_url).json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("dingtalk send failed: {error}"))?;
    let payload = read_dingtalk_json_response(response).await?;

    let errcode = payload.get("errcode").and_then(Value::as_i64).unwrap_or(-1);
    if errcode != 0 {
        let errmsg = payload
            .get("errmsg")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| payload.to_string());
        return Err(format!("dingtalk send did not succeed: {errmsg}"));
    }

    Ok(())
}

fn build_dingtalk_request_url(
    endpoint_url: &str,
    secret: Option<&str>,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<String> {
    let mut url = validate_outbound_http_target("dingtalk webhook url", endpoint_url, policy)?;

    let secret = secret
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(secret) = secret {
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("current system time is invalid: {error}"))?
            .as_millis()
            .to_string();
        let sign = build_dingtalk_sign(timestamp_ms.as_str(), secret.as_str())?;
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.append_pair("timestamp", timestamp_ms.as_str());
        query_pairs.append_pair("sign", sign.as_str());
    }

    Ok(url.to_string())
}

fn build_dingtalk_sign(timestamp_ms: &str, secret: &str) -> CliResult<String> {
    let string_to_sign = format!("{timestamp_ms}\n{secret}");
    let mut mac = DingtalkHmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|error| format!("build dingtalk webhook signature failed: {error}"))?;
    mac.update(string_to_sign.as_bytes());
    let signature = mac.finalize().into_bytes();
    let encoded_signature = base64::engine::general_purpose::STANDARD.encode(signature);
    Ok(encoded_signature)
}

async fn read_dingtalk_json_response(response: reqwest::Response) -> CliResult<Value> {
    let (status, body, payload) = read_json_or_text_response(response, "dingtalk send").await?;

    if status.is_success() {
        if payload.is_object() {
            return Ok(payload);
        }

        let detail = response_body_detail(body.as_str());
        return Err(format!(
            "dingtalk send returned a non-json success payload: {detail}"
        ));
    }

    let detail = payload
        .get("errmsg")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| response_body_detail(body.as_str()));
    Err(format!(
        "dingtalk send failed with status {}: {detail}",
        status.as_u16()
    ))
}
