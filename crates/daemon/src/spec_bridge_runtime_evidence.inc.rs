#[derive(Debug, Clone, Serialize)]
struct HttpJsonRuntimeBase {
    executor: &'static str,
    method: String,
    url: String,
    timeout_ms: u64,
    enforce_protocol_contract: bool,
    #[serde(flatten)]
    protocol: BridgeProtocolRuntimeContext,
}

#[derive(Debug)]
enum HttpJsonRuntimeEvidenceKind {
    BaseOnly,
    RequestOnly {
        request: Value,
    },
    Response {
        status_code: u16,
        request: Value,
        response_text: String,
        response_json: Value,
        response_method: Option<String>,
        response_id: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct HttpJsonRuntimeRequestOnly {
    #[serde(flatten)]
    base: HttpJsonRuntimeBase,
    request: Value,
}

#[derive(Debug, Serialize)]
struct HttpJsonRuntimeResponse {
    #[serde(flatten)]
    base: HttpJsonRuntimeBase,
    status_code: u16,
    request: Value,
    response_text: String,
    response_json: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_id: Option<String>,
}

fn http_json_runtime_evidence(
    context: &ConnectorProtocolContext,
    method: &str,
    url: &str,
    timeout_ms: u64,
    enforce_protocol_contract: bool,
    evidence_kind: HttpJsonRuntimeEvidenceKind,
) -> Value {
    const EXECUTOR: &str = "http_json_reqwest";
    let base = HttpJsonRuntimeBase {
        executor: EXECUTOR,
        method: method.to_owned(),
        url: url.to_owned(),
        timeout_ms,
        enforce_protocol_contract,
        protocol: BridgeProtocolRuntimeContext::from_connector_context(context),
    };
    match evidence_kind {
        HttpJsonRuntimeEvidenceKind::BaseOnly => serialize_runtime_evidence(EXECUTOR, &base),
        HttpJsonRuntimeEvidenceKind::RequestOnly { request } => serialize_runtime_evidence(
            EXECUTOR,
            &HttpJsonRuntimeRequestOnly { base, request },
        ),
        HttpJsonRuntimeEvidenceKind::Response {
            status_code,
            request,
            response_text,
            response_json,
            response_method,
            response_id,
        } => serialize_runtime_evidence(
            EXECUTOR,
            &HttpJsonRuntimeResponse {
                base,
                status_code,
                request,
                response_text,
                response_json,
                response_method,
                response_id,
            },
        ),
    }
}

#[derive(Debug, Clone, Serialize)]
struct ProcessStdioRuntimeBase {
    executor: &'static str,
    transport_kind: &'static str,
    command: String,
    args: Vec<String>,
    timeout_ms: u64,
    #[serde(flatten)]
    protocol: BridgeProtocolRuntimeContext,
}

#[derive(Debug)]
enum ProcessStdioRuntimeEvidenceKind {
    BaseOnly,
    Execution {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        stdout_json: Value,
        response_method: String,
        response_id: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct ProcessStdioRuntimeExecution {
    #[serde(flatten)]
    base: ProcessStdioRuntimeBase,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_json: Value,
    response_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_id: Option<String>,
}

fn process_stdio_runtime_evidence(
    context: &ConnectorProtocolContext,
    command: &str,
    args: &[String],
    timeout_ms: u64,
    evidence_kind: ProcessStdioRuntimeEvidenceKind,
) -> Value {
    const EXECUTOR: &str = "process_stdio_local";
    let base = ProcessStdioRuntimeBase {
        executor: EXECUTOR,
        transport_kind: "json_line",
        command: command.to_owned(),
        args: args.to_vec(),
        timeout_ms,
        protocol: BridgeProtocolRuntimeContext::from_connector_context(context),
    };
    match evidence_kind {
        ProcessStdioRuntimeEvidenceKind::BaseOnly => serialize_runtime_evidence(EXECUTOR, &base),
        ProcessStdioRuntimeEvidenceKind::Execution {
            exit_code,
            stdout,
            stderr,
            stdout_json,
            response_method,
            response_id,
        } => serialize_runtime_evidence(
            EXECUTOR,
            &ProcessStdioRuntimeExecution {
                base,
                exit_code,
                stdout,
                stderr,
                stdout_json,
                response_method,
                response_id,
            },
        ),
    }
}

fn serialize_runtime_evidence<T: Serialize>(executor: &str, runtime: &T) -> Value {
    serde_json::to_value(runtime).unwrap_or_else(|error| {
        json!({
            "executor": executor,
            "serialization_error": error.to_string(),
        })
    })
}
