use crate::trusted_host_runtime::TrustedHostSessionShutdownReason;
use crate::{CliResult, mvp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcpResolvedCloseTarget {
    pub resolved_session_key: String,
    pub status: mvp::acp::AcpSessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcpCloseOutcome {
    pub resolved_session_key: String,
    pub hook_dispatched: bool,
    pub shutdown_reason: TrustedHostSessionShutdownReason,
}

pub(crate) async fn resolve_acp_close_target(
    config: &mvp::config::LoongConfig,
    manager: &mvp::acp::AcpSessionManager,
    session_key: Option<&str>,
    conversation_id: Option<&str>,
    route_session_id: Option<&str>,
) -> CliResult<AcpResolvedCloseTarget> {
    let resolved_session_key = crate::resolve_acp_status_session_key(
        config,
        session_key,
        conversation_id,
        route_session_id,
    )?;
    let status = manager
        .get_status(config, resolved_session_key.as_str())
        .await?;

    Ok(AcpResolvedCloseTarget {
        resolved_session_key,
        status,
    })
}

pub(crate) async fn close_resolved_acp_target(
    config: &mvp::config::LoongConfig,
    manager: &mvp::acp::AcpSessionManager,
    target: &AcpResolvedCloseTarget,
    reason: TrustedHostSessionShutdownReason,
) -> CliResult<AcpCloseOutcome> {
    manager
        .close(config, target.resolved_session_key.as_str())
        .await?;
    crate::trusted_host_runtime::dispatch_session_shutdown_hook_for_acp_status(
        config,
        &target.status,
        reason,
    )
    .await?;

    Ok(AcpCloseOutcome {
        resolved_session_key: target.resolved_session_key.clone(),
        hook_dispatched: true,
        shutdown_reason: reason,
    })
}
