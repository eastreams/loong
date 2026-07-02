use crate::gateway::read_models::GatewayToolAccessReadModel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusAccessPresentation {
    pub ordinary_network_detail: String,
    pub query_search_detail: String,
    pub browser_page_detail: String,
    pub managed_browser_detail: String,
    pub governance_detail: String,
    pub boundary_note: String,
}

pub(crate) fn build_status_access_presentation(
    access: &GatewayToolAccessReadModel,
) -> StatusAccessPresentation {
    StatusAccessPresentation {
        ordinary_network_detail: format!("enabled={}", access.ordinary_network_access_enabled),
        query_search_detail: format!(
            "enabled={} · source={} · provider={} · credential_ready={}",
            access.query_search_enabled,
            access.query_search_source,
            access.query_search_provider_label,
            access.query_search_credential_ready,
        ),
        browser_page_detail: format!("enabled={}", access.browser_page_access_enabled),
        managed_browser_detail: format!(
            "enabled={} · ready={}",
            access.managed_browser_session_enabled, access.managed_browser_session_ready
        ),
        governance_detail: format!(
            "consent_mode={} · approval_mode={}",
            access.consent_mode, access.approval_mode
        ),
        boundary_note: access.separation_note.clone(),
    }
}

pub(crate) fn query_search_is_ready(access: &GatewayToolAccessReadModel) -> bool {
    !access.query_search_enabled || access.query_search_credential_ready
}

pub(crate) fn ordinary_network_is_ready(access: &GatewayToolAccessReadModel) -> bool {
    access.ordinary_network_access_enabled
}

pub(crate) fn browser_page_is_ready(access: &GatewayToolAccessReadModel) -> bool {
    access.browser_page_access_enabled
}

pub(crate) fn managed_browser_is_ready(access: &GatewayToolAccessReadModel) -> bool {
    !access.managed_browser_session_enabled || access.managed_browser_session_ready
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_access() -> GatewayToolAccessReadModel {
        GatewayToolAccessReadModel {
            ordinary_network_access_enabled: true,
            query_search_enabled: true,
            query_search_default_provider: "duckduckgo".to_owned(),
            query_search_source: "external_provider".to_owned(),
            query_search_provider_label: "DuckDuckGo".to_owned(),
            query_search_credential_ready: false,
            browser_page_access_enabled: true,
            managed_browser_session_enabled: true,
            managed_browser_session_ready: false,
            consent_mode: "full".to_owned(),
            approval_mode: "disabled".to_owned(),
            separation_note: "query search is separate from ordinary network and browser lanes"
                .to_owned(),
        }
    }

    #[test]
    fn build_status_access_presentation_renders_canonical_details() {
        let presentation = build_status_access_presentation(&sample_access());

        assert_eq!(presentation.ordinary_network_detail, "enabled=true");
        assert_eq!(
            presentation.query_search_detail,
            "enabled=true · source=external_provider · provider=DuckDuckGo · credential_ready=false"
        );
        assert_eq!(presentation.browser_page_detail, "enabled=true");
        assert_eq!(
            presentation.managed_browser_detail,
            "enabled=true · ready=false"
        );
        assert_eq!(
            presentation.governance_detail,
            "consent_mode=full · approval_mode=disabled"
        );
        assert_eq!(
            presentation.boundary_note,
            "query search is separate from ordinary network and browser lanes"
        );
    }

    #[test]
    fn access_readiness_helpers_match_runtime_expectations() {
        let access = sample_access();

        assert!(ordinary_network_is_ready(&access));
        assert!(!query_search_is_ready(&access));
        assert!(browser_page_is_ready(&access));
        assert!(!managed_browser_is_ready(&access));
    }
}
