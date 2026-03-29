use loongclaw_app as mvp;

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

pub(super) fn bootstrap_mcp_server_summary(bootstrap_mcp_servers: &[String]) -> Option<String> {
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
