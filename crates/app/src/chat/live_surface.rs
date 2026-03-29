use super::*;

pub(super) type CliChatLiveSurfaceSink = Arc<dyn Fn(Vec<String>) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatLiveSurfaceSnapshot {
    pub(super) phase: ConversationTurnPhase,
    pub(super) provider_round: Option<usize>,
    pub(super) lane: Option<ExecutionLane>,
    pub(super) tool_call_count: usize,
    pub(super) message_count: Option<usize>,
    pub(super) estimated_tokens: Option<usize>,
    pub(super) draft_preview: Option<String>,
    pub(super) tool_activity_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatLiveToolState {
    tool_call_id: String,
    display_order: usize,
    name: Option<String>,
    args: String,
    status: ConversationTurnToolState,
    detail: Option<String>,
}

impl CliChatLiveToolState {
    fn new(tool_call_id: String, display_order: usize) -> Self {
        Self {
            tool_call_id,
            display_order,
            name: None,
            args: String::new(),
            status: ConversationTurnToolState::Running,
            detail: None,
        }
    }
}

#[derive(Debug, Default)]
struct CliChatLiveSurfaceState {
    latest_phase_event: Option<ConversationTurnPhaseEvent>,
    draft_preview: String,
    tool_states: BTreeMap<String, CliChatLiveToolState>,
    tool_call_index_map: BTreeMap<usize, String>,
    next_tool_display_order: usize,
    total_text_chars_seen: usize,
    last_preview_emit_chars_seen: usize,
    last_emitted_snapshot: Option<CliChatLiveSurfaceSnapshot>,
}

pub(super) struct CliChatLiveSurfaceObserver {
    render_width: usize,
    render_sink: CliChatLiveSurfaceSink,
    state: StdMutex<CliChatLiveSurfaceState>,
}

pub(super) fn build_cli_chat_live_surface_observer(
    render_width: usize,
) -> ConversationTurnObserverHandle {
    let render_sink: CliChatLiveSurfaceSink = Arc::new(|lines| {
        print_rendered_cli_chat_lines(&lines);
    });
    let observer = CliChatLiveSurfaceObserver::new(render_width, render_sink);
    Arc::new(observer)
}

impl CliChatLiveSurfaceObserver {
    pub(super) fn new(render_width: usize, render_sink: CliChatLiveSurfaceSink) -> Self {
        Self {
            render_width,
            render_sink,
            state: StdMutex::new(CliChatLiveSurfaceState::default()),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, CliChatLiveSurfaceState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        }
    }

    fn record_phase_event(&self, event: ConversationTurnPhaseEvent) {
        let lines_to_render = {
            let mut state = self.lock_state();
            if cli_chat_live_phase_starts_provider_request(event.phase) {
                reset_cli_chat_live_request_state(&mut state);
            }
            state.latest_phase_event = Some(event.clone());
            reconcile_cli_chat_live_tool_states_for_phase(&mut state.tool_states, event.phase);
            if !should_render_cli_chat_live_phase(event.phase) {
                None
            } else {
                self.prepare_live_surface_lines(&mut state)
            }
        };

        if let Some(lines) = lines_to_render {
            (self.render_sink)(lines);
        }
    }

    fn record_tool_event(&self, event: ConversationTurnToolEvent) {
        let lines_to_render = {
            let mut state = self.lock_state();
            apply_cli_chat_live_tool_event(&mut state, &event, self.render_width);
            let current_phase = match state.latest_phase_event.as_ref() {
                Some(phase_event) => phase_event.phase,
                None => return,
            };
            if should_render_cli_chat_live_phase(current_phase) {
                self.prepare_live_surface_lines(&mut state)
            } else {
                None
            }
        };

        if let Some(lines) = lines_to_render {
            (self.render_sink)(lines);
        }
    }

    fn record_streaming_token_event(&self, event: crate::acp::StreamingTokenEvent) {
        let lines_to_render = {
            let mut state = self.lock_state();
            let current_phase = match state.latest_phase_event.as_ref() {
                Some(phase_event) => phase_event.phase,
                None => return,
            };

            let text_delta = event.delta.text;
            let tool_call_delta = event.delta.tool_call;
            let tool_call_index = event.index;
            let mut should_render = false;

            if let Some(text_delta) = text_delta {
                let preview_char_limit = cli_chat_live_preview_char_limit(self.render_width);
                append_cli_chat_live_buffer(
                    &mut state.draft_preview,
                    text_delta.as_str(),
                    preview_char_limit,
                );
                let delta_chars = text_delta.chars().count();
                state.total_text_chars_seen =
                    state.total_text_chars_seen.saturating_add(delta_chars);

                if should_emit_cli_chat_live_preview(&state, self.render_width)
                    && phase_supports_cli_chat_live_preview(current_phase)
                {
                    should_render = true;
                }
            }

            let tool_call_update = match (tool_call_delta, tool_call_index) {
                (Some(tool_call_delta), Some(index)) => Some((tool_call_delta, index)),
                (Some(_), None) | (None, Some(_)) | (None, None) => None,
            };

            if let Some((tool_call_delta, index)) = tool_call_update {
                update_cli_chat_live_tool_state(
                    &mut state,
                    index,
                    &tool_call_delta,
                    self.render_width,
                );

                let render_tool_activity_now = event.event_type == "tool_call_start"
                    && current_phase == ConversationTurnPhase::RunningTools;
                if render_tool_activity_now {
                    should_render = true;
                }
            }

            if should_render {
                self.prepare_live_surface_lines(&mut state)
            } else {
                None
            }
        };

        if let Some(lines) = lines_to_render {
            (self.render_sink)(lines);
        }
    }

    fn prepare_live_surface_lines(
        &self,
        state: &mut CliChatLiveSurfaceState,
    ) -> Option<Vec<String>> {
        let snapshot = build_cli_chat_live_surface_snapshot(state)?;
        if state.last_emitted_snapshot.as_ref() == Some(&snapshot) {
            return None;
        }

        let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, self.render_width);
        state.last_preview_emit_chars_seen = state.total_text_chars_seen;
        state.last_emitted_snapshot = Some(snapshot);
        Some(lines)
    }
}

impl ConversationTurnObserver for CliChatLiveSurfaceObserver {
    fn on_phase(&self, event: ConversationTurnPhaseEvent) {
        self.record_phase_event(event);
    }

    fn on_tool(&self, event: ConversationTurnToolEvent) {
        self.record_tool_event(event);
    }

    fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
        self.record_streaming_token_event(event);
    }
}

fn cli_chat_live_phase_starts_provider_request(phase: ConversationTurnPhase) -> bool {
    matches!(
        phase,
        ConversationTurnPhase::RequestingProvider
            | ConversationTurnPhase::RequestingFollowupProvider
    )
}

fn reset_cli_chat_live_request_state(state: &mut CliChatLiveSurfaceState) {
    state.draft_preview.clear();
    state.tool_states.clear();
    state.tool_call_index_map.clear();
    state.next_tool_display_order = 0;
    state.total_text_chars_seen = 0;
    state.last_preview_emit_chars_seen = 0;
}

fn should_render_cli_chat_live_phase(phase: ConversationTurnPhase) -> bool {
    match phase {
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed
        | ConversationTurnPhase::Failed => true,
        ConversationTurnPhase::ContextReady => false,
    }
}

fn phase_supports_cli_chat_live_preview(phase: ConversationTurnPhase) -> bool {
    match phase {
        ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RequestingFollowupProvider => true,
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed
        | ConversationTurnPhase::Failed => false,
    }
}

fn should_emit_cli_chat_live_preview(state: &CliChatLiveSurfaceState, render_width: usize) -> bool {
    if state.total_text_chars_seen == 0 {
        return false;
    }

    if state.last_preview_emit_chars_seen == 0 {
        return true;
    }

    let emit_stride = cli_chat_live_preview_emit_stride(render_width);
    let unseen_chars = state
        .total_text_chars_seen
        .saturating_sub(state.last_preview_emit_chars_seen);
    unseen_chars >= emit_stride
}

fn cli_chat_live_preview_emit_stride(render_width: usize) -> usize {
    let doubled_width = render_width.saturating_mul(2);
    doubled_width.clamp(
        CLI_CHAT_LIVE_PREVIEW_MIN_EMIT_CHARS,
        CLI_CHAT_LIVE_PREVIEW_MAX_EMIT_CHARS,
    )
}

fn cli_chat_live_preview_char_limit(render_width: usize) -> usize {
    let expanded_width = render_width.saturating_mul(16);
    expanded_width.clamp(
        CLI_CHAT_LIVE_PREVIEW_MIN_BUFFER_CHARS,
        CLI_CHAT_LIVE_PREVIEW_MAX_BUFFER_CHARS,
    )
}

fn cli_chat_live_tool_args_char_limit(render_width: usize) -> usize {
    let expanded_width = render_width.saturating_mul(8);
    expanded_width.clamp(
        CLI_CHAT_LIVE_TOOL_ARGS_MIN_BUFFER_CHARS,
        CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS,
    )
}

fn append_cli_chat_live_buffer(buffer: &mut String, chunk: &str, char_limit: usize) {
    buffer.push_str(chunk);
    trim_cli_chat_live_buffer(buffer, char_limit);
}

fn trim_cli_chat_live_buffer(buffer: &mut String, char_limit: usize) {
    let current_char_count = buffer.chars().count();
    if current_char_count <= char_limit {
        return;
    }

    let retained_char_count = char_limit.saturating_sub(1);
    let skipped_char_count = current_char_count.saturating_sub(retained_char_count);
    let trimmed_tail = buffer.chars().skip(skipped_char_count).collect::<String>();

    buffer.clear();
    buffer.push('…');
    buffer.push_str(trimmed_tail.as_str());
}

fn truncate_cli_chat_live_text(value: &str, char_limit: usize) -> String {
    let mut truncated = value.to_owned();
    trim_cli_chat_live_buffer(&mut truncated, char_limit);
    truncated
}

fn cli_chat_live_pending_tool_call_id(index: usize) -> String {
    format!("pending-stream-tool-{index}")
}

fn ensure_cli_chat_live_tool_state<'a>(
    state: &'a mut CliChatLiveSurfaceState,
    tool_call_id: &str,
) -> &'a mut CliChatLiveToolState {
    let tool_call_key = tool_call_id.to_owned();
    let entry = state.tool_states.entry(tool_call_key.clone());

    match entry {
        std::collections::btree_map::Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
        std::collections::btree_map::Entry::Vacant(vacant_entry) => {
            let display_order = state.next_tool_display_order;
            let tool_state = CliChatLiveToolState::new(tool_call_key, display_order);
            state.next_tool_display_order = state.next_tool_display_order.saturating_add(1);
            vacant_entry.insert(tool_state)
        }
    }
}

fn merge_cli_chat_live_pending_tool_state(
    state: &mut CliChatLiveSurfaceState,
    pending_tool_call_id: &str,
    tool_call_id: &str,
) {
    if pending_tool_call_id == tool_call_id {
        return;
    }

    let pending_state = match state.tool_states.remove(pending_tool_call_id) {
        Some(pending_state) => pending_state,
        None => return,
    };
    let target_state = ensure_cli_chat_live_tool_state(state, tool_call_id);

    if target_state.name.is_none() {
        target_state.name = pending_state.name;
    }
    if target_state.args.is_empty() {
        target_state.args = pending_state.args;
    }
    if target_state.detail.is_none() {
        target_state.detail = pending_state.detail;
    }
    if target_state.status == ConversationTurnToolState::Running {
        target_state.status = pending_state.status;
    }
}

fn update_cli_chat_live_tool_state(
    state: &mut CliChatLiveSurfaceState,
    index: usize,
    delta: &crate::acp::ToolCallDelta,
    render_width: usize,
) {
    let pending_tool_call_id = cli_chat_live_pending_tool_call_id(index);
    let tool_call_id = delta.id.clone().unwrap_or_else(|| {
        state
            .tool_call_index_map
            .get(&index)
            .cloned()
            .unwrap_or_else(|| pending_tool_call_id.clone())
    });
    let args_char_limit = cli_chat_live_tool_args_char_limit(render_width);

    state
        .tool_call_index_map
        .insert(index, tool_call_id.clone());
    merge_cli_chat_live_pending_tool_state(
        state,
        pending_tool_call_id.as_str(),
        tool_call_id.as_str(),
    );

    let tool_state = ensure_cli_chat_live_tool_state(state, tool_call_id.as_str());
    tool_state.status = ConversationTurnToolState::Running;
    tool_state.detail = None;

    if let Some(name) = delta.name.as_ref() {
        tool_state.name = Some(name.clone());
    }

    if let Some(args) = delta.args.as_ref() {
        append_cli_chat_live_buffer(&mut tool_state.args, args.as_str(), args_char_limit);
    }
}

fn apply_cli_chat_live_tool_event(
    state: &mut CliChatLiveSurfaceState,
    event: &ConversationTurnToolEvent,
    render_width: usize,
) {
    let tool_state = ensure_cli_chat_live_tool_state(state, event.tool_call_id.as_str());
    let detail_char_limit = cli_chat_live_tool_args_char_limit(render_width);

    tool_state.name = Some(event.tool_name.clone());
    tool_state.status = event.state;
    tool_state.detail = event
        .detail
        .as_deref()
        .map(|detail| truncate_cli_chat_live_text(detail, detail_char_limit));
}

fn reconcile_cli_chat_live_tool_states_for_phase(
    tool_states: &mut BTreeMap<String, CliChatLiveToolState>,
    phase: ConversationTurnPhase,
) {
    let fallback_status = match phase {
        ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed => Some(ConversationTurnToolState::Completed),
        ConversationTurnPhase::Failed => Some(ConversationTurnToolState::Interrupted),
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools => None,
    };
    let Some(fallback_status) = fallback_status else {
        return;
    };

    for tool_state in tool_states.values_mut() {
        if tool_state.status != ConversationTurnToolState::Running {
            continue;
        }

        tool_state.status = fallback_status;
        if fallback_status == ConversationTurnToolState::Interrupted && tool_state.detail.is_none()
        {
            tool_state.detail =
                Some("turn failed before a terminal tool result was recorded".to_owned());
        }
    }
}

fn build_cli_chat_live_surface_snapshot(
    state: &CliChatLiveSurfaceState,
) -> Option<CliChatLiveSurfaceSnapshot> {
    let phase_event = state.latest_phase_event.as_ref()?;
    let draft_preview = if state.draft_preview.trim().is_empty() {
        None
    } else {
        Some(state.draft_preview.clone())
    };
    let tool_activity_lines = format_cli_chat_live_tool_activity_lines(&state.tool_states);

    Some(CliChatLiveSurfaceSnapshot {
        phase: phase_event.phase,
        provider_round: phase_event.provider_round,
        lane: phase_event.lane,
        tool_call_count: phase_event.tool_call_count,
        message_count: phase_event.message_count,
        estimated_tokens: phase_event.estimated_tokens,
        draft_preview,
        tool_activity_lines,
    })
}

fn format_cli_chat_live_tool_activity_lines(
    tool_states: &BTreeMap<String, CliChatLiveToolState>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut ordered_states = tool_states.values().collect::<Vec<_>>();
    ordered_states.sort_by_key(|tool_state| tool_state.display_order);

    for tool_state in ordered_states {
        let status = tool_state.status.as_str().replace('_', " ");
        let name = tool_state.name.as_deref().unwrap_or("pending");
        let tool_call_id = tool_state.tool_call_id.as_str();
        let tool_line = if let Some(detail) = tool_state.detail.as_deref() {
            format!("[{status}] {name} (id={tool_call_id}) - {detail}")
        } else {
            format!("[{status}] {name} (id={tool_call_id})")
        };
        lines.push(tool_line);

        if !tool_state.args.is_empty() {
            let args_line = format!("args: {}", tool_state.args);
            lines.push(args_line);
        }
    }

    lines
}

pub(super) fn render_cli_chat_live_surface_lines_with_width(
    snapshot: &CliChatLiveSurfaceSnapshot,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_live_surface_message_spec(snapshot);
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_live_surface_message_spec(
    snapshot: &CliChatLiveSurfaceSnapshot,
) -> TuiMessageSpec {
    let phase_tone = cli_chat_live_surface_tone(snapshot.phase);
    let phase_title = cli_chat_live_surface_title(snapshot.phase);
    let phase_detail = cli_chat_live_surface_detail(snapshot);
    let phase_section = TuiSectionSpec::Callout {
        tone: phase_tone,
        title: Some(phase_title.to_owned()),
        lines: vec![phase_detail],
    };
    let pipeline_items = build_cli_chat_live_pipeline_items(snapshot);
    let pipeline_section = TuiSectionSpec::Checklist {
        title: Some("turn pipeline".to_owned()),
        items: pipeline_items,
    };
    let status_items = build_cli_chat_live_status_items(snapshot);
    let mut sections = vec![phase_section, pipeline_section];

    if !status_items.is_empty() {
        let status_section = TuiSectionSpec::KeyValues {
            title: Some("status".to_owned()),
            items: status_items,
        };
        sections.push(status_section);
    }

    if let Some(preview_section) = build_cli_chat_live_preview_section(snapshot) {
        sections.push(preview_section);
    }

    if let Some(tool_section) = build_cli_chat_live_tool_section(snapshot) {
        sections.push(tool_section);
    }

    TuiMessageSpec {
        role: "loongclaw".to_owned(),
        caption: Some("live".to_owned()),
        sections,
        footer_lines: Vec::new(),
    }
}

fn cli_chat_live_surface_tone(phase: ConversationTurnPhase) -> TuiCalloutTone {
    match phase {
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply => TuiCalloutTone::Info,
        ConversationTurnPhase::Completed => TuiCalloutTone::Success,
        ConversationTurnPhase::Failed => TuiCalloutTone::Warning,
    }
}

fn cli_chat_live_surface_title(phase: ConversationTurnPhase) -> &'static str {
    match phase {
        ConversationTurnPhase::Preparing => "assembling context",
        ConversationTurnPhase::ContextReady => "context ready",
        ConversationTurnPhase::RequestingProvider => "querying model",
        ConversationTurnPhase::RunningTools => "running tools",
        ConversationTurnPhase::RequestingFollowupProvider => "requesting follow-up",
        ConversationTurnPhase::FinalizingReply => "finalizing reply",
        ConversationTurnPhase::Completed => "reply ready",
        ConversationTurnPhase::Failed => "turn failed",
    }
}

fn cli_chat_live_surface_detail(snapshot: &CliChatLiveSurfaceSnapshot) -> String {
    match snapshot.phase {
        ConversationTurnPhase::Preparing => {
            "Building the session context and preparing the next provider turn.".to_owned()
        }
        ConversationTurnPhase::ContextReady => {
            "Context is ready for the next provider round.".to_owned()
        }
        ConversationTurnPhase::RequestingProvider => {
            let provider_round = snapshot.provider_round.unwrap_or(1);
            format!("Requesting provider round {provider_round} and waiting for the reply.")
        }
        ConversationTurnPhase::RunningTools => {
            let lane_label = snapshot
                .lane
                .map(format_cli_chat_live_lane)
                .unwrap_or_else(|| "-".to_owned());
            format!(
                "Executing {} tool call(s) in the {lane_label} lane.",
                snapshot.tool_call_count
            )
        }
        ConversationTurnPhase::RequestingFollowupProvider => {
            let provider_round = snapshot.provider_round.unwrap_or(1);
            format!("Sending tool results back for provider round {provider_round}.")
        }
        ConversationTurnPhase::FinalizingReply => {
            "Persisting the assistant reply and finishing after-turn work.".to_owned()
        }
        ConversationTurnPhase::Completed => "The assistant reply is ready.".to_owned(),
        ConversationTurnPhase::Failed => {
            "The turn failed before a stable reply could be finalized.".to_owned()
        }
    }
}

fn build_cli_chat_live_pipeline_items(
    snapshot: &CliChatLiveSurfaceSnapshot,
) -> Vec<TuiChecklistItemSpec> {
    let prepare_item = TuiChecklistItemSpec {
        status: cli_chat_live_prepare_status(snapshot.phase),
        label: "prepare context".to_owned(),
        detail: cli_chat_live_prepare_detail(snapshot.phase),
    };
    let model_item = TuiChecklistItemSpec {
        status: cli_chat_live_model_status(snapshot.phase),
        label: "call model".to_owned(),
        detail: cli_chat_live_model_detail(snapshot),
    };
    let tools_item = TuiChecklistItemSpec {
        status: cli_chat_live_tools_status(snapshot),
        label: "run tools".to_owned(),
        detail: cli_chat_live_tools_detail(snapshot),
    };
    let finalize_item = TuiChecklistItemSpec {
        status: cli_chat_live_finalize_status(snapshot.phase),
        label: "finalize reply".to_owned(),
        detail: cli_chat_live_finalize_detail(snapshot.phase),
    };

    vec![prepare_item, model_item, tools_item, finalize_item]
}

fn cli_chat_live_prepare_status(phase: ConversationTurnPhase) -> TuiChecklistStatus {
    match phase {
        ConversationTurnPhase::Preparing => TuiChecklistStatus::Warn,
        ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed
        | ConversationTurnPhase::Failed => TuiChecklistStatus::Pass,
    }
}

fn cli_chat_live_prepare_detail(phase: ConversationTurnPhase) -> String {
    match phase {
        ConversationTurnPhase::Preparing => "assembling the next turn context".to_owned(),
        ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed
        | ConversationTurnPhase::Failed => "context assembled".to_owned(),
    }
}

fn cli_chat_live_model_status(phase: ConversationTurnPhase) -> TuiChecklistStatus {
    match phase {
        ConversationTurnPhase::Preparing | ConversationTurnPhase::ContextReady => {
            TuiChecklistStatus::Warn
        }
        ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RequestingFollowupProvider => TuiChecklistStatus::Warn,
        ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed => TuiChecklistStatus::Pass,
        ConversationTurnPhase::Failed => TuiChecklistStatus::Fail,
    }
}

fn cli_chat_live_model_detail(snapshot: &CliChatLiveSurfaceSnapshot) -> String {
    match snapshot.phase {
        ConversationTurnPhase::Preparing => "waiting for a provider round".to_owned(),
        ConversationTurnPhase::ContextReady => "provider request is about to start".to_owned(),
        ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RequestingFollowupProvider => {
            let provider_round = snapshot.provider_round.unwrap_or(1);
            format!("provider round {provider_round} in progress")
        }
        ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed => "provider reply resolved".to_owned(),
        ConversationTurnPhase::Failed => "provider step did not finish cleanly".to_owned(),
    }
}

fn cli_chat_live_tools_status(snapshot: &CliChatLiveSurfaceSnapshot) -> TuiChecklistStatus {
    let tools_needed = snapshot.tool_call_count > 0;
    if !tools_needed {
        return match snapshot.phase {
            ConversationTurnPhase::FinalizingReply
            | ConversationTurnPhase::Completed
            | ConversationTurnPhase::Failed => TuiChecklistStatus::Pass,
            ConversationTurnPhase::Preparing
            | ConversationTurnPhase::ContextReady
            | ConversationTurnPhase::RequestingProvider
            | ConversationTurnPhase::RunningTools
            | ConversationTurnPhase::RequestingFollowupProvider => TuiChecklistStatus::Warn,
        };
    }

    match snapshot.phase {
        ConversationTurnPhase::RunningTools => TuiChecklistStatus::Warn,
        ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed => TuiChecklistStatus::Pass,
        ConversationTurnPhase::Failed => TuiChecklistStatus::Fail,
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider => TuiChecklistStatus::Warn,
    }
}

fn cli_chat_live_tools_detail(snapshot: &CliChatLiveSurfaceSnapshot) -> String {
    let tools_needed = snapshot.tool_call_count > 0;
    if !tools_needed {
        return match snapshot.phase {
            ConversationTurnPhase::FinalizingReply | ConversationTurnPhase::Completed => {
                "no tool calls were needed for this turn".to_owned()
            }
            ConversationTurnPhase::Failed => "no tool step was completed".to_owned(),
            ConversationTurnPhase::Preparing
            | ConversationTurnPhase::ContextReady
            | ConversationTurnPhase::RequestingProvider
            | ConversationTurnPhase::RunningTools
            | ConversationTurnPhase::RequestingFollowupProvider => {
                "waiting to see whether tools are needed".to_owned()
            }
        };
    }

    let lane_label = snapshot
        .lane
        .map(format_cli_chat_live_lane)
        .unwrap_or_else(|| "-".to_owned());
    match snapshot.phase {
        ConversationTurnPhase::RunningTools => {
            format!(
                "{} tool call(s) currently running in the {lane_label} lane",
                snapshot.tool_call_count
            )
        }
        ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Completed => {
            format!(
                "{} tool call(s) finished in the {lane_label} lane",
                snapshot.tool_call_count
            )
        }
        ConversationTurnPhase::Failed => {
            format!(
                "{} tool call(s) did not converge cleanly",
                snapshot.tool_call_count
            )
        }
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider => {
            format!(
                "{} tool call(s) are queued if the provider asks for them",
                snapshot.tool_call_count
            )
        }
    }
}

fn cli_chat_live_finalize_status(phase: ConversationTurnPhase) -> TuiChecklistStatus {
    match phase {
        ConversationTurnPhase::FinalizingReply => TuiChecklistStatus::Warn,
        ConversationTurnPhase::Completed => TuiChecklistStatus::Pass,
        ConversationTurnPhase::Failed => TuiChecklistStatus::Fail,
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::RequestingFollowupProvider => TuiChecklistStatus::Warn,
    }
}

fn cli_chat_live_finalize_detail(phase: ConversationTurnPhase) -> String {
    match phase {
        ConversationTurnPhase::FinalizingReply => {
            "persisting reply state and final runtime side effects".to_owned()
        }
        ConversationTurnPhase::Completed => "reply finalized".to_owned(),
        ConversationTurnPhase::Failed => "reply finalization did not complete".to_owned(),
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::ContextReady
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::RequestingFollowupProvider => {
            "waiting for a final reply".to_owned()
        }
    }
}

fn build_cli_chat_live_status_items(snapshot: &CliChatLiveSurfaceSnapshot) -> Vec<TuiKeyValueSpec> {
    let mut items = Vec::new();

    items.push(TuiKeyValueSpec::Plain {
        key: "phase".to_owned(),
        value: snapshot.phase.as_str().to_owned(),
    });

    if let Some(provider_round) = snapshot.provider_round {
        items.push(TuiKeyValueSpec::Plain {
            key: "round".to_owned(),
            value: provider_round.to_string(),
        });
    }

    if let Some(lane) = snapshot.lane {
        items.push(TuiKeyValueSpec::Plain {
            key: "lane".to_owned(),
            value: format_cli_chat_live_lane(lane),
        });
    }

    if snapshot.tool_call_count > 0 {
        items.push(TuiKeyValueSpec::Plain {
            key: "tool calls".to_owned(),
            value: snapshot.tool_call_count.to_string(),
        });
    }

    if let Some(message_count) = snapshot.message_count {
        items.push(TuiKeyValueSpec::Plain {
            key: "context messages".to_owned(),
            value: message_count.to_string(),
        });
    }

    if let Some(estimated_tokens) = snapshot.estimated_tokens {
        items.push(TuiKeyValueSpec::Plain {
            key: "estimated tokens".to_owned(),
            value: estimated_tokens.to_string(),
        });
    }

    items
}

fn format_cli_chat_live_lane(lane: ExecutionLane) -> String {
    match lane {
        ExecutionLane::Fast => "fast".to_owned(),
        ExecutionLane::Safe => "safe".to_owned(),
    }
}

fn build_cli_chat_live_preview_section(
    snapshot: &CliChatLiveSurfaceSnapshot,
) -> Option<TuiSectionSpec> {
    let preview = snapshot.draft_preview.as_ref()?;
    let preview_lines = preview
        .lines()
        .map(|line| line.to_owned())
        .collect::<Vec<_>>();

    Some(TuiSectionSpec::Narrative {
        title: Some("draft preview".to_owned()),
        lines: preview_lines,
    })
}

fn build_cli_chat_live_tool_section(
    snapshot: &CliChatLiveSurfaceSnapshot,
) -> Option<TuiSectionSpec> {
    if snapshot.tool_activity_lines.is_empty() {
        return None;
    }

    Some(TuiSectionSpec::Narrative {
        title: Some("tool activity".to_owned()),
        lines: snapshot.tool_activity_lines.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_phase_still_renders_final_snapshot() {
        assert!(
            should_render_cli_chat_live_phase(ConversationTurnPhase::Completed),
            "completed turns should emit one last live surface snapshot"
        );
    }
}
