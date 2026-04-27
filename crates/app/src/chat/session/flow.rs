use super::*;

pub(super) fn push_transcript_message(state: &mut SurfaceState, lines: Vec<String>) {
    state.transcript.push(SurfaceEntry { lines });
    state.selected_entry = Some(state.transcript.len().saturating_sub(1));
    state.sticky_bottom = true;
    state.focus = SurfaceFocus::Transcript;
}

pub(super) fn control_plane_unavailable_title(error: &str) -> &'static str {
    if error.contains("memory-sqlite support") {
        "feature unavailable"
    } else {
        "control plane unavailable"
    }
}

pub(super) fn render_control_plane_unavailable_lines_with_width(
    role: &str,
    caption: &str,
    error: &str,
    footer_lines: Vec<String>,
    width: usize,
) -> Vec<String> {
    let detail = format!("{role} unavailable: {error}");
    render_cli_chat_message_spec_with_width(
        &TuiMessageSpec {
            role: role.to_owned(),
            caption: Some(caption.to_owned()),
            sections: vec![TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Warning,
                title: Some(control_plane_unavailable_title(error).to_owned()),
                lines: vec![detail],
            }],
            footer_lines,
        },
        width,
    )
}

pub(super) fn build_review_queue_lines_from_items(
    approval_items: &[ApprovalQueueItemSummary],
) -> Vec<String> {
    if approval_items.is_empty() {
        return vec!["approval queue: empty".to_owned()];
    }

    let total_count = approval_items.len();
    let mut lines = vec![format!("approval queue: {total_count}")];

    for item in approval_items {
        let list_line = item.list_line();
        lines.push(list_line);

        let maybe_reason = item.reason.as_deref();
        if let Some(reason) = maybe_reason {
            lines.push(format!("  reason={reason}"));
        }

        let maybe_rule_id = item.rule_id.as_deref();
        if let Some(rule_id) = maybe_rule_id {
            lines.push(format!("  rule_id={rule_id}"));
        }

        let maybe_last_error = item.last_error.as_deref();
        if let Some(last_error) = maybe_last_error {
            lines.push(format!("  last_error={last_error}"));
        }
    }

    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SurfaceLoopAction {
    Continue,
    Submit,
    RunCommand(String),
    Exit,
}
