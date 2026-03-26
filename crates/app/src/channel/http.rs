use std::time::Duration;

use serde_json::Value;

use crate::CliResult;

const DEFAULT_OUTBOUND_HTTP_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn build_outbound_http_client(context: &str) -> CliResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(DEFAULT_OUTBOUND_HTTP_TIMEOUT)
        .build()
        .map_err(|error| format!("build {context} http client failed: {error}"))
}

pub(super) async fn read_json_or_text_response(
    response: reqwest::Response,
    context: &str,
) -> CliResult<(reqwest::StatusCode, String, Value)> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("read {context} response failed: {error}"))?;
    let payload =
        serde_json::from_str::<Value>(&body).unwrap_or_else(|_| Value::String(body.clone()));
    Ok((status, body, payload))
}

pub(super) fn response_body_detail(body: &str) -> String {
    let trimmed_body = body.trim();
    if trimmed_body.is_empty() {
        return "empty response body".to_owned();
    }

    trimmed_body.to_owned()
}
