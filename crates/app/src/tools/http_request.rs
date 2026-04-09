use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::download_guard::ByteBudget;

const DEFAULT_HTTP_REQUEST_USER_AGENT: &str = "LoongClaw-HttpRequest/0.1";
const MAX_HEADER_COUNT: usize = 64;

pub(super) fn execute_http_request_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "http.request payload must be an object".to_owned())?;

    if !config.web_fetch.enabled {
        return Err("http.request is disabled by config.tools.web.enabled=false".to_owned());
    }

    let method = parse_method(payload)?;
    let raw_url = required_string_field(payload, "url", "http.request")?.to_owned();
    let max_bytes = parse_max_bytes(payload, config)?;
    let headers = parse_headers(payload)?;
    let body = optional_string_field(payload.get("body")).map(str::to_owned);
    let content_type = optional_string_field(payload.get("content_type")).map(str::to_owned);
    let url = reqwest::Url::parse(raw_url.as_str())
        .map_err(|error| format!("invalid http.request url `{raw_url}`: {error}"))?;
    let options = super::web_http::HttpTargetValidationOptions {
        allow_private_hosts: config.web_fetch.allow_private_hosts,
        reject_userinfo: true,
        resolve_dns: true,
        enforce_allowed_domains: config.web_fetch.enforce_allowed_domains,
        allowed_domains: Some(&config.web_fetch.allowed_domains),
        blocked_domains: Some(&config.web_fetch.blocked_domains),
    };
    let host = super::web_http::validate_http_target(&url, &options, "http.request")?;
    let timeout_seconds = config.web_fetch.timeout_seconds;
    let allow_private_hosts = config.web_fetch.allow_private_hosts;
    let tool_name = request.tool_name;

    super::web_http::run_async(async move {
        let client = super::web_http::build_ssrf_safe_client(
            allow_private_hosts,
            timeout_seconds,
            DEFAULT_HTTP_REQUEST_USER_AGENT,
        )?;
        let mut request_builder = client.request(method.clone(), url.clone());

        for (header_name, header_value) in &headers {
            request_builder = request_builder.header(header_name, header_value);
        }

        if let Some(content_type_value) = content_type.as_deref() {
            request_builder =
                request_builder.header(reqwest::header::CONTENT_TYPE, content_type_value);
        }

        if let Some(body_value) = body {
            request_builder = request_builder.body(body_value);
        }

        let response = request_builder
            .send()
            .await
            .map_err(|error| format!("http.request failed: {error}"))?;
        let status = response.status();
        let final_url = response.url().to_string();
        let response_headers = response_headers_json(response.headers());
        let content_type_header = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut budget = ByteBudget::new(max_bytes);
        budget
            .reject_if_content_length_exceeds(response.content_length(), "http.request response")?;

        let mut body_bytes = Vec::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|error| format!("failed to read http.request response body: {error}"))?;
            budget.try_consume(chunk.len(), "http.request response")?;
            body_bytes.extend_from_slice(&chunk);
        }

        let response_body =
            response_body_payload(content_type_header.as_deref(), body_bytes.as_slice());

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": tool_name,
                "method": method.as_str(),
                "requested_url": raw_url,
                "final_url": final_url,
                "host": host,
                "status_code": status.as_u16(),
                "status_text": status.canonical_reason(),
                "headers": response_headers,
                "content_type": content_type_header,
                "body_kind": response_body.kind,
                "body": response_body.value,
                "bytes_downloaded": budget.consumed(),
            }),
        })
    })?
}

fn parse_method(payload: &Map<String, Value>) -> Result<Method, String> {
    let raw_method = optional_string_field(payload.get("method")).unwrap_or("GET");
    let uppercase_method = raw_method.to_ascii_uppercase();
    let parsed_method = Method::from_bytes(uppercase_method.as_bytes())
        .map_err(|error| format!("http.request payload.method is invalid: {error}"))?;

    match parsed_method {
        Method::GET
        | Method::HEAD
        | Method::OPTIONS
        | Method::POST
        | Method::PUT
        | Method::PATCH
        | Method::DELETE => Ok(parsed_method),
        _ => Err(
            "http.request payload.method must be one of GET, HEAD, OPTIONS, POST, PUT, PATCH, or DELETE"
                .to_owned(),
        ),
    }
}

fn parse_headers(payload: &Map<String, Value>) -> Result<Vec<(String, String)>, String> {
    let Some(headers_value) = payload.get("headers") else {
        return Ok(Vec::new());
    };

    let headers_object = headers_value
        .as_object()
        .ok_or_else(|| "http.request payload.headers must be an object".to_owned())?;

    if headers_object.len() > MAX_HEADER_COUNT {
        return Err(format!(
            "http.request payload.headers exceeds maximum count ({MAX_HEADER_COUNT})"
        ));
    }

    let mut headers = Vec::new();

    for (raw_header_name, raw_header_value) in headers_object {
        let header_name = raw_header_name.trim();
        let header_value = raw_header_value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!("http.request payload.headers.{raw_header_name} must be a non-empty string")
            })?;

        if header_name.eq_ignore_ascii_case("host") {
            return Err("http.request payload.headers must not override Host".to_owned());
        }

        if header_name.eq_ignore_ascii_case("content-length") {
            return Err("http.request payload.headers must not override Content-Length".to_owned());
        }

        headers.push((header_name.to_owned(), header_value.to_owned()));
    }

    Ok(headers)
}

fn parse_max_bytes(
    payload: &Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<usize, String> {
    let configured_max = config.web_fetch.max_bytes;
    let Some(value) = payload.get("max_bytes") else {
        return Ok(configured_max);
    };
    let parsed_value = value
        .as_u64()
        .ok_or_else(|| "http.request payload.max_bytes must be an integer".to_owned())?;
    let max_bytes = usize::try_from(parsed_value)
        .map_err(|error| format!("invalid http.request payload.max_bytes: {error}"))?;

    if max_bytes == 0 {
        return Err("http.request payload.max_bytes must be >= 1".to_owned());
    }

    if max_bytes > configured_max {
        return Err(format!(
            "http.request payload.max_bytes exceeds configured limit ({configured_max} bytes)"
        ));
    }

    Ok(max_bytes)
}

fn required_string_field<'a>(
    payload: &'a Map<String, Value>,
    field: &str,
    tool_name: &str,
) -> Result<&'a str, String> {
    optional_string_field(payload.get(field))
        .ok_or_else(|| format!("{tool_name} requires payload.{field}"))
}

fn optional_string_field(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
}

fn response_headers_json(headers: &reqwest::header::HeaderMap) -> Value {
    let mut header_object = serde_json::Map::new();

    for (header_name, header_value) in headers {
        let header_value_text = header_value.to_str().unwrap_or("<binary>").to_owned();
        let header_name_text = header_name.as_str().to_owned();
        let entry = header_object
            .entry(header_name_text)
            .or_insert_with(|| Value::Array(Vec::new()));

        if let Value::Array(values) = entry {
            values.push(Value::String(header_value_text));
        }
    }

    Value::Object(header_object)
}

fn response_body_payload(content_type: Option<&str>, body_bytes: &[u8]) -> ResponseBodyPayload {
    let looks_binary = super::web_fetch::response_is_probably_binary(content_type, body_bytes);

    if !looks_binary {
        let body_text = String::from_utf8_lossy(body_bytes).into_owned();
        return ResponseBodyPayload {
            kind: "text".to_owned(),
            value: Value::String(body_text),
        };
    }

    let encoded_body = STANDARD.encode(body_bytes);

    ResponseBodyPayload {
        kind: "base64".to_owned(),
        value: Value::String(encoded_body),
    }
}

struct ResponseBodyPayload {
    kind: String,
    value: Value,
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    use serde_json::json;

    use super::*;
    use crate::tools::runtime_config::ToolRuntimeConfig;

    fn spawn_http_server(response: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let address = listener.local_addr().expect("listener addr");
        let (ready_tx, ready_rx) = mpsc::channel();

        thread::spawn(move || {
            ready_tx.send(()).expect("send ready");
            let (mut stream, _) = listener.accept().expect("accept client");
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            stream
                .write_all(response.as_slice())
                .expect("write response");
        });

        ready_rx.recv().expect("wait for ready");
        format!("http://{}", address)
    }

    #[test]
    fn http_request_returns_typed_text_response() {
        let base_url = spawn_http_server(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nX-Test: ok\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
        );
        let request = ToolCoreRequest {
            tool_name: "http.request".to_owned(),
            payload: json!({
                "method": "GET",
                "url": base_url,
            }),
        };
        let mut runtime_config = ToolRuntimeConfig::default();
        runtime_config.web_fetch.allow_private_hosts = true;

        let outcome = execute_http_request_tool_with_config(request, &runtime_config)
            .expect("http.request should succeed");

        assert_eq!(outcome.payload["status_code"], 200);
        assert_eq!(outcome.payload["body_kind"], "text");
        assert_eq!(outcome.payload["body"], "hello");
        assert_eq!(outcome.payload["headers"]["x-test"][0], "ok");
    }

    #[test]
    fn http_request_rejects_host_header_override() {
        let request = ToolCoreRequest {
            tool_name: "http.request".to_owned(),
            payload: json!({
                "url": "https://example.com",
                "headers": {
                    "Host": "evil.example"
                }
            }),
        };
        let runtime_config = ToolRuntimeConfig::default();
        let error = execute_http_request_tool_with_config(request, &runtime_config)
            .expect_err("Host override should fail");

        assert!(error.contains("must not override Host"));
    }

    #[test]
    fn http_request_encodes_binary_response_as_base64() {
        let mut response = Vec::new();
        response.extend_from_slice(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 2\r\nConnection: close\r\n\r\n",
        );
        response.push(0x00);
        response.push(0xff);
        let base_url = spawn_http_server(response);
        let request = ToolCoreRequest {
            tool_name: "http.request".to_owned(),
            payload: json!({
                "url": base_url,
            }),
        };
        let mut runtime_config = ToolRuntimeConfig::default();
        runtime_config.web_fetch.allow_private_hosts = true;

        let outcome = execute_http_request_tool_with_config(request, &runtime_config)
            .expect("binary http.request should succeed");

        assert_eq!(outcome.payload["body_kind"], "base64");
        assert_eq!(outcome.payload["body"], "AP8=");
    }
}
