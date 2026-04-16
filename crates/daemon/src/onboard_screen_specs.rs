use super::*;

pub(super) fn render_onboarding_risk_screen_lines_with_style(
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let copy = crate::onboard_presentation::risk_screen_copy();
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
                    "Loong can invoke tools and read local files when enabled.".to_owned(),
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
    config: &mvp::config::LoongConfig,
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
    let status_line =
        crate::onboard_presentation::write_confirmation_status_line(warnings_kept).to_owned();

    intro_lines.push(config_line);
    intro_lines.push(status_line);

    let choices = vec![
        TuiChoiceSpec {
            key: "y".to_owned(),
            label: crate::onboard_presentation::write_confirmation_label().to_owned(),
            detail_lines: vec![crate::onboard_presentation::write_confirmation_detail().to_owned()],
            recommended: false,
        },
        TuiChoiceSpec {
            key: "n".to_owned(),
            label: crate::onboard_presentation::write_confirmation_cancel_label().to_owned(),
            detail_lines: vec![
                crate::onboard_presentation::write_confirmation_cancel_detail().to_owned(),
            ],
            recommended: false,
        },
    ];

    let default_choice_line = render_default_choice_footer_line(
        "y",
        crate::onboard_presentation::write_confirmation_default_choice_description(),
    );
    let footer_lines = append_escape_cancel_hint(vec![default_choice_line]);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: None,
        title: Some(crate::onboard_presentation::write_confirmation_title().to_owned()),
        progress_line: Some(flow_style.progress_line()),
        intro_lines,
        sections: Vec::new(),
        choices,
        footer_lines,
    }
}
