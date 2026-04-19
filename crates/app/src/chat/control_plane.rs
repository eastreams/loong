use crate::CliResult;
#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{ApprovalRequestRecord, SessionRepository};

pub(crate) const CHAT_SESSION_KIND_DELEGATE_CHILD: &str = "delegate_child";
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChatControlPlaneApprovalSummary {
    pub(crate) approval_request_id: String,
    pub(crate) status: String,
    pub(crate) tool_name: String,
    pub(crate) turn_id: String,
    pub(crate) requested_at: i64,
    pub(crate) reason: Option<String>,
    pub(crate) rule_id: Option<String>,
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChatControlPlaneSessionSummary {
    pub(crate) session_id: String,
    pub(crate) label: String,
    pub(crate) state: String,
    pub(crate) kind: String,
    pub(crate) parent_session_id: Option<String>,
    pub(crate) turn_count: usize,
    pub(crate) updated_at: i64,
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChatControlPlaneSessionDetails {
    pub(crate) lineage_root_session_id: Option<String>,
    pub(crate) lineage_depth: usize,
    pub(crate) trajectory_turn_count: usize,
    pub(crate) event_count: usize,
    pub(crate) approval_count: usize,
    pub(crate) terminal_status: Option<String>,
    pub(crate) terminal_recorded_at: Option<i64>,
    pub(crate) last_turn_role: Option<String>,
    pub(crate) last_turn_excerpt: Option<String>,
    pub(crate) last_turn_ts: Option<i64>,
    pub(crate) recent_events: Vec<String>,
    pub(crate) delegate_events: Vec<String>,
}

#[cfg(feature = "memory-sqlite")]
pub(crate) struct ChatControlPlaneStore {
    repo: SessionRepository,
}

#[cfg(not(feature = "memory-sqlite"))]
pub(crate) struct ChatControlPlaneStore;

impl ChatControlPlaneApprovalSummary {
    #[cfg(feature = "memory-sqlite")]
    fn from_record(record: &ApprovalRequestRecord) -> Self {
        let reason = record
            .governance_snapshot_json
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let rule_id = record
            .governance_snapshot_json
            .get("rule_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let status = record.status.as_str().to_owned();

        Self {
            approval_request_id: record.approval_request_id.clone(),
            status,
            tool_name: record.tool_name.clone(),
            turn_id: record.turn_id.clone(),
            requested_at: record.requested_at,
            reason,
            rule_id,
            last_error: record.last_error.clone(),
        }
    }
}

impl ChatControlPlaneSessionSummary {
    #[cfg(feature = "memory-sqlite")]
    fn from_session_summary(summary: &crate::session::repository::SessionSummaryRecord) -> Self {
        let label = match summary.label.as_deref() {
            Some(label) => label.to_owned(),
            None => summary.kind.as_str().to_owned(),
        };
        let state = summary.state.as_str().to_owned();
        let kind = summary.kind.as_str().to_owned();

        Self {
            session_id: summary.session_id.clone(),
            label,
            state,
            kind,
            parent_session_id: summary.parent_session_id.clone(),
            turn_count: summary.turn_count,
            updated_at: summary.updated_at,
            last_error: summary.last_error.clone(),
        }
    }
}

impl ChatControlPlaneStore {
    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn new(memory_config: &MemoryRuntimeConfig) -> CliResult<Self> {
        let repo = SessionRepository::new(memory_config)?;
        Ok(Self { repo })
    }

    #[cfg(not(feature = "memory-sqlite"))]
    pub(crate) fn new<T>(_memory_config: &T) -> CliResult<Self> {
        Err("control plane requires memory-sqlite support".to_owned())
    }

    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn visible_sessions(
        &self,
        scope_session_id: &str,
        limit: usize,
    ) -> CliResult<Vec<ChatControlPlaneSessionSummary>> {
        let visible_sessions = self.repo.list_visible_sessions(scope_session_id)?;
        let limited_sessions = visible_sessions.into_iter().take(limit);
        let mut summaries = Vec::new();

        for session in limited_sessions {
            let summary = ChatControlPlaneSessionSummary::from_session_summary(&session);
            summaries.push(summary);
        }

        Ok(summaries)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    pub(crate) fn visible_sessions(
        &self,
        _scope_session_id: &str,
        _limit: usize,
    ) -> CliResult<Vec<ChatControlPlaneSessionSummary>> {
        Err("control plane requires memory-sqlite support".to_owned())
    }

    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn visible_worker_sessions(
        &self,
        scope_session_id: &str,
        limit: usize,
    ) -> CliResult<Vec<ChatControlPlaneSessionSummary>> {
        let visible_sessions = self.visible_sessions(scope_session_id, usize::MAX)?;
        let mut workers = Vec::new();

        for session in visible_sessions {
            if session.kind != CHAT_SESSION_KIND_DELEGATE_CHILD {
                continue;
            }
            workers.push(session);
            if workers.len() >= limit {
                break;
            }
        }

        Ok(workers)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    pub(crate) fn visible_worker_sessions(
        &self,
        _scope_session_id: &str,
        _limit: usize,
    ) -> CliResult<Vec<ChatControlPlaneSessionSummary>> {
        Err("control plane requires memory-sqlite support".to_owned())
    }

    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn approval_queue(
        &self,
        session_id: &str,
        limit: usize,
    ) -> CliResult<Vec<ChatControlPlaneApprovalSummary>> {
        let records = self
            .repo
            .list_approval_requests_for_session(session_id, None)?;
        let limited_records = records.into_iter().take(limit);
        let mut approvals = Vec::new();

        for record in limited_records {
            let summary = ChatControlPlaneApprovalSummary::from_record(&record);
            approvals.push(summary);
        }

        Ok(approvals)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    pub(crate) fn approval_queue(
        &self,
        _session_id: &str,
        _limit: usize,
    ) -> CliResult<Vec<ChatControlPlaneApprovalSummary>> {
        Err("control plane requires memory-sqlite support".to_owned())
    }

    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn session_details(
        &self,
        session_id: &str,
        include_delegate_lifecycle: bool,
    ) -> CliResult<Option<ChatControlPlaneSessionDetails>> {
        let snapshot_result = self
            .repo
            .load_session_trajectory_read_snapshot(session_id, 12)?;
        let snapshot = match snapshot_result {
            Some(snapshot) => snapshot,
            None => return Ok(None),
        };

        let trajectory_turn_count = snapshot.turns.len();
        let event_count = snapshot.events.len();
        let approval_count = snapshot.approval_requests.len();
        let terminal_status = snapshot
            .terminal_outcome
            .as_ref()
            .map(|outcome| outcome.status.clone());
        let terminal_recorded_at = snapshot
            .terminal_outcome
            .as_ref()
            .map(|outcome| outcome.recorded_at);
        let last_turn_role = snapshot.turns.last().map(|turn| turn.role.clone());
        let last_turn_ts = snapshot.turns.last().map(|turn| turn.ts);
        let last_turn_excerpt = snapshot
            .turns
            .last()
            .map(|turn| truncate_excerpt(turn.content.as_str(), 96));
        let recent_event_lines = snapshot
            .events
            .iter()
            .map(|event| {
                let event_id = event.id;
                let event_kind = event.event_kind.as_str();
                format!("event#{event_id}={event_kind}")
            })
            .collect::<Vec<_>>();
        let recent_events = build_recent_event_lines(recent_event_lines);

        let delegate_events = if include_delegate_lifecycle {
            let lifecycle_events = self.repo.list_delegate_lifecycle_events(session_id)?;
            let delegate_event_lines = lifecycle_events
                .iter()
                .map(|event| {
                    let event_id = event.id;
                    let event_kind = event.event_kind.as_str();
                    format!("delegate_event#{event_id}={event_kind}")
                })
                .collect::<Vec<_>>();
            build_recent_event_lines(delegate_event_lines)
        } else {
            Vec::new()
        };

        let details = ChatControlPlaneSessionDetails {
            lineage_root_session_id: snapshot.lineage_root_session_id,
            lineage_depth: snapshot.lineage_depth,
            trajectory_turn_count,
            event_count,
            approval_count,
            terminal_status,
            terminal_recorded_at,
            last_turn_role,
            last_turn_excerpt,
            last_turn_ts,
            recent_events,
            delegate_events,
        };

        Ok(Some(details))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    pub(crate) fn session_details(
        &self,
        _session_id: &str,
        _include_delegate_lifecycle: bool,
    ) -> CliResult<Option<ChatControlPlaneSessionDetails>> {
        Err("control plane requires memory-sqlite support".to_owned())
    }
}

fn build_recent_event_lines(lines: Vec<String>) -> Vec<String> {
    let reversed_items = lines.into_iter().rev().take(4);
    let mut recent_items = Vec::new();

    for line in reversed_items {
        recent_items.push(line);
    }

    recent_items.reverse();
    recent_items
}

fn truncate_excerpt(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_owned();
    }

    let keep_count = max_chars.saturating_sub(1);
    let mut excerpt = text.chars().take(keep_count).collect::<String>();
    excerpt.push('…');
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_recent_event_lines_keeps_latest_four_in_display_order() {
        let items = [
            "event#1=a".to_owned(),
            "event#2=b".to_owned(),
            "event#3=c".to_owned(),
            "event#4=d".to_owned(),
            "event#5=e".to_owned(),
        ];
        let recent = build_recent_event_lines(items.to_vec());

        assert_eq!(recent.len(), 4);
        assert_eq!(recent[0], "event#2=b");
        assert_eq!(recent[3], "event#5=e");
    }

    #[test]
    fn truncate_excerpt_adds_ellipsis_when_needed() {
        let excerpt = truncate_excerpt("abcdef", 4);
        let short = truncate_excerpt("abc", 4);

        assert_eq!(excerpt, "abc…");
        assert_eq!(short, "abc");
    }
}
