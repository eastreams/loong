use super::{AppContext, ToolIntent, TurnFailure};

pub(super) fn effective_visible_tool_name(
    _intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
) -> String {
    descriptor.name.to_owned()
}

pub(super) fn provider_tool_denial_should_conceal_name(
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
    tool_is_visible: bool,
) -> bool {
    if !intent.source.starts_with("provider_") {
        return false;
    }

    if !descriptor.is_provider_exposed() {
        return true;
    }

    !tool_is_visible
}

pub(super) fn concealed_provider_tool_denial() -> TurnFailure {
    let base_reason = "tool_not_found: requested tool is not available";
    let reason = provider_tool_denial_reason(base_reason, "provider_tool_call");
    TurnFailure::policy_denied("tool_not_found", reason)
}

pub(super) fn provider_tool_denial_reason(reason: &str, source: &str) -> String {
    if !source.starts_with("provider_") {
        return reason.to_owned();
    }
    reason.to_owned()
}

pub(super) fn tool_intent_is_visible(
    session_context: &AppContext,
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
) -> bool {
    if descriptor.is_provider_exposed() {
        return true;
    }

    if intent.source.starts_with("provider_") {
        return false;
    }

    session_context.tool_view.contains(descriptor.name)
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn concealed_provider_tool_denial_is_plain_policy_denial() {
        let failure = concealed_provider_tool_denial();

        assert_eq!(failure.code, "tool_not_found");
        assert_eq!(
            failure.reason,
            "tool_not_found: requested tool is not available"
        );
        assert!(!failure.supports_discovery_recovery);
    }
}
