use loongclaw_app as mvp;

pub(crate) const MODEL_CATALOG_PROBE_FAILED_MARKER: &str = "model catalog probe failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderModelProbeFailureLevel {
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderModelProbeFailureKind {
    TransportFailure,
    ExplicitModel {
        model: String,
    },
    PreferredModels {
        fallback_models: Vec<String>,
    },
    RequiresExplicitModel {
        recommended_onboarding_model: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderModelProbeFailure {
    pub(crate) level: ProviderModelProbeFailureLevel,
    pub(crate) detail: String,
    pub(crate) kind: ProviderModelProbeFailureKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderModelProbeRecoveryAdvice {
    pub(crate) kind: ProviderModelProbeFailureKind,
    pub(crate) region_endpoint_hint: Option<String>,
}

pub(crate) fn provider_model_probe_failure(
    config: &mvp::config::LoongClawConfig,
    error: &str,
) -> ProviderModelProbeFailure {
    let provider_prefix = crate::provider_presentation::active_provider_detail_label(config);
    let is_transport_failure =
        crate::provider_route_diagnostics::is_transport_style_model_probe_failure(error);
    if is_transport_failure {
        let detail = render_transport_failure_detail(provider_prefix.as_str(), error);
        return ProviderModelProbeFailure {
            level: ProviderModelProbeFailureLevel::Fail,
            detail,
            kind: ProviderModelProbeFailureKind::TransportFailure,
        };
    }

    let configured_recovery = config.provider.model_catalog_probe_recovery();
    let kind = configured_recovery_kind(configured_recovery);
    let detail = render_provider_model_probe_failure_detail(provider_prefix.as_str(), error, &kind);
    let detail = append_region_hint(config, error, detail);
    let level = provider_model_probe_failure_level(&kind);

    ProviderModelProbeFailure {
        level,
        detail,
        kind,
    }
}

pub(crate) fn provider_model_probe_failed_detail(detail: &str) -> bool {
    let has_model_catalog_failure = detail.contains(MODEL_CATALOG_PROBE_FAILED_MARKER);
    let has_transport_failure =
        detail.contains(crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER);

    has_model_catalog_failure || has_transport_failure
}

pub(crate) fn provider_model_probe_transport_failure_detail(detail: &str) -> bool {
    detail.contains(crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER)
}

pub(crate) fn provider_model_probe_auth_failure_detail(detail: &str) -> bool {
    let has_model_catalog_failure = detail.contains(MODEL_CATALOG_PROBE_FAILED_MARKER);
    if !has_model_catalog_failure {
        return false;
    }

    mvp::provider::is_auth_style_failure_message(detail)
}

pub(crate) fn provider_model_probe_recovery_advice(
    config: &mvp::config::LoongClawConfig,
    detail: &str,
) -> Option<ProviderModelProbeRecoveryAdvice> {
    let matches_probe_failure_detail = provider_model_probe_failed_detail(detail);
    if !matches_probe_failure_detail {
        return None;
    }

    let kind = if provider_model_probe_transport_failure_detail(detail) {
        ProviderModelProbeFailureKind::TransportFailure
    } else {
        let configured_recovery = config.provider.model_catalog_probe_recovery();
        configured_recovery_kind(configured_recovery)
    };
    let region_endpoint_hint = provider_model_probe_region_endpoint_hint(config, detail);

    Some(ProviderModelProbeRecoveryAdvice {
        kind,
        region_endpoint_hint,
    })
}

fn configured_recovery_kind(
    configured_recovery: mvp::config::ModelCatalogProbeRecovery,
) -> ProviderModelProbeFailureKind {
    match configured_recovery {
        mvp::config::ModelCatalogProbeRecovery::ExplicitModel(model) => {
            ProviderModelProbeFailureKind::ExplicitModel { model }
        }
        mvp::config::ModelCatalogProbeRecovery::ConfiguredPreferredModels(fallback_models) => {
            ProviderModelProbeFailureKind::PreferredModels { fallback_models }
        }
        mvp::config::ModelCatalogProbeRecovery::RequiresExplicitModel {
            recommended_onboarding_model,
        } => ProviderModelProbeFailureKind::RequiresExplicitModel {
            recommended_onboarding_model: recommended_onboarding_model.map(str::to_owned),
        },
    }
}

fn provider_model_probe_failure_level(
    kind: &ProviderModelProbeFailureKind,
) -> ProviderModelProbeFailureLevel {
    match kind {
        ProviderModelProbeFailureKind::TransportFailure => ProviderModelProbeFailureLevel::Fail,
        ProviderModelProbeFailureKind::ExplicitModel { .. } => ProviderModelProbeFailureLevel::Warn,
        ProviderModelProbeFailureKind::PreferredModels { .. } => {
            ProviderModelProbeFailureLevel::Warn
        }
        ProviderModelProbeFailureKind::RequiresExplicitModel { .. } => {
            ProviderModelProbeFailureLevel::Fail
        }
    }
}

fn render_provider_model_probe_failure_detail(
    provider_prefix: &str,
    error: &str,
    kind: &ProviderModelProbeFailureKind,
) -> String {
    match kind {
        ProviderModelProbeFailureKind::TransportFailure => {
            render_transport_failure_detail(provider_prefix, error)
        }
        ProviderModelProbeFailureKind::ExplicitModel { model } => format!(
            "{provider_prefix}: {MODEL_CATALOG_PROBE_FAILED_MARKER} ({error}); chat may still work because model `{model}` is explicitly configured"
        ),
        ProviderModelProbeFailureKind::PreferredModels { fallback_models } => format!(
            "{provider_prefix}: {MODEL_CATALOG_PROBE_FAILED_MARKER} ({error}); runtime will try configured preferred model fallback(s): {}",
            render_model_candidate_list(fallback_models)
        ),
        ProviderModelProbeFailureKind::RequiresExplicitModel {
            recommended_onboarding_model,
        } => render_requires_explicit_model_detail(
            provider_prefix,
            error,
            recommended_onboarding_model.as_deref(),
        ),
    }
}

fn render_transport_failure_detail(provider_prefix: &str, error: &str) -> String {
    let marker = crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER;
    format!(
        "{provider_prefix}: {marker} ({error}); runtime could not verify the provider route. inspect provider route diagnostics and retry once dns / proxy / TUN routing is stable"
    )
}

fn render_requires_explicit_model_detail(
    provider_prefix: &str,
    error: &str,
    recommended_onboarding_model: Option<&str>,
) -> String {
    match recommended_onboarding_model {
        Some(model) => format!(
            "{provider_prefix}: {MODEL_CATALOG_PROBE_FAILED_MARKER} ({error}); current config still uses `model = auto`; rerun onboarding and accept reviewed model `{model}`, or set `provider.model` / `preferred_models` explicitly"
        ),
        None => format!(
            "{provider_prefix}: {MODEL_CATALOG_PROBE_FAILED_MARKER} ({error}); current config still uses `model = auto`; set `provider.model` explicitly or configure `preferred_models` before retrying"
        ),
    }
}

fn render_model_candidate_list(models: &[String]) -> String {
    models
        .iter()
        .map(|model| format!("`{model}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn append_region_hint(
    config: &mvp::config::LoongClawConfig,
    error: &str,
    mut detail: String,
) -> String {
    let is_auth_style_failure = mvp::provider::is_auth_style_failure_message(error);
    if !is_auth_style_failure {
        return detail;
    }

    let support_facts = config.provider.support_facts();
    let region_endpoint_support = support_facts.region_endpoint;
    let Some(hint) = region_endpoint_support.catalog_failure_hint else {
        return detail;
    };

    detail.push(' ');
    detail.push_str(hint.as_str());
    detail
}

fn provider_model_probe_region_endpoint_hint(
    config: &mvp::config::LoongClawConfig,
    detail: &str,
) -> Option<String> {
    let is_auth_style_failure = provider_model_probe_auth_failure_detail(detail);
    if !is_auth_style_failure {
        return None;
    }

    let support_facts = config.provider.support_facts();
    let region_endpoint_support = support_facts.region_endpoint;

    region_endpoint_support.catalog_failure_hint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_model_probe_failure_marks_transport_route_failures() {
        let config = mvp::config::LoongClawConfig::default();
        let failure = provider_model_probe_failure(
            &config,
            "provider model-list request failed on attempt 3/3: operation timed out",
        );

        assert_eq!(failure.level, ProviderModelProbeFailureLevel::Fail);
        assert_eq!(
            failure.kind,
            ProviderModelProbeFailureKind::TransportFailure
        );
        assert!(
            provider_model_probe_transport_failure_detail(failure.detail.as_str()),
            "transport-style failures should keep the route-focused marker in the rendered detail"
        );
    }

    #[test]
    fn provider_model_probe_failure_preserves_reviewed_model_recovery() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();

        let failure = provider_model_probe_failure(&config, "401 Unauthorized");

        assert_eq!(failure.level, ProviderModelProbeFailureLevel::Fail);
        assert_eq!(
            failure.kind,
            ProviderModelProbeFailureKind::RequiresExplicitModel {
                recommended_onboarding_model: Some("deepseek-chat".to_owned()),
            }
        );
    }

    #[test]
    fn provider_model_probe_failure_appends_region_hint_for_auth_failures() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();

        let failure = provider_model_probe_failure(&config, "provider returned status 401");

        assert!(
            failure.detail.contains("https://api.minimax.io"),
            "auth-style failures should keep provider-specific endpoint guidance in the shared policy detail"
        );
        assert!(
            provider_model_probe_auth_failure_detail(failure.detail.as_str()),
            "the shared detail classifier should recognize auth-style model probe failures"
        );
    }

    #[test]
    fn provider_model_probe_recovery_advice_reconstructs_transport_failures_from_detail() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.model = "custom-explicit-model".to_owned();

        let failure = provider_model_probe_failure(
            &config,
            "provider model-list request failed on attempt 3/3: operation timed out",
        );
        let advice =
            provider_model_probe_recovery_advice(&config, failure.detail.as_str()).unwrap();

        assert_eq!(advice.kind, ProviderModelProbeFailureKind::TransportFailure);
        assert_eq!(advice.region_endpoint_hint, None);
    }

    #[test]
    fn provider_model_probe_recovery_advice_preserves_reviewed_default_recovery() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();

        let failure = provider_model_probe_failure(&config, "provider returned status 401");
        let advice =
            provider_model_probe_recovery_advice(&config, failure.detail.as_str()).unwrap();

        assert_eq!(
            advice.kind,
            ProviderModelProbeFailureKind::RequiresExplicitModel {
                recommended_onboarding_model: Some("deepseek-chat".to_owned()),
            }
        );
    }

    #[test]
    fn provider_model_probe_recovery_advice_keeps_region_hint_for_auth_failures() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();

        let failure = provider_model_probe_failure(&config, "provider returned status 401");
        let advice =
            provider_model_probe_recovery_advice(&config, failure.detail.as_str()).unwrap();
        let region_endpoint_hint = advice.region_endpoint_hint.unwrap();

        assert!(
            region_endpoint_hint.contains("https://api.minimax.io"),
            "recovery advice should keep provider-specific region guidance for auth-style failures"
        );
    }
}
