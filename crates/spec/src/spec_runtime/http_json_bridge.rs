use std::time::Duration;

use kernel::ConnectorCommand;
use reqwest::Method;
use serde_json::{Value, json};

use super::{
    ConnectorProtocolContext, HttpJsonRuntimeEvidenceKind, authorize_connector_protocol_context,
    http_json_runtime_evidence, parse_http_enforce_protocol_contract, parse_http_timeout_ms,
};

#[allow(clippy::indexing_slicing)] // serde_json::Value string-keyed IndexMut is infallible
pub fn execute_http_json_bridge(
    mut execution: Value,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
) -> Value {
    let method_label = provider
        .metadata
        .get("http_method")
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "POST".to_owned());
    let method = match Method::from_bytes(method_label.as_bytes()) {
        Ok(method) => method,
        Err(error) => {
            execution["status"] = Value::String("blocked".to_owned());
            execution["reason"] =
                Value::String(format!("invalid http_method {method_label}: {error}"));
            return execution;
        }
    };

    let timeout_ms = parse_http_timeout_ms(provider);
    let enforce_protocol_contract = parse_http_enforce_protocol_contract(provider);
    let mut protocol_context =
        ConnectorProtocolContext::from_connector_command(provider, channel, command);
    if let Err(reason) = authorize_connector_protocol_context(&mut protocol_context) {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] = Value::String(format!("http_json {reason}"));
        execution["runtime"] = http_json_runtime_evidence(
            &protocol_context,
            &method_label,
            &channel.endpoint,
            timeout_ms,
            enforce_protocol_contract,
            HttpJsonRuntimeEvidenceKind::BaseOnly,
        );
        return execution;
    }

    let request_payload = json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    });
    let url = channel.endpoint.clone();
    let request_payload_for_runtime = request_payload.clone();
    let request_payload_for_worker = request_payload.clone();
    let request_method_for_worker = protocol_context.request_method.clone();
    let request_id_for_worker = protocol_context.request_id.clone();

    type HttpJsonBridgeWorkerResult =
        Result<(u16, bool, String, Value, Option<String>, Option<String>), String>;
    let run = std::thread::spawn(move || -> HttpJsonBridgeWorkerResult {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|error| format!("failed to initialize http_json client: {error}"))?;

        let response = client
            .request(method, &url)
            .header("content-type", "application/json")
            .json(&request_payload_for_worker)
            .send()
            .map_err(|error| format!("http_json bridge request failed: {error}"))?;

        let status = response.status();
        let status_code = status.as_u16();
        let success = status.is_success();
        let body = response
            .text()
            .map_err(|error| format!("failed to read http_json response body: {error}"))?;
        let body_json = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);

        let response_method = body_json
            .as_object()
            .and_then(|value| value.get("method"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let response_id = body_json
            .as_object()
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if enforce_protocol_contract {
            let method = response_method.as_deref().ok_or_else(|| {
                "http_json strict protocol contract requires response.method".to_owned()
            })?;
            if method != request_method_for_worker {
                return Err(format!(
                    "http_json response method mismatch: expected `{}`, got `{method}`",
                    request_method_for_worker
                ));
            }
            if response_id != request_id_for_worker {
                return Err(format!(
                    "http_json response id mismatch: expected `{:?}`, got `{:?}`",
                    request_id_for_worker, response_id
                ));
            }
        }

        Ok((
            status_code,
            success,
            body,
            body_json,
            response_method,
            response_id,
        ))
    })
    .join();

    match run {
        Ok(Ok((status_code, success, body, body_json, response_method, response_id))) => {
            execution["status"] = Value::String(if success {
                "executed".to_owned()
            } else {
                "failed".to_owned()
            });
            if !success {
                execution["reason"] = Value::String(format!(
                    "http_json bridge request failed with status {status_code}"
                ));
            }
            execution["runtime"] = http_json_runtime_evidence(
                &protocol_context,
                &method_label,
                &channel.endpoint,
                timeout_ms,
                enforce_protocol_contract,
                HttpJsonRuntimeEvidenceKind::Response {
                    status_code,
                    request: request_payload_for_runtime,
                    response_text: body,
                    response_json: body_json,
                    response_method,
                    response_id,
                },
            );
            execution
        }
        Ok(Err(reason)) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            execution["runtime"] = http_json_runtime_evidence(
                &protocol_context,
                &method_label,
                &channel.endpoint,
                timeout_ms,
                enforce_protocol_contract,
                HttpJsonRuntimeEvidenceKind::RequestOnly {
                    request: request_payload_for_runtime,
                },
            );
            execution
        }
        Err(_) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] =
                Value::String("http_json bridge worker thread panicked".to_owned());
            execution["runtime"] = http_json_runtime_evidence(
                &protocol_context,
                &method_label,
                &channel.endpoint,
                timeout_ms,
                enforce_protocol_contract,
                HttpJsonRuntimeEvidenceKind::RequestOnly {
                    request: request_payload_for_runtime,
                },
            );
            execution
        }
    }
}
