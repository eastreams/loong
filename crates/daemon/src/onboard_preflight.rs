use std::fs;
use std::path::Path;

use loong_app as mvp;
use mvp::tui_surface::render_onboard_screen_spec;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardCheck {
    pub name: &'static str,
    pub level: OnboardCheckLevel,
    pub detail: String,
    pub non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy,
}

pub(crate) async fn run_preflight_checks(
    config: &mvp::config::LoongConfig,
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
                let probe_failure =
                    crate::provider_model_probe_policy::provider_model_probe_failure(
                        config,
                        error.as_str(),
                    );
                let should_collect_route_probe = matches!(
                    &probe_failure.kind,
                    crate::provider_model_probe_policy::ProviderModelProbeFailureKind::TransportFailure
                );
                let check = onboard_check_from_provider_model_probe_failure(probe_failure);
                checks.push(check);

                if should_collect_route_probe
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

    let file_root_resolution = config.tools.file_root_resolution();
    let file_root = file_root_resolution.path().clone();
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
                && crate::provider_model_probe_policy::provider_model_probe_transport_failure_detail(
                    check.detail.as_str(),
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

pub fn provider_credential_check(config: &mvp::config::LoongConfig) -> OnboardCheck {
    let provider = &config.provider;
    let provider_prefix = provider_check_detail_prefix(config);
    let status = crate::provider_credentials_guidance::provider_credential_status(
        provider,
        crate::provider_credential_policy::provider_has_locally_available_credentials(provider),
    );

    OnboardCheck {
        name: crate::provider_credentials_guidance::PROVIDER_CREDENTIALS_LABEL,
        level: if status.is_ready() {
            OnboardCheckLevel::Pass
        } else {
            OnboardCheckLevel::Warn
        },
        detail: format!("{provider_prefix}: {}", status.detail),
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

fn web_search_provider_check(config: &mvp::config::LoongConfig) -> OnboardCheck {
    let provider_status = crate::query_search_guidance::query_search_provider_status(config);
    if provider_status.credential_available {
        return OnboardCheck {
            name: crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL,
            level: OnboardCheckLevel::Pass,
            detail: provider_status.ready_detail(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    OnboardCheck {
        name: crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL,
        level: OnboardCheckLevel::Warn,
        detail: provider_status.blocked_detail(true),
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

pub fn collect_channel_preflight_checks(config: &mvp::config::LoongConfig) -> Vec<OnboardCheck> {
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
    let spec = crate::onboard_preflight_presentation::build_preflight_summary_screen_spec(
        checks,
        progress_line,
    );
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

fn config_validation_check(config: &mvp::config::LoongConfig) -> Option<OnboardCheck> {
    config.validate().err().map(|detail| OnboardCheck {
        name: "config validation",
        level: OnboardCheckLevel::Fail,
        detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    })
}

fn provider_check_detail_prefix(config: &mvp::config::LoongConfig) -> String {
    crate::provider_presentation::active_provider_detail_label(config)
}

fn onboard_check_from_provider_model_probe_failure(
    probe_failure: crate::provider_model_probe_policy::ProviderModelProbeFailure,
) -> OnboardCheck {
    let level = match probe_failure.level {
        crate::provider_model_probe_policy::ProviderModelProbeFailureLevel::Warn => {
            OnboardCheckLevel::Warn
        }
        crate::provider_model_probe_policy::ProviderModelProbeFailureLevel::Fail => {
            OnboardCheckLevel::Fail
        }
    };
    let non_interactive_warning_policy = match probe_failure.kind {
        crate::provider_model_probe_policy::ProviderModelProbeFailureKind::TransportFailure => {
            OnboardNonInteractiveWarningPolicy::Block
        }
        crate::provider_model_probe_policy::ProviderModelProbeFailureKind::ExplicitModel {
            ..
        } => OnboardNonInteractiveWarningPolicy::AcceptedByExplicitModel,
        crate::provider_model_probe_policy::ProviderModelProbeFailureKind::PreferredModels {
            ..
        } => OnboardNonInteractiveWarningPolicy::AcceptedByPreferredModels,
        crate::provider_model_probe_policy::ProviderModelProbeFailureKind::RequiresExplicitModel {
            recommended_onboarding_model: Some(_),
        } => OnboardNonInteractiveWarningPolicy::RequiresExplicitModel,
        crate::provider_model_probe_policy::ProviderModelProbeFailureKind::RequiresExplicitModel {
            recommended_onboarding_model: None,
        } => OnboardNonInteractiveWarningPolicy::RequiresExplicitModelWithoutReviewedDefault,
    };

    OnboardCheck {
        name: "provider model probe",
        level,
        detail: probe_failure.detail,
        non_interactive_warning_policy,
    }
}

#[cfg(test)]
pub(crate) fn provider_model_probe_failure_check(
    config: &mvp::config::LoongConfig,
    error: String,
) -> OnboardCheck {
    let probe_failure =
        crate::provider_model_probe_policy::provider_model_probe_failure(config, error.as_str());
    onboard_check_from_provider_model_probe_failure(probe_failure)
}

async fn collect_browser_companion_preflight_checks(
    config: &mvp::config::LoongConfig,
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

fn provider_transport_check(config: &mvp::config::LoongConfig) -> OnboardCheck {
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
