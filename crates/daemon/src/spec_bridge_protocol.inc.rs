#[derive(Debug, Clone)]
struct ConnectorProtocolContext {
    request_method: String,
    request_id: Option<String>,
    route_method: Option<String>,
    required_capability: Option<String>,
    capabilities: BTreeSet<String>,
}

impl ConnectorProtocolContext {
    fn from_connector_command(
        provider: &kernel::ProviderConfig,
        channel: &kernel::ChannelConfig,
        command: &ConnectorCommand,
    ) -> Self {
        Self {
            request_method: "tools/call".to_owned(),
            request_id: Some(format!(
                "{}:{}:{}",
                provider.provider_id, channel.channel_id, command.operation
            )),
            route_method: None,
            required_capability: None,
            capabilities: protocol_capabilities_for_connector_command(command),
        }
    }

    fn capabilities_vec(&self) -> Vec<String> {
        self.capabilities.iter().cloned().collect::<Vec<_>>()
    }

    fn outbound_frame(&self, payload: Value) -> OutboundFrame {
        OutboundFrame {
            method: self.request_method.clone(),
            id: self.request_id.clone(),
            payload,
        }
    }
}

fn authorize_connector_protocol_context(context: &mut ConnectorProtocolContext) -> Result<(), String> {
    let router = ProtocolRouter::default();
    let resolved_route = router
        .resolve(&context.request_method)
        .map_err(|error| {
            format!(
                "protocol method {} is invalid: {error}",
                context.request_method
            )
        })?;
    context.route_method = Some(resolved_route.method().to_owned());
    context.required_capability = resolved_route.policy.required_capability.clone();
    router
        .authorize(
            &resolved_route,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: context.capabilities.clone(),
            },
        )
        .map_err(|error| format!("protocol route authorization failed: {error}"))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct BridgeProtocolRuntimeContext {
    request_method: String,
    request_id: Option<String>,
    protocol_route: Option<String>,
    protocol_required_capability: Option<String>,
    protocol_capabilities: Vec<String>,
}

impl BridgeProtocolRuntimeContext {
    fn from_connector_context(context: &ConnectorProtocolContext) -> Self {
        Self {
            request_method: context.request_method.clone(),
            request_id: context.request_id.clone(),
            protocol_route: context.route_method.clone(),
            protocol_required_capability: context.required_capability.clone(),
            protocol_capabilities: context.capabilities_vec(),
        }
    }
}

include!("spec_bridge_runtime_evidence.inc.rs");

fn parse_http_timeout_ms(provider: &kernel::ProviderConfig) -> u64 {
    parse_clamped_timeout_ms(provider, "http_timeout_ms", 8_000, 300_000)
}

fn parse_http_enforce_protocol_contract(provider: &kernel::ProviderConfig) -> bool {
    parse_bool_flag(
        provider
            .metadata
            .get("http_enforce_protocol_contract")
            .map(String::as_str),
    )
}

fn parse_process_timeout_ms(provider: &kernel::ProviderConfig) -> u64 {
    parse_clamped_timeout_ms(provider, "process_timeout_ms", 5_000, 300_000)
}

fn parse_bool_flag(raw: Option<&str>) -> bool {
    raw.map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn parse_clamped_timeout_ms(
    provider: &kernel::ProviderConfig,
    metadata_key: &str,
    default_ms: u64,
    max_ms: u64,
) -> u64 {
    provider
        .metadata
        .get(metadata_key)
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(max_ms))
        .unwrap_or(default_ms)
}

fn protocol_capabilities_for_connector_command(command: &ConnectorCommand) -> BTreeSet<String> {
    let mut capabilities = BTreeSet::new();
    for capability in &command.required_capabilities {
        match capability {
            Capability::MemoryRead
            | Capability::FilesystemRead
            | Capability::ObserveTelemetry => {
                capabilities.insert("discover".to_owned());
            }
            _ => {
                capabilities.insert("invoke".to_owned());
            }
        }
    }
    if capabilities.is_empty() {
        capabilities.insert("invoke".to_owned());
    }
    capabilities
}
