use crate::mvp;

pub(crate) const RUNTIME_TOOL_ACCESS_SEPARATION_NOTE: &str = "web-search provider settings affect only query search mode; ordinary network access and browser lanes stay separately governed";

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
    let query_search_status =
        crate::query_search_surface::query_search_provider_status_with_runtime_policy(
            config,
            &runtime.web_search,
        );
    let browser_page_access_enabled = runtime.browser.enabled;
    let managed_browser_session_enabled = false;
    let managed_browser_session_ready = false;
    let consent_mode = config.tools.consent.default_mode.as_str();
    let approval_mode = render_tool_approval_mode(config.tools.approval.mode);

    RuntimeToolAccessSummary {
        ordinary_network_access_enabled,
        query_search_enabled: runtime.web_search.enabled,
        query_search_default_provider: runtime.web_search.default_provider.clone(),
        query_search_source: query_search_status.source,
        query_search_provider_label: query_search_status.provider_label,
        query_search_credential_ready: query_search_status.credential_available,
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
