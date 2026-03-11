use crate::CliResult;

use crate::config::{LoongClawConfig, ProviderConfig, ProviderKind};

pub(super) fn provider_request_failed_for_all_models(last_error: Option<String>) -> String {
    last_error.unwrap_or_else(|| "provider request failed for every model candidate".to_owned())
}

pub(super) fn validate_provider_feature_gate(config: &LoongClawConfig) -> CliResult<()> {
    match config.provider.kind {
        ProviderKind::Volcengine => {
            if !cfg!(feature = "provider-volcengine") {
                return Err(
                    "volcengine provider is disabled (enable feature `provider-volcengine`)"
                        .to_owned(),
                );
            }
        }
        _ => {
            if !cfg!(feature = "provider-openai") {
                return Err(
                    "openai-compatible provider family is disabled (enable feature `provider-openai`)"
                        .to_owned(),
                );
            }
        }
    }
    Ok(())
}

pub(super) fn validate_provider_configuration(config: &LoongClawConfig) -> CliResult<()> {
    if config.provider.kind == ProviderKind::Kimi
        && provider_uses_kimi_coding_endpoint(&config.provider)
    {
        return Err(
            "kimi provider cannot target Kimi Coding endpoints; use `kind = \"kimi_coding\"`"
                .to_owned(),
        );
    }

    if config.provider.kind == ProviderKind::KimiCoding {
        if let Some(user_agent) = config.provider.header_value("user-agent") {
            if !is_kimi_cli_user_agent(user_agent) {
                return Err(format!(
                    "kimi_coding provider requires a `User-Agent` header starting with `KimiCLI/`; got `{user_agent}`"
                ));
            }
        }
    }

    Ok(())
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
