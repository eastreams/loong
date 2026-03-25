use std::fs;
use std::path::Path;

use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;

use crate::tui_surface::{
    TuiChecklistItemSpec, TuiChecklistStatus, TuiChoiceSpec, TuiHeaderStyle, TuiScreenSpec,
    TuiSectionSpec, render_onboard_screen_spec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardCheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnboardNonInteractiveWarningPolicy {
    #[default]
    Block,
    AcceptedBySkipModelProbe,
    AcceptedByExplicitModel,
    AcceptedByPreferredModels,
    RequiresExplicitModel,
    RequiresExplicitModelWithoutReviewedDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct OnboardCheckCounts {
    pass: usize,
    warn: usize,
    fail: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardCheck {
    pub name: &'static str,
    pub level: OnboardCheckLevel,
    pub detail: String,
    pub non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy,
}

pub(crate) async fn run_preflight_checks(
    config: &mvp::config::LoongClawConfig,
    skip_model_probe: bool,
) -> Vec<OnboardCheck> {
    let mut checks = Vec::new();

    if let Some(check) = config_validation_check(config) {
        checks.push(check);
    }

    let credential_check = provider_credential_check(config);
    let has_credentials = credential_check.level == OnboardCheckLevel::Pass;
    checks.push(credential_check);
    checks.push(provider_transport_check(config));
    checks.push(web_search_provider_check(config));

    if skip_model_probe {
        checks.push(OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "skipped by --skip-model-probe".to_owned(),
            non_interactive_warning_policy:
                OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe,
        });
    } else if !has_credentials {
        checks.push(OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "skipped because credentials are missing".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        });
    } else {
        match mvp::provider::fetch_available_models(config).await {
            Ok(models) => {
                let detail = format!("{} model(s) available", models.len());

                checks.push(OnboardCheck {
                    name: "provider model probe",
                    level: OnboardCheckLevel::Pass,
                    detail,
                    non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
                });
            }
            Err(error) => {
                let transport_style_failure =
                    crate::provider_route_diagnostics::is_transport_style_model_probe_failure(
                        error.as_str(),
                    );

                checks.push(provider_model_probe_failure_check(config, error));

                if transport_style_failure
                    && let Some(route_probe) =
                        crate::provider_route_diagnostics::collect_provider_route_probe(
                            &config.provider,
                        )
                        .await
                {
                    checks.push(provider_route_probe_preflight_check(&route_probe));
                }
            }
        }
    }

    let sqlite_path = config.memory.resolved_sqlite_path();
    let sqlite_parent = sqlite_path.parent().unwrap_or(Path::new("."));
    checks.push(directory_preflight_check("memory path", sqlite_parent));

    let file_root = config.tools.resolved_file_root();
    checks.push(directory_preflight_check("tool file root", &file_root));

    let browser_companion_checks = collect_browser_companion_preflight_checks(config).await;
    checks.extend(browser_companion_checks);

    let channel_checks = collect_channel_preflight_checks(config);
    checks.extend(channel_checks);

    checks
}

pub(crate) fn config_validation_failure_message(checks: &[OnboardCheck]) -> Option<String> {
    checks
        .iter()
        .find(|check| check.name == "config validation" && check.level == OnboardCheckLevel::Fail)
        .map(|check| format!("onboard preflight failed: {}", check.detail))
}

pub(crate) fn non_interactive_preflight_failure_message(checks: &[OnboardCheck]) -> String {
    let detail = checks
        .iter()
        .find(|check| check.level == OnboardCheckLevel::Fail)
        .map(|check| {
            let mut detail = check.detail.clone();

            if check.name == "provider model probe"
                && check.detail.contains(
                    crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER,
                )
                && let Some(route_probe) = checks.iter().find(|candidate| {
                    candidate.name
                        == crate::provider_route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME
                })
            {
                detail.push_str(" provider route probe: ");
                detail.push_str(route_probe.detail.as_str());
            }

            detail
        })
        .unwrap_or_else(|| "preflight checks failed".to_owned());

    format!("onboard preflight failed: {detail}")
}

pub(crate) fn is_explicitly_accepted_non_interactive_warning(
    check: &OnboardCheck,
    skip_model_probe: bool,
) -> bool {
    (skip_model_probe
        && matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe
        ))
        || matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::AcceptedByExplicitModel
                | OnboardNonInteractiveWarningPolicy::AcceptedByPreferredModels
        )
}

pub fn provider_credential_check(config: &mvp::config::LoongClawConfig) -> OnboardCheck {
    let provider = &config.provider;
    let provider_prefix = provider_check_detail_prefix(config);
    let inline_oauth = secret_ref_has_inline_literal(provider.oauth_access_token.as_ref());

    if inline_oauth {
        return OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: format!("{provider_prefix}: inline oauth access token configured"),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    let inline_api_key = secret_ref_has_inline_literal(provider.api_key.as_ref());

    if inline_api_key {
        return OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: format!("{provider_prefix}: inline api key configured"),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    if provider.authorization_header().is_some() {
        let detail = crate::provider_credential_policy::provider_credential_env_hint(provider)
            .map(|env_name| format!("{env_name} is available"))
            .unwrap_or_else(|| "provider credentials are available".to_owned());

        return OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: format!("{provider_prefix}: {detail}"),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    let mut detail = crate::provider_credential_policy::provider_credential_env_hint(provider)
        .map(|env_name| format!("{env_name} is not set"))
        .unwrap_or_else(|| "provider credentials are not configured".to_owned());

    if let Some(hint) = provider.auth_guidance_hint() {
        detail.push(' ');
        detail.push_str(hint.as_str());
    }

    OnboardCheck {
        name: "provider credentials",
        level: OnboardCheckLevel::Warn,
        detail: format!("{provider_prefix}: {detail}"),
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

fn web_search_provider_check(config: &mvp::config::LoongClawConfig) -> OnboardCheck {
    let provider = mvp::config::normalize_web_search_provider(
        config.tools.web_search.default_provider.as_str(),
    )
    .unwrap_or(mvp::config::DEFAULT_WEB_SEARCH_PROVIDER);
    let provider_label = crate::onboard_cli::web_search_provider_display_name(provider);
    let credential_summary =
        crate::onboard_cli::summarize_web_search_provider_credential(config, provider);

    let has_available_credential =
        crate::onboard_cli::web_search_provider_has_available_credential(config, provider);
    if has_available_credential {
        let detail = credential_summary
            .map(|summary| format!("{provider_label}: {}", summary.value))
            .unwrap_or_else(|| provider_label.clone());

        return OnboardCheck {
            name: "web search provider",
            level: OnboardCheckLevel::Pass,
            detail,
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    let detail = credential_summary
        .map(|summary| {
            format!(
                "{provider_label}: {}. web.search will stay unavailable until the provider credential is supplied",
                summary.value
            )
        })
        .unwrap_or_else(|| provider_label.clone());

    OnboardCheck {
        name: "web search provider",
        level: OnboardCheckLevel::Warn,
        detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

pub fn directory_preflight_check(name: &'static str, target: &Path) -> OnboardCheck {
    if target.exists() {
        return match fs::metadata(target) {
            Ok(metadata) if metadata.is_dir() => OnboardCheck {
                name,
                level: OnboardCheckLevel::Pass,
                detail: target.display().to_string(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
            Ok(_) => OnboardCheck {
                name,
                level: OnboardCheckLevel::Fail,
                detail: format!("{} exists but is not a directory", target.display()),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
            Err(error) => OnboardCheck {
                name,
                level: OnboardCheckLevel::Fail,
                detail: format!("failed to inspect {}: {error}", target.display()),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
        };
    }

    let mut ancestor = target;

    while !ancestor.exists() {
        let Some(parent) = ancestor.parent() else {
            return OnboardCheck {
                name,
                level: OnboardCheckLevel::Fail,
                detail: format!("no existing parent found for {}", target.display()),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            };
        };

        ancestor = parent;
    }

    match fs::metadata(ancestor) {
        Ok(metadata) if metadata.is_dir() => OnboardCheck {
            name,
            level: OnboardCheckLevel::Pass,
            detail: format!("would create under {}", ancestor.display()),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
        Ok(_) => OnboardCheck {
            name,
            level: OnboardCheckLevel::Fail,
            detail: format!("{} exists but is not a directory", ancestor.display()),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
        Err(error) => OnboardCheck {
            name,
            level: OnboardCheckLevel::Fail,
            detail: format!("failed to inspect {}: {error}", ancestor.display()),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
    }
}

pub fn collect_channel_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<OnboardCheck> {
    crate::migration::channels::collect_channel_preflight_checks(config)
        .into_iter()
        .map(|check| {
            let level = match check.level {
                crate::migration::channels::ChannelCheckLevel::Pass => OnboardCheckLevel::Pass,
                crate::migration::channels::ChannelCheckLevel::Warn => OnboardCheckLevel::Warn,
                crate::migration::channels::ChannelCheckLevel::Fail => OnboardCheckLevel::Fail,
            };

            OnboardCheck {
                name: check.name,
                level,
                detail: check.detail,
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            }
        })
        .collect()
}

pub(crate) fn render_preflight_summary_screen_lines_with_progress(
    checks: &[OnboardCheck],
    width: usize,
    progress_line: &str,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_preflight_summary_screen_spec(checks, progress_line);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub fn render_preflight_summary_screen_lines(checks: &[OnboardCheck], width: usize) -> Vec<String> {
    let progress_line = crate::onboard_presentation::review_flow_copy(
        crate::onboard_presentation::ReviewFlowKind::Guided,
    )
    .progress_line;

    render_preflight_summary_screen_lines_with_progress(checks, width, progress_line, false)
}

pub fn render_current_setup_preflight_summary_screen_lines(
    checks: &[OnboardCheck],
    width: usize,
) -> Vec<String> {
    let progress_line = crate::onboard_presentation::review_flow_copy(
        crate::onboard_presentation::ReviewFlowKind::QuickCurrentSetup,
    )
    .progress_line;

    render_preflight_summary_screen_lines_with_progress(checks, width, progress_line, false)
}

pub fn render_detected_setup_preflight_summary_screen_lines(
    checks: &[OnboardCheck],
    width: usize,
) -> Vec<String> {
    let progress_line = crate::onboard_presentation::review_flow_copy(
        crate::onboard_presentation::ReviewFlowKind::QuickDetectedSetup,
    )
    .progress_line;

    render_preflight_summary_screen_lines_with_progress(checks, width, progress_line, false)
}

fn config_validation_check(config: &mvp::config::LoongClawConfig) -> Option<OnboardCheck> {
    config.validate().err().map(|detail| OnboardCheck {
        name: "config validation",
        level: OnboardCheckLevel::Fail,
        detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    })
}

fn provider_check_detail_prefix(config: &mvp::config::LoongClawConfig) -> String {
    crate::provider_presentation::active_provider_detail_label(config)
}

fn render_onboard_model_candidate_list(models: &[String]) -> String {
    models
        .iter()
        .map(|model| format!("`{model}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn provider_model_probe_failure_check(
    config: &mvp::config::LoongClawConfig,
    error: String,
) -> OnboardCheck {
    let provider_prefix = provider_check_detail_prefix(config);

    if crate::provider_route_diagnostics::is_transport_style_model_probe_failure(error.as_str()) {
        return OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Fail,
            detail: format!(
                "{provider_prefix}: {} ({error}); runtime could not verify the provider route. inspect provider route diagnostics and retry once dns / proxy / TUN routing is stable",
                crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER
            ),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    let auth_style_failure = mvp::provider::is_auth_style_failure_message(error.as_str());
    let append_region_hint = |mut detail: String| {
        if auth_style_failure && let Some(hint) = config.provider.region_endpoint_failure_hint() {
            detail.push(' ');
            detail.push_str(hint.as_str());
        }

        detail
    };

    let recovery = config.provider.model_catalog_probe_recovery();
    let (level, detail, non_interactive_warning_policy) = match recovery {
        mvp::config::ModelCatalogProbeRecovery::ExplicitModel(model) => (
            OnboardCheckLevel::Warn,
            append_region_hint(format!(
                "{provider_prefix}: model catalog probe failed ({error}); chat may still work because model `{model}` is explicitly configured"
            )),
            OnboardNonInteractiveWarningPolicy::AcceptedByExplicitModel,
        ),
        mvp::config::ModelCatalogProbeRecovery::ConfiguredPreferredModels(fallback_models) => (
            OnboardCheckLevel::Warn,
            append_region_hint(format!(
                "{provider_prefix}: model catalog probe failed ({error}); runtime will try configured preferred model fallback(s): {}",
                render_onboard_model_candidate_list(&fallback_models)
            )),
            OnboardNonInteractiveWarningPolicy::AcceptedByPreferredModels,
        ),
        mvp::config::ModelCatalogProbeRecovery::RequiresExplicitModel {
            recommended_onboarding_model,
        } => {
            let detail = provider_model_probe_requires_explicit_model_detail(
                provider_prefix.as_str(),
                error.as_str(),
                recommended_onboarding_model,
            );
            let detail = append_region_hint(detail);
            let policy = if recommended_onboarding_model.is_some() {
                OnboardNonInteractiveWarningPolicy::RequiresExplicitModel
            } else {
                OnboardNonInteractiveWarningPolicy::RequiresExplicitModelWithoutReviewedDefault
            };

            (OnboardCheckLevel::Fail, detail, policy)
        }
    };

    OnboardCheck {
        name: "provider model probe",
        level,
        detail,
        non_interactive_warning_policy,
    }
}

async fn collect_browser_companion_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<OnboardCheck> {
    let Some(diagnostics) =
        crate::browser_companion_diagnostics::collect_browser_companion_diagnostics(config).await
    else {
        return Vec::new();
    };

    let level = if diagnostics.install_ready() && diagnostics.runtime_ready {
        OnboardCheckLevel::Pass
    } else {
        OnboardCheckLevel::Warn
    };
    let detail = if diagnostics.install_ready() {
        diagnostics
            .runtime_gate_detail()
            .unwrap_or_else(|| diagnostics.install_detail())
    } else {
        diagnostics.install_detail()
    };

    vec![OnboardCheck {
        name: crate::browser_companion_diagnostics::BROWSER_COMPANION_INSTALL_CHECK_NAME,
        level,
        detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }]
}

fn provider_model_probe_requires_explicit_model_detail(
    provider_prefix: &str,
    error: &str,
    recommended_onboarding_model: Option<&str>,
) -> String {
    match recommended_onboarding_model {
        Some(model) => format!(
            "{provider_prefix}: model catalog probe failed ({error}); current config still uses `model = auto`; rerun onboarding and accept reviewed model `{model}`, or set `provider.model` / `preferred_models` explicitly"
        ),
        None => format!(
            "{provider_prefix}: model catalog probe failed ({error}); current config still uses `model = auto`; set `provider.model` explicitly or configure `preferred_models` before retrying"
        ),
    }
}

fn provider_transport_check(config: &mvp::config::LoongClawConfig) -> OnboardCheck {
    let readiness = config.provider.transport_readiness();
    let level = match readiness.level {
        mvp::config::ProviderTransportReadinessLevel::Ready => OnboardCheckLevel::Pass,
        mvp::config::ProviderTransportReadinessLevel::Review => OnboardCheckLevel::Warn,
        mvp::config::ProviderTransportReadinessLevel::Unsupported => OnboardCheckLevel::Fail,
    };

    OnboardCheck {
        name: "provider transport",
        level,
        detail: readiness.detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

fn provider_route_probe_preflight_check(
    probe: &crate::provider_route_diagnostics::ProviderRouteProbe,
) -> OnboardCheck {
    let level = match probe.level {
        crate::provider_route_diagnostics::ProviderRouteProbeLevel::Pass => OnboardCheckLevel::Pass,
        crate::provider_route_diagnostics::ProviderRouteProbeLevel::Warn => OnboardCheckLevel::Warn,
        crate::provider_route_diagnostics::ProviderRouteProbeLevel::Fail => OnboardCheckLevel::Fail,
    };

    OnboardCheck {
        name: crate::provider_route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME,
        level,
        detail: probe.detail.clone(),
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

fn summarize_onboard_checks(checks: &[OnboardCheck]) -> OnboardCheckCounts {
    let mut counts = OnboardCheckCounts::default();

    for check in checks {
        match check.level {
            OnboardCheckLevel::Pass => counts.pass += 1,
            OnboardCheckLevel::Warn => counts.warn += 1,
            OnboardCheckLevel::Fail => counts.fail += 1,
        }
    }

    counts
}

fn build_preflight_summary_screen_spec(
    checks: &[OnboardCheck],
    progress_line: &str,
) -> TuiScreenSpec {
    let counts = summarize_onboard_checks(checks);
    let has_attention = counts.warn > 0 || counts.fail > 0;
    let mut summary_lines = vec![format!(
        "- status: {} pass · {} warn · {} fail",
        counts.pass, counts.warn, counts.fail
    )];

    if has_attention {
        summary_lines
            .push(crate::onboard_presentation::preflight_attention_summary_line().to_owned());

        if let Some(hint) = preflight_attention_hint_line(checks) {
            summary_lines.push(hint.to_owned());
        }
    } else {
        summary_lines.push(crate::onboard_presentation::preflight_green_summary_line().to_owned());
    }

    let mut sections = Vec::new();
    if !checks.is_empty() {
        sections.push(TuiSectionSpec::Checklist {
            title: None,
            items: tui_checklist_items_from_preflight_checks(checks),
        });
    }

    let choices = if has_attention {
        vec![
            TuiChoiceSpec {
                key: "y".to_owned(),
                label: crate::onboard_presentation::preflight_continue_label().to_owned(),
                detail_lines: vec![
                    crate::onboard_presentation::preflight_continue_detail().to_owned(),
                ],
                recommended: false,
            },
            TuiChoiceSpec {
                key: "n".to_owned(),
                label: crate::onboard_presentation::preflight_cancel_label().to_owned(),
                detail_lines: vec![
                    crate::onboard_presentation::preflight_cancel_detail().to_owned(),
                ],
                recommended: false,
            },
        ]
    } else {
        Vec::new()
    };

    let footer_lines = if has_attention {
        crate::onboard_cli::append_escape_cancel_hint(vec![
            crate::onboard_cli::render_default_choice_footer_line(
                "n",
                crate::onboard_presentation::preflight_default_choice_description(),
            ),
        ])
    } else {
        Vec::new()
    };

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(crate::onboard_presentation::preflight_header_title().to_owned()),
        title: Some(crate::onboard_presentation::preflight_section_title().to_owned()),
        progress_line: Some(progress_line.to_owned()),
        intro_lines: summary_lines,
        sections,
        choices,
        footer_lines,
    }
}

fn tui_checklist_items_from_preflight_checks(checks: &[OnboardCheck]) -> Vec<TuiChecklistItemSpec> {
    checks
        .iter()
        .map(|check| TuiChecklistItemSpec {
            status: tui_checklist_status(check.level),
            label: check.name.to_owned(),
            detail: check.detail.clone(),
        })
        .collect()
}

fn tui_checklist_status(level: OnboardCheckLevel) -> TuiChecklistStatus {
    match level {
        OnboardCheckLevel::Pass => TuiChecklistStatus::Pass,
        OnboardCheckLevel::Warn => TuiChecklistStatus::Warn,
        OnboardCheckLevel::Fail => TuiChecklistStatus::Fail,
    }
}

fn preflight_attention_hint_line(checks: &[OnboardCheck]) -> Option<&'static str> {
    if checks.iter().any(|check| {
        matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::RequiresExplicitModel
        )
    }) {
        return Some(crate::onboard_presentation::preflight_explicit_model_rerun_hint());
    }

    if checks.iter().any(|check| {
        matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::RequiresExplicitModelWithoutReviewedDefault
        )
    }) {
        return Some(crate::onboard_presentation::preflight_explicit_model_only_rerun_hint());
    }

    None
}

fn secret_ref_has_inline_literal(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };

    secret_ref.inline_literal_value().is_some()
}
