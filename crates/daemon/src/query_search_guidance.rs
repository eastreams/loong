pub(crate) use crate::query_search_surface::{
    configured_query_search_credential_env_name, configured_query_search_credential_source_value,
    configured_query_search_secret, preferred_query_search_credential_env_default,
    query_search_has_available_credential, query_search_has_inline_credential,
    query_search_provider_display_name, query_search_provider_status, query_search_repair_steps,
    summarize_query_search_credential,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;
    use loong_app as mvp;

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
    fn query_search_provider_status_accepts_openai_native_search_without_external_credential() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        config.provider.wire_api = mvp::config::ProviderWireApi::Responses;
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
        config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());

        let status = query_search_provider_status(&config);

        assert!(status.provider_native);
        assert!(status.credential_available);
        assert_eq!(status.provider_label, "OpenAI Responses native web search");
        assert_eq!(status.ready_detail(), "OpenAI Responses native web search");
        assert!(
            query_search_repair_steps(&config, "loong onboard --config /tmp/loong.toml").is_empty()
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
