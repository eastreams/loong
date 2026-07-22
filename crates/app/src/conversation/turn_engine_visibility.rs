use super::TurnFailure;

/// Return the single provider-facing denial used when naming a hidden or absent
/// tool would disclose registry contents.
pub(super) fn concealed_provider_tool_denial() -> TurnFailure {
    TurnFailure::policy_denied(
        "tool_not_found",
        "tool_not_found: requested tool is not available",
    )
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
