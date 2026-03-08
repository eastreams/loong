use std::collections::BTreeMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};

use crate::CliResult;

pub(super) fn build_request_headers(
    config_headers: &BTreeMap<String, String>,
) -> CliResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (key, value) in config_headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|error| format!("invalid provider header name `{key}`: {error}"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid provider header value for `{key}`: {error}"))?;
        headers.insert(name, header_value);
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
