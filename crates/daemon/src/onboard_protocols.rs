use loongclaw_app as mvp;

use crate::CliResult;
use crate::onboard_state::{OnboardDraft, OnboardProtocolDraft, OnboardValueOrigin};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProtocolStepValues {
    pub acp_enabled: bool,
    pub acp_enabled_origin: Option<OnboardValueOrigin>,
    pub acp_backend: Option<String>,
    pub acp_backend_origin: Option<OnboardValueOrigin>,
    pub bootstrap_mcp_servers: Vec<String>,
    pub bootstrap_mcp_servers_origin: Option<OnboardValueOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AvailableAcpBackend {
    pub id: String,
    pub summary: String,
}

pub(super) fn protocol_draft_from_config(
    config: &mvp::config::LoongClawConfig,
) -> OnboardProtocolDraft {
    OnboardProtocolDraft {
        acp_enabled: config.acp.enabled,
        acp_backend: config.acp.backend_id(),
        bootstrap_mcp_servers: config
            .acp
            .dispatch
            .bootstrap_mcp_server_names()
            .unwrap_or_else(|_error| config.acp.dispatch.bootstrap_mcp_servers.clone()),
    }
}

pub(super) fn list_available_acp_backends() -> CliResult<Vec<AvailableAcpBackend>> {
    Ok(mvp::acp::list_acp_backend_metadata()?
        .into_iter()
        .map(|metadata| AvailableAcpBackend {
            id: metadata.id.to_owned(),
            summary: metadata.summary.to_owned(),
        })
        .collect())
}

pub(super) fn default_acp_backend_id(
    draft: &OnboardDraft,
    available_backends: &[AvailableAcpBackend],
) -> Option<String> {
    let current_backend = draft
        .protocols
        .acp_backend
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(current_backend) = current_backend
        && available_backends
            .iter()
            .any(|backend| backend.id == current_backend)
    {
        return Some(current_backend);
    }

    let resolved_backend = mvp::acp::resolve_acp_backend_selection(&draft.config).id;
    if available_backends
        .iter()
        .any(|backend| backend.id == resolved_backend)
    {
        return Some(resolved_backend);
    }

    available_backends.first().map(|backend| backend.id.clone())
}

pub(super) fn derive_protocol_step_values(draft: &OnboardDraft) -> ProtocolStepValues {
    let protocols = protocol_draft_from_config(&draft.config);

    ProtocolStepValues {
        acp_enabled: protocols.acp_enabled,
        acp_enabled_origin: draft.origin_for(OnboardDraft::ACP_ENABLED_KEY),
        acp_backend: protocols.acp_backend,
        acp_backend_origin: draft.origin_for(OnboardDraft::ACP_BACKEND_KEY),
        bootstrap_mcp_servers: protocols.bootstrap_mcp_servers,
        bootstrap_mcp_servers_origin: draft.origin_for(OnboardDraft::ACP_BOOTSTRAP_MCP_SERVERS_KEY),
    }
}

pub(super) fn bootstrap_mcp_server_summary(
    acp_enabled: bool,
    bootstrap_mcp_servers: &[String],
) -> Option<String> {
    if !acp_enabled {
        return None;
    }

    let servers = bootstrap_mcp_servers
        .iter()
        .map(|server| server.trim())
        .filter(|server| !server.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if servers.is_empty() {
        None
    } else {
        Some(servers.join(", "))
    }
}
