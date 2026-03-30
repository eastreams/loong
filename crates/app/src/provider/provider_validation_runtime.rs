use crate::CliResult;
use crate::config::{LoongClawConfig, ProviderConfig};

use super::contracts::provider_runtime_contract;

pub(super) fn validate_provider_feature_gate(config: &LoongClawConfig) -> CliResult<()> {
    let support_facts = config.provider.support_facts();
    let feature_support = support_facts.feature;
    if !feature_support.enabled_in_build {
        return Err(feature_support.disabled_message);
    }
    Ok(())
}

pub(super) fn validate_provider_configuration(config: &LoongClawConfig) -> CliResult<()> {
    let runtime_contract = provider_runtime_contract(&config.provider);
    if runtime_contract.validation.forbid_kimi_coding_endpoint
        && provider_uses_kimi_coding_endpoint(&config.provider)
    {
        return Err(
            "kimi provider cannot target Kimi Coding endpoints; use `kind = \"kimi_coding\"`"
                .to_owned(),
        );
    }

    if runtime_contract
        .validation
        .require_kimi_cli_user_agent_prefix
        && let Some(user_agent) = config.provider.header_value("user-agent")
        && !is_kimi_cli_user_agent(user_agent)
    {
        return Err(format!(
            "kimi_coding provider requires a `User-Agent` header starting with `KimiCLI/`; got `{user_agent}`"
        ));
    }

    Ok(())
}

pub(super) async fn validate_provider_auth_readiness(config: &LoongClawConfig) -> CliResult<()> {
    let support_facts = config.provider.support_facts();
    let auth_support = support_facts.auth;
    if !auth_support.requires_explicit_configuration {
        return Ok(());
    }

    if super::provider_auth_ready(config).await {
        return Ok(());
    }

    Err(auth_support.missing_configuration_message)
}

fn provider_uses_kimi_coding_endpoint(provider: &ProviderConfig) -> bool {
    is_kimi_coding_endpoint(provider.endpoint().as_str())
        || provider
            .endpoint
            .as_deref()
            .is_some_and(is_kimi_coding_endpoint)
}

fn is_kimi_coding_endpoint(endpoint: &str) -> bool {
    endpoint
        .trim()
        .to_ascii_lowercase()
        .contains("://api.kimi.com/coding/")
}

fn is_kimi_cli_user_agent(user_agent: &str) -> bool {
    user_agent.trim().starts_with("KimiCLI/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderKind;

    fn build_config(provider: ProviderConfig) -> LoongClawConfig {
        LoongClawConfig {
            provider,
            ..LoongClawConfig::default()
        }
    }

    #[test]
    fn validate_provider_configuration_rejects_plain_kimi_on_coding_endpoint() {
        let config = build_config(ProviderConfig {
            kind: ProviderKind::Kimi,
            endpoint: Some("https://api.kimi.com/coding/v1/chat/completions".to_owned()),
            ..ProviderConfig::default()
        });

        let error = validate_provider_configuration(&config).expect_err("should fail");
        assert!(
            error.contains("kimi provider cannot target Kimi Coding endpoints"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_provider_configuration_rejects_incompatible_kimi_coding_user_agent() {
        let config = build_config(ProviderConfig {
            kind: ProviderKind::KimiCoding,
            headers: [("User-Agent".to_owned(), "curl/8.7.1".to_owned())]
                .into_iter()
                .collect(),
            ..ProviderConfig::default()
        });

        let error = validate_provider_configuration(&config).expect_err("should fail");
        assert!(
            error.contains("requires a `User-Agent` header starting with `KimiCLI/`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_provider_configuration_accepts_compatible_kimi_coding_user_agent() {
        let config = build_config(ProviderConfig {
            kind: ProviderKind::KimiCoding,
            headers: [("User-Agent".to_owned(), "KimiCLI/custom".to_owned())]
                .into_iter()
                .collect(),
            ..ProviderConfig::default()
        });

        validate_provider_configuration(&config).expect("compatible Kimi coding user-agent");
    }
}
