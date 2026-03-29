use super::focus::FocusTarget;

pub(crate) fn render_composer_title(focus_target: FocusTarget) -> String {
    match focus_target {
        FocusTarget::Composer => "Composer [focused]".to_owned(),
        FocusTarget::Drawer => "Composer".to_owned(),
    }
}

pub(crate) fn render_composer_text(input: &str, focus_target: FocusTarget) -> String {
    match focus_target {
        FocusTarget::Composer => format!("> {input}"),
        FocusTarget::Drawer => format!("composer idle: {input}"),
    }
}
