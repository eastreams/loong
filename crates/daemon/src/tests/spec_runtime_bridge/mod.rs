use super::*;

const HTTP_JSON_RUNTIME_BASE_KEYS: &[&str] = &[
    "enforce_protocol_contract",
    "executor",
    "method",
    "protocol_capabilities",
    "protocol_required_capability",
    "protocol_route",
    "request_id",
    "request_method",
    "timeout_ms",
    "url",
];

const PROCESS_STDIO_RUNTIME_BASE_KEYS: &[&str] = &[
    "args",
    "command",
    "executor",
    "protocol_capabilities",
    "protocol_required_capability",
    "protocol_route",
    "request_id",
    "request_method",
    "timeout_ms",
    "transport_kind",
];

fn assert_bridge_runtime_protocol_context(
    runtime: &Value,
    expected_request_id: &str,
    expected_required_capability: &str,
    expected_granted_capability: &str,
) {
    assert_eq!(runtime["request_method"], "tools/call");
    assert_eq!(runtime["request_id"], expected_request_id);
    assert_eq!(runtime["protocol_route"], "tools/call");
    assert_eq!(
        runtime["protocol_required_capability"],
        expected_required_capability
    );
    let capabilities = runtime["protocol_capabilities"]
        .as_array()
        .expect("protocol_capabilities should be an array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        capabilities
            .iter()
            .any(|capability| *capability == expected_granted_capability),
        "protocol_capabilities should include {expected_granted_capability}, got {capabilities:?}",
    );
}

fn assert_http_json_runtime_shape(runtime: &Value) {
    for key in HTTP_JSON_RUNTIME_BASE_KEYS {
        assert!(
            runtime.get(*key).is_some(),
            "http_json runtime should include key `{key}`"
        );
    }
    assert_eq!(runtime["executor"], "http_json_reqwest");
}

fn assert_process_stdio_runtime_shape(runtime: &Value) {
    for key in PROCESS_STDIO_RUNTIME_BASE_KEYS {
        assert!(
            runtime.get(*key).is_some(),
            "process_stdio runtime should include key `{key}`"
        );
    }
    assert_eq!(runtime["executor"], "process_stdio_local");
    assert_eq!(runtime["transport_kind"], "json_line");
}

fn assert_runtime_keys_exact(runtime: &Value, expected_keys: &[&str]) {
    let runtime_object = runtime
        .as_object()
        .expect("runtime payload should be a JSON object");
    let mut actual = runtime_object.keys().cloned().collect::<Vec<_>>();
    actual.sort();
    let mut expected = expected_keys
        .iter()
        .map(|key| key.to_string())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(actual, expected, "runtime key set should stay stable");
}

fn assert_runtime_keys_with_base(runtime: &Value, base_keys: &[&str], extra_keys: &[&str]) {
    let mut expected = Vec::with_capacity(base_keys.len() + extra_keys.len());
    expected.extend_from_slice(base_keys);
    expected.extend_from_slice(extra_keys);
    assert_runtime_keys_exact(runtime, &expected);
}

fn assert_http_json_runtime_keys(runtime: &Value, extra_keys: &[&str]) {
    assert_runtime_keys_with_base(runtime, HTTP_JSON_RUNTIME_BASE_KEYS, extra_keys);
}

fn assert_process_stdio_runtime_keys(runtime: &Value, extra_keys: &[&str]) {
    assert_runtime_keys_with_base(runtime, PROCESS_STDIO_RUNTIME_BASE_KEYS, extra_keys);
}

fn snapshot_protocol_context() -> ConnectorProtocolContext {
    let provider = kernel::ProviderConfig {
        provider_id: "snapshot-provider".to_owned(),
        connector_name: "snapshot-provider".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::new(),
    };
    let channel = kernel::ChannelConfig {
        channel_id: "primary".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: "http://snapshot.local/invoke".to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = ConnectorCommand {
        connector_name: "snapshot-provider".to_owned(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({"query":"ping"}),
    };
    let mut context =
        ConnectorProtocolContext::from_connector_command(&provider, &channel, &command);
    authorize_connector_protocol_context(&mut context)
        .expect("snapshot protocol context should authorize");
    context
}

#[test]
fn bridge_http_json_runtime_evidence_snapshots_stable() {
    let context = snapshot_protocol_context();
    let base = http_json_runtime_evidence(
        &context,
        "POST",
        "http://snapshot.local/invoke",
        3_000,
        true,
        HttpJsonRuntimeEvidenceKind::BaseOnly,
    );
    assert_http_json_runtime_keys(&base, &[]);
    assert_eq!(
        base,
        json!({
            "executor":"http_json_reqwest",
            "method":"POST",
            "url":"http://snapshot.local/invoke",
            "timeout_ms":3000,
            "enforce_protocol_contract":true,
            "request_method":"tools/call",
            "request_id":"snapshot-provider:primary:invoke",
            "protocol_route":"tools/call",
            "protocol_required_capability":"invoke",
            "protocol_capabilities":["invoke"],
        })
    );

    let request_only = http_json_runtime_evidence(
        &context,
        "POST",
        "http://snapshot.local/invoke",
        3_000,
        true,
        HttpJsonRuntimeEvidenceKind::RequestOnly {
            request: json!({"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"query":"ping"}}),
        },
    );
    assert_http_json_runtime_keys(&request_only, &["request"]);
    assert_eq!(
        request_only,
        json!({
            "executor":"http_json_reqwest",
            "method":"POST",
            "url":"http://snapshot.local/invoke",
            "timeout_ms":3000,
            "enforce_protocol_contract":true,
            "request_method":"tools/call",
            "request_id":"snapshot-provider:primary:invoke",
            "protocol_route":"tools/call",
            "protocol_required_capability":"invoke",
            "protocol_capabilities":["invoke"],
            "request":{"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"query":"ping"}},
        })
    );

    let response = http_json_runtime_evidence(
        &context,
        "POST",
        "http://snapshot.local/invoke",
        3_000,
        true,
        HttpJsonRuntimeEvidenceKind::Response {
            status_code: 200,
            request: json!({"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"query":"ping"}}),
            response_text: "{\"method\":\"tools/call\",\"id\":\"snapshot-provider:primary:invoke\",\"payload\":{\"reply\":\"pong\"}}".to_owned(),
            response_json: json!({"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"reply":"pong"}}),
            response_method: Some("tools/call".to_owned()),
            response_id: Some("snapshot-provider:primary:invoke".to_owned()),
        },
    );
    assert_http_json_runtime_keys(
        &response,
        &[
            "request",
            "response_id",
            "response_json",
            "response_method",
            "response_text",
            "status_code",
        ],
    );
    assert_eq!(
        response,
        json!({
            "executor":"http_json_reqwest",
            "method":"POST",
            "url":"http://snapshot.local/invoke",
            "timeout_ms":3000,
            "enforce_protocol_contract":true,
            "request_method":"tools/call",
            "request_id":"snapshot-provider:primary:invoke",
            "protocol_route":"tools/call",
            "protocol_required_capability":"invoke",
            "protocol_capabilities":["invoke"],
            "status_code":200,
            "request":{"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"query":"ping"}},
            "response_text":"{\"method\":\"tools/call\",\"id\":\"snapshot-provider:primary:invoke\",\"payload\":{\"reply\":\"pong\"}}",
            "response_json":{"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"reply":"pong"}},
            "response_method":"tools/call",
            "response_id":"snapshot-provider:primary:invoke",
        })
    );
}

#[test]
fn bridge_process_stdio_runtime_evidence_snapshots_stable() {
    let context = snapshot_protocol_context();
    let base = process_stdio_runtime_evidence(
        &context,
        "cat",
        &["/tmp/input.txt".to_owned()],
        5_000,
        ProcessStdioRuntimeEvidenceKind::BaseOnly,
    );
    assert_process_stdio_runtime_keys(&base, &[]);
    assert_eq!(
        base,
        json!({
            "executor":"process_stdio_local",
            "transport_kind":"json_line",
            "command":"cat",
            "args":["/tmp/input.txt"],
            "timeout_ms":5000,
            "request_method":"tools/call",
            "request_id":"snapshot-provider:primary:invoke",
            "protocol_route":"tools/call",
            "protocol_required_capability":"invoke",
            "protocol_capabilities":["invoke"],
        })
    );

    let execution = process_stdio_runtime_evidence(
        &context,
        "cat",
        &["/tmp/input.txt".to_owned()],
        5_000,
        ProcessStdioRuntimeEvidenceKind::Execution {
            exit_code: Some(0),
            stdout: "{\"method\":\"tools/call\",\"id\":\"snapshot-provider:primary:invoke\",\"payload\":{\"reply\":\"pong\"}}".to_owned(),
            stderr: String::new(),
            stdout_json: json!({"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"reply":"pong"}}),
            response_method: "tools/call".to_owned(),
            response_id: Some("snapshot-provider:primary:invoke".to_owned()),
        },
    );
    assert_process_stdio_runtime_keys(
        &execution,
        &[
            "exit_code",
            "response_id",
            "response_method",
            "stderr",
            "stdout",
            "stdout_json",
        ],
    );
    assert_eq!(
        execution,
        json!({
            "executor":"process_stdio_local",
            "transport_kind":"json_line",
            "command":"cat",
            "args":["/tmp/input.txt"],
            "timeout_ms":5000,
            "request_method":"tools/call",
            "request_id":"snapshot-provider:primary:invoke",
            "protocol_route":"tools/call",
            "protocol_required_capability":"invoke",
            "protocol_capabilities":["invoke"],
            "exit_code":0,
            "stdout":"{\"method\":\"tools/call\",\"id\":\"snapshot-provider:primary:invoke\",\"payload\":{\"reply\":\"pong\"}}",
            "stderr":"",
            "stdout_json":{"method":"tools/call","id":"snapshot-provider:primary:invoke","payload":{"reply":"pong"}},
            "response_method":"tools/call",
            "response_id":"snapshot-provider:primary:invoke",
        })
    );
}

mod http_json;
mod process_stdio;
