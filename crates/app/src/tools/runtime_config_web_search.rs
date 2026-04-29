use crate::config::LoongConfig;
use crate::secrets::{SecretLookup, resolve_secret_lookup};
use loong_contracts::SecretRef;

use super::{parse_env_bool, parse_env_string, parse_env_u64, parse_env_usize};

// Query-style web search policy for `web { query }` / `web.search` only. Keep
// this separate from normal network egress so missing web-search credentials do
// not imply that plain fetch/request or browser access is unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchRuntimePolicy {
    pub enabled: bool,
    pub default_provider: String,
    pub brave_api_key: Option<String>,
    pub tavily_api_key: Option<String>,
    pub perplexity_api_key: Option<String>,
    pub exa_api_key: Option<String>,
    pub firecrawl_api_key: Option<String>,
    pub jina_api_key: Option<String>,
    pub timeout_seconds: u64,
    pub max_results: usize,
}

impl Default for WebSearchRuntimePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            default_provider: crate::config::DEFAULT_WEB_SEARCH_PROVIDER.to_owned(),
            brave_api_key: None,
            tavily_api_key: None,
            perplexity_api_key: None,
            exa_api_key: None,
            firecrawl_api_key: None,
            jina_api_key: None,
            timeout_seconds: crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS,
            max_results: crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS,
        }
    }
}

impl WebSearchRuntimePolicy {
    pub fn configured_api_key_for_provider(&self, provider: &str) -> Option<&str> {
        let normalized_provider =
            crate::config::normalize_web_search_provider(provider).unwrap_or(provider);

        match normalized_provider {
            crate::config::WEB_SEARCH_PROVIDER_BRAVE => self.brave_api_key.as_deref(),
            crate::config::WEB_SEARCH_PROVIDER_TAVILY => self.tavily_api_key.as_deref(),
            crate::config::WEB_SEARCH_PROVIDER_PERPLEXITY => self.perplexity_api_key.as_deref(),
            crate::config::WEB_SEARCH_PROVIDER_EXA => self.exa_api_key.as_deref(),
            crate::config::WEB_SEARCH_PROVIDER_FIRECRAWL => self.firecrawl_api_key.as_deref(),
            crate::config::WEB_SEARCH_PROVIDER_JINA => self.jina_api_key.as_deref(),
            _ => None,
        }
    }

    pub fn from_loong_config(config: &LoongConfig) -> Self {
        let mut policy = Self {
            enabled: config.tools.web_search.enabled,
            default_provider: crate::config::normalize_web_search_provider(
                config.tools.web_search.default_provider.as_str(),
            )
            .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_PROVIDER)
            .to_owned(),
            brave_api_key: None,
            tavily_api_key: None,
            perplexity_api_key: None,
            exa_api_key: None,
            firecrawl_api_key: None,
            jina_api_key: None,
            timeout_seconds: config.tools.web_search.timeout_seconds,
            max_results: config.tools.web_search.max_results,
        };

        populate_provider_api_keys(
            &mut policy,
            crate::config::web_search_provider_descriptors()
                .iter()
                .filter(|descriptor| descriptor.requires_api_key)
                .map(|descriptor| {
                    (
                        descriptor.id,
                        resolve_web_search_secret_binding(
                            config
                                .tools
                                .web_search
                                .configured_api_key_for_provider(descriptor.id),
                            descriptor.api_key_env_names,
                        ),
                    )
                }),
        );

        policy
    }

    pub fn from_env() -> Self {
        let default_provider = parse_env_string("LOONG_WEB_SEARCH_PROVIDER")
            .as_deref()
            .and_then(crate::config::normalize_web_search_provider)
            .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_PROVIDER)
            .to_owned();

        let mut policy = Self {
            enabled: parse_env_bool("LOONG_WEB_SEARCH_ENABLED").unwrap_or(true),
            default_provider,
            brave_api_key: None,
            tavily_api_key: None,
            perplexity_api_key: None,
            exa_api_key: None,
            firecrawl_api_key: None,
            jina_api_key: None,
            timeout_seconds: parse_env_u64("LOONG_WEB_SEARCH_TIMEOUT_SECONDS")
                .map(|seconds| seconds.clamp(1, 60))
                .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS),
            max_results: parse_env_usize("LOONG_WEB_SEARCH_MAX_RESULTS")
                .map(|count| count.clamp(1, 10))
                .unwrap_or(crate::config::DEFAULT_WEB_SEARCH_MAX_RESULTS),
        };

        populate_provider_api_keys(
            &mut policy,
            crate::config::web_search_provider_descriptors()
                .iter()
                .filter(|descriptor| descriptor.requires_api_key)
                .map(|descriptor| {
                    (
                        descriptor.id,
                        resolve_web_search_secret_binding(None, descriptor.api_key_env_names),
                    )
                }),
        );

        policy
    }

    fn set_configured_api_key_for_provider(
        &mut self,
        provider: &str,
        value: Option<String>,
    ) -> bool {
        let normalized_provider =
            crate::config::normalize_web_search_provider(provider).unwrap_or(provider);

        let configured_api_key_slot = match normalized_provider {
            crate::config::WEB_SEARCH_PROVIDER_BRAVE => &mut self.brave_api_key,
            crate::config::WEB_SEARCH_PROVIDER_TAVILY => &mut self.tavily_api_key,
            crate::config::WEB_SEARCH_PROVIDER_PERPLEXITY => &mut self.perplexity_api_key,
            crate::config::WEB_SEARCH_PROVIDER_EXA => &mut self.exa_api_key,
            crate::config::WEB_SEARCH_PROVIDER_FIRECRAWL => &mut self.firecrawl_api_key,
            crate::config::WEB_SEARCH_PROVIDER_JINA => &mut self.jina_api_key,
            _ => return false,
        };

        *configured_api_key_slot = value;
        true
    }
}

fn populate_provider_api_keys(
    policy: &mut WebSearchRuntimePolicy,
    bindings: impl IntoIterator<Item = (&'static str, Option<String>)>,
) {
    for (provider, value) in bindings {
        let _ = policy.set_configured_api_key_for_provider(provider, value);
    }
}

fn resolve_web_search_secret_binding(
    configured_value: Option<&str>,
    env_names: &[&str],
) -> Option<String> {
    if let Some(secret_ref) = configured_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| SecretRef::Inline(value.to_owned()))
    {
        match resolve_secret_lookup(Some(&secret_ref)) {
            SecretLookup::Value(value) => return Some(value),
            SecretLookup::Missing => return None,
            SecretLookup::Absent => {}
        }
    }

    env_names
        .iter()
        .find_map(|env_name| std::env::var(env_name).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;

    #[test]
    fn from_loong_config_keeps_keyless_duckduckgo_defaults() {
        let mut env = ScopedEnv::new();
        for descriptor in crate::config::web_search_provider_descriptors() {
            for env_name in descriptor.api_key_env_names {
                env.remove(env_name);
            }
        }

        let config = LoongConfig::default();

        let policy = WebSearchRuntimePolicy::from_loong_config(&config);

        assert!(policy.enabled);
        assert_eq!(
            policy.default_provider,
            crate::config::DEFAULT_WEB_SEARCH_PROVIDER
        );
        assert!(policy.brave_api_key.is_none());
        assert!(policy.tavily_api_key.is_none());
    }

    #[test]
    fn configured_api_key_for_provider_normalizes_aliases() {
        let policy = WebSearchRuntimePolicy {
            perplexity_api_key: Some("perplexity-token".to_owned()),
            jina_api_key: Some("jina-token".to_owned()),
            ..WebSearchRuntimePolicy::default()
        };

        assert_eq!(
            policy.configured_api_key_for_provider("perplexity_search"),
            Some("perplexity-token")
        );
        assert_eq!(
            policy.configured_api_key_for_provider("jina-ai"),
            Some("jina-token")
        );
        assert_eq!(policy.configured_api_key_for_provider("ddg"), None);
    }
}
