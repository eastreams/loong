use std::env;

use loong_app as mvp;
use loong_contracts::SecretRef;

use crate::onboard_types::OnboardingCredentialSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuerySearchProviderStatus {
    pub provider: &'static str,
    pub provider_label: String,
    pub credential_summary: Option<OnboardingCredentialSummary>,
    pub credential_available: bool,
}

pub(crate) fn query_search_provider_display_name(provider: &str) -> String {
    mvp::config::web_search_provider_descriptor(provider)
        .map(|descriptor| descriptor.display_name.to_owned())
        .unwrap_or_else(|| provider.to_owned())
}

pub(crate) fn query_search_provider_status(
    config: &mvp::config::LoongConfig,
) -> QuerySearchProviderStatus {
    let configured_provider = config.tools.web_search.default_provider.as_str();
    let provider = mvp::config::normalize_web_search_provider(configured_provider)
        .unwrap_or(mvp::config::DEFAULT_WEB_SEARCH_PROVIDER);
    let provider_label = query_search_provider_display_name(provider);
    let credential_summary = summarize_query_search_credential(config, provider);
    let credential_available = query_search_has_available_credential(config, provider);

    QuerySearchProviderStatus {
        provider,
        provider_label,
        credential_summary,
        credential_available,
    }
}

impl QuerySearchProviderStatus {
    pub(crate) fn ready_detail(&self) -> String {
        self.credential_summary
            .as_ref()
            .map(|summary| format!("{}: {}", self.provider_label, summary.value))
            .unwrap_or_else(|| self.provider_label.clone())
    }

    pub(crate) fn blocked_detail(&self, mention_network_separation: bool) -> String {
        let tail = if mention_network_separation {
            "web.search will stay unavailable until the provider credential is supplied, but ordinary network access remains separately governed"
        } else {
            "web.search will stay unavailable until the provider credential is supplied"
        };

        self.credential_summary
            .as_ref()
            .map(|summary| format!("{}: {}. {tail}", self.provider_label, summary.value))
            .unwrap_or_else(|| self.provider_label.clone())
    }
}

pub(crate) fn configured_query_search_credential_source_value(
    config: &mvp::config::LoongConfig,
    provider: &str,
) -> Option<String> {
    let configured_secret = configured_query_search_secret(config, provider);
    configured_secret.and_then(|value| render_query_search_credential_source_value(Some(value)))
}

pub(crate) fn configured_query_search_credential_env_name(
    config: &mvp::config::LoongConfig,
    provider: &str,
) -> Option<String> {
    let raw = configured_query_search_secret(config, provider)?;
    let secret_ref = SecretRef::Inline(raw.trim().to_owned());
    secret_ref.explicit_env_name()
}

pub(crate) fn query_search_has_inline_credential(
    config: &mvp::config::LoongConfig,
    provider: &str,
) -> bool {
    let configured_secret = configured_query_search_secret(config, provider);
    configured_secret.is_some_and(|value| {
        let secret_ref = SecretRef::Inline(value.trim().to_owned());
        secret_ref.inline_literal_value().is_some()
    })
}

pub(crate) fn preferred_query_search_credential_env_default(
    config: &mvp::config::LoongConfig,
    provider: &str,
) -> String {
    if let Some(env_name) = configured_query_search_credential_env_name(config, provider) {
        return env_name;
    }
    if query_search_has_inline_credential(config, provider) {
        return String::new();
    }

    let Some(descriptor) = mvp::config::web_search_provider_descriptor(provider) else {
        return String::new();
    };
    if let Some(env_name) = descriptor
        .api_key_env_names
        .iter()
        .find(|env_name| env_var_has_non_empty_value(env_name))
    {
        return (*env_name).to_owned();
    }

    descriptor
        .default_api_key_env
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn summarize_query_search_credential(
    config: &mvp::config::LoongConfig,
    provider: &str,
) -> Option<OnboardingCredentialSummary> {
    let descriptor = mvp::config::web_search_provider_descriptor(provider)?;
    if !descriptor.requires_api_key {
        return Some(OnboardingCredentialSummary {
            label: crate::access_terms::QUERY_SEARCH_CREDENTIAL_LABEL,
            value: "not required".to_owned(),
        });
    }

    if let Some(configured_value) = configured_query_search_secret(config, descriptor.id) {
        let trimmed = configured_value.trim();
        if !trimmed.is_empty() {
            let secret_ref = SecretRef::Inline(trimmed.to_owned());
            if let Some(env_name) = secret_ref.explicit_env_name() {
                let suffix = if env_var_has_non_empty_value(env_name.as_str()) {
                    ""
                } else {
                    " (missing in env)"
                };
                return Some(OnboardingCredentialSummary {
                    label: crate::access_terms::QUERY_SEARCH_CREDENTIAL_SOURCE_LABEL,
                    value: format!("{env_name}{suffix}"),
                });
            }
            if secret_ref.inline_literal_value().is_some() {
                return Some(OnboardingCredentialSummary {
                    label: crate::access_terms::QUERY_SEARCH_CREDENTIAL_LABEL,
                    value: "inline api key".to_owned(),
                });
            }
        }
    }

    if let Some(env_name) = descriptor
        .api_key_env_names
        .iter()
        .find(|env_name| env_var_has_non_empty_value(env_name))
    {
        return Some(OnboardingCredentialSummary {
            label: crate::access_terms::QUERY_SEARCH_CREDENTIAL_SOURCE_LABEL,
            value: (*env_name).to_owned(),
        });
    }

    descriptor
        .default_api_key_env
        .map(|env_name| OnboardingCredentialSummary {
            label: crate::access_terms::QUERY_SEARCH_CREDENTIAL_SOURCE_LABEL,
            value: format!("{env_name} (expected)"),
        })
}

pub(crate) fn query_search_has_available_credential(
    config: &mvp::config::LoongConfig,
    provider: &str,
) -> bool {
    let Some(descriptor) = mvp::config::web_search_provider_descriptor(provider) else {
        return false;
    };
    if !descriptor.requires_api_key {
        return true;
    }

    if let Some(configured_value) = configured_query_search_secret(config, descriptor.id) {
        let trimmed = configured_value.trim();
        if !trimmed.is_empty() {
            let secret_ref = SecretRef::Inline(trimmed.to_owned());
            if let Some(env_name) = secret_ref.explicit_env_name() {
                return env_var_has_non_empty_value(env_name.as_str());
            }
            if secret_ref.inline_literal_value().is_some() {
                return true;
            }
        }
    }

    descriptor
        .api_key_env_names
        .iter()
        .any(|env_name| env_var_has_non_empty_value(env_name))
}

pub(crate) fn configured_query_search_secret<'a>(
    config: &'a mvp::config::LoongConfig,
    provider: &str,
) -> Option<&'a str> {
    config
        .tools
        .web_search
        .configured_api_key_for_provider(provider)
}

pub(crate) fn query_search_repair_steps(
    config: &mvp::config::LoongConfig,
    rerun_onboard_command: &str,
) -> Vec<String> {
    let provider = query_search_provider_status(config).provider;
    let default_env_name = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| descriptor.default_api_key_env);

    let mut steps = Vec::new();
    if let Some(default_env_name) = default_env_name {
        steps.push(crate::access_terms::set_query_search_credential_step(
            default_env_name,
        ));
    }
    steps
        .push(crate::access_terms::review_query_search_provider_choice_step(rerun_onboard_command));
    steps
}

fn render_query_search_credential_source_value(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }

    let secret_ref = SecretRef::Inline(trimmed.to_owned());
    if let Some(env_name) = secret_ref.explicit_env_name() {
        return Some(env_name);
    }
    if secret_ref.inline_literal_value().is_some() {
        return Some("inline api key".to_owned());
    }

    Some("configured credential".to_owned())
}

fn env_var_has_non_empty_value(env_name: &str) -> bool {
    env::var(env_name)
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;

    #[test]
    fn summarize_query_search_credential_marks_keyless_provider_as_not_required() {
        let summary = summarize_query_search_credential(
            &mvp::config::LoongConfig::default(),
            mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO,
        )
        .expect("duckduckgo summary");

        assert_eq!(
            summary.label,
            crate::access_terms::QUERY_SEARCH_CREDENTIAL_LABEL
        );
        assert_eq!(summary.value, "not required");
    }

    #[test]
    fn query_search_provider_status_renders_blocked_detail_variants() {
        let mut env = ScopedEnv::new();
        env.remove("FIRECRAWL_API_KEY");

        let mut config = mvp::config::LoongConfig::default();
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();
        config.tools.web_search.firecrawl_api_key = Some("${FIRECRAWL_API_KEY}".to_owned());

        let status = query_search_provider_status(&config);

        assert_eq!(status.provider, mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL);
        assert_eq!(
            status.blocked_detail(false),
            "Firecrawl Search: FIRECRAWL_API_KEY (missing in env). web.search will stay unavailable until the provider credential is supplied"
        );
        assert_eq!(
            status.blocked_detail(true),
            "Firecrawl Search: FIRECRAWL_API_KEY (missing in env). web.search will stay unavailable until the provider credential is supplied, but ordinary network access remains separately governed"
        );
    }

    #[test]
    fn query_search_repair_steps_surface_env_and_onboard_handoff() {
        let mut config = mvp::config::LoongConfig::default();
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();

        let steps = query_search_repair_steps(&config, "loong onboard --config /tmp/loong.toml");

        assert_eq!(
            steps,
            vec![
                crate::access_terms::set_query_search_credential_step("FIRECRAWL_API_KEY"),
                crate::access_terms::review_query_search_provider_choice_step(
                    "loong onboard --config /tmp/loong.toml",
                ),
            ]
        );
    }

    #[test]
    fn configured_query_search_secret_reads_firecrawl_field() {
        let mut config = mvp::config::LoongConfig::default();
        config.tools.web_search.firecrawl_api_key = Some("${FIRECRAWL_API_KEY}".to_owned());

        let configured_secret =
            configured_query_search_secret(&config, mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL);

        assert_eq!(configured_secret, Some("${FIRECRAWL_API_KEY}"));
    }

    #[test]
    fn query_search_has_available_credential_accepts_env_backed_provider() {
        let mut env = ScopedEnv::new();
        env.set("TAVILY_API_KEY", "tavily-test-token");

        let mut config = mvp::config::LoongConfig::default();
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
        config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());

        assert!(query_search_has_available_credential(
            &config,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY
        ));
    }
}
