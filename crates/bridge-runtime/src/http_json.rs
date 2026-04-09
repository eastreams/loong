use std::time::Duration;

use loongclaw_kernel as kernel;
use reqwest::Method;
use serde_json::{Value, json};

use crate::protocol::{
    ConnectorProtocolContext, HttpJsonRuntimeEvidenceKind, authorize_connector_protocol_context,
    http_json_runtime_evidence, parse_http_enforce_protocol_contract, parse_http_timeout_ms,
};

#[derive(Debug, Clone)]
pub struct BridgeExecutionSuccess {
    pub response_payload: Value,
    pub runtime_evidence: Value,
}

#[derive(Debug, Clone)]
pub struct BridgeExecutionFailure {
    pub blocked: bool,
    pub reason: String,
    pub runtime_evidence: Value,
}

pub fn execute_http_json_bridge_call(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &kernel::ConnectorCommand,
) -> Result<BridgeExecutionSuccess, BridgeExecutionFailure> {
    let method_label = provider
        .metadata
        .get("http_method")
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "POST".to_owned());
    let method = Method::from_bytes(method_label.as_bytes());
    let method = match method {
        Ok(method) => method,
        Err(error) => {
            return Err(BridgeExecutionFailure {
                blocked: false,
                reason: format!("invalid http_method {method_label}: {error}"),
                runtime_evidence: Value::Null,
            });
        }
    };

    let timeout_ms = parse_http_timeout_ms(provider);
    let enforce_protocol_contract = parse_http_enforce_protocol_contract(provider);
    let mut protocol_context =
        ConnectorProtocolContext::from_connector_command(provider, channel, command);
    let authorized = authorize_connector_protocol_context(&mut protocol_context);
    if let Err(reason) = authorized {
        let reason = format!("http_json {reason}");
        let runtime_evidence = http_json_runtime_evidence(
            &protocol_context,
            &method_label,
            &channel.endpoint,
            timeout_ms,
            enforce_protocol_contract,
            HttpJsonRuntimeEvidenceKind::BaseOnly,
        );
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason,
            runtime_evidence,
        });
    }

    let request_payload = json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    });
    let request_payload_for_runtime = request_payload.clone();
    let request_payload_for_worker = request_payload;
    let url = channel.endpoint.clone();
    let request_method_for_worker = protocol_context.request_method.clone();
    let request_id_for_worker = protocol_context.request_id.clone();

    type WorkerResult = Result<(u16, bool, String, Value, Option<String>, Option<String>), String>;
    let run = std::thread::spawn(move || -> WorkerResult {
        let client_builder = reqwest::blocking::Client::builder();
        let client_builder = client_builder.timeout(Duration::from_millis(timeout_ms));
        let client = client_builder
            .build()
            .map_err(|error| format!("failed to initialize http_json client: {error}"))?;

        let request_builder = client.request(method, &url);
        let request_builder = request_builder.header("content-type", "application/json");
        let request_builder = request_builder.json(&request_payload_for_worker);
        let response = request_builder
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
                    request_method_for_worker,
                ));
            }
            if response_id != request_id_for_worker {
                return Err(format!(
                    "http_json response id mismatch: expected `{:?}`, got `{:?}`",
                    request_id_for_worker, response_id,
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
            let response_payload = extract_http_json_response_payload(&body_json);
            let runtime_evidence = http_json_runtime_evidence(
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
            if !success {
                let reason = format!("http_json bridge request failed with status {status_code}");
                return Err(BridgeExecutionFailure {
                    blocked: false,
                    reason,
                    runtime_evidence,
                });
            }

            Ok(BridgeExecutionSuccess {
                response_payload,
                runtime_evidence,
            })
        }
        Ok(Err(reason)) => {
            let runtime_evidence = http_json_runtime_evidence(
                &protocol_context,
                &method_label,
                &channel.endpoint,
                timeout_ms,
                enforce_protocol_contract,
                HttpJsonRuntimeEvidenceKind::RequestOnly {
                    request: request_payload_for_runtime,
                },
            );
            Err(BridgeExecutionFailure {
                blocked: false,
                reason,
                runtime_evidence,
            })
        }
        Err(_) => {
            let runtime_evidence = http_json_runtime_evidence(
                &protocol_context,
                &method_label,
                &channel.endpoint,
                timeout_ms,
                enforce_protocol_contract,
                HttpJsonRuntimeEvidenceKind::RequestOnly {
                    request: request_payload_for_runtime,
                },
            );
            Err(BridgeExecutionFailure {
                blocked: false,
                reason: "http_json bridge worker thread panicked".to_owned(),
                runtime_evidence,
            })
        }
    }
}

fn extract_http_json_response_payload(response_body: &Value) -> Value {
    let response_object = response_body.as_object();
    let Some(response_object) = response_object else {
        return response_body.clone();
    };

    let payload = response_object.get("payload");
    let Some(payload) = payload else {
        return response_body.clone();
    };

    payload.clone()
}
