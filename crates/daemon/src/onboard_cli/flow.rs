use super::*;

pub(super) struct OnboardSessionPreparation {
    pub(super) output_path: PathBuf,
    pub(super) starting_selection: StartingConfigSelection,
    pub(super) config: mvp::config::LoongConfig,
    pub(super) skip_detailed_setup: bool,
    pub(super) review_flow_style: ReviewFlowStyle,
}

pub(super) struct OnboardReviewContext {
    pub(super) review_candidate: crate::migration::ImportCandidate,
    pub(super) review_flow_style: ReviewFlowStyle,
}

pub(super) struct OnboardPreflightResult {
    pub(super) checks: Vec<OnboardCheck>,
    pub(super) skip_config_write: bool,
}

pub(super) fn is_explicitly_accepted_non_interactive_warning(
    check: &OnboardCheck,
    options: &OnboardCommandOptions,
) -> bool {
    preflight_accepts_non_interactive_warning(check, options.skip_model_probe)
}

pub(super) fn prepare_onboard_session(
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<OnboardSessionPreparation> {
    acknowledge_onboard_risk(options, ui, context)?;

    let output_path = options
        .output
        .as_deref()
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let starting_selection = load_import_starting_config(&output_path, options, ui, context)?;
    let (config, skip_detailed_setup, review_flow_style) =
        prepare_onboard_starting_selection(options, &starting_selection, ui, context)?;

    Ok(OnboardSessionPreparation {
        output_path,
        starting_selection,
        config,
        skip_detailed_setup,
        review_flow_style,
    })
}

pub(super) fn acknowledge_onboard_risk(
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    validate_non_interactive_risk_gate(options.non_interactive, options.accept_risk)?;

    if !options.non_interactive && !options.accept_risk {
        print_lines(
            ui,
            render_onboarding_risk_screen_lines_with_style(context.render_width, true),
        )?;
        if !ui.prompt_confirm(
            crate::onboard::presentation::risk_screen_copy().confirm_prompt,
            false,
        )? {
            return Err("onboarding cancelled: risk acknowledgement declined".to_owned());
        }
    }

    Ok(())
}

pub(super) fn prepare_onboard_starting_selection(
    options: &OnboardCommandOptions,
    starting_selection: &StartingConfigSelection,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<(mvp::config::LoongConfig, bool, ReviewFlowStyle)> {
    let shortcut_kind = resolve_onboard_shortcut_kind(options, starting_selection);
    let config = starting_selection.config.clone();
    let skip_detailed_setup = if let Some(shortcut_kind) = shortcut_kind {
        print_lines(
            ui,
            render_onboard_shortcut_header_lines_with_style(
                shortcut_kind,
                &config,
                starting_selection.import_source.as_deref(),
                context.render_width,
                true,
            ),
        )?;
        matches!(
            prompt_onboard_shortcut_choice(ui, shortcut_kind)?,
            OnboardShortcutChoice::UseShortcut
        )
    } else {
        false
    };
    let review_flow_style = if skip_detailed_setup {
        shortcut_kind
            .map(OnboardShortcutKind::review_flow_style)
            .unwrap_or(ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack))
    } else {
        ReviewFlowStyle::Guided(resolve_guided_prompt_path(options, &config))
    };

    Ok((config, skip_detailed_setup, review_flow_style))
}

pub(super) fn build_onboard_review_context(
    config: &mvp::config::LoongConfig,
    starting_selection: &StartingConfigSelection,
    review_flow_style: ReviewFlowStyle,
    context: &OnboardRuntimeContext,
) -> OnboardReviewContext {
    let workspace_guidance = context
        .workspace_root
        .as_deref()
        .map(crate::migration::detect_workspace_guidance)
        .unwrap_or_default();
    let review_candidate = build_onboard_review_candidate_with_selected_context(
        config,
        &workspace_guidance,
        starting_selection.review_candidate.as_ref(),
    );
    OnboardReviewContext {
        review_candidate,
        review_flow_style,
    }
}

pub(super) async fn complete_onboard_preflight(
    options: &OnboardCommandOptions,
    output_path: &Path,
    config: &mvp::config::LoongConfig,
    reuse_existing_non_interactive_config: bool,
    review_flow_style: ReviewFlowStyle,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<OnboardPreflightResult> {
    let checks = run_preflight_checks(config, options.skip_model_probe).await;
    let config_validation_failure = config_validation_failure_message(&checks);
    let credential_ok = checks
        .iter()
        .find(|check| check.name == "provider credentials")
        .is_some_and(|check| check.level == OnboardCheckLevel::Pass);
    let has_failures = checks
        .iter()
        .any(|check| check.level == OnboardCheckLevel::Fail);
    let has_warnings = checks
        .iter()
        .any(|check| check.level == OnboardCheckLevel::Warn);
    let existing_output_config = load_existing_output_config(output_path);
    let skip_config_write = reuse_existing_non_interactive_config
        || should_skip_config_write(existing_output_config.as_ref(), config);
    let has_blocking_non_interactive_warnings = !skip_config_write
        && checks.iter().any(|check| {
            check.level == OnboardCheckLevel::Warn
                && !is_explicitly_accepted_non_interactive_warning(check, options)
        });

    if options.non_interactive {
        if let Some(message) = config_validation_failure {
            return Err(message);
        }
        if !skip_config_write {
            if !credential_ok {
                let credential_hint =
                    provider_credential_policy::provider_credential_env_hint(&config.provider)
                        .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
                return Err(format!(
                    "onboard preflight failed: provider credentials missing. configure inline credentials or set {} in env",
                    credential_hint
                ));
            }
            if has_failures {
                return Err(non_interactive_preflight_failure_message(&checks));
            }
            if has_blocking_non_interactive_warnings {
                let warning_message = non_interactive_preflight_warning_message(&checks, options);
                return Err(warning_message);
            }
        }
    } else {
        print_lines(
            ui,
            render_preflight_summary_screen_lines_with_style(
                &checks,
                context.render_width,
                review_flow_style,
                true,
            ),
        )?;
        if let Some(message) = config_validation_failure {
            return Err(message);
        }
        if (has_failures || has_warnings)
            && !ui.prompt_confirm(
                crate::onboard::presentation::preflight_confirm_prompt(),
                false,
            )?
        {
            return Err("onboarding cancelled: unresolved preflight warnings".to_owned());
        }
    }

    Ok(OnboardPreflightResult {
        checks,
        skip_config_write,
    })
}

pub(super) async fn finalize_onboard_closeout(
    options: &OnboardCommandOptions,
    output_path: &Path,
    config: &mvp::config::LoongConfig,
    selected_preinstalled_skill_ids: &[String],
    starting_selection: &StartingConfigSelection,
    review: &OnboardReviewContext,
    preflight: OnboardPreflightResult,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    let has_failures = preflight
        .checks
        .iter()
        .any(|check| check.level == OnboardCheckLevel::Fail);
    let has_warnings = preflight
        .checks
        .iter()
        .any(|check| check.level == OnboardCheckLevel::Warn);

    let workspace_guidance = context
        .workspace_root
        .as_deref()
        .map(crate::migration::detect_workspace_guidance)
        .unwrap_or_default();
    if !options.non_interactive {
        print_lines(
            ui,
            render_onboard_review_lines_with_guidance_and_style(
                config,
                starting_selection.import_source.as_deref(),
                &workspace_guidance,
                starting_selection.review_candidate.as_ref(),
                context.render_width,
                review.review_flow_style,
                true,
            ),
        )?;
    }

    if !options.non_interactive && !preflight.skip_config_write {
        print_lines(
            ui,
            render_write_confirmation_screen_lines_with_style(
                &output_path.display().to_string(),
                has_failures || has_warnings,
                context.render_width,
                review.review_flow_style,
                true,
            ),
        )?;
        if !ui.prompt_confirm(
            crate::onboard::presentation::write_confirmation_prompt(),
            true,
        )? {
            return Err("onboarding cancelled: review declined before write".to_owned());
        }
    }

    let (path, config_status, write_recovery): (
        PathBuf,
        Option<String>,
        Option<OnboardWriteRecovery>,
    ) = if preflight.skip_config_write {
        (
            output_path.to_path_buf(),
            Some("existing config kept; no changes were needed".to_owned()),
            None,
        )
    } else {
        let write_plan = resolve_write_plan(output_path, options, ui, context)?;
        let write_recovery = prepare_output_path_for_write(output_path, &write_plan)?;
        let backup_path = if write_recovery.keep_backup_on_success {
            write_recovery.backup_path.as_deref()
        } else {
            None
        };
        if let Some(backup_path) = backup_path {
            let backup_message = format!("Backed up existing config to: {}", backup_path.display());
            print_message(ui, backup_message)?;
        }
        let path = match mvp::config::write(options.output.as_deref(), config, write_plan.force) {
            Ok(path) => path,
            Err(error) => {
                return Err(rollback_onboard_write_failure(
                    output_path,
                    &write_recovery,
                    error,
                ));
            }
        };
        (path, None, Some(write_recovery))
    };

    #[cfg(feature = "memory-sqlite")]
    let memory_path = {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        match mvp::memory::ensure_memory_db_ready(
            Some(config.memory.resolved_sqlite_path()),
            &mem_config,
        ) {
            Ok(path) => path,
            Err(error) => {
                let failure = format!("failed to bootstrap sqlite memory: {error}");
                if let Some(write_recovery) = write_recovery.as_ref() {
                    return Err(rollback_onboard_write_failure(
                        output_path,
                        write_recovery,
                        failure,
                    ));
                }
                return Err(failure);
            }
        }
    };

    let memory_path_display = Some(memory_path.display().to_string());
    #[cfg(not(feature = "memory-sqlite"))]
    let memory_path_display: Option<String> = None;

    if let Err(error) =
        install_selected_preinstalled_skills(&path, config, selected_preinstalled_skill_ids)
    {
        if let Some(write_recovery) = write_recovery.as_ref() {
            return Err(rollback_onboard_write_failure(
                output_path,
                write_recovery,
                error,
            ));
        }
        return Err(error);
    }

    if let Some(write_recovery) = write_recovery.as_ref() {
        write_recovery.finish_success();
    }

    let success_summary = build_onboarding_success_summary_with_memory(
        &path,
        config,
        starting_selection.import_source.as_deref(),
        Some(&review.review_candidate),
        memory_path_display.as_deref(),
        config_status.as_deref(),
    );
    let success_summary_lines =
        render_onboarding_success_summary_lines(&success_summary, context.render_width, true);
    print_lines(ui, success_summary_lines)?;
    Ok(())
}
