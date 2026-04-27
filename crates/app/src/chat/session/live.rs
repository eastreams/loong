use super::*;

struct SurfaceLiveObserver {
    state: Arc<Mutex<SurfaceState>>,
    term: Term,
}

fn live_surface_content_width(term: &Term, state: &SurfaceState) -> usize {
    let (_, width_u16) = term.size();
    let total_width = usize::from(width_u16);
    let sidebar_visible = state.sidebar_visible && total_width >= MIN_SIDEBAR_TOTAL_WIDTH;
    let sidebar_width = if sidebar_visible { SIDEBAR_WIDTH } else { 0 };

    total_width
        .saturating_sub(sidebar_width)
        .saturating_sub(if sidebar_visible { 3 } else { 2 })
        .max(24)
}

impl ConversationTurnObserver for SurfaceLiveObserver {
    fn on_phase(&self, event: ConversationTurnPhaseEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };

        if cli_chat_live_phase_starts_provider_request(event.phase) {
            reset_cli_chat_live_request_state(&mut state.live.state);
        }

        state.live.state.latest_phase_event = Some(event.clone());
        reconcile_cli_chat_live_tool_states_for_phase(
            &mut state.live.state.tool_states,
            event.phase,
        );
        sync_live_surface_snapshot(&mut state.live);
        state.live.last_phase_label = event.phase.as_str().to_owned();
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }

    fn on_tool(&self, event: ConversationTurnToolEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };
        let render_width = live_surface_content_width(&self.term, &state);

        apply_cli_chat_live_tool_event(&mut state.live.state, &event, render_width);
        sync_live_surface_snapshot(&mut state.live);
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }

    fn on_runtime(&self, event: ConversationTurnRuntimeEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };
        let render_width = live_surface_content_width(&self.term, &state);

        apply_cli_chat_live_runtime_event(&mut state.live.state, &event, render_width);
        sync_live_surface_snapshot(&mut state.live);
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }

    fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };
        let render_width = live_surface_content_width(&self.term, &state);
        let current_phase = state
            .live
            .state
            .latest_phase_event
            .as_ref()
            .map(|phase_event| phase_event.phase);

        if let Some(text_delta) = event.delta.text
            && let Some(current_phase) = current_phase
            && phase_supports_cli_chat_live_preview(current_phase)
        {
            if state.live.state.first_token_latency_ms.is_none() {
                state.live.state.first_token_latency_ms = event.elapsed_ms;
            }
            let preview_char_limit = cli_chat_live_preview_char_limit(render_width);
            state.live.state.total_text_chars_seen = state
                .live
                .state
                .total_text_chars_seen
                .saturating_add(text_delta.chars().count());
            append_cli_chat_live_buffer(
                &mut state.live.state.draft_preview,
                text_delta.as_str(),
                preview_char_limit,
            );
        }

        let tool_call_update = match (event.delta.tool_call, event.index) {
            (Some(tool_call_delta), Some(index)) => Some((tool_call_delta, index)),
            (Some(_), None) | (None, Some(_)) | (None, None) => None,
        };

        if let Some((tool_call_delta, index)) = tool_call_update {
            update_cli_chat_live_tool_state(
                &mut state.live.state,
                index,
                &tool_call_delta,
                render_width,
            );
        }

        sync_live_surface_snapshot(&mut state.live);
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }
}

pub(super) fn build_surface_live_observer(
    state: Arc<Mutex<SurfaceState>>,
    term: Term,
) -> ConversationTurnObserverHandle {
    Arc::new(SurfaceLiveObserver { state, term })
}

pub(super) struct SurfaceRenderData {
    pub(super) header_lines: Vec<String>,
    pub(super) header_status_line: String,
    pub(super) transcript_lines: Vec<String>,
    pub(super) sidebar_visible: bool,
    pub(super) sidebar_tab: SidebarTab,
    pub(super) sidebar_lines: Vec<String>,
    pub(super) composer_lines: Vec<String>,
    pub(super) status_line: String,
}

pub(super) fn render_live_update(term: Term, state: Arc<Mutex<SurfaceState>>) -> CliResult<()> {
    let snapshot_state = match state.lock() {
        Ok(state) => state.clone(),
        Err(poisoned_state) => poisoned_state.into_inner().clone(),
    };
    let (height_u16, width_u16) = term.size();
    let total_height = usize::from(height_u16);
    let total_width = usize::from(width_u16);
    let header_lines = crate::presentation::render_compact_brand_header(
        total_width.saturating_sub(2),
        &crate::presentation::BuildVersionInfo::current(),
        Some(session_subtitle(&snapshot_state)),
    )
    .into_iter()
    .map(|line| line.text)
    .collect::<Vec<_>>();
    let sidebar_visible = snapshot_state.sidebar_visible && total_width >= MIN_SIDEBAR_TOTAL_WIDTH;
    let sidebar_width = if sidebar_visible { SIDEBAR_WIDTH } else { 0 };
    let content_width = total_width
        .saturating_sub(sidebar_width)
        .saturating_sub(if sidebar_visible { 3 } else { 2 })
        .max(24);
    let reserved_height = header_lines.len() + HEADER_GAP + COMPOSER_HEIGHT + STATUS_BAR_HEIGHT + 1;
    let transcript_height = total_height.saturating_sub(reserved_height).max(5);
    let transcript_lines = {
        let mut lines = Vec::new();
        for (entry_index, entry) in snapshot_state.transcript.iter().enumerate() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            for (line_index, line) in entry.lines.iter().enumerate() {
                let clipped = clipped_display_line(line, content_width.saturating_sub(2));
                if line_index == 0 && snapshot_state.selected_entry == Some(entry_index) {
                    lines.push(format!("▶ {clipped}"));
                } else {
                    lines.push(clipped);
                }
            }
        }
        if snapshot_state.pending_turn {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(
                render_cli_chat_live_surface_lines_with_width(
                    &snapshot_state
                        .live
                        .snapshot
                        .clone()
                        .unwrap_or_else(fallback_live_surface_snapshot),
                    content_width,
                )
                .into_iter()
                .map(|line| clipped_display_line(&line, content_width)),
            );
        }
        if lines.len() > transcript_height {
            let start = lines.len().saturating_sub(transcript_height);
            lines.into_iter().skip(start).collect()
        } else {
            lines
        }
    };
    let startup_summary = snapshot_state
        .startup_summary
        .clone()
        .unwrap_or_else(|| fallback_startup_summary("default"));
    let mut sidebar_lines = vec![
        format!("session: {}", startup_summary.session_id),
        format!("focus: {}", snapshot_state.focus.label()),
        format!("sticky: {}", snapshot_state.sticky_bottom),
        format!("phase: {}", snapshot_state.live.last_phase_label),
    ];
    if let Some(preview) = snapshot_state.live.last_assistant_preview.as_deref() {
        sidebar_lines.push(String::new());
        sidebar_lines.push("last reply".to_owned());
        sidebar_lines.extend(
            crate::presentation::render_wrapped_display_line(
                preview,
                SIDEBAR_WIDTH.saturating_sub(4),
            )
            .into_iter()
            .take(8),
        );
    }
    let draft_lines = composer_display_lines(
        &composer_text_with_cursor(&snapshot_state.composer, snapshot_state.composer_cursor),
        total_width.saturating_sub(6),
        2,
    );
    let composer_lines = vec![
        format!("draft · focus={}", snapshot_state.focus.label()),
        draft_lines.first().cloned().unwrap_or_default(),
        if draft_lines.len() > 1 {
            draft_lines.get(1).cloned().unwrap_or_default()
        } else if let Some(hint) = slash_command_hint(&snapshot_state.composer) {
            hint
        } else {
            "turn running…".to_owned()
        },
        "Enter send · ? help · : or / command menu".to_owned(),
    ];
    let mut status_text = format!(
        "?: help · : command menu · M mission · Esc clear · PgUp/PgDn transcript · Tab focus · focus={} · deck={} · sticky={}",
        snapshot_state.focus.label(),
        snapshot_state.sidebar_tab.title(),
        snapshot_state.sticky_bottom
    );
    if snapshot_state.pending_turn {
        status_text.push_str(" · turn running");
    }
    let render_data = SurfaceRenderData {
        header_lines,
        header_status_line: clipped_display_line(
            format!(
                "session={} · provider={} · phase={} · focus={} · overlay={}",
                startup_summary.session_id,
                snapshot_state.active_provider_label,
                snapshot_state.live.last_phase_label,
                snapshot_state.focus.label(),
                current_overlay_label(&snapshot_state)
            )
            .as_str(),
            total_width.saturating_sub(4),
        ),
        transcript_lines,
        sidebar_visible,
        sidebar_tab: snapshot_state.sidebar_tab,
        sidebar_lines,
        composer_lines,
        status_line: clipped_display_line(status_text.as_str(), total_width.saturating_sub(4)),
    };
    let output = render_surface_to_string(
        &snapshot_state,
        &render_data,
        Rect::new(0, 0, width_u16, height_u16),
    );
    term.write_str(format!("{CLEAR_AND_HOME}{output}").as_str())
        .map_err(|error| format!("failed to refresh live surface: {error}"))?;
    term.flush()
        .map_err(|error| format!("failed to flush live surface: {error}"))?;
    Ok(())
}
