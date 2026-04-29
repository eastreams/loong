use crate::mvp;

pub(crate) const RUNTIME_TOOL_ACCESS_SEPARATION_NOTE: &str = "web-search provider settings affect only query search mode; ordinary network access and browser lanes stay separately governed";
pub(crate) const QUERY_SEARCH_SOURCE_EXTERNAL_PROVIDER: &str = "external_provider";
pub(crate) const QUERY_SEARCH_SOURCE_PROVIDER_NATIVE: &str = "provider_native";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeToolAccessSummary {
    pub ordinary_network_access_enabled: bool,
    pub query_search_enabled: bool,
    pub query_search_default_provider: String,
    pub query_search_source: &'static str,
    pub query_search_provider_label: String,
    pub query_search_credential_ready: bool,
    pub browser_page_access_enabled: bool,
    pub managed_browser_session_enabled: bool,
    pub managed_browser_session_ready: bool,
    pub consent_mode: &'static str,
    pub approval_mode: &'static str,
    pub separation_note: &'static str,
}

pub(crate) fn runtime_tool_access_summary(
    config: &mvp::config::LoongConfig,
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> RuntimeToolAccessSummary {
    let ordinary_network_access_enabled = runtime.web_fetch.enabled;
    let query_search_enabled = runtime.web_search.enabled;
    let query_search_default_provider = runtime.web_search.default_provider.clone();
    let native_query_search_label = mvp::provider::native_query_search_label(config);
    let query_search_source = if native_query_search_label.is_some() {
        QUERY_SEARCH_SOURCE_PROVIDER_NATIVE
    } else {
        QUERY_SEARCH_SOURCE_EXTERNAL_PROVIDER
    };
    let query_search_provider_label = native_query_search_label.unwrap_or_else(|| {
        mvp::config::web_search_provider_descriptor(query_search_default_provider.as_str())
            .map(|descriptor| descriptor.display_name.to_owned())
            .unwrap_or_else(|| query_search_default_provider.clone())
    });
    let query_search_credential_ready = query_search_source == QUERY_SEARCH_SOURCE_PROVIDER_NATIVE
        || web_search_provider_credential_ready(&runtime.web_search);
    let browser_page_access_enabled = runtime.browser.enabled;
    let managed_browser_session_enabled = runtime.browser_companion.enabled;
    let managed_browser_session_ready =
        runtime.browser_companion.enabled && runtime.browser_companion.ready;
    let consent_mode = config.tools.consent.default_mode.as_str();
    let approval_mode = render_tool_approval_mode(config.tools.approval.mode);

    RuntimeToolAccessSummary {
        ordinary_network_access_enabled,
        query_search_enabled,
        query_search_default_provider,
        query_search_source,
        query_search_provider_label,
        query_search_credential_ready,
        browser_page_access_enabled,
        managed_browser_session_enabled,
        managed_browser_session_ready,
        consent_mode,
        approval_mode,
        separation_note: RUNTIME_TOOL_ACCESS_SEPARATION_NOTE,
    }
}

const fn render_tool_approval_mode(mode: mvp::config::GovernedToolApprovalMode) -> &'static str {
    match mode {
        mvp::config::GovernedToolApprovalMode::Disabled => "disabled",
        mvp::config::GovernedToolApprovalMode::MediumBalanced => "medium_balanced",
        mvp::config::GovernedToolApprovalMode::Strict => "strict",
    }
}

fn web_search_provider_credential_ready(
    policy: &mvp::tools::runtime_config::WebSearchRuntimePolicy,
) -> bool {
    let provider = policy.default_provider.trim();
    match provider {
        mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO => true,
        mvp::config::WEB_SEARCH_PROVIDER_BRAVE => {
            option_has_non_empty_runtime_text(policy.brave_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY => {
            option_has_non_empty_runtime_text(policy.tavily_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_PERPLEXITY => {
            option_has_non_empty_runtime_text(policy.perplexity_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_EXA => {
            option_has_non_empty_runtime_text(policy.exa_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL => {
            option_has_non_empty_runtime_text(policy.firecrawl_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_JINA => {
            option_has_non_empty_runtime_text(policy.jina_api_key.as_deref())
        }
        _ => false,
    }
}

fn option_has_non_empty_runtime_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}
