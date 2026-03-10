use super::*;

#[test]
fn parse_provider_kind_accepts_primary_and_legacy_aliases() {
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("openai"),
        Some(mvp::config::ProviderKind::Openai)
    );
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("openrouter_compatible"),
        Some(mvp::config::ProviderKind::Openrouter)
    );
    assert_eq!(
        crate::onboard_cli::parse_provider_kind("volcengine_custom"),
        Some(mvp::config::ProviderKind::Volcengine)
    );
    assert_eq!(crate::onboard_cli::parse_provider_kind("unsupported"), None);
}

#[test]
fn provider_default_env_mapping_is_stable() {
    assert_eq!(
        crate::onboard_cli::provider_default_api_key_env(mvp::config::ProviderKind::Openai),
        "OPENAI_API_KEY"
    );
    assert_eq!(
        crate::onboard_cli::provider_default_api_key_env(mvp::config::ProviderKind::Anthropic),
        "ANTHROPIC_API_KEY"
    );
    assert_eq!(
        crate::onboard_cli::provider_default_api_key_env(mvp::config::ProviderKind::Openrouter),
        "OPENROUTER_API_KEY"
    );
}

#[test]
fn non_interactive_requires_explicit_risk_acknowledgement() {
    let denied = crate::onboard_cli::validate_non_interactive_risk_gate(true, false)
        .expect_err("risk gate should reject non-interactive without acknowledgement");
    assert!(denied.contains("--accept-risk"));

    crate::onboard_cli::validate_non_interactive_risk_gate(true, true)
        .expect("risk gate should pass after acknowledgement");
    crate::onboard_cli::validate_non_interactive_risk_gate(false, false)
        .expect("interactive mode should not require explicit flag");
}
