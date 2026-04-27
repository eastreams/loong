use super::*;

impl ChatSessionSurface {
    pub(super) fn control_plane_store(&self) -> CliResult<ChatControlPlaneStore> {
        ChatControlPlaneStore::new(&self.runtime.memory_config)
    }

    pub(super) fn try_build_mission_control_lines(
        &self,
        state: &SurfaceState,
        session_limit: usize,
        worker_limit: usize,
        review_limit: usize,
    ) -> CliResult<Vec<String>> {
        let visible_sessions =
            self.load_visible_sessions(session_limit.saturating_mul(2).max(8))?;
        let worker_items = visible_sessions
            .iter()
            .filter(|item| item.kind == CHAT_SESSION_KIND_DELEGATE_CHILD)
            .take(worker_limit)
            .cloned()
            .collect::<Vec<_>>();
        let approval_items = self.load_review_queue_items(review_limit)?;
        let maybe_snapshot = state.live.snapshot.as_ref();
        let phase = maybe_snapshot
            .map(|snapshot| snapshot.phase.as_str())
            .unwrap_or("idle");
        let provider_round = maybe_snapshot
            .and_then(|snapshot| snapshot.provider_round)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned());
        let tool_calls = maybe_snapshot
            .map(|snapshot| snapshot.tool_call_count)
            .unwrap_or(0);

        let visible_session_count = visible_sessions.len();
        let delegate_count = visible_sessions
            .iter()
            .filter(|item| item.kind == CHAT_SESSION_KIND_DELEGATE_CHILD)
            .count();
        let root_count = visible_session_count.saturating_sub(delegate_count);
        let failing_sessions = visible_sessions
            .iter()
            .filter(|item| item.state == "failed" || item.state == "timed_out")
            .count();

        let mut lines = vec![
            format!("scope: {}", self.runtime.session_id),
            format!("provider: {}", state.active_provider_label),
            format!("phase: {phase} · round={provider_round} · tools={tool_calls}"),
            format!(
                "lanes: sessions={} · roots={} · delegates={} · approvals={}",
                visible_session_count,
                root_count,
                delegate_count,
                approval_items.len()
            ),
        ];

        let session_state_mix =
            summarize_state_mix(visible_sessions.iter().map(|item| item.state.as_str()));
        if let Some(state_mix) = session_state_mix {
            lines.push(format!("session mix: {state_mix}"));
        }

        let worker_state_mix =
            summarize_state_mix(worker_items.iter().map(|item| item.state.as_str()));
        if let Some(worker_mix) = worker_state_mix {
            lines.push(format!("worker mix: {worker_mix}"));
        }

        if failing_sessions > 0 {
            lines.push(format!("attention: failing lanes={failing_sessions}"));
        }

        let recent_sessions = visible_sessions.iter().take(session_limit);
        let recent_session_lines = recent_sessions
            .map(SessionQueueItemSummary::list_line)
            .collect::<Vec<_>>();
        if !recent_session_lines.is_empty() {
            lines.push(String::new());
            lines.push("recent sessions".to_owned());
            lines.extend(recent_session_lines);
        }

        if !worker_items.is_empty() {
            lines.push(String::new());
            lines.push("recent workers".to_owned());
            lines.extend(worker_items.iter().map(SessionQueueItemSummary::list_line));
        }

        if !approval_items.is_empty() {
            lines.push(String::new());
            lines.push("review queue".to_owned());
            lines.extend(
                approval_items
                    .iter()
                    .take(review_limit)
                    .map(ApprovalQueueItemSummary::list_line),
            );
        }

        let maybe_approval = state.last_approval.as_ref();
        if let Some(approval) = maybe_approval {
            lines.push(String::new());
            lines.push(format!("latest approval: {}", approval.title));
            let maybe_subtitle = approval.subtitle.as_deref();
            if let Some(subtitle) = maybe_subtitle {
                lines.push(format!("mode: {subtitle}"));
            }
        }

        lines.push(String::new());
        lines.push("controls".to_owned());
        lines.push("S sessions · W workers · R approval queue".to_owned());
        lines.push("r latest approval · M mission control".to_owned());
        Ok(lines)
    }

    pub(super) fn build_mission_control_lines(
        &self,
        state: &SurfaceState,
        session_limit: usize,
        worker_limit: usize,
        review_limit: usize,
    ) -> Vec<String> {
        match self.try_build_mission_control_lines(state, session_limit, worker_limit, review_limit)
        {
            Ok(lines) => lines,
            Err(error) => vec![format!("control_plane_unavailable={error}")],
        }
    }

    pub(super) fn build_review_queue_lines(&self, limit: usize) -> Vec<String> {
        match self.load_review_queue_items(usize::MAX) {
            Ok(approval_items) => {
                if approval_items.is_empty() {
                    return vec!["approval queue: empty".to_owned()];
                }

                let total_count = approval_items.len();
                let mut lines = vec![format!("approval queue: {total_count}")];
                for item in approval_items.iter().take(limit) {
                    let list_line = item.list_line();
                    lines.push(list_line);

                    if let Some(reason) = item.reason.as_deref() {
                        lines.push(format!("  reason={reason}"));
                    }
                    if let Some(rule_id) = item.rule_id.as_deref() {
                        lines.push(format!("  rule_id={rule_id}"));
                    }
                    if let Some(last_error) = item.last_error.as_deref() {
                        lines.push(format!("  last_error={last_error}"));
                    }
                }
                lines
            }
            Err(error) => vec![format!("approval queue unavailable: {error}")],
        }
    }

    pub(super) fn build_worker_queue_lines(&self, limit: usize) -> Vec<String> {
        match self.load_visible_worker_sessions(usize::MAX) {
            Ok(items) => {
                if items.is_empty() {
                    vec!["worker sessions: empty".to_owned()]
                } else {
                    let total_count = items.len();
                    let limited_items = items.into_iter().take(limit);
                    let mut lines = vec![format!("worker sessions: {total_count}")];
                    for item in limited_items {
                        let list_line = item.list_line();
                        lines.push(list_line);
                    }
                    lines
                }
            }
            Err(error) => {
                let error_line = format!("worker sessions unavailable: {error}");
                vec![error_line]
            }
        }
    }

    pub(super) fn build_session_detail_lines(&self, item: &SessionQueueItemSummary) -> Vec<String> {
        let base_lines = item.detail_lines();
        let session_id = item.session_id.as_str();
        let include_delegate_lifecycle = false;

        self.build_session_detail_lines_with_runtime(
            session_id,
            base_lines,
            include_delegate_lifecycle,
        )
    }

    pub(super) fn build_worker_detail_lines(&self, item: &WorkerQueueItemSummary) -> Vec<String> {
        let base_lines = item.detail_lines();
        let session_id = item.session_id.as_str();
        let include_delegate_lifecycle = true;

        self.build_session_detail_lines_with_runtime(
            session_id,
            base_lines,
            include_delegate_lifecycle,
        )
    }

    pub(super) fn build_session_detail_lines_with_runtime(
        &self,
        session_id: &str,
        mut lines: Vec<String>,
        include_delegate_lifecycle: bool,
    ) -> Vec<String> {
        let store_result = self.control_plane_store();
        let store = match store_result {
            Ok(store) => store,
            Err(error) => {
                let detail_line = format!("detail_runtime_unavailable={error}");
                lines.push(detail_line);
                return lines;
            }
        };

        let details_result = store.session_details(session_id, include_delegate_lifecycle);
        let maybe_details = match details_result {
            Ok(details) => details,
            Err(error) => {
                let detail_line = format!("trajectory_unavailable={error}");
                lines.push(detail_line);
                return lines;
            }
        };

        let details = match maybe_details {
            Some(details) => details,
            None => {
                lines.push("trajectory_unavailable=session_not_found".to_owned());
                return lines;
            }
        };

        let maybe_lineage_root = details.lineage_root_session_id.as_deref();
        let lineage_root = maybe_lineage_root.unwrap_or("-");
        let turn_count = details.trajectory_turn_count;
        let event_count = details.event_count;
        let approval_count = details.approval_count;

        lines.push(String::new());
        lines.push(format!("lineage_root_session_id={lineage_root}"));
        lines.push(format!("lineage_depth={}", details.lineage_depth));
        lines.push(format!("trajectory_turn_count={turn_count}"));
        lines.push(format!("trajectory_event_count={event_count}"));
        lines.push(format!("approval_request_count={approval_count}"));

        let maybe_terminal_status = details.terminal_status.as_deref();
        let maybe_terminal_recorded_at = details.terminal_recorded_at;
        if let Some(terminal_status) = maybe_terminal_status {
            lines.push(format!("terminal_status={terminal_status}"));
        }
        if let Some(terminal_recorded_at) = maybe_terminal_recorded_at {
            lines.push(format!("terminal_recorded_at={terminal_recorded_at}"));
        }

        let maybe_last_turn_role = details.last_turn_role.as_deref();
        let maybe_last_turn_ts = details.last_turn_ts;
        let maybe_last_turn_excerpt = details.last_turn_excerpt.as_deref();
        if let Some(last_turn_role) = maybe_last_turn_role {
            lines.push(format!("last_turn_role={last_turn_role}"));
        }
        if let Some(last_turn_ts) = maybe_last_turn_ts {
            lines.push(format!("last_turn_ts={last_turn_ts}"));
        }
        if let Some(last_turn_excerpt) = maybe_last_turn_excerpt {
            lines.push(format!("last_turn_excerpt={last_turn_excerpt}"));
        }

        if !details.recent_events.is_empty() {
            lines.push(String::new());
            lines.push("recent_events".to_owned());
            lines.extend(details.recent_events);
        }

        let approval_items_result = store.approval_queue(session_id, 1);
        let approval_items = approval_items_result.unwrap_or_default();
        let maybe_latest_approval = approval_items.first();
        if let Some(latest_approval) = maybe_latest_approval {
            let approval_id = latest_approval.approval_request_id.as_str();
            let approval_status = latest_approval.status.as_str();
            let approval_tool = latest_approval.tool_name.as_str();
            lines.push(String::new());
            lines.push(format!("latest_approval_id={approval_id}"));
            lines.push(format!("latest_approval_status={approval_status}"));
            lines.push(format!("latest_approval_tool={approval_tool}"));
        }

        if !details.delegate_events.is_empty() {
            lines.push(String::new());
            lines.push("delegate_lifecycle".to_owned());
            lines.extend(details.delegate_events);
        }

        lines
    }

    pub(super) fn load_review_queue_items(
        &self,
        limit: usize,
    ) -> CliResult<Vec<ApprovalQueueItemSummary>> {
        let store = self.control_plane_store()?;

        let approvals = store.approval_queue(&self.runtime.session_id, limit)?;

        let mut items = Vec::new();
        for approval in approvals {
            let item = ApprovalQueueItemSummary::from_control_plane_summary(&approval);
            items.push(item);
        }
        Ok(items)
    }
}
