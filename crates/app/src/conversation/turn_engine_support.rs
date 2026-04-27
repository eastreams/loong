use std::collections::BTreeSet;

use loong_contracts::{KernelError, ToolPlaneError};

use super::{
    ApprovalRequirement, KernelFailureClass, TOOL_PREFLIGHT_ALLOW_RULE_ID, ToolDecisionTelemetry,
    ToolView, TurnFailure,
};

pub(crate) fn classify_kernel_error(error: &KernelError) -> KernelFailureClass {
    #[allow(clippy::wildcard_enum_match_arm)]
    match error {
        KernelError::Policy(_)
        | KernelError::PackCapabilityBoundary { .. }
        | KernelError::ConnectorNotAllowed { .. } => KernelFailureClass::PolicyDenied,
        KernelError::ToolPlane(ToolPlaneError::Execution(reason)) => {
            classify_tool_execution_reason(reason)
        }
        _ => KernelFailureClass::NonRetryable,
    }
}

pub(super) fn generic_allow_tool_decision(tool_name: &str) -> ToolDecisionTelemetry {
    let reason = format!("tool preflight allowed `{tool_name}`");
    ToolDecisionTelemetry::allow(tool_name, reason, TOOL_PREFLIGHT_ALLOW_RULE_ID)
}

pub(super) fn approval_required_tool_decision(
    tool_name: &str,
    requirement: &ApprovalRequirement,
) -> ToolDecisionTelemetry {
    ToolDecisionTelemetry::approval_required(
        tool_name,
        requirement.reason.clone(),
        requirement.rule_id.clone(),
    )
}

pub(super) fn denied_tool_decision(
    tool_name: &str,
    failure: &TurnFailure,
) -> ToolDecisionTelemetry {
    ToolDecisionTelemetry::deny(tool_name, failure.reason.clone(), failure.code.clone())
}

fn classify_tool_execution_reason(reason: &str) -> KernelFailureClass {
    if reason.starts_with("policy_denied: ") {
        KernelFailureClass::PolicyDenied
    } else {
        KernelFailureClass::RetryableExecution
    }
}

pub(super) struct RepairableToolPreflight;

impl RepairableToolPreflight {
    const PREFIX: &str = "tool_preflight_repairable: ";

    pub(super) fn encode(reason: &str) -> String {
        format!("{}{reason}", Self::PREFIX)
    }

    pub(super) fn parse(encoded: &str) -> Option<&str> {
        encoded.strip_prefix(Self::PREFIX)
    }

    pub(super) fn render(reason: &str) -> String {
        format!("tool_preflight_denied: tool input needs repair: {reason}")
    }
}

pub(super) fn render_app_tool_denied_reason(reason: &str) -> String {
    reason
        .strip_prefix("app_tool_denied: ")
        .unwrap_or(reason)
        .to_owned()
}

pub(super) fn with_runtime_ready_browser_companion_tools(
    base_view: ToolView,
    session_tool_view: &ToolView,
) -> ToolView {
    let mut names: BTreeSet<String> = base_view.tool_names().map(str::to_owned).collect();
    names.extend(
        session_tool_view
            .tool_names()
            .filter(|name| name.starts_with("browser.companion."))
            .map(str::to_owned),
    );
    ToolView::from_tool_names(names)
}
