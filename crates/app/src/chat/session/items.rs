use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct ApprovalSurfaceSummary {
    pub(super) title: String,
    pub(super) subtitle: Option<String>,
    pub(super) request_items: Vec<String>,
    pub(super) rationale_lines: Vec<String>,
    pub(super) choice_lines: Vec<String>,
    pub(super) footer_lines: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ApprovalQueueItemSummary {
    pub(super) approval_request_id: String,
    pub(super) status: String,
    pub(super) tool_name: String,
    pub(super) raw_tool_name: String,
    pub(super) request_summary: Option<String>,
    pub(super) turn_id: String,
    pub(super) requested_at: i64,
    pub(super) reason: Option<String>,
    pub(super) rule_id: Option<String>,
    pub(super) last_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct WorkerQueueItemSummary {
    pub(super) session_id: String,
    pub(super) label: String,
    pub(super) state: String,
    pub(super) kind: String,
    pub(super) parent_session_id: Option<String>,
    pub(super) turn_count: usize,
    pub(super) updated_at: i64,
    pub(super) last_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SessionQueueItemSummary {
    pub(super) session_id: String,
    pub(super) label: String,
    pub(super) state: String,
    pub(super) kind: String,
    pub(super) parent_session_id: Option<String>,
    pub(super) turn_count: usize,
    pub(super) updated_at: i64,
    pub(super) last_error: Option<String>,
}

impl ApprovalSurfaceSummary {
    pub(super) fn from_screen_spec(screen: &TuiScreenSpec) -> Self {
        let mut request_items = Vec::new();
        let mut rationale_lines = Vec::new();

        for section in &screen.sections {
            match section {
                TuiSectionSpec::KeyValues { items, .. } => {
                    request_items.extend(items.iter().map(|item| match item {
                        TuiKeyValueSpec::Plain { key, value } => format!("{key}: {value}"),
                        TuiKeyValueSpec::Csv { key, values } => {
                            format!("{key}: {}", values.join(", "))
                        }
                    }));
                }
                TuiSectionSpec::Callout { lines, .. } | TuiSectionSpec::Narrative { lines, .. } => {
                    rationale_lines.extend(lines.clone());
                }
                TuiSectionSpec::ActionGroup { .. }
                | TuiSectionSpec::Checklist { .. }
                | TuiSectionSpec::Preformatted { .. } => {}
            }
        }

        let choice_lines = screen
            .choices
            .iter()
            .map(|choice| {
                if choice.recommended {
                    format!("{}: {} (recommended)", choice.key, choice.label)
                } else {
                    format!("{}: {}", choice.key, choice.label)
                }
            })
            .collect::<Vec<_>>();

        Self {
            title: screen
                .title
                .clone()
                .unwrap_or_else(|| "approval".to_owned()),
            subtitle: screen.subtitle.clone(),
            request_items,
            rationale_lines,
            choice_lines,
            footer_lines: screen.footer_lines.clone(),
        }
    }

    pub(super) fn screen_spec(&self) -> TuiScreenSpec {
        let mut sections = Vec::new();
        if !self.rationale_lines.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("reason".to_owned()),
                lines: self.rationale_lines.clone(),
            });
        }
        if !self.request_items.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("request".to_owned()),
                lines: self.request_items.clone(),
            });
        }
        let choices = self
            .choice_lines
            .iter()
            .enumerate()
            .map(|(index, line)| TuiChoiceSpec {
                key: (index + 1).to_string(),
                label: line.clone(),
                detail_lines: Vec::new(),
                recommended: line.contains("(recommended)"),
            })
            .collect::<Vec<_>>();

        TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: self.subtitle.clone(),
            title: Some(self.title.clone()),
            progress_line: None,
            intro_lines: Vec::new(),
            sections,
            choices,
            footer_lines: self.footer_lines.clone(),
        }
    }
}

impl ApprovalQueueItemSummary {
    pub(super) fn from_control_plane_summary(summary: &ChatControlPlaneApprovalSummary) -> Self {
        let request_summary = if summary.request_summary.is_null() {
            None
        } else {
            serde_json::to_string(&summary.request_summary).ok()
        };

        Self {
            approval_request_id: summary.approval_request_id.clone(),
            status: summary.status.clone(),
            tool_name: summary.visible_tool_name.clone(),
            raw_tool_name: summary.tool_name.clone(),
            request_summary,
            turn_id: summary.turn_id.clone(),
            requested_at: summary.requested_at,
            reason: summary.reason.clone(),
            rule_id: summary.rule_id.clone(),
            last_error: summary.last_error.clone(),
        }
    }

    pub(super) fn list_line(&self) -> String {
        let reason = self.reason.as_deref().unwrap_or("-");
        let mut line = format!(
            "{} status={} tool={} reason={}",
            self.approval_request_id, self.status, self.tool_name, reason
        );
        if let Some(request_summary) = self.request_summary.as_deref() {
            line.push_str(" request=");
            line.push_str(request_summary);
        }
        line
    }

    pub(super) fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("approval_request_id={}", self.approval_request_id),
            format!("status={}", self.status),
            format!("tool_name={}", self.tool_name),
            format!("turn_id={}", self.turn_id),
            format!("requested_at={}", self.requested_at),
        ];
        if self.raw_tool_name != self.tool_name {
            lines.push(format!("raw_tool_name={}", self.raw_tool_name));
        }
        if let Some(request_summary) = self.request_summary.as_deref() {
            lines.push(format!("request_summary={request_summary}"));
        }
        if let Some(reason) = self.reason.as_deref() {
            lines.push(format!("reason={reason}"));
        }
        if let Some(rule_id) = self.rule_id.as_deref() {
            lines.push(format!("rule_id={rule_id}"));
        }
        if let Some(last_error) = self.last_error.as_deref() {
            lines.push(format!("last_error={last_error}"));
        }
        lines
    }
}

impl WorkerQueueItemSummary {
    pub(super) fn from_control_plane_summary(summary: &ChatControlPlaneSessionSummary) -> Self {
        Self {
            session_id: summary.session_id.clone(),
            label: summary.label.clone(),
            state: summary.state.clone(),
            kind: summary.kind.clone(),
            parent_session_id: summary.parent_session_id.clone(),
            turn_count: summary.turn_count,
            updated_at: summary.updated_at,
            last_error: summary.last_error.clone(),
        }
    }

    pub(super) fn list_line(&self) -> String {
        format!(
            "{} state={} kind={} turns={}",
            self.label, self.state, self.kind, self.turn_count
        )
    }

    pub(super) fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("session_id={}", self.session_id),
            format!("label={}", self.label),
            format!("state={}", self.state),
            format!("kind={}", self.kind),
            format!("turn_count={}", self.turn_count),
            format!("updated_at={}", self.updated_at),
        ];
        if let Some(parent_session_id) = self.parent_session_id.as_deref() {
            lines.push(format!("parent_session_id={parent_session_id}"));
        }
        if let Some(last_error) = self.last_error.as_deref() {
            lines.push(format!("last_error={last_error}"));
        }
        lines
    }
}

impl SessionQueueItemSummary {
    pub(super) fn from_control_plane_summary(summary: &ChatControlPlaneSessionSummary) -> Self {
        Self {
            session_id: summary.session_id.clone(),
            label: summary.label.clone(),
            state: summary.state.clone(),
            kind: summary.kind.clone(),
            parent_session_id: summary.parent_session_id.clone(),
            turn_count: summary.turn_count,
            updated_at: summary.updated_at,
            last_error: summary.last_error.clone(),
        }
    }

    pub(super) fn list_line(&self) -> String {
        format!(
            "{} state={} kind={} turns={}",
            self.label, self.state, self.kind, self.turn_count
        )
    }

    pub(super) fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("session_id={}", self.session_id),
            format!("label={}", self.label),
            format!("state={}", self.state),
            format!("kind={}", self.kind),
            format!("turn_count={}", self.turn_count),
            format!("updated_at={}", self.updated_at),
        ];
        if let Some(parent_session_id) = self.parent_session_id.as_deref() {
            lines.push(format!("parent_session_id={parent_session_id}"));
        }
        if let Some(last_error) = self.last_error.as_deref() {
            lines.push(format!("last_error={last_error}"));
        }
        lines
    }
}
