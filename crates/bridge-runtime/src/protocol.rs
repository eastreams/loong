use std::collections::BTreeSet;

use loongclaw_contracts::ExecutionSecurityTier;
use loongclaw_kernel as kernel;
use loongclaw_protocol::{
    OutboundFrame, PROTOCOL_VERSION, ProtocolRouter, RouteAuthorizationRequest,
};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct ConnectorProtocolContext {
    pub request_method: String,
    pub request_id: Option<String>,
    pub route_method: Option<String>,
    pub required_capability: Option<String>,
    pub capabilities: BTreeSet<String>,
}

impl ConnectorProtocolContext {
    pub fn from_connector_command(
        provider: &kernel::ProviderConfig,
        channel: &kernel::ChannelConfig,
        command: &kernel::ConnectorCommand,
    ) -> Self {
        let request_method = "tools/call".to_owned();
        let request_id = Some(format!(
            "{}:{}:{}",
            provider.provider_id, channel.channel_id, command.operation,
        ));
        let route_method = None;
        let required_capability = None;
        let capabilities = protocol_capabilities_for_connector_command(command);

        Self {
            request_method,
            request_id,
            route_method,
            required_capability,
            capabilities,
        }
    }

    pub fn capabilities_vec(&self) -> Vec<String> {
        self.capabilities.iter().cloned().collect::<Vec<_>>()
    }

    pub fn outbound_frame(&self, payload: Value) -> OutboundFrame {
        let method = self.request_method.clone();
        let id = self.request_id.clone();
        let version = PROTOCOL_VERSION;

        OutboundFrame {
            method,
            id,
            payload,
            version,
        }
    }
}

pub fn authorize_connector_protocol_context(
    context: &mut ConnectorProtocolContext,
) -> Result<(), String> {
    let router = ProtocolRouter::default();
    let resolved_route = router.resolve(&context.request_method);
    let resolved_route = resolved_route.map_err(|error| {
        let reason = format!(
            "protocol method {} is invalid: {error}",
            context.request_method,
        );
        reason
    })?;

    let route_method = resolved_route.method().to_owned();
    let required_capability = resolved_route.policy.required_capability.clone();
    context.route_method = Some(route_method);
    context.required_capability = required_capability;

    let request = RouteAuthorizationRequest {
        authenticated: true,
        capabilities: context.capabilities.clone(),
    };
    let authorization = router.authorize(&resolved_route, &request);
    authorization.map_err(|error| {
        let reason = format!("protocol route authorization failed: {error}");
        reason
    })?;

    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeProtocolRuntimeContext {
    pub request_method: String,
    pub request_id: Option<String>,
    pub protocol_route: Option<String>,
    pub protocol_required_capability: Option<String>,
    pub protocol_capabilities: Vec<String>,
}

impl BridgeProtocolRuntimeContext {
    pub fn from_connector_context(context: &ConnectorProtocolContext) -> Self {
        let request_method = context.request_method.clone();
        let request_id = context.request_id.clone();
        let protocol_route = context.route_method.clone();
        let protocol_required_capability = context.required_capability.clone();
        let protocol_capabilities = context.capabilities_vec();

        Self {
            request_method,
            request_id,
            protocol_route,
            protocol_required_capability,
            protocol_capabilities,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpJsonRuntimeBase {
    pub executor: &'static str,
    pub method: String,
    pub url: String,
    pub timeout_ms: u64,
    pub enforce_protocol_contract: bool,
    #[serde(flatten)]
    pub protocol: BridgeProtocolRuntimeContext,
}

#[derive(Debug)]
pub enum HttpJsonRuntimeEvidenceKind {
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
pub struct HttpJsonRuntimeRequestOnly {
    #[serde(flatten)]
    pub base: HttpJsonRuntimeBase,
    pub request: Value,
}

#[derive(Debug, Serialize)]
pub struct HttpJsonRuntimeResponse {
    #[serde(flatten)]
    pub base: HttpJsonRuntimeBase,
    pub status_code: u16,
    pub request: Value,
    pub response_text: String,
    pub response_json: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
}

pub fn http_json_runtime_evidence(
    context: &ConnectorProtocolContext,
    method: &str,
    url: &str,
    timeout_ms: u64,
    enforce_protocol_contract: bool,
    evidence_kind: HttpJsonRuntimeEvidenceKind,
) -> Value {
    const EXECUTOR: &str = "http_json_reqwest";
    let method = method.to_owned();
    let url = url.to_owned();
    let protocol = BridgeProtocolRuntimeContext::from_connector_context(context);

    let base = HttpJsonRuntimeBase {
        executor: EXECUTOR,
        method,
        url,
        timeout_ms,
        enforce_protocol_contract,
        protocol,
    };

    match evidence_kind {
        HttpJsonRuntimeEvidenceKind::BaseOnly => serialize_runtime_evidence(EXECUTOR, &base),
        HttpJsonRuntimeEvidenceKind::RequestOnly { request } => {
            let runtime = HttpJsonRuntimeRequestOnly { base, request };
            serialize_runtime_evidence(EXECUTOR, &runtime)
        }
        HttpJsonRuntimeEvidenceKind::Response {
            status_code,
            request,
            response_text,
            response_json,
            response_method,
            response_id,
        } => {
            let runtime = HttpJsonRuntimeResponse {
                base,
                status_code,
                request,
                response_text,
                response_json,
                response_method,
                response_id,
            };
            serialize_runtime_evidence(EXECUTOR, &runtime)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessStdioRuntimeBase {
    pub executor: &'static str,
    pub transport_kind: &'static str,
    pub execution_tier: ExecutionSecurityTier,
    pub command: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    #[serde(flatten)]
    pub protocol: BridgeProtocolRuntimeContext,
}

#[derive(Debug)]
pub enum ProcessStdioRuntimeEvidenceKind {
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
pub struct ProcessStdioRuntimeExecution {
    #[serde(flatten)]
    pub base: ProcessStdioRuntimeBase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_json: Value,
    pub response_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
}

pub fn process_stdio_runtime_evidence(
    context: &ConnectorProtocolContext,
    execution_tier: ExecutionSecurityTier,
    command: &str,
    args: &[String],
    timeout_ms: u64,
    evidence_kind: ProcessStdioRuntimeEvidenceKind,
) -> Value {
    const EXECUTOR: &str = "process_stdio_local";
    let command = command.to_owned();
    let args = args.to_vec();
    let protocol = BridgeProtocolRuntimeContext::from_connector_context(context);

    let base = ProcessStdioRuntimeBase {
        executor: EXECUTOR,
        transport_kind: "json_line",
        execution_tier,
        command,
        args,
        timeout_ms,
        protocol,
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
        } => {
            let runtime = ProcessStdioRuntimeExecution {
                base,
                exit_code,
                stdout,
                stderr,
                stdout_json,
                response_method,
                response_id,
            };
            serialize_runtime_evidence(EXECUTOR, &runtime)
        }
    }
}

pub fn serialize_runtime_evidence<T: Serialize>(executor: &str, runtime: &T) -> Value {
    let encoded_runtime = serde_json::to_value(runtime);
    encoded_runtime.unwrap_or_else(|error| {
        let executor = executor.to_owned();
        let serialization_error = error.to_string();
        json!({
            "executor": executor,
            "serialization_error": serialization_error,
        })
    })
}

pub fn parse_http_timeout_ms(provider: &kernel::ProviderConfig) -> u64 {
    parse_clamped_timeout_ms(provider, "http_timeout_ms", 8_000, 300_000)
}

pub fn parse_http_enforce_protocol_contract(provider: &kernel::ProviderConfig) -> bool {
    let raw_flag = provider
        .metadata
        .get("http_enforce_protocol_contract")
        .map(String::as_str);
    parse_bool_flag(raw_flag)
}

pub fn parse_process_timeout_ms(provider: &kernel::ProviderConfig) -> u64 {
    parse_clamped_timeout_ms(provider, "process_timeout_ms", 5_000, 300_000)
}

pub fn parse_bool_flag(raw: Option<&str>) -> bool {
    let normalized_flag = raw.map(|value| value.trim().to_ascii_lowercase());
    normalized_flag.is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

pub fn parse_clamped_timeout_ms(
    provider: &kernel::ProviderConfig,
    metadata_key: &str,
    default_ms: u64,
    max_ms: u64,
) -> u64 {
    let configured_value = provider
        .metadata
        .get(metadata_key)
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0);
    let timeout_ms = configured_value.map(|value| value.min(max_ms));
    timeout_ms.unwrap_or(default_ms)
}

pub fn protocol_capabilities_for_connector_command(
    command: &kernel::ConnectorCommand,
) -> BTreeSet<String> {
    let mut capabilities = BTreeSet::new();

    for capability in &command.required_capabilities {
        match capability {
            kernel::Capability::MemoryRead
            | kernel::Capability::FilesystemRead
            | kernel::Capability::ObserveTelemetry => {
                capabilities.insert("discover".to_owned());
            }
            kernel::Capability::InvokeTool
            | kernel::Capability::InvokeConnector
            | kernel::Capability::MemoryWrite
            | kernel::Capability::FilesystemWrite
            | kernel::Capability::NetworkEgress
            | kernel::Capability::ControlRead
            | kernel::Capability::ControlWrite
            | kernel::Capability::ControlApprovals
            | kernel::Capability::ControlPairing
            | kernel::Capability::ControlAcp
            | _ => {
                capabilities.insert("invoke".to_owned());
            }
        }
    }

    let capabilities_are_empty = capabilities.is_empty();
    if capabilities_are_empty {
        capabilities.insert("invoke".to_owned());
    }

    capabilities
}
