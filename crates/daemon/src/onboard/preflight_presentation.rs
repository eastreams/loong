use loong_app::tui_surface::{
    TuiChecklistItemSpec, TuiChecklistStatus, TuiChoiceSpec, TuiHeaderStyle, TuiScreenSpec,
    TuiSectionSpec,
};

use crate::onboard::preflight::{
    OnboardCheck, OnboardCheckLevel, OnboardNonInteractiveWarningPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct OnboardCheckCounts {
    pass: usize,
    warn: usize,
    fail: usize,
}

pub(crate) fn build_preflight_summary_screen_spec(
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
            .push(crate::onboard::presentation::preflight_attention_summary_line().to_owned());

        if let Some(hint) = preflight_attention_hint_line(checks) {
            summary_lines.push(hint.to_owned());
        }
    } else {
        summary_lines.push(crate::onboard::presentation::preflight_green_summary_line().to_owned());
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
                label: crate::onboard::presentation::preflight_continue_label().to_owned(),
                detail_lines: vec![
                    crate::onboard::presentation::preflight_continue_detail().to_owned(),
                ],
                recommended: false,
            },
            TuiChoiceSpec {
                key: "n".to_owned(),
                label: crate::onboard::presentation::preflight_cancel_label().to_owned(),
                detail_lines: vec![
                    crate::onboard::presentation::preflight_cancel_detail().to_owned(),
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
                crate::onboard::presentation::preflight_default_choice_description(),
            ),
        ])
    } else {
        Vec::new()
    };

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(crate::onboard::presentation::preflight_header_title().to_owned()),
        title: Some(crate::onboard::presentation::preflight_section_title().to_owned()),
        progress_line: Some(progress_line.to_owned()),
        intro_lines: summary_lines,
        sections,
        choices,
        footer_lines,
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
        return Some(crate::onboard::presentation::preflight_explicit_model_rerun_hint());
    }

    if checks.iter().any(|check| {
        matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::RequiresExplicitModelWithoutReviewedDefault
        )
    }) {
        return Some(crate::onboard::presentation::preflight_explicit_model_only_rerun_hint());
    }

    None
}
