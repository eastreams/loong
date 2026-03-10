use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::{json, Value};

use crate::config::ProviderConfig;
use crate::CliResult;

pub(super) fn build_request_headers(provider: &ProviderConfig) -> CliResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (key, value) in &provider.headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|error| format!("invalid provider header name `{key}`: {error}"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid provider header value for `{key}`: {error}"))?;
        headers.insert(name, header_value);
    }
    if !headers.contains_key(USER_AGENT) {
        if let Some(default_user_agent) = provider.kind.default_user_agent() {
            let header_value = HeaderValue::from_str(default_user_agent).map_err(|error| {
                format!("invalid default provider user-agent `{default_user_agent}`: {error}")
            })?;
            headers.insert(USER_AGENT, header_value);
        }
    }
    Ok(headers)
}

pub(super) async fn decode_response_body(response: reqwest::Response) -> CliResult<Value> {
    let raw = response
        .text()
        .await
        .map_err(|error| format!("read response body failed: {error}"))?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({"raw_body": raw})))
}
