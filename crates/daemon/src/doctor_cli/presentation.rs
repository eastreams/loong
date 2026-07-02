use std::path::Path;

use loong_app as mvp;

use crate::doctor_cli::{DoctorCheck, DoctorCheckLevel};

#[derive(Debug, Clone, Copy)]
pub(crate) struct DoctorSummary {
    pub(crate) pass: usize,
    pub(crate) warn: usize,
    pub(crate) fail: usize,
}

pub(crate) fn summarize_checks(checks: &[DoctorCheck]) -> DoctorSummary {
    let mut pass = 0_usize;
    let mut warn = 0_usize;
    let mut fail = 0_usize;
    for check in checks {
        match check.level {
            DoctorCheckLevel::Pass => pass += 1,
            DoctorCheckLevel::Warn => warn += 1,
            DoctorCheckLevel::Fail => fail += 1,
        }
    }
    DoctorSummary { pass, warn, fail }
}

pub(crate) fn render_doctor_text(
    checks: &[DoctorCheck],
    summary: DoctorSummary,
    fixes: &[String],
    next_steps: &[String],
    config_path: &Path,
    fix_requested: bool,
) -> String {
    let mut sections = Vec::new();
    sections.push(mvp::tui_surface::TuiSectionSpec::Callout {
        tone: if summary.fail == 0 {
            mvp::tui_surface::TuiCalloutTone::Success
        } else {
            mvp::tui_surface::TuiCalloutTone::Warning
        },
        title: Some("summary".to_owned()),
        lines: vec![format!(
            "{} ok · {} warn · {} fail",
            summary.pass, summary.warn, summary.fail
        )],
    });
    sections.push(mvp::tui_surface::TuiSectionSpec::Checklist {
        title: Some("checks".to_owned()),
        items: checks
            .iter()
            .map(|check| mvp::tui_surface::TuiChecklistItemSpec {
                status: match check.level {
                    DoctorCheckLevel::Pass => mvp::tui_surface::TuiChecklistStatus::Pass,
                    DoctorCheckLevel::Warn => mvp::tui_surface::TuiChecklistStatus::Warn,
                    DoctorCheckLevel::Fail => mvp::tui_surface::TuiChecklistStatus::Fail,
                },
                label: check.name.clone(),
                detail: check.detail.clone(),
            })
            .collect(),
    });

    let action_items = next_steps
        .iter()
        .filter_map(|step| {
            let (label, command) = step.split_once(": ")?;
            Some(mvp::tui_surface::TuiActionSpec {
                label: label.to_owned(),
                command: command.to_owned(),
            })
        })
        .take(3)
        .collect::<Vec<_>>();
    if !action_items.is_empty() {
        sections.push(mvp::tui_surface::TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: false,
            items: action_items,
        });
    }
    if fix_requested {
        let fix_lines = if fixes.is_empty() {
            vec!["applied fixes: none".to_owned()]
        } else {
            fixes.iter().map(|fix| format!("- {fix}")).collect()
        };
        sections.push(mvp::tui_surface::TuiSectionSpec::Narrative {
            title: Some("applied fixes".to_owned()),
            lines: fix_lines,
        });
    }
    if !next_steps.is_empty() {
        sections.push(mvp::tui_surface::TuiSectionSpec::Narrative {
            title: Some("next actions".to_owned()),
            lines: next_steps.iter().map(|step| format!("- {step}")).collect(),
        });
    }

    let screen = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some("runtime health".to_owned()),
        title: Some("doctor".to_owned()),
        progress_line: None,
        intro_lines: vec![format!("config={}", config_path.display())],
        sections,
        choices: Vec::new(),
        footer_lines: vec![
            "Use `loong doctor --json` for machine-readable diagnostics.".to_owned(),
        ],
    };

    mvp::tui_surface::render_tui_screen_spec_ratatui(
        &screen,
        mvp::presentation::detect_render_width(),
        false,
    )
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn render_doctor_text_keeps_summary_actions_and_footer() {
        let checks = vec![DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "missing".to_owned(),
        }];
        let summary = summarize_checks(&checks);
        let rendered = render_doctor_text(
            &checks,
            summary,
            &[],
            &["Run diagnostics: loong doctor --config /tmp/loong.toml".to_owned()],
            Path::new("/tmp/loong.toml"),
            false,
        );

        assert!(rendered.contains("summary"));
        assert!(rendered.contains("checks"));
        assert!(rendered.contains("start here"));
        assert!(rendered.contains("next actions"));
        assert!(rendered.contains("Use `loong doctor --json` for machine-readable diagnostics."));
    }
}
