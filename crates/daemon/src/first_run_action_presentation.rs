use loong_app::tui_surface::{TuiActionSpec, TuiSectionSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FirstRunActionGroup {
    GeneralFollowup,
    ContinueSetup,
}

pub(crate) fn build_first_run_action_sections<T>(
    actions: &[T],
    group_for_action: impl Fn(&T) -> FirstRunActionGroup,
    to_action_spec: impl Fn(&T) -> TuiActionSpec,
) -> Vec<TuiSectionSpec> {
    let mut sections = Vec::new();

    if let Some(primary) = actions.first() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: false,
            items: vec![to_action_spec(primary)],
        });
    }

    let mut general_actions = Vec::new();
    let mut setup_actions = Vec::new();
    for action in actions.iter().skip(1) {
        match group_for_action(action) {
            FirstRunActionGroup::GeneralFollowup => general_actions.push(to_action_spec(action)),
            FirstRunActionGroup::ContinueSetup => setup_actions.push(to_action_spec(action)),
        }
    }

    if !general_actions.is_empty() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("also available".to_owned()),
            inline_title_when_wide: false,
            items: general_actions,
        });
    }

    if !setup_actions.is_empty() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("continue setup".to_owned()),
            inline_title_when_wide: false,
            items: setup_actions,
        });
    }

    sections
}
