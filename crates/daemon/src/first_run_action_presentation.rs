use loong_app::tui_surface::{TuiActionSpec, TuiSectionSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FirstRunActionGroup {
    GeneralFollowup,
    ContinueSetup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FirstRunActionSections<T> {
    pub(crate) primary: Option<T>,
    pub(crate) general_followups: Vec<T>,
    pub(crate) continue_setup: Vec<T>,
}

pub(crate) fn partition_first_run_actions<T>(
    actions: &[T],
    group_for_action: impl Fn(&T) -> FirstRunActionGroup,
) -> FirstRunActionSections<&T> {
    let primary = actions.first();
    let mut general_followups = Vec::new();
    let mut continue_setup = Vec::new();

    for action in actions.iter().skip(1) {
        match group_for_action(action) {
            FirstRunActionGroup::GeneralFollowup => general_followups.push(action),
            FirstRunActionGroup::ContinueSetup => continue_setup.push(action),
        }
    }

    FirstRunActionSections {
        primary,
        general_followups,
        continue_setup,
    }
}

pub(crate) fn build_first_run_action_sections<T>(
    actions: &[T],
    group_for_action: impl Fn(&T) -> FirstRunActionGroup,
    to_action_spec: impl Fn(&T) -> TuiActionSpec,
) -> Vec<TuiSectionSpec> {
    let mut sections = Vec::new();
    let grouped = partition_first_run_actions(actions, group_for_action);

    if let Some(primary) = grouped.primary {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: false,
            items: vec![to_action_spec(primary)],
        });
    }

    if !grouped.general_followups.is_empty() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("also available".to_owned()),
            inline_title_when_wide: false,
            items: grouped
                .general_followups
                .into_iter()
                .map(&to_action_spec)
                .collect(),
        });
    }

    if !grouped.continue_setup.is_empty() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("continue setup".to_owned()),
            inline_title_when_wide: false,
            items: grouped
                .continue_setup
                .into_iter()
                .map(&to_action_spec)
                .collect(),
        });
    }

    sections
}

pub(crate) fn build_first_run_action_text_lines<T>(
    actions: &[T],
    width: usize,
    group_for_action: impl Fn(&T) -> FirstRunActionGroup,
    render_primary: impl Fn(&T, usize) -> Vec<String>,
    render_secondary: impl Fn(&T, usize) -> Vec<String>,
) -> Vec<String> {
    let grouped = partition_first_run_actions(actions, group_for_action);
    let mut lines = Vec::new();

    if let Some(primary) = grouped.primary {
        lines.push("start here".to_owned());
        lines.extend(render_primary(primary, width));
    }

    if !grouped.general_followups.is_empty() {
        lines.push("also available".to_owned());
        for action in grouped.general_followups {
            lines.extend(render_secondary(action, width));
        }
    }

    if !grouped.continue_setup.is_empty() {
        lines.push("continue setup".to_owned());
        for action in grouped.continue_setup {
            lines.extend(render_secondary(action, width));
        }
    }

    lines
}
