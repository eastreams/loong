//! Screen rendering and display functions for onboarding CLI.
//!
//! Contains all screen rendering, review display, starting point detail,
//! and utility functions for the onboarding wizard.

#[allow(unused_imports)]
use super::*;

pub(super) fn render_onboard_wrapped_display_lines<I, S>(
    display_lines: I,
    width: usize,
) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    display_lines
        .into_iter()
        .flat_map(|line| mvp::presentation::render_wrapped_display_line(line.as_ref(), width))
        .collect()
}

#[cfg(test)]
pub fn render_onboard_option_lines(options: &[OnboardScreenOption], width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for option in options {
        let suffix = if option.recommended {
            " (recommended)"
        } else {
            ""
        };
        let prefix = render_onboard_option_prefix(&option.key);
        let continuation = " ".repeat(prefix.chars().count());
        lines.extend(
            mvp::presentation::render_wrapped_text_line_with_continuation(
                &prefix,
                &continuation,
                &format!("{}{}", option.label, suffix),
                width,
            ),
        );
        lines.extend(render_onboard_wrapped_display_lines(
            option
                .detail_lines
                .iter()
                .map(|detail| format!("    {detail}"))
                .collect::<Vec<_>>(),
            width,
        ));
    }
    lines
}

pub(crate) fn render_default_choice_footer_line(key: &str, description: &str) -> String {
    format!("press Enter to use default {key}, {description}")
}

#[cfg(test)]
pub(super) fn render_prompt_with_default_text(label: &str, default: &str) -> String {
    format!("{label} (default: {default}): ")
}

#[cfg(test)]
pub fn render_onboard_option_prefix(key: &str) -> String {
    format!("{key}) ")
}

pub(super) fn render_default_input_hint_line(description: impl AsRef<str>) -> String {
    format!("- press Enter to {}", description.as_ref())
}

pub(super) fn render_clear_input_hint_line(description: impl AsRef<str>) -> String {
    format!(
        "- type {ONBOARD_CLEAR_INPUT_TOKEN} to {}",
        description.as_ref()
    )
}

pub(super) fn render_model_selection_default_hint_line(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_model = config.provider.model.trim();
    if prompt_default == current_model {
        render_default_input_hint_line("keep current model")
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("leave the model blank")
    } else {
        render_default_input_hint_line(format!("use prefilled model: {prompt_default}"))
    }
}

pub(super) fn render_api_key_env_selection_default_hint_line(
    config: &mvp::config::LoongClawConfig,
    suggested_env: &str,
    prompt_default: &str,
) -> String {
    let prompt_default =
        provider_credential_policy::render_provider_credential_source_value(Some(prompt_default))
            .unwrap_or_default();
    let suggested_env =
        provider_credential_policy::render_provider_credential_source_value(Some(suggested_env))
            .unwrap_or_default();
    let current_env =
        provider_credential_policy::configured_provider_credential_env_binding(&config.provider)
            .and_then(|binding| {
                provider_credential_policy::render_provider_credential_source_value(Some(
                    binding.env_name.as_str(),
                ))
            });

    if prompt_default.is_empty() {
        return render_default_input_hint_line("leave this blank");
    }

    if current_env
        .as_deref()
        .is_some_and(|current_env| current_env == prompt_default)
    {
        return render_default_input_hint_line("keep current source");
    }

    if !suggested_env.is_empty() && prompt_default == suggested_env {
        return render_default_input_hint_line(format!("use suggested source: {prompt_default}"));
    }

    render_default_input_hint_line(format!("use prefilled source: {prompt_default}"))
}

pub(super) fn render_system_prompt_selection_default_hint_line(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_prompt = config.cli.system_prompt.trim();

    if prompt_default == current_prompt {
        if current_prompt.is_empty() {
            render_default_input_hint_line("keep the built-in default")
        } else {
            render_default_input_hint_line("keep current prompt")
        }
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("keep the built-in default")
    } else {
        render_default_input_hint_line(format!("use prefilled prompt: {prompt_default}"))
    }
}

pub(super) fn with_default_choice_footer(
    mut footer_lines: Vec<String>,
    default_choice_line: Option<String>,
) -> Vec<String> {
    if let Some(default_choice_line) = default_choice_line {
        footer_lines.insert(0, default_choice_line);
    }
    footer_lines
}

pub(crate) fn append_escape_cancel_hint(mut lines: Vec<String>) -> Vec<String> {
    if !lines.iter().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("esc") && lower.contains("cancel")
    }) {
        lines.push(ONBOARD_ESCAPE_CANCEL_HINT.to_owned());
    }
    lines
}

pub(super) fn render_onboard_choice_screen(
    header_style: OnboardHeaderStyle,
    width: usize,
    subtitle: &str,
    title: &str,
    step: Option<(GuidedOnboardStep, GuidedPromptPath)>,
    intro_lines: Vec<String>,
    options: Vec<OnboardScreenOption>,
    footer_lines: Vec<String>,
    show_escape_cancel_hint: bool,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_choice_screen_spec(
        header_style,
        subtitle,
        title,
        step,
        intro_lines,
        options,
        footer_lines,
        show_escape_cancel_hint,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_onboard_input_screen(
    width: usize,
    title: &str,
    step: GuidedOnboardStep,
    guided_prompt_path: GuidedPromptPath,
    context_lines: Vec<String>,
    hint_lines: Vec<String>,
    color_enabled: bool,
) -> Vec<String> {
    let spec =
        build_onboard_input_screen_spec(title, step, guided_prompt_path, context_lines, hint_lines);

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub fn render_continue_current_setup_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::CurrentSetup,
        config,
        None,
        width,
        false,
    )
}

pub fn render_continue_detected_setup_screen_lines(
    config: &mvp::config::LoongClawConfig,
    import_source: &str,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::DetectedSetup,
        config,
        Some(import_source),
        width,
        false,
    )
}

pub(super) fn render_onboard_shortcut_screen_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, true);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_onboard_shortcut_header_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, false);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_shortcut_default_choice_footer_line(
    shortcut_kind: OnboardShortcutKind,
) -> String {
    render_default_choice_footer_line("1", shortcut_kind.default_choice_description())
}

pub fn render_onboarding_risk_screen_lines(width: usize) -> Vec<String> {
    render_onboarding_risk_screen_lines_with_style(width, false)
}

pub(super) fn render_onboarding_risk_screen_lines_with_style(
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let copy = presentation::risk_screen_copy();
    let footer_lines = append_escape_cancel_hint(vec![render_default_choice_footer_line(
        "n",
        copy.default_choice_description,
    )]);
    let spec = TuiScreenSpec {
        header_style: TuiHeaderStyle::Brand,
        subtitle: Some(copy.subtitle.to_owned()),
        title: Some(copy.title.to_owned()),
        progress_line: None,
        intro_lines: vec!["review the trust boundary before writing any config".to_owned()],
        sections: vec![
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Warning,
                title: Some("what onboarding can do".to_owned()),
                lines: vec![
                    "LoongClaw can invoke tools and read local files when enabled.".to_owned(),
                    "Keep credentials in environment variables, not in prompts.".to_owned(),
                    "Prefer allowlist-style tool policy for shared environments.".to_owned(),
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("recommended baseline".to_owned()),
                lines: vec![
                    "start with the narrowest tool scope that still lets you verify first success"
                        .to_owned(),
                    "you can widen channels, models, and local automation after doctor and review"
                        .to_owned(),
                ],
            },
        ],
        choices: vec![
            TuiChoiceSpec {
                key: "y".to_owned(),
                label: copy.continue_label.to_owned(),
                detail_lines: vec![copy.continue_detail.to_owned()],
                recommended: false,
            },
            TuiChoiceSpec {
                key: "n".to_owned(),
                label: copy.cancel_label.to_owned(),
                detail_lines: vec![copy.cancel_detail.to_owned()],
                recommended: false,
            },
        ],
        footer_lines,
    };

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn build_onboard_shortcut_screen_spec(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    include_choices: bool,
) -> TuiScreenSpec {
    let mut snapshot_lines = Vec::new();
    if let Some(source) = import_source {
        let starting_point_label = onboard_starting_point_label(None, source);
        snapshot_lines.push(onboard_display_line(
            "- starting point: ",
            &starting_point_label,
        ));
    }
    snapshot_lines.extend(build_onboard_review_digest_display_lines(config));
    let snapshot_title = if import_source.is_some() {
        "detected starting point snapshot"
    } else {
        "current setup snapshot"
    };

    let choices = if include_choices {
        tui_choices_from_screen_options(&build_onboard_shortcut_screen_options(shortcut_kind))
    } else {
        Vec::new()
    };
    let default_choice_footer_line = render_shortcut_default_choice_footer_line(shortcut_kind);
    let footer_lines = append_escape_cancel_hint(vec![default_choice_footer_line]);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(shortcut_kind.subtitle().to_owned()),
        title: Some(shortcut_kind.title().to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections: vec![
            TuiSectionSpec::Narrative {
                title: Some(snapshot_title.to_owned()),
                lines: snapshot_lines,
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Success,
                title: Some("fast lane".to_owned()),
                lines: vec![shortcut_kind.summary_line().to_owned()],
            },
        ],
        choices,
        footer_lines,
    }
}

pub(super) fn render_preflight_summary_screen_lines_with_style(
    checks: &[OnboardCheck],
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let progress_line = flow_style.progress_line();

    render_preflight_summary_screen_lines_with_progress(
        checks,
        width,
        progress_line.as_str(),
        color_enabled,
    )
}

pub fn render_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack),
        false,
    )
}

pub fn render_current_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickCurrentSetup,
        false,
    )
}

pub fn render_detected_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickDetectedSetup,
        false,
    )
}

pub(super) fn render_write_confirmation_screen_lines_with_style(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_write_confirmation_screen_spec(config_path, warnings_kept, flow_style);

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn build_onboard_choice_screen_spec(
    header_style: OnboardHeaderStyle,
    subtitle: &str,
    title: &str,
    step: Option<(GuidedOnboardStep, GuidedPromptPath)>,
    intro_lines: Vec<String>,
    options: Vec<OnboardScreenOption>,
    footer_lines: Vec<String>,
    show_escape_cancel_hint: bool,
) -> TuiScreenSpec {
    let resolved_subtitle = screen_subtitle(subtitle);
    let resolved_progress_line =
        step.map(|(step, guided_prompt_path)| step.progress_line(guided_prompt_path));
    let resolved_footer_lines = if show_escape_cancel_hint {
        append_escape_cancel_hint(footer_lines)
    } else {
        footer_lines
    };
    let resolved_choices = tui_choices_from_screen_options(&options);

    TuiScreenSpec {
        header_style: tui_header_style(header_style),
        subtitle: resolved_subtitle,
        title: Some(title.to_owned()),
        progress_line: resolved_progress_line,
        intro_lines,
        sections: Vec::new(),
        choices: resolved_choices,
        footer_lines: resolved_footer_lines,
    }
}

pub(super) fn build_onboard_input_screen_spec(
    title: &str,
    step: GuidedOnboardStep,
    guided_prompt_path: GuidedPromptPath,
    context_lines: Vec<String>,
    hint_lines: Vec<String>,
) -> TuiScreenSpec {
    let resolved_footer_lines = append_escape_cancel_hint(hint_lines);
    let progress_line = step.progress_line(guided_prompt_path);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: None,
        title: Some(title.to_owned()),
        progress_line: Some(progress_line),
        intro_lines: context_lines,
        sections: Vec::new(),
        choices: Vec::new(),
        footer_lines: resolved_footer_lines,
    }
}

pub(super) fn build_write_confirmation_screen_spec(
    config_path: &str,
    warnings_kept: bool,
    flow_style: ReviewFlowStyle,
) -> TuiScreenSpec {
    let mut intro_lines = Vec::new();
    let config_line = format!("- config: {config_path}");
    let status_line = presentation::write_confirmation_status_line(warnings_kept).to_owned();

    intro_lines.push(config_line);
    intro_lines.push(status_line);

    let choices = vec![
        TuiChoiceSpec {
            key: "y".to_owned(),
            label: presentation::write_confirmation_label().to_owned(),
            detail_lines: vec![presentation::write_confirmation_detail().to_owned()],
            recommended: false,
        },
        TuiChoiceSpec {
            key: "n".to_owned(),
            label: presentation::write_confirmation_cancel_label().to_owned(),
            detail_lines: vec![presentation::write_confirmation_cancel_detail().to_owned()],
            recommended: false,
        },
    ];

    let default_choice_line = render_default_choice_footer_line(
        "y",
        presentation::write_confirmation_default_choice_description(),
    );
    let footer_lines = append_escape_cancel_hint(vec![default_choice_line]);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: None,
        title: Some(presentation::write_confirmation_title().to_owned()),
        progress_line: Some(match flow_style {
            ReviewFlowStyle::Guided(_) => guided_step_progress_line(OnboardWizardStep::Ready),
            ReviewFlowStyle::QuickCurrentSetup | ReviewFlowStyle::QuickDetectedSetup => {
                flow_style.progress_line()
            }
        }),
        intro_lines,
        sections: Vec::new(),
        choices,
        footer_lines,
    }
}

pub(super) fn tui_header_style(style: OnboardHeaderStyle) -> TuiHeaderStyle {
    match style {
        OnboardHeaderStyle::Compact => TuiHeaderStyle::Compact,
    }
}

pub(super) fn screen_subtitle(subtitle: &str) -> Option<String> {
    let trimmed_subtitle = subtitle.trim();

    if trimmed_subtitle.is_empty() {
        return None;
    }

    Some(trimmed_subtitle.to_owned())
}

pub(super) fn push_starting_point_fit_hint(
    hints: &mut Vec<StartingPointFitHint>,
    seen: &mut std::collections::BTreeSet<&'static str>,
    key: &'static str,
    detail: impl Into<String>,
    domain: Option<crate::migration::SetupDomainKind>,
) {
    if seen.insert(key) {
        hints.push(StartingPointFitHint {
            key,
            detail: detail.into(),
            domain,
        });
    }
}

pub(super) fn summarize_direct_starting_point_source_reason(
    candidate: &ImportCandidate,
) -> Option<&'static str> {
    candidate.source_kind.direct_starting_point_reason()
}

pub(super) fn collect_starting_point_fit_hints(
    candidate: &ImportCandidate,
) -> Vec<StartingPointFitHint> {
    let mut hints = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if let Some(reason) = summarize_direct_starting_point_source_reason(candidate) {
        push_starting_point_fit_hint(&mut hints, &mut seen, "direct_source", reason, None);
    } else if let Some(provider_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Provider)
        && let Some(decision) = provider_domain.decision
        && let Some(reason) = provider_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::KeepCurrent => "provider_keep",
            crate::migration::types::PreviewDecision::UseDetected => "provider_detected",
            crate::migration::types::PreviewDecision::Supplement
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "provider",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Provider),
        );
    }

    if let Some(channels_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Channels)
        && let Some(decision) = channels_domain.decision
        && let Some(reason) = channels_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::Supplement => "channels_add",
            crate::migration::types::PreviewDecision::UseDetected => "channels_detected",
            crate::migration::types::PreviewDecision::KeepCurrent
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "channels",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    } else if !candidate.channel_candidates.is_empty()
        && let Some(reason) = crate::migration::SetupDomainKind::Channels
            .starting_point_reason(crate::migration::types::PreviewDecision::Supplement)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "channels_add",
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    }

    if (!candidate.workspace_guidance.is_empty()
        || candidate.domains.iter().any(|domain| {
            domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }))
        && let Some(reason) = crate::migration::SetupDomainKind::WorkspaceGuidance
            .starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "workspace_guidance",
            reason,
            Some(crate::migration::SetupDomainKind::WorkspaceGuidance),
        );
    }

    for (kind, key) in [
        (crate::migration::SetupDomainKind::Cli, "cli"),
        (crate::migration::SetupDomainKind::Memory, "memory"),
        (crate::migration::SetupDomainKind::Tools, "tools"),
    ] {
        if hints.len() >= 3 {
            break;
        }
        if candidate.domains.iter().any(|domain| {
            domain.kind == kind
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }) && let Some(reason) =
            kind.starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
        {
            push_starting_point_fit_hint(&mut hints, &mut seen, key, reason, Some(kind));
        }
    }

    if hints.is_empty() {
        let source_count = crate::migration::render::candidate_source_rollup_labels(
            &migration_candidate_from_onboard(candidate),
        )
        .len();
        if source_count > 1 {
            push_starting_point_fit_hint(
                &mut hints,
                &mut seen,
                "combined_sources",
                format!("combine {source_count} reusable sources"),
                None,
            );
        }
    }

    hints
}

pub(super) fn format_starting_point_reason(hints: &[StartingPointFitHint]) -> Option<String> {
    if hints.is_empty() {
        return None;
    }

    Some(format!(
        "good fit: {}",
        hints
            .iter()
            .take(3)
            .map(|hint| hint.detail.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    ))
}

pub(super) fn should_include_starting_point_domain_decision(candidate: &ImportCandidate) -> bool {
    candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
}

pub(super) fn format_starting_point_domain_detail(
    candidate: &ImportCandidate,
    domain: &crate::migration::DomainPreview,
) -> String {
    let mut detail = format!("{}: ", domain.kind.label());
    if should_include_starting_point_domain_decision(candidate)
        && let Some(decision) = domain.decision
    {
        detail.push_str(decision.label());
        detail.push_str(" · ");
    }
    detail.push_str(&domain.summary);
    detail
}

pub(super) fn summarize_starting_point_detail_lines(
    candidate: &ImportCandidate,
    width: usize,
) -> Vec<String> {
    let mut details = Vec::new();
    let max_lines = if width < 68 { 4 } else { 5 };
    let mut detail_lines_used = 0usize;
    let has_channel_details = !candidate.channel_candidates.is_empty();
    let has_workspace_guidance_details = !candidate.workspace_guidance.is_empty();
    let migration_candidate = migration_candidate_from_onboard(candidate);
    let fit_hints = collect_starting_point_fit_hints(candidate);
    let emphasized_domains = if width < 68 {
        fit_hints
            .iter()
            .filter_map(|hint| hint.domain)
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        std::collections::BTreeSet::new()
    };

    if let Some(reason_line) = format_starting_point_reason(&fit_hints) {
        details.push(reason_line);
    }

    let mut source_labels =
        crate::migration::render::candidate_source_rollup_labels(&migration_candidate);
    if has_workspace_guidance_details {
        source_labels.retain(|label| label != "workspace guidance");
    }
    let should_render_source_summary =
        if candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan {
            !source_labels.is_empty()
        } else {
            source_labels.len() > 1
        };
    if should_render_source_summary {
        details.push(format!("sources: {}", source_labels.join(" + ")));
        detail_lines_used += 1;
    }

    for domain in &candidate.domains {
        if has_channel_details && domain.kind == crate::migration::SetupDomainKind::Channels {
            continue;
        }
        if has_workspace_guidance_details
            && domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
        {
            continue;
        }
        if emphasized_domains.contains(&domain.kind) {
            continue;
        }
        details.push(format_starting_point_domain_detail(candidate, domain));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    for channel in &candidate.channel_candidates {
        details.push(format!(
            "{}: {}",
            channel.label.to_ascii_lowercase(),
            channel.summary
        ));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    if details.len() < max_lines && !candidate.workspace_guidance.is_empty() {
        let files = candidate
            .workspace_guidance
            .iter()
            .filter_map(|guidance| Path::new(&guidance.path).file_name())
            .map(|name| name.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        if !files.is_empty() {
            details.push(format!("workspace guidance: {}", files.join(", ")));
        }
    }

    if details.is_empty() {
        details.push("ready to use as a starting point".to_owned());
    }

    details
}

pub(super) fn start_fresh_starting_point_detail_lines() -> Vec<String> {
    vec![
        presentation::start_fresh_starting_point_fit_line().to_owned(),
        presentation::start_fresh_starting_point_detail_line().to_owned(),
    ]
}

pub(super) fn render_starting_point_selection_footer_lines(
    sorted_candidates: &[ImportCandidate],
) -> Vec<String> {
    let Some(first_candidate) = sorted_candidates.first() else {
        return Vec::new();
    };

    let first_hint = render_default_choice_footer_line(
        "1",
        presentation::starting_point_footer_description(first_candidate.source_kind),
    );

    vec![first_hint]
}

pub fn render_starting_point_selection_screen_lines(
    candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_starting_point_selection_screen_lines_with_style(candidates, width, false)
}

pub(super) fn render_starting_point_selection_screen_lines_with_style(
    candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let sorted_candidates = sort_starting_point_candidates(candidates.to_vec());
    let mut options = sorted_candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: onboard_starting_point_label(Some(candidate.source_kind), &candidate.source),
            detail_lines: summarize_starting_point_detail_lines(candidate, width),
            recommended: matches!(
                candidate.source_kind,
                crate::migration::ImportSourceKind::RecommendedPlan
            ),
        })
        .collect::<Vec<_>>();
    options.push(OnboardScreenOption {
        key: "0".to_owned(),
        label: presentation::start_fresh_option_label().to_owned(),
        detail_lines: start_fresh_starting_point_detail_lines(),
        recommended: false,
    });
    let footer_lines = render_starting_point_selection_footer_lines(&sorted_candidates);

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        presentation::starting_point_selection_subtitle(),
        presentation::starting_point_selection_title(),
        None,
        vec![presentation::starting_point_selection_hint().to_owned()],
        options,
        footer_lines,
        true,
        color_enabled,
    )
}

pub(super) fn render_starting_point_selection_header_lines_with_style(
    _candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        presentation::starting_point_selection_subtitle(),
        presentation::starting_point_selection_title(),
        None,
        vec![presentation::starting_point_selection_hint().to_owned()],
        Vec::new(),
        Vec::new(),
        true,
        color_enabled,
    )
}

pub fn render_provider_selection_screen_lines(
    plan: &crate::migration::ProviderSelectionPlan,
    width: usize,
) -> Vec<String> {
    render_provider_selection_screen_lines_with_style(
        plan,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub(super) fn render_provider_selection_screen_lines_with_style(
    plan: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let intro = provider_selection_intro_lines(plan);
    let options = plan
        .imported_choices
        .iter()
        .map(|choice| OnboardScreenOption {
            key: choice.profile_id.clone(),
            label: provider_kind_display_name(choice.kind).to_owned(),
            detail_lines: {
                let mut detail_lines = vec![
                    format!("source: {}", choice.source),
                    format!("summary: {}", choice.summary),
                ];
                if let Some(selector_detail) =
                    crate::migration::provider_selection::selector_detail_line(
                        plan,
                        &choice.profile_id,
                        width,
                    )
                {
                    detail_lines.push(selector_detail);
                }
                if let Some(transport_summary) = choice.config.preview_transport_summary() {
                    detail_lines.push(format!("transport: {transport_summary}"));
                }
                detail_lines
            },
            recommended: Some(choice.profile_id.as_str()) == plan.default_profile_id.as_deref(),
        })
        .collect::<Vec<_>>();
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose the current provider",
        "choose active provider",
        Some((GuidedOnboardStep::Provider, guided_prompt_path)),
        intro,
        options,
        with_default_choice_footer(
            crate::migration::guidance_lines(plan, width),
            render_provider_selection_default_choice_footer_line(plan),
        ),
        true,
        color_enabled,
    )
}

pub(super) fn provider_selection_intro_lines(
    plan: &crate::migration::ProviderSelectionPlan,
) -> Vec<String> {
    if plan.imported_choices.is_empty() {
        vec!["pick the provider that should back this setup".to_owned()]
    } else if plan.requires_explicit_choice {
        vec!["other detected settings stay merged".to_owned()]
    } else {
        vec!["review the detected provider choices for this setup".to_owned()]
    }
}

pub(super) fn render_provider_selection_default_choice_footer_line(
    plan: &crate::migration::ProviderSelectionPlan,
) -> Option<String> {
    if plan.requires_explicit_choice {
        return None;
    }
    let default_profile_id = plan.default_profile_id.as_deref()?;
    let default_kind = plan
        .imported_choices
        .iter()
        .find(|choice| choice.profile_id == default_profile_id)
        .map(|choice| choice.kind)
        .or(plan.default_kind)?;
    Some(render_default_choice_footer_line(
        default_profile_id,
        &format!("the {} provider", provider_kind_display_name(default_kind)),
    ))
}

pub fn render_model_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        config.provider.model.as_str(),
        GuidedPromptPath::NativePromptPack,
        width,
        false,
        false,
    )
}

pub fn render_model_selection_screen_lines_with_default(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
        false,
    )
}

pub(super) fn render_model_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
    catalog_models_available: bool,
) -> Vec<String> {
    let selection_context =
        onboarding_model_policy::onboarding_model_selection_context(&config.provider);
    let current_model = selection_context.current_model;
    let recommended_model = selection_context.recommended_model;
    let preferred_fallback_models = selection_context.preferred_fallback_models;
    let allows_auto_fallback_hint = selection_context.allows_auto_fallback_hint;
    let mut context_lines = vec![
        format!(
            "- provider: {}",
            crate::provider_presentation::guided_provider_label(config.provider.kind)
        ),
        format!("- current model: {current_model}"),
    ];
    if let Some(recommended_model) = recommended_model {
        context_lines.push(format!("- recommended model: {recommended_model}"));
    }
    if !preferred_fallback_models.is_empty() {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        context_lines.push(format!(
            "- configured preferred fallback: {preferred_fallback_summary}",
        ));
    }

    let mut hint_lines = vec![render_model_selection_default_hint_line(
        config,
        prompt_default,
    )];
    if catalog_models_available {
        hint_lines.push(
            "- use arrow keys to browse or type to filter available provider models".to_owned(),
        );
        hint_lines.push(
            "- choose `enter custom model id` if you want to type an override manually".to_owned(),
        );
    } else {
        hint_lines.push("- type any provider model id to override it".to_owned());
    }
    if allows_auto_fallback_hint {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        hint_lines.push(format!(
            "- type `auto` to let runtime try configured preferred fallbacks first: {preferred_fallback_summary}",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose model",
        GuidedOnboardStep::Model,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub fn render_api_key_env_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    default_api_key_env: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        default_api_key_env,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub fn render_api_key_env_selection_screen_lines_with_default(
    config: &mvp::config::LoongClawConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub(super) fn render_api_key_env_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut context_lines = vec![format!(
        "- provider: {}",
        crate::provider_presentation::guided_provider_label(config.provider.kind)
    )];
    if let Some(current_env) = render_configured_provider_credential_source_value(&config.provider)
    {
        context_lines.push(format!("- current source: {current_env}"));
    }
    if let Some(suggested_source) =
        provider_credential_policy::render_provider_credential_source_value(Some(
            default_api_key_env,
        ))
    {
        context_lines.push(format!("- suggested source: {suggested_source}"));
    }

    let example_env_name =
        provider_credential_policy::provider_credential_env_hint(&config.provider)
            .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
    let mut hint_lines = vec![render_api_key_env_selection_default_hint_line(
        config,
        default_api_key_env,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty() {
        if provider_credential_policy::provider_has_inline_credential(&config.provider) {
            hint_lines.push("- leave this blank to keep inline credentials".to_owned());
        }
    } else if provider_supports_blank_api_key_env(config) {
        hint_lines.push(render_clear_input_hint_line(
            "clear the configured credential env",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose credential source",
        GuidedOnboardStep::CredentialEnv,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub fn render_system_prompt_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        config.cli.system_prompt.as_str(),
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

pub fn render_system_prompt_selection_screen_lines_with_default(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

pub(super) fn render_system_prompt_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_prompt = config.cli.system_prompt.trim();
    let current_prompt_display = if current_prompt.is_empty() {
        "built-in default".to_owned()
    } else {
        current_prompt.to_owned()
    };

    render_onboard_input_screen(
        width,
        "adjust cli behavior",
        GuidedOnboardStep::PromptCustomization,
        guided_prompt_path,
        vec![format!("- current prompt: {current_prompt_display}")],
        vec![
            render_system_prompt_selection_default_hint_line(config, prompt_default),
            if prompt_default.trim().is_empty() {
                "- leave this blank to use the built-in behavior".to_owned()
            } else {
                render_clear_input_hint_line("use the built-in behavior")
            },
            ONBOARD_SINGLE_LINE_INPUT_HINT.to_owned(),
        ],
        color_enabled,
    )
}

pub fn render_personality_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_personality_selection_screen_lines_with_style(
        config,
        config.cli.resolved_personality(),
        width,
        false,
    )
}

pub(super) fn render_personality_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    default_personality: mvp::prompt::PromptPersonality,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let options = [
        (
            mvp::prompt::PromptPersonality::CalmEngineering,
            "calm engineering",
            "rigorous, direct, and technically grounded",
        ),
        (
            mvp::prompt::PromptPersonality::FriendlyCollab,
            "friendly collab",
            "warm, cooperative, and explanatory when helpful",
        ),
        (
            mvp::prompt::PromptPersonality::AutonomousExecutor,
            "autonomous executor",
            "decisive, high-initiative, and execution-oriented",
        ),
    ]
    .into_iter()
    .map(|(personality, label, detail)| OnboardScreenOption {
        key: prompt_personality_id(personality).to_owned(),
        label: label.to_owned(),
        detail_lines: vec![detail.to_owned()],
        recommended: personality == default_personality,
    })
    .collect::<Vec<_>>();

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose how LoongClaw should speak and take initiative",
        "choose personality",
        Some((
            GuidedOnboardStep::Personality,
            GuidedPromptPath::NativePromptPack,
        )),
        vec![format!(
            "- current personality: {}",
            prompt_personality_id(config.cli.resolved_personality())
        )],
        options,
        vec![render_default_choice_footer_line(
            prompt_personality_id(default_personality),
            "the current personality",
        )],
        true,
        color_enabled,
    )
}

pub fn render_prompt_addendum_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_prompt_addendum_selection_screen_lines_with_style(config, width, false)
}

pub(super) fn render_prompt_addendum_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_addendum = config
        .cli
        .system_prompt_addendum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("none");

    render_onboard_input_screen(
        width,
        "adjust prompt addendum",
        GuidedOnboardStep::PromptCustomization,
        GuidedPromptPath::NativePromptPack,
        vec![
            format!(
                "- personality: {}",
                prompt_personality_id(config.cli.resolved_personality())
            ),
            format!("- current addendum: {current_addendum}"),
        ],
        vec![
            "- press Enter to keep current addendum".to_owned(),
            "- type '-' to clear it".to_owned(),
            ONBOARD_SINGLE_LINE_INPUT_HINT.to_owned(),
        ],
        color_enabled,
    )
}

pub fn render_memory_profile_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_memory_profile_selection_screen_lines_with_style(
        config,
        config.memory.profile,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub(super) fn render_memory_profile_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    default_profile: mvp::config::MemoryProfile,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let options = MEMORY_PROFILE_CHOICES
        .into_iter()
        .map(|(profile, label, detail)| OnboardScreenOption {
            key: memory_profile_id(profile).to_owned(),
            label: label.to_owned(),
            detail_lines: vec![detail.to_owned()],
            recommended: profile == default_profile,
        })
        .collect::<Vec<_>>();

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose how much memory context LoongClaw should inject",
        "choose memory profile",
        Some((GuidedOnboardStep::MemoryProfile, guided_prompt_path)),
        vec![format!(
            "- current profile: {}",
            memory_profile_id(config.memory.profile)
        )],
        options,
        vec![render_default_choice_footer_line(
            memory_profile_id(default_profile),
            "the current memory profile",
        )],
        true,
        color_enabled,
    )
}

pub fn render_existing_config_write_screen_lines(config_path: &str, width: usize) -> Vec<String> {
    render_existing_config_write_screen_lines_with_style(config_path, width, false)
}

pub(super) fn render_existing_config_write_screen_lines_with_style(
    config_path: &str,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "decide how to write the config",
        "existing config found",
        None,
        vec![
            format!("- config: {config_path}"),
            "- choose whether to replace it, keep a backup, or cancel".to_owned(),
        ],
        build_existing_config_write_screen_options(),
        vec![render_default_choice_footer_line(
            "b",
            "create backup and replace",
        )],
        true,
        color_enabled,
    )
}

pub(super) fn render_existing_config_write_header_lines_with_style(
    config_path: &str,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "decide how to write the config",
        "existing config found",
        None,
        vec![
            format!("- config: {config_path}"),
            "- choose whether to replace it, keep a backup, or cancel".to_owned(),
        ],
        Vec::new(),
        Vec::new(),
        true,
        color_enabled,
    )
}

pub(super) fn onboard_display_line(prefix: &str, value: &str) -> String {
    format!("{prefix}{value}")
}

pub(super) fn review_value_origin_label(origin: OnboardValueOrigin) -> &'static str {
    match origin {
        OnboardValueOrigin::CurrentSetup => presentation::current_value_label(),
        OnboardValueOrigin::DetectedStartingPoint => presentation::detected_value_label(),
        OnboardValueOrigin::UserSelected => presentation::user_override_label(),
    }
}

pub(super) fn onboard_review_value_line(
    label: &str,
    value: &str,
    origin: Option<OnboardValueOrigin>,
) -> String {
    match origin {
        Some(origin) => format!("- {label} ({}): {value}", review_value_origin_label(origin)),
        None => format!("- {label}: {value}"),
    }
}

pub(super) fn draft_output_path_origin(draft: &OnboardDraft) -> Option<OnboardValueOrigin> {
    if draft.output_path.exists() {
        return Some(OnboardValueOrigin::CurrentSetup);
    }

    None
}

pub(super) fn build_onboard_review_digest_display_lines_for_draft(
    draft: &OnboardDraft,
) -> Vec<String> {
    let mut lines = vec![
        onboard_review_value_line(
            "config output path",
            &draft.output_path.display().to_string(),
            draft_output_path_origin(draft),
        ),
        onboard_review_value_line(
            "sqlite memory path",
            &draft.workspace.sqlite_path.display().to_string(),
            draft.origin_for(OnboardDraft::WORKSPACE_SQLITE_PATH_KEY),
        ),
        onboard_review_value_line(
            "tool file root",
            &draft.workspace.file_root.display().to_string(),
            draft.origin_for(OnboardDraft::WORKSPACE_FILE_ROOT_KEY),
        ),
    ];
    lines.extend(build_onboard_protocol_review_digest_display_lines_for_draft(draft));
    lines.extend(build_onboard_review_digest_display_lines_without_protocols(
        &draft.config,
    ));
    lines
}

pub(super) fn build_onboard_protocol_review_digest_display_lines_for_draft(
    draft: &OnboardDraft,
) -> Vec<String> {
    let protocol_values = onboard_protocols::derive_protocol_step_values(draft);
    let mut lines = vec![onboard_display_line(
        "- ACP: ",
        if protocol_values.acp_enabled {
            "enabled"
        } else {
            "disabled"
        },
    )];

    if let Some(acp_backend) = protocol_values.acp_backend.as_deref() {
        if protocol_values.acp_enabled {
            lines.push(onboard_review_value_line(
                "ACP backend",
                acp_backend,
                protocol_values.acp_backend_origin,
            ));
        }
    } else if protocol_values.acp_enabled {
        lines.push(onboard_review_value_line(
            "ACP backend",
            "not configured",
            protocol_values.acp_backend_origin,
        ));
    }

    if let Some(summary) = onboard_protocols::bootstrap_mcp_server_summary(
        protocol_values.acp_enabled,
        &protocol_values.bootstrap_mcp_servers,
    ) {
        lines.push(onboard_review_value_line(
            "bootstrap MCP servers",
            &summary,
            protocol_values.bootstrap_mcp_servers_origin,
        ));
    }

    lines
}

pub(super) fn build_onboard_review_digest_display_lines(
    config: &mvp::config::LoongClawConfig,
) -> Vec<String> {
    let mut lines = build_onboard_review_digest_display_lines_without_protocols(config);
    lines.extend(build_onboard_protocol_review_digest_display_lines(config));
    lines
}

pub(super) fn build_onboard_review_digest_display_lines_without_protocols(
    config: &mvp::config::LoongClawConfig,
) -> Vec<String> {
    let mut lines = crate::provider_presentation::provider_profile_state_display_lines(
        config,
        Some("- provider: "),
    );
    lines.push(onboard_display_line("- model: ", &config.provider.model));
    lines.push(onboard_display_line(
        "- transport: ",
        &config.provider.transport_readiness().summary,
    ));

    if let Some(provider_endpoint) = config.provider.region_endpoint_note() {
        lines.push(onboard_display_line(
            "- provider endpoint: ",
            &provider_endpoint,
        ));
    }

    if let Some(credential_line) = render_onboard_review_credential_line(&config.provider) {
        lines.push(credential_line);
    }

    let prompt_mode = summarize_prompt_mode(config);
    lines.push(onboard_display_line("- prompt mode: ", &prompt_mode));

    if config.cli.uses_native_prompt_pack() {
        lines.push(onboard_display_line(
            "- personality: ",
            prompt_personality_id(config.cli.resolved_personality()),
        ));

        if let Some(prompt_addendum) = summarize_prompt_addendum(config) {
            lines.push(onboard_display_line(
                "- prompt addendum: ",
                &prompt_addendum,
            ));
        }
    }

    lines.push(onboard_display_line("- cli: ", cli_status_value(config)));
    lines.push(onboard_display_line(
        "- external skills: ",
        &external_skills_status_value(config),
    ));

    lines.push(onboard_display_line(
        "- memory profile: ",
        memory_profile_id(config.memory.profile),
    ));

    let web_search_provider =
        web_search_provider_display_name(config.tools.web_search.default_provider.as_str());
    lines.push(onboard_display_line("- web search: ", &web_search_provider));

    if let Some(web_search_credential) = summarize_web_search_provider_credential(
        config,
        config.tools.web_search.default_provider.as_str(),
    ) {
        let credential_prefix = format!("- {}: ", web_search_credential.label);
        lines.push(onboard_display_line(
            &credential_prefix,
            &web_search_credential.value,
        ));
    }

    let enabled_channels = enabled_channel_ids(config)
        .into_iter()
        .filter(|channel| channel != "cli")
        .collect::<Vec<_>>();
    let mut enabled_channels = enabled_channels;
    enabled_channels.sort();
    if !enabled_channels.is_empty() {
        lines.push(onboard_display_line(
            "- channels: ",
            &enabled_channels.join(", "),
        ));
    }
    for channel_id in enabled_channels {
        let channel_pairing_lines = channel_pairing_review_lines(config, &channel_id);
        lines.extend(channel_pairing_lines);
    }

    lines
}

fn channel_pairing_review_lines(
    config: &mvp::config::LoongClawConfig,
    channel_id: &str,
) -> Vec<String> {
    let Some(entry) = channel_catalog_entry_by_id(channel_id) else {
        return Vec::new();
    };
    let Some(operation) = channel_primary_operation_for_review(&entry) else {
        return Vec::new();
    };

    let mut lines = Vec::new();
    let mut seen_labels = std::collections::BTreeSet::new();
    for requirement in operation.requirements {
        if requirement.id == "enabled" {
            continue;
        }
        let Some(line) = channel_requirement_review_line(config, entry.label, requirement) else {
            continue;
        };
        if seen_labels.insert(line.clone()) {
            lines.push(line);
        }
    }

    lines
}

fn channel_catalog_entry_by_id(channel_id: &str) -> Option<mvp::channel::ChannelCatalogEntry> {
    let catalog = mvp::channel::list_channel_catalog();
    catalog.into_iter().find(|entry| entry.id == channel_id)
}

fn channel_primary_operation_for_review(
    entry: &mvp::channel::ChannelCatalogEntry,
) -> Option<&mvp::channel::ChannelCatalogOperation> {
    let implemented_send = entry.operation("send").filter(|operation| {
        operation.availability == mvp::channel::ChannelCatalogOperationAvailability::Implemented
    });
    let implemented_serve = entry.operation("serve").filter(|operation| {
        operation.availability == mvp::channel::ChannelCatalogOperationAvailability::Implemented
    });

    match entry.implementation_status {
        mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked => {
            implemented_serve.or(implemented_send)
        }
        mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
        | mvp::channel::ChannelCatalogImplementationStatus::Stub => {
            implemented_send.or(implemented_serve)
        }
    }
}

fn channel_requirement_review_line(
    config: &mvp::config::LoongClawConfig,
    channel_label: &str,
    requirement: &mvp::channel::ChannelCatalogOperationRequirement,
) -> Option<String> {
    let display_channel_label = onboarding_channel_display_name(channel_label);

    let env_pointer_path = requirement
        .env_pointer_paths
        .iter()
        .find(|path| !path.contains("<account>"))
        .copied();
    if let Some(env_pointer_path) = env_pointer_path {
        let env_value = review_display_path_value(config, env_pointer_path)?;
        let review_line = format!(
            "- {display_channel_label} {} env: {env_value}",
            requirement.label
        );
        return Some(review_line);
    }

    let config_path = requirement
        .config_paths
        .iter()
        .find(|path| !path.contains("<account>"))
        .copied()?;
    let config_value = review_display_path_value(config, config_path)?;
    let review_line = format!(
        "- {display_channel_label} {}: {config_value}",
        requirement.label
    );
    Some(review_line)
}

fn onboarding_channel_display_name(raw: &str) -> String {
    let mut words = Vec::new();
    for segment in raw.split(['-', ' ']) {
        let trimmed_segment = segment.trim();
        if trimmed_segment.is_empty() {
            continue;
        }

        let mut characters = trimmed_segment.chars();
        let Some(first_character) = characters.next() else {
            continue;
        };
        let first_character = first_character.to_ascii_uppercase();
        let remainder = characters.as_str().to_ascii_lowercase();
        let word = format!("{first_character}{remainder}");
        words.push(word);
    }

    words.join(" ")
}

fn review_display_path_value(config: &mvp::config::LoongClawConfig, path: &str) -> Option<String> {
    let config_value = serde_json::to_value(config).ok()?;
    let path_value = review_json_path_value(&config_value, path)?;

    match path_value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::String(value) => {
            let trimmed_value = value.trim();
            if trimmed_value.is_empty() {
                return None;
            }
            Some(trimmed_value.to_owned())
        }
        serde_json::Value::Array(values) => {
            let mut rendered_values = Vec::new();
            for value in values {
                let Some(rendered_value) = review_json_scalar_value(value) else {
                    continue;
                };
                rendered_values.push(rendered_value);
            }
            if rendered_values.is_empty() {
                return None;
            }
            Some(rendered_values.join(", "))
        }
        serde_json::Value::Object(_) => serde_json::to_string(path_value).ok(),
    }
}

fn review_json_path_value<'a>(
    config_value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current_value = config_value;
    for segment in path.split('.') {
        let trimmed_segment = segment.trim();
        if trimmed_segment.is_empty() {
            continue;
        }
        let object = current_value.as_object()?;
        current_value = object.get(trimmed_segment)?;
    }
    Some(current_value)
}

fn review_json_scalar_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::String(value) => {
            let trimmed_value = value.trim();
            if trimmed_value.is_empty() {
                return None;
            }
            Some(trimmed_value.to_owned())
        }
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(value).ok()
        }
    }
}

pub(super) fn build_onboard_protocol_review_digest_display_lines(
    config: &mvp::config::LoongClawConfig,
) -> Vec<String> {
    let protocols = onboard_protocols::protocol_draft_from_config(config);
    let mut lines = vec![onboard_display_line(
        "- ACP: ",
        if protocols.acp_enabled {
            "enabled"
        } else {
            "disabled"
        },
    )];

    if let Some(acp_backend) = protocols.acp_backend.as_deref() {
        if protocols.acp_enabled {
            lines.push(onboard_display_line("- ACP backend: ", acp_backend));
        }
    } else if protocols.acp_enabled {
        lines.push("- ACP backend: not configured".to_owned());
    }

    if let Some(summary) = onboard_protocols::bootstrap_mcp_server_summary(
        protocols.acp_enabled,
        &protocols.bootstrap_mcp_servers,
    ) {
        lines.push(onboard_display_line("- bootstrap MCP servers: ", &summary));
    }

    lines
}

pub(super) fn render_onboard_review_credential_line(
    provider: &mvp::config::ProviderConfig,
) -> Option<String> {
    summarize_provider_credential(provider)
        .map(|credential| format!("- {}: {}", credential.label, credential.value))
}

pub(crate) fn summarize_prompt_mode(config: &mvp::config::LoongClawConfig) -> String {
    if config.cli.uses_native_prompt_pack() {
        return "native prompt pack".to_owned();
    }

    "inline system prompt override".to_owned()
}

fn cli_status_value(config: &mvp::config::LoongClawConfig) -> &'static str {
    if config.cli.enabled {
        "enabled"
    } else {
        "disabled"
    }
}

fn external_skills_status_value(config: &mvp::config::LoongClawConfig) -> String {
    if !config.external_skills.enabled {
        return "disabled".to_owned();
    }

    let mut notes = Vec::new();
    if config.external_skills.require_download_approval {
        notes.push("approval enforced");
    }
    if !config.external_skills.auto_expose_installed {
        notes.push("auto expose disabled");
    }

    if notes.is_empty() {
        "enabled".to_owned()
    } else {
        format!("enabled ({})", notes.join(", "))
    }
}

pub(crate) fn summarize_prompt_addendum(config: &mvp::config::LoongClawConfig) -> Option<String> {
    config
        .cli
        .system_prompt_addendum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn summarize_provider_credential(
    provider: &mvp::config::ProviderConfig,
) -> Option<OnboardingCredentialSummary> {
    if secret_ref_has_inline_literal(provider.oauth_access_token.as_ref()) {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "authorized via browser OAuth".to_owned(),
        });
    }
    if let Some(configured_env) = render_configured_provider_credential_source_value(provider) {
        return Some(OnboardingCredentialSummary {
            label: "credential source",
            value: configured_env,
        });
    }
    if secret_ref_has_inline_literal(provider.api_key.as_ref()) {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "inline api key".to_owned(),
        });
    }

    let fallback_env_name = if provider.kind == mvp::config::ProviderKind::Openai {
        provider.kind.default_api_key_env().map(str::to_owned)
    } else {
        provider_credential_policy::preferred_provider_credential_env_binding(provider)
            .map(|binding| binding.env_name)
    };
    fallback_env_name
        .as_deref()
        .and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(env_name))
        })
        .map(|credential_env| OnboardingCredentialSummary {
            label: "credential source",
            value: credential_env,
        })
}

pub(super) fn provider_supports_blank_api_key_env(config: &mvp::config::LoongClawConfig) -> bool {
    provider_credential_policy::provider_has_inline_credential(&config.provider)
        || provider_credential_policy::provider_has_configured_credential_env(&config.provider)
}

pub(super) fn prompt_import_candidate_choice(
    candidates: &[ImportCandidate],
    width: usize,
) -> CliResult<Option<usize>> {
    let screen_options = build_starting_point_selection_screen_options(candidates, width);
    let idx = select_screen_option("Starting point", &screen_options, Some("1"))?;
    let selected = screen_options
        .get(idx)
        .ok_or_else(|| format!("starting point selection index {idx} out of range"))?;
    if selected.key == "0" {
        return Ok(None);
    }
    selected
        .key
        .parse::<usize>()
        .map(|value| Some(value - 1))
        .map_err(|error| {
            format!(
                "invalid starting point selection key {}: {error}",
                selected.key
            )
        })
}

pub(super) fn prompt_onboard_shortcut_choice(
    shortcut_kind: OnboardShortcutKind,
) -> CliResult<OnboardShortcutChoice> {
    let options = build_onboard_shortcut_screen_options(shortcut_kind);
    match select_screen_option("Your choice", &options, Some("1"))? {
        0 => Ok(OnboardShortcutChoice::UseShortcut),
        1 => Ok(OnboardShortcutChoice::AdjustSettings),
        idx => Err(format!("shortcut selection index {idx} out of range")),
    }
}

pub fn detect_import_starting_config_with_channel_readiness(
    readiness: ChannelImportReadiness,
) -> mvp::config::LoongClawConfig {
    crate::migration::detect_import_starting_config_with_channel_readiness(to_migration_readiness(
        readiness,
    ))
}

pub(super) fn resolve_channel_import_readiness(
    config: &mvp::config::LoongClawConfig,
) -> ChannelImportReadiness {
    crate::migration::resolve_channel_import_readiness_from_config(config)
}

pub(super) fn default_codex_config_paths() -> Vec<PathBuf> {
    crate::migration::discovery::default_detected_codex_config_paths()
}

pub(super) fn to_migration_readiness(
    readiness: ChannelImportReadiness,
) -> crate::migration::ChannelImportReadiness {
    readiness
}

pub(super) fn import_surface_from_migration(
    surface: crate::migration::ImportSurface,
) -> ImportSurface {
    ImportSurface {
        name: surface.name,
        domain: surface.domain,
        level: match surface.level {
            crate::migration::ImportSurfaceLevel::Ready => ImportSurfaceLevel::Ready,
            crate::migration::ImportSurfaceLevel::Review => ImportSurfaceLevel::Review,
            crate::migration::ImportSurfaceLevel::Blocked => ImportSurfaceLevel::Blocked,
        },
        detail: surface.detail,
    }
}

pub(super) fn import_surface_to_migration(
    surface: &ImportSurface,
) -> crate::migration::ImportSurface {
    crate::migration::ImportSurface {
        name: surface.name,
        domain: surface.domain,
        level: match surface.level {
            ImportSurfaceLevel::Ready => crate::migration::ImportSurfaceLevel::Ready,
            ImportSurfaceLevel::Review => crate::migration::ImportSurfaceLevel::Review,
            ImportSurfaceLevel::Blocked => crate::migration::ImportSurfaceLevel::Blocked,
        },
        detail: surface.detail.clone(),
    }
}

pub(super) fn import_candidate_from_migration(
    candidate: crate::migration::ImportCandidate,
) -> ImportCandidate {
    ImportCandidate {
        source_kind: candidate.source_kind,
        source: candidate.source,
        config: candidate.config,
        surfaces: candidate
            .surfaces
            .into_iter()
            .map(import_surface_from_migration)
            .collect(),
        domains: candidate.domains,
        channel_candidates: candidate.channel_candidates,
        workspace_guidance: candidate.workspace_guidance,
    }
}

pub(super) fn migration_candidate_from_onboard(
    candidate: &ImportCandidate,
) -> crate::migration::ImportCandidate {
    crate::migration::ImportCandidate {
        source_kind: candidate.source_kind,
        source: candidate.source.clone(),
        config: candidate.config.clone(),
        surfaces: candidate
            .surfaces
            .iter()
            .map(import_surface_to_migration)
            .collect(),
        domains: candidate.domains.clone(),
        channel_candidates: candidate.channel_candidates.clone(),
        workspace_guidance: candidate.workspace_guidance.clone(),
    }
}

pub(super) fn migration_candidate_for_onboard_display(
    candidate: &ImportCandidate,
) -> crate::migration::ImportCandidate {
    let mut migration_candidate = migration_candidate_from_onboard(candidate);
    migration_candidate.source =
        onboard_starting_point_label(Some(candidate.source_kind), &candidate.source);
    migration_candidate
}

pub(super) fn onboard_starting_point_label(
    source_kind: Option<crate::migration::ImportSourceKind>,
    source: &str,
) -> String {
    crate::migration::ImportSourceKind::onboarding_label(source_kind, source)
}

pub(super) fn detect_render_width() -> usize {
    mvp::presentation::detect_render_width()
}

pub(super) fn enabled_channel_ids(config: &mvp::config::LoongClawConfig) -> Vec<String> {
    config.enabled_channel_ids()
}

pub fn validate_non_interactive_risk_gate(
    non_interactive: bool,
    accept_risk: bool,
) -> CliResult<()> {
    if non_interactive && !accept_risk {
        return Err(
            "non-interactive onboarding requires --accept-risk (explicit acknowledgement)"
                .to_owned(),
        );
    }
    Ok(())
}

pub fn should_offer_current_setup_shortcut(
    options: &OnboardCommandOptions,
    current_setup_state: crate::migration::CurrentSetupState,
    entry_choice: OnboardEntryChoice,
) -> bool {
    !options.non_interactive
        && entry_choice == OnboardEntryChoice::ContinueCurrentSetup
        && current_setup_state == crate::migration::CurrentSetupState::Healthy
        && !onboard_has_explicit_overrides(options)
}

pub fn should_offer_detected_setup_shortcut(
    options: &OnboardCommandOptions,
    entry_choice: OnboardEntryChoice,
    provider_selection: &crate::migration::ProviderSelectionPlan,
) -> bool {
    !options.non_interactive
        && entry_choice == OnboardEntryChoice::ImportDetectedSetup
        && !provider_selection.requires_explicit_choice
        && !onboard_has_explicit_overrides(options)
}

pub(super) fn resolve_onboard_shortcut_kind(
    options: &OnboardCommandOptions,
    starting_selection: &StartingConfigSelection,
) -> Option<OnboardShortcutKind> {
    let requires_guided_protocol_review =
        crate::onboard_preflight::onboard_acp_backend_requires_guided_review(
            &starting_selection.config,
        );
    if requires_guided_protocol_review {
        return None;
    }

    if should_offer_current_setup_shortcut(
        options,
        starting_selection.current_setup_state,
        starting_selection.entry_choice,
    ) {
        return Some(OnboardShortcutKind::CurrentSetup);
    }
    if should_offer_detected_setup_shortcut(
        options,
        starting_selection.entry_choice,
        &starting_selection.provider_selection,
    ) {
        return Some(OnboardShortcutKind::DetectedSetup);
    }
    None
}

pub(super) fn secret_ref_has_inline_literal(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };

    secret_ref.inline_literal_value().is_some()
}

pub(super) fn onboard_has_explicit_overrides(options: &OnboardCommandOptions) -> bool {
    option_has_non_empty_value(options.provider.as_deref())
        || option_has_non_empty_value(options.model.as_deref())
        || option_has_non_empty_value(options.api_key_env.as_deref())
        || option_has_non_empty_value(options.web_search_provider.as_deref())
        || option_has_non_empty_value(options.web_search_api_key_env.as_deref())
        || option_has_non_empty_value(options.personality.as_deref())
        || option_has_non_empty_value(options.memory_profile.as_deref())
        || option_has_non_empty_value(options.system_prompt.as_deref())
        || option_has_non_empty_value(env::var("LOONGCLAW_WEB_SEARCH_PROVIDER").ok().as_deref())
}

pub(super) fn option_has_non_empty_value(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| !value.trim().is_empty())
}

pub(super) fn load_existing_output_config(
    output_path: &Path,
) -> Option<mvp::config::LoongClawConfig> {
    let path_str = output_path.to_str()?;
    mvp::config::load(Some(path_str))
        .ok()
        .map(|(_, config)| config)
}

pub fn should_skip_config_write(
    existing_config: Option<&mvp::config::LoongClawConfig>,
    draft: &mvp::config::LoongClawConfig,
) -> bool {
    existing_config.is_some_and(|existing| {
        if existing == draft {
            return true;
        }

        match (mvp::config::render(existing), mvp::config::render(draft)) {
            (Ok(existing_rendered), Ok(draft_rendered)) => existing_rendered == draft_rendered,
            _ => false,
        }
    })
}

pub fn parse_provider_kind(raw: &str) -> Option<mvp::config::ProviderKind> {
    mvp::config::ProviderKind::parse(raw)
}

pub fn parse_prompt_personality(raw: &str) -> Option<mvp::prompt::PromptPersonality> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "calm_engineering" | "engineering" | "calm" => {
            Some(mvp::prompt::PromptPersonality::CalmEngineering)
        }
        "friendly_collab" | "friendly" | "collab" => {
            Some(mvp::prompt::PromptPersonality::FriendlyCollab)
        }
        "autonomous_executor" | "autonomous" | "executor" => {
            Some(mvp::prompt::PromptPersonality::AutonomousExecutor)
        }
        _ => None,
    }
}

pub fn parse_memory_profile(raw: &str) -> Option<mvp::config::MemoryProfile> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "window_only" | "window" => Some(mvp::config::MemoryProfile::WindowOnly),
        "window_plus_summary" | "summary" | "summary_window" => {
            Some(mvp::config::MemoryProfile::WindowPlusSummary)
        }
        "profile_plus_window" | "profile" | "profile_window" => {
            Some(mvp::config::MemoryProfile::ProfilePlusWindow)
        }
        _ => None,
    }
}

pub fn provider_default_api_key_env(kind: mvp::config::ProviderKind) -> Option<&'static str> {
    kind.default_api_key_env()
}

pub fn provider_kind_id(kind: mvp::config::ProviderKind) -> &'static str {
    kind.as_str()
}

pub fn provider_kind_display_name(kind: mvp::config::ProviderKind) -> &'static str {
    kind.display_name()
}

pub fn prompt_personality_id(personality: mvp::prompt::PromptPersonality) -> &'static str {
    match personality {
        mvp::prompt::PromptPersonality::CalmEngineering => "calm_engineering",
        mvp::prompt::PromptPersonality::FriendlyCollab => "friendly_collab",
        mvp::prompt::PromptPersonality::AutonomousExecutor => "autonomous_executor",
    }
}

pub fn memory_profile_id(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

pub fn supported_provider_list() -> String {
    mvp::config::ProviderKind::all_sorted()
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn supported_provider_selector_list() -> String {
    let mut selectors = mvp::config::ProviderKind::all_sorted()
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>();
    selectors.push("openai-codex-oauth");
    selectors.join(", ")
}

pub fn supported_personality_list() -> &'static str {
    "calm_engineering, friendly_collab, autonomous_executor"
}

pub fn supported_memory_profile_list() -> &'static str {
    "window_only, window_plus_summary, profile_plus_window"
}

pub(super) fn resolve_write_plan(
    output_path: &Path,
    options: &OnboardCommandOptions,

    context: &OnboardRuntimeContext,
) -> CliResult<ConfigWritePlan> {
    if !output_path.exists() {
        return Ok(ConfigWritePlan {
            force: false,
            backup_path: None,
        });
    }
    if options.force {
        return Ok(ConfigWritePlan {
            force: true,
            backup_path: None,
        });
    }

    if options.non_interactive {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    let existing_path = output_path.display().to_string();
    print_stdout_lines(render_existing_config_write_header_lines_with_style(
        &existing_path,
        context.render_width,
        true,
    ))?;
    let options = build_existing_config_write_screen_options();
    let selected = options
        .get(select_screen_option("Your choice", &options, Some("b"))?)
        .ok_or_else(|| "existing-config write selection out of range".to_owned())?;
    match selected.key.as_str() {
        "o" => Ok(ConfigWritePlan {
            force: true,
            backup_path: None,
        }),
        "b" => Ok(ConfigWritePlan {
            force: true,
            backup_path: Some(resolve_backup_path(output_path)?),
        }),
        "c" => Err("onboarding cancelled: config file already exists".to_owned()),
        key => Err(format!(
            "unexpected existing-config write selection key: {key}"
        )),
    }
}
