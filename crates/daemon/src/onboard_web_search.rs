use std::env;

use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;
use loongclaw_spec::CliResult;

use crate::onboard_cli::OnboardCommandOptions;
use crate::onboard_finalize::OnboardingCredentialSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebSearchProviderRecommendation {
    pub(crate) provider: &'static str,
    pub(crate) reason: String,
    pub(crate) source: WebSearchProviderRecommendationSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct WebSearchEnvironmentSignals {
    pub(crate) domestic_locale_hint: bool,
    pub(crate) duckduckgo_reachable: bool,
    pub(crate) tavily_reachable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebSearchProviderRecommendationSource {
    ExplicitCli,
    ExplicitEnv,
    Configured,
    DetectedCredential,
    DetectedSignals,
}

pub(crate) async fn resolve_web_search_provider_recommendation(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<WebSearchProviderRecommendation> {
    if let Some(explicit_recommendation) = explicit_web_search_provider_override(options)? {
        return Ok(explicit_recommendation);
    }

    let user_configured = onboard_web_search_is_user_configured(config);
    if user_configured
        && let Some(configured_provider) = mvp::config::normalize_web_search_provider(
            config.tools.web_search.default_provider.as_str(),
        )
    {
        return Ok(WebSearchProviderRecommendation {
            provider: configured_provider,
            reason: "reusing the configured web search provider from the current starting point"
                .to_owned(),
            source: WebSearchProviderRecommendationSource::Configured,
        });
    }

    let credential_recommendation =
        recommend_web_search_provider_from_available_credentials(config);
    if let Some(recommendation) = credential_recommendation {
        return Ok(recommendation);
    }

    let signals = detect_web_search_environment_signals().await;
    let recommendation = recommend_web_search_provider_from_signals(signals);
    Ok(recommendation)
}

pub(crate) fn resolve_effective_web_search_default_provider(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    recommendation: &WebSearchProviderRecommendation,
) -> &'static str {
    if !options.non_interactive {
        return recommendation.provider;
    }

    match recommendation.source {
        WebSearchProviderRecommendationSource::ExplicitCli => {
            return recommendation.provider;
        }
        WebSearchProviderRecommendationSource::ExplicitEnv => {
            return recommendation.provider;
        }
        WebSearchProviderRecommendationSource::Configured => {
            return recommendation.provider;
        }
        WebSearchProviderRecommendationSource::DetectedCredential => {
            return recommendation.provider;
        }
        WebSearchProviderRecommendationSource::DetectedSignals => {}
    }

    let descriptor = mvp::config::web_search_provider_descriptor(recommendation.provider);
    let Some(descriptor) = descriptor else {
        return mvp::config::DEFAULT_WEB_SEARCH_PROVIDER;
    };
    if !descriptor.requires_api_key {
        return descriptor.id;
    }

    let has_available_credential =
        web_search_provider_has_available_credential(config, descriptor.id);
    if has_available_credential {
        return descriptor.id;
    }

    mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO
}

fn onboard_web_search_is_user_configured(config: &mvp::config::LoongClawConfig) -> bool {
    config.tools.web_search != mvp::config::WebSearchToolConfig::default()
}

pub(crate) fn explicit_web_search_provider_override(
    options: &OnboardCommandOptions,
) -> CliResult<Option<WebSearchProviderRecommendation>> {
    if let Some(raw_provider) = options.web_search_provider.as_deref() {
        let trimmed_provider = raw_provider.trim();
        if trimmed_provider.is_empty() {
            return Ok(None);
        }

        let normalized_provider =
            normalize_selected_web_search_provider("web-search-provider", trimmed_provider)?;
        let reason = "set by --web-search-provider".to_owned();
        let source = WebSearchProviderRecommendationSource::ExplicitCli;
        let recommendation = WebSearchProviderRecommendation {
            provider: normalized_provider,
            reason,
            source,
        };
        return Ok(Some(recommendation));
    }

    let raw_provider = match env::var("LOONGCLAW_WEB_SEARCH_PROVIDER") {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let trimmed_provider = raw_provider.trim();
    if trimmed_provider.is_empty() {
        return Ok(None);
    }

    let normalized_provider =
        normalize_selected_web_search_provider("LOONGCLAW_WEB_SEARCH_PROVIDER", trimmed_provider)?;
    let reason = "set by LOONGCLAW_WEB_SEARCH_PROVIDER".to_owned();
    let source = WebSearchProviderRecommendationSource::ExplicitEnv;
    let recommendation = WebSearchProviderRecommendation {
        provider: normalized_provider,
        reason,
        source,
    };
    Ok(Some(recommendation))
}

fn normalize_selected_web_search_provider(
    source_name: &str,
    raw_provider: &str,
) -> CliResult<&'static str> {
    let normalized_provider = mvp::config::normalize_web_search_provider(raw_provider);
    if let Some(provider) = normalized_provider {
        return Ok(provider);
    }

    Err(format!(
        "unsupported {source_name} value \"{raw_provider}\". supported: {}",
        mvp::config::WEB_SEARCH_PROVIDER_VALID_VALUES
    ))
}

pub(crate) fn recommend_web_search_provider_from_available_credentials(
    config: &mvp::config::LoongClawConfig,
) -> Option<WebSearchProviderRecommendation> {
    let mut ready_providers = mvp::config::web_search_provider_descriptors()
        .iter()
        .filter(|descriptor| descriptor.requires_api_key)
        .filter(|descriptor| web_search_provider_has_available_credential(config, descriptor.id));
    let descriptor = ready_providers.next()?;
    if ready_providers.next().is_some() {
        return None;
    }

    let credential_summary = summarize_web_search_provider_credential(config, descriptor.id);
    let reason = if let Some(summary) = credential_summary {
        format!(
            "found exactly one ready web search credential for {} ({})",
            descriptor.display_name, summary.value
        )
    } else {
        format!(
            "found exactly one ready web search provider with credentials: {}",
            descriptor.display_name
        )
    };
    Some(WebSearchProviderRecommendation {
        provider: descriptor.id,
        reason,
        source: WebSearchProviderRecommendationSource::DetectedCredential,
    })
}

pub(crate) fn recommend_web_search_provider_from_signals(
    signals: WebSearchEnvironmentSignals,
) -> WebSearchProviderRecommendation {
    let detected_domestic = signals.domestic_locale_hint;
    let duckduckgo_reachable = signals.duckduckgo_reachable;
    let tavily_reachable = signals.tavily_reachable;

    if detected_domestic && (tavily_reachable || !duckduckgo_reachable) {
        let reason = if tavily_reachable {
            "domestic locale or timezone was detected and Tavily looked reachable from this host"
                .to_owned()
        } else {
            "domestic locale or timezone was detected and DuckDuckGo did not look reachable from this host"
                .to_owned()
        };
        return WebSearchProviderRecommendation {
            provider: mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            reason,
            source: WebSearchProviderRecommendationSource::DetectedSignals,
        };
    }

    if duckduckgo_reachable {
        let reason =
            "DuckDuckGo looked reachable from this host, so the key-free fallback stays the default"
                .to_owned();
        return WebSearchProviderRecommendation {
            provider: mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO,
            reason,
            source: WebSearchProviderRecommendationSource::DetectedSignals,
        };
    }

    if tavily_reachable {
        let reason =
            "DuckDuckGo did not look reachable, but Tavily's API route responded from this host"
                .to_owned();
        return WebSearchProviderRecommendation {
            provider: mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            reason,
            source: WebSearchProviderRecommendationSource::DetectedSignals,
        };
    }

    if detected_domestic {
        let reason =
            "domestic locale or timezone was detected, so Tavily is the safer API-first recommendation"
                .to_owned();
        return WebSearchProviderRecommendation {
            provider: mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            reason,
            source: WebSearchProviderRecommendationSource::DetectedSignals,
        };
    }

    let reason = "falling back to DuckDuckGo as the key-free default".to_owned();
    WebSearchProviderRecommendation {
        provider: mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO,
        reason,
        source: WebSearchProviderRecommendationSource::DetectedSignals,
    }
}

async fn detect_web_search_environment_signals() -> WebSearchEnvironmentSignals {
    let domestic_locale_hint = onboarding_locale_looks_domestic_cn();
    let duckduckgo_reachable = probe_duckduckgo_route().await;
    let tavily_reachable = probe_tavily_route().await;
    WebSearchEnvironmentSignals {
        domestic_locale_hint,
        duckduckgo_reachable,
        tavily_reachable,
    }
}

fn onboarding_locale_looks_domestic_cn() -> bool {
    let locale_matches = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|key| env::var(key).ok())
        .any(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            normalized.contains("zh_cn")
                || normalized.contains("zh-hans")
                || normalized.starts_with("zh-cn")
        });
    if locale_matches {
        return true;
    }

    let timezone = env::var("TZ").ok();
    let Some(timezone) = timezone else {
        return false;
    };
    let normalized = timezone.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "asia/shanghai" | "asia/chongqing" | "asia/harbin" | "asia/urumqi" | "asia/beijing"
    )
}

async fn probe_duckduckgo_route() -> bool {
    let client = build_onboard_probe_client();
    let request = client.get("https://html.duckduckgo.com/html/?q=loongclaw");
    let response = request.send().await;
    match response {
        Ok(response) => response.status().is_success() || response.status().is_redirection(),
        Err(_) => false,
    }
}

async fn probe_tavily_route() -> bool {
    let client = build_onboard_probe_client();
    let request = client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .body(r#"{"query":"loongclaw","max_results":1}"#);
    let response = request.send().await;
    match response {
        Ok(response) => {
            let status = response.status();
            status.is_success() || status.is_redirection() || status.is_client_error()
        }
        Err(_) => false,
    }
}

fn build_onboard_probe_client() -> reqwest::Client {
    let client = mvp::tools::build_ssrf_safe_client(false, 2, "LoongClaw-Onboard/0.1");
    client.unwrap_or_else(|_| reqwest::Client::new())
}

pub(crate) fn web_search_provider_display_name(provider: &str) -> String {
    let descriptor = mvp::config::web_search_provider_descriptor(provider);
    descriptor
        .map(|descriptor| descriptor.display_name.to_owned())
        .unwrap_or_else(|| provider.to_owned())
}

fn render_web_search_credential_source_value(raw: Option<&str>) -> Option<String> {
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

pub(crate) fn configured_web_search_provider_credential_source_value(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
) -> Option<String> {
    let configured_secret = configured_web_search_provider_secret(config, provider);
    configured_secret.and_then(|value| render_web_search_credential_source_value(Some(value)))
}

pub(crate) fn configured_web_search_provider_env_name(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
) -> Option<String> {
    let raw = configured_web_search_provider_secret(config, provider)?;
    let secret_ref = SecretRef::Inline(raw.trim().to_owned());
    secret_ref.explicit_env_name()
}

pub(crate) fn web_search_provider_has_inline_credential(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
) -> bool {
    let configured_secret = configured_web_search_provider_secret(config, provider);
    configured_secret.is_some_and(|value| {
        let trimmed = value.trim().to_owned();
        let secret_ref = SecretRef::Inline(trimmed);
        secret_ref.inline_literal_value().is_some()
    })
}

pub(crate) fn preferred_web_search_credential_env_default(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
) -> String {
    if let Some(env_name) = configured_web_search_provider_env_name(config, provider) {
        return env_name;
    }
    if web_search_provider_has_inline_credential(config, provider) {
        return String::new();
    }

    let descriptor = mvp::config::web_search_provider_descriptor(provider);
    let Some(descriptor) = descriptor else {
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

pub(crate) fn summarize_web_search_provider_credential(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
) -> Option<OnboardingCredentialSummary> {
    let descriptor = mvp::config::web_search_provider_descriptor(provider)?;
    if !descriptor.requires_api_key {
        return Some(OnboardingCredentialSummary {
            label: "web search credential",
            value: "not required".to_owned(),
        });
    }

    if let Some(configured_value) = configured_web_search_provider_secret(config, descriptor.id) {
        let trimmed = configured_value.trim();
        if !trimmed.is_empty() {
            let secret_ref = SecretRef::Inline(trimmed.to_owned());
            if let Some(env_name) = secret_ref.explicit_env_name() {
                let env_present = env_var_has_non_empty_value(env_name.as_str());
                let suffix = if env_present { "" } else { " (missing in env)" };
                return Some(OnboardingCredentialSummary {
                    label: "web search credential source",
                    value: format!("{env_name}{suffix}"),
                });
            }
            if secret_ref.inline_literal_value().is_some() {
                return Some(OnboardingCredentialSummary {
                    label: "web search credential",
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
            label: "web search credential source",
            value: (*env_name).to_owned(),
        });
    }

    descriptor
        .default_api_key_env
        .map(|env_name| OnboardingCredentialSummary {
            label: "web search credential source",
            value: format!("{env_name} (expected)"),
        })
}

pub(crate) fn web_search_provider_has_available_credential(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
) -> bool {
    let descriptor = mvp::config::web_search_provider_descriptor(provider);
    let Some(descriptor) = descriptor else {
        return false;
    };
    if !descriptor.requires_api_key {
        return true;
    }

    if let Some(configured_value) = configured_web_search_provider_secret(config, descriptor.id) {
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

pub(crate) fn configured_web_search_provider_secret<'a>(
    config: &'a mvp::config::LoongClawConfig,
    provider: &str,
) -> Option<&'a str> {
    match provider {
        mvp::config::WEB_SEARCH_PROVIDER_BRAVE => config.tools.web_search.brave_api_key.as_deref(),
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY => {
            config.tools.web_search.tavily_api_key.as_deref()
        }
        mvp::config::WEB_SEARCH_PROVIDER_PERPLEXITY => {
            config.tools.web_search.perplexity_api_key.as_deref()
        }
        mvp::config::WEB_SEARCH_PROVIDER_EXA => config.tools.web_search.exa_api_key.as_deref(),
        mvp::config::WEB_SEARCH_PROVIDER_JINA => config.tools.web_search.jina_api_key.as_deref(),
        _ => None,
    }
}

fn env_var_has_non_empty_value(env_name: &str) -> bool {
    env::var(env_name)
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}
