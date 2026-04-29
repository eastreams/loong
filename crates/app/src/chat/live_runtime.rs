use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::chat_surface::markdown;
use super::chat_surface::utils::compact_structured_preview;
use super::*;
use serde_json::Value;

const CLI_CHAT_LIVE_PREVIEW_MIN_EMIT_CHARS: usize = 8;
const CLI_CHAT_LIVE_PREVIEW_MAX_EMIT_CHARS: usize = 48;
const CLI_CHAT_LIVE_PREVIEW_INITIAL_EMIT_CHARS: usize = 4;
const CLI_CHAT_LIVE_PREVIEW_MAX_BUFFER_CHARS: usize = 4096;
const CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS: usize = 1024;
const CLI_CHAT_LIVE_OUTPUT_MAX_BUFFER_CHARS: usize = 1536;
const CLI_CHAT_LIVE_OUTPUT_RENDER_MAX_LINES: usize = 4;
const CLI_CHAT_LIVE_DIFF_PREVIEW_MAX_LINES: usize = 6;
const CLI_CHAT_LIVE_PREVIEW_CATCH_UP_ENTER_VISUAL_LINE_GROWTH: usize = 2;
const CLI_CHAT_LIVE_PREVIEW_CATCH_UP_ENTER_MIN_VISUAL_LINES: usize = 4;
const CLI_CHAT_LIVE_PREVIEW_SMOOTH_MIN_INTERVAL_MS: u64 = 40;
const CLI_CHAT_LIVE_PREVIEW_CATCH_UP_MIN_INTERVAL_MS: u64 = 16;
pub(super) type CliChatLiveSurfaceSink = Arc<dyn Fn(Vec<String>) + Send + Sync>;
pub(super) type CliChatLiveSurfaceRerender = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliChatLiveSurfaceRenderMode {
    Card,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliChatLivePreviewEmitMode {
    Smooth,
    CatchUp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatLiveOutputView {
    pub text: String,
    pub total_bytes: usize,
    pub total_lines: usize,
    pub truncated: bool,
}

impl CliChatLiveOutputView {
    fn new() -> Self {
        Self {
            text: String::new(),
            total_bytes: 0,
            total_lines: 0,
            truncated: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatLiveFileChangeView {
    pub path: String,
    pub operation: ToolFileChangeKind,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatLiveToolSnapshot {
    pub tool_call_id: String,
    pub name: Option<String>,
    pub request_summary: Option<String>,
    pub args: String,
    pub status: ConversationTurnToolState,
    pub detail: Option<String>,
    pub stdout: CliChatLiveOutputView,
    pub stderr: CliChatLiveOutputView,
    pub file_change: Option<CliChatLiveFileChangeView>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatLiveSurfaceSnapshot {
    pub phase: ConversationTurnPhase,
    pub provider_round: Option<usize>,
    pub lane: Option<ExecutionLane>,
    pub tool_call_count: usize,
    pub message_count: Option<usize>,
    pub estimated_tokens: Option<usize>,
    pub first_token_latency_ms: Option<u64>,
    pub draft_preview: Option<String>,
    pub tools: Vec<CliChatLiveToolSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatLiveToolState {
    pub tool_call_id: String,
    pub display_order: usize,
    pub name: Option<String>,
    pub request_summary: Option<String>,
    pub args: String,
    pub status: ConversationTurnToolState,
    pub detail: Option<String>,
    pub stdout: CliChatLiveOutputView,
    pub stderr: CliChatLiveOutputView,
    pub file_change: Option<CliChatLiveFileChangeView>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

impl CliChatLiveToolState {
    fn new(tool_call_id: String, display_order: usize) -> Self {
        let stdout = CliChatLiveOutputView::new();
        let stderr = CliChatLiveOutputView::new();

        Self {
            tool_call_id,
            display_order,
            name: None,
            request_summary: None,
            args: String::new(),
            status: ConversationTurnToolState::Running,
            detail: None,
            stdout,
            stderr,
            file_change: None,
            duration_ms: None,
            exit_code: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct CliChatLiveSurfaceState {
    pub latest_phase_event: Option<ConversationTurnPhaseEvent>,
    pub first_token_latency_ms: Option<u64>,
    pub draft_preview: String,
    pub tool_states: BTreeMap<String, CliChatLiveToolState>,
    pub tool_call_index_map: BTreeMap<usize, String>,
    pub next_tool_display_order: usize,
    pub total_text_chars_seen: usize,
    pub last_preview_emit_chars_seen: usize,
    pub last_preview_emit_visual_line_count: usize,
    pub last_preview_emit_elapsed_ms: Option<u64>,
    pub last_emitted_snapshot: Option<CliChatLiveSurfaceSnapshot>,
    pub last_emitted_lines: Option<Vec<String>>,
}

pub(super) struct CliChatLiveSurfaceObserver {
    render_width: Arc<AtomicUsize>,
    render_sink: CliChatLiveSurfaceSink,
    render_mode: CliChatLiveSurfaceRenderMode,
    state: StdMutex<CliChatLiveSurfaceState>,
}

pub(super) fn build_cli_chat_live_surface_observer(
    render_width: usize,
) -> ConversationTurnObserverHandle {
    let render_sink: CliChatLiveSurfaceSink = Arc::new(|lines| {
        print_rendered_cli_chat_lines(&lines);
    });
    build_cli_chat_live_surface_observer_with_sink(render_width, render_sink)
}

pub(super) fn build_cli_chat_live_surface_observer_with_sink(
    render_width: usize,
    render_sink: CliChatLiveSurfaceSink,
) -> ConversationTurnObserverHandle {
    build_cli_chat_live_surface_observer_with_dynamic_width_sink(
        Arc::new(AtomicUsize::new(render_width.max(1))),
        render_sink,
    )
}

pub(super) fn build_cli_chat_live_surface_observer_with_dynamic_width_sink(
    render_width: Arc<AtomicUsize>,
    render_sink: CliChatLiveSurfaceSink,
) -> ConversationTurnObserverHandle {
    let observer = CliChatLiveSurfaceObserver::new_with_mode(
        render_width,
        render_sink,
        CliChatLiveSurfaceRenderMode::Card,
    );
    Arc::new(observer)
}

#[allow(dead_code)]
pub(super) fn build_cli_chat_live_compact_observer_with_sink(
    render_width: usize,
    render_sink: CliChatLiveSurfaceSink,
) -> ConversationTurnObserverHandle {
    build_cli_chat_live_compact_observer_with_dynamic_width_sink(
        Arc::new(AtomicUsize::new(render_width.max(1))),
        render_sink,
    )
}

pub(super) fn build_cli_chat_live_compact_observer_with_dynamic_width_sink(
    render_width: Arc<AtomicUsize>,
    render_sink: CliChatLiveSurfaceSink,
) -> ConversationTurnObserverHandle {
    let observer = CliChatLiveSurfaceObserver::new_with_mode(
        render_width,
        render_sink,
        CliChatLiveSurfaceRenderMode::Compact,
    );
    Arc::new(observer)
}

pub(super) fn build_cli_chat_live_compact_observer_controller(
    render_width: Arc<AtomicUsize>,
    render_sink: CliChatLiveSurfaceSink,
) -> (ConversationTurnObserverHandle, CliChatLiveSurfaceRerender) {
    let observer = Arc::new(CliChatLiveSurfaceObserver::new_with_mode(
        render_width,
        render_sink,
        CliChatLiveSurfaceRenderMode::Compact,
    ));
    let rerender_observer = Arc::clone(&observer);
    let rerender: CliChatLiveSurfaceRerender = Arc::new(move || {
        rerender_observer.rerender_current_lines();
    });
    (observer as ConversationTurnObserverHandle, rerender)
}

impl CliChatLiveSurfaceObserver {
    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn new(render_width: usize, render_sink: CliChatLiveSurfaceSink) -> Self {
        Self::new_with_mode(
            Arc::new(AtomicUsize::new(render_width.max(1))),
            render_sink,
            CliChatLiveSurfaceRenderMode::Card,
        )
    }

    fn new_with_mode(
        render_width: Arc<AtomicUsize>,
        render_sink: CliChatLiveSurfaceSink,
        render_mode: CliChatLiveSurfaceRenderMode,
    ) -> Self {
        Self {
            render_width,
            render_sink,
            render_mode,
            state: StdMutex::new(CliChatLiveSurfaceState::default()),
        }
    }

    fn render_width(&self) -> usize {
        self.render_width.load(Ordering::Relaxed).max(1)
    }

    fn rerender_current_lines(&self) {
        let lines_to_render = {
            let mut state = self.lock_state();
            if state.latest_phase_event.is_none() {
                None
            } else {
                build_cli_chat_live_surface_snapshot(&state).and_then(|snapshot| {
                    let lines = match self.render_mode {
                        CliChatLiveSurfaceRenderMode::Card => {
                            render_cli_chat_live_surface_lines_with_width(
                                &snapshot,
                                self.render_width(),
                            )
                        }
                        CliChatLiveSurfaceRenderMode::Compact => {
                            render_cli_chat_live_compact_lines_with_width(
                                &snapshot,
                                self.render_width(),
                            )
                        }
                    };
                    if state.last_emitted_lines.as_ref() == Some(&lines) {
                        state.last_emitted_snapshot = Some(snapshot);
                        return None;
                    }
                    state.last_preview_emit_visual_line_count =
                        cli_chat_live_preview_visual_line_count(
                            snapshot.draft_preview.as_deref(),
                            self.render_width(),
                        );
                    state.last_emitted_snapshot = Some(snapshot);
                    state.last_emitted_lines = Some(lines.clone());
                    Some(lines)
                })
            }
        };

        if let Some(lines) = lines_to_render {
            (self.render_sink)(lines);
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
            let render_width = self.render_width();
            apply_cli_chat_live_tool_event(&mut state, &event, render_width);
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

    fn record_runtime_event(&self, event: ConversationTurnRuntimeEvent) {
        let lines_to_render = {
            let mut state = self.lock_state();
            let render_width = self.render_width();
            apply_cli_chat_live_runtime_event(&mut state, &event, render_width);
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
            let render_width = self.render_width();
            let current_phase = match state.latest_phase_event.as_ref() {
                Some(phase_event) => phase_event.phase,
                None => return,
            };

            let text_delta = event.delta.text;
            let tool_call_delta = event.delta.tool_call;
            let tool_call_index = event.index;
            let mut should_render = false;

            if let Some(text_delta) = text_delta {
                if state.first_token_latency_ms.is_none() {
                    state.first_token_latency_ms = event.elapsed_ms;
                }
                let preview_char_limit = cli_chat_live_preview_char_limit(render_width);
                append_cli_chat_live_buffer(
                    &mut state.draft_preview,
                    text_delta.as_str(),
                    preview_char_limit,
                );
                let delta_chars = text_delta.chars().count();
                state.total_text_chars_seen =
                    state.total_text_chars_seen.saturating_add(delta_chars);

                if (should_emit_cli_chat_live_preview(&state, render_width, event.elapsed_ms)
                    || cli_chat_live_delta_has_commit_boundary(text_delta.as_str()))
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
                update_cli_chat_live_tool_state(&mut state, index, &tool_call_delta, render_width);

                let render_tool_activity_now = event.event_type == "tool_call_start"
                    && current_phase == ConversationTurnPhase::RunningTools;
                if render_tool_activity_now {
                    should_render = true;
                }
            }

            if should_render {
                let rendered = self.prepare_live_surface_lines(&mut state);
                if rendered.is_some() {
                    state.last_preview_emit_elapsed_ms = event.elapsed_ms;
                }
                rendered
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

        let lines = match self.render_mode {
            CliChatLiveSurfaceRenderMode::Card => {
                render_cli_chat_live_surface_lines_with_width(&snapshot, self.render_width())
            }
            CliChatLiveSurfaceRenderMode::Compact => {
                render_cli_chat_live_compact_lines_with_width(&snapshot, self.render_width())
            }
        };
        state.last_preview_emit_chars_seen = state.total_text_chars_seen;
        state.last_preview_emit_visual_line_count = cli_chat_live_preview_visual_line_count(
            snapshot.draft_preview.as_deref(),
            self.render_width(),
        );
        state.last_emitted_snapshot = Some(snapshot);
        if state.last_emitted_lines.as_ref() == Some(&lines) {
            return None;
        }
        state.last_emitted_lines = Some(lines.clone());
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

    fn on_runtime(&self, event: ConversationTurnRuntimeEvent) {
        self.record_runtime_event(event);
    }

    fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
        self.record_streaming_token_event(event);
    }
}

pub(super) fn cli_chat_live_phase_starts_provider_request(phase: ConversationTurnPhase) -> bool {
    matches!(
        phase,
        ConversationTurnPhase::RequestingProvider
            | ConversationTurnPhase::RequestingFollowupProvider
    )
}

pub(super) fn reset_cli_chat_live_request_state(state: &mut CliChatLiveSurfaceState) {
    state.first_token_latency_ms = None;
    state.draft_preview.clear();
    state.tool_states.clear();
    state.tool_call_index_map.clear();
    state.next_tool_display_order = 0;
    state.total_text_chars_seen = 0;
    state.last_preview_emit_chars_seen = 0;
    state.last_preview_emit_visual_line_count = 0;
    state.last_preview_emit_elapsed_ms = None;
    state.last_emitted_snapshot = None;
    state.last_emitted_lines = None;
}

fn should_render_cli_chat_live_phase(phase: ConversationTurnPhase) -> bool {
    match phase {
        ConversationTurnPhase::Preparing
        | ConversationTurnPhase::RequestingProvider
        | ConversationTurnPhase::RunningTools
        | ConversationTurnPhase::RequestingFollowupProvider
        | ConversationTurnPhase::FinalizingReply
        | ConversationTurnPhase::Failed => true,
        ConversationTurnPhase::ContextReady | ConversationTurnPhase::Completed => false,
    }
}

pub(super) fn phase_supports_cli_chat_live_preview(phase: ConversationTurnPhase) -> bool {
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

fn should_emit_cli_chat_live_preview(
    state: &CliChatLiveSurfaceState,
    render_width: usize,
    event_elapsed_ms: Option<u64>,
) -> bool {
    if state.total_text_chars_seen == 0 {
        return false;
    }

    let emit_stride = cli_chat_live_preview_emit_stride(render_width);
    let preview_has_stable_suffix =
        cli_chat_live_preview_has_stable_suffix(state.draft_preview.as_str());
    let preview_has_initial_phrase =
        cli_chat_live_preview_has_initial_phrase_boundary(state.draft_preview.as_str());
    let visual_line_count =
        cli_chat_live_preview_visual_line_count(Some(state.draft_preview.as_str()), render_width);
    let visual_line_growth =
        visual_line_count.saturating_sub(state.last_preview_emit_visual_line_count);
    let emit_mode = cli_chat_live_preview_emit_mode(state, render_width);
    let cadence_ready =
        cli_chat_live_preview_emit_cadence_ready(state, emit_mode, event_elapsed_ms);

    if state.last_preview_emit_chars_seen == 0 {
        return (state.total_text_chars_seen >= CLI_CHAT_LIVE_PREVIEW_INITIAL_EMIT_CHARS
            && preview_has_stable_suffix)
            || (state.total_text_chars_seen
                >= CLI_CHAT_LIVE_PREVIEW_INITIAL_EMIT_CHARS.saturating_mul(2)
                && preview_has_initial_phrase)
            || (visual_line_count >= 2
                && state.total_text_chars_seen
                    >= CLI_CHAT_LIVE_PREVIEW_INITIAL_EMIT_CHARS.saturating_add(1))
            || state.total_text_chars_seen >= emit_stride;
    }

    if !cadence_ready {
        return false;
    }

    let unseen_chars = state
        .total_text_chars_seen
        .saturating_sub(state.last_preview_emit_chars_seen);
    match emit_mode {
        CliChatLivePreviewEmitMode::Smooth => {
            if visual_line_growth >= CLI_CHAT_LIVE_PREVIEW_CATCH_UP_ENTER_VISUAL_LINE_GROWTH
                && unseen_chars >= emit_stride.saturating_div(2).max(1)
            {
                return true;
            }
            (unseen_chars >= emit_stride && preview_has_stable_suffix)
                || unseen_chars >= emit_stride.saturating_mul(2)
        }
        CliChatLivePreviewEmitMode::CatchUp => {
            visual_line_growth >= 1
                || (preview_has_stable_suffix
                    && unseen_chars >= emit_stride.saturating_div(2).max(1))
                || unseen_chars >= emit_stride
        }
    }
}

fn cli_chat_live_preview_emit_cadence_ready(
    state: &CliChatLiveSurfaceState,
    emit_mode: CliChatLivePreviewEmitMode,
    event_elapsed_ms: Option<u64>,
) -> bool {
    let Some(current_elapsed_ms) = event_elapsed_ms else {
        return true;
    };
    let Some(last_emit_elapsed_ms) = state.last_preview_emit_elapsed_ms else {
        return true;
    };

    let min_interval_ms = match emit_mode {
        CliChatLivePreviewEmitMode::Smooth => CLI_CHAT_LIVE_PREVIEW_SMOOTH_MIN_INTERVAL_MS,
        CliChatLivePreviewEmitMode::CatchUp => CLI_CHAT_LIVE_PREVIEW_CATCH_UP_MIN_INTERVAL_MS,
    };

    current_elapsed_ms.saturating_sub(last_emit_elapsed_ms) >= min_interval_ms
}

fn cli_chat_live_preview_emit_mode(
    state: &CliChatLiveSurfaceState,
    render_width: usize,
) -> CliChatLivePreviewEmitMode {
    if state.total_text_chars_seen == 0 {
        return CliChatLivePreviewEmitMode::Smooth;
    }

    let emit_stride = cli_chat_live_preview_emit_stride(render_width);
    let visual_line_count =
        cli_chat_live_preview_visual_line_count(Some(state.draft_preview.as_str()), render_width);
    let visual_line_growth =
        visual_line_count.saturating_sub(state.last_preview_emit_visual_line_count);
    let unseen_chars = state
        .total_text_chars_seen
        .saturating_sub(state.last_preview_emit_chars_seen);

    if visual_line_growth >= CLI_CHAT_LIVE_PREVIEW_CATCH_UP_ENTER_VISUAL_LINE_GROWTH
        || unseen_chars >= emit_stride.saturating_mul(2)
        || (visual_line_count >= CLI_CHAT_LIVE_PREVIEW_CATCH_UP_ENTER_MIN_VISUAL_LINES
            && unseen_chars >= emit_stride.saturating_div(2).max(1))
    {
        CliChatLivePreviewEmitMode::CatchUp
    } else {
        CliChatLivePreviewEmitMode::Smooth
    }
}

fn cli_chat_live_preview_emit_stride(render_width: usize) -> usize {
    render_width.clamp(
        CLI_CHAT_LIVE_PREVIEW_MIN_EMIT_CHARS,
        CLI_CHAT_LIVE_PREVIEW_MAX_EMIT_CHARS,
    )
}

fn cli_chat_live_delta_has_commit_boundary(text_delta: &str) -> bool {
    text_delta.contains('\n')
        || text_delta.contains("<think>")
        || text_delta.contains("</think>")
        || text_delta.contains("```")
}

fn cli_chat_live_preview_has_stable_suffix(preview: &str) -> bool {
    if preview.is_empty() {
        return false;
    }

    if preview.ends_with('\n') || preview.ends_with(' ') || preview.ends_with('\t') {
        return true;
    }

    let trimmed = preview.trim_end_matches([' ', '\t']);
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.ends_with("```") && trimmed.matches("```").count().is_multiple_of(2) {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("<think>") || lower.ends_with("</think>") {
        return true;
    }

    let Some(last_char) = trimmed.chars().last() else {
        return false;
    };

    last_char.is_ascii_punctuation()
        || matches!(
            last_char,
            '，' | '。' | '！' | '？' | '、' | '；' | '：' | '）' | '】' | '」' | '』'
        )
        || live_preview_is_cjk(last_char)
}

fn cli_chat_live_preview_has_initial_phrase_boundary(preview: &str) -> bool {
    let mut saw_token = false;
    let mut saw_separator_after_token = false;

    for character in preview.trim().chars() {
        if character.is_whitespace() {
            if saw_token {
                saw_separator_after_token = true;
            }
            continue;
        }

        if saw_separator_after_token {
            return true;
        }

        saw_token = true;
    }

    false
}

fn cli_chat_live_preview_visual_line_count(preview: Option<&str>, render_width: usize) -> usize {
    let Some(preview) = preview else {
        return 0;
    };
    if preview.trim().is_empty() {
        return 0;
    }

    let wrap_width = render_width.saturating_sub(2).max(1);
    let (thinking_preview, visible_preview) = split_live_preview_text(preview);
    let mut line_count = 0usize;

    if let Some(thinking_preview) = thinking_preview.as_deref() {
        line_count = line_count
            .saturating_add(render_live_preview_segment_lines(thinking_preview, wrap_width).len());
    }

    if thinking_preview.is_some() && visible_preview.is_some() && line_count > 0 {
        line_count = line_count.saturating_add(1);
    }

    if let Some(visible_preview) = visible_preview.as_deref() {
        line_count = line_count
            .saturating_add(render_live_preview_segment_lines(visible_preview, wrap_width).len());
    }

    line_count
}

pub(super) fn cli_chat_live_preview_char_limit(render_width: usize) -> usize {
    let _ = render_width;
    CLI_CHAT_LIVE_PREVIEW_MAX_BUFFER_CHARS
}

fn cli_chat_live_tool_args_char_limit(render_width: usize) -> usize {
    let _ = render_width;
    CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS
}

pub(super) fn append_cli_chat_live_buffer(buffer: &mut String, chunk: &str, char_limit: usize) {
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

pub(super) fn truncate_cli_chat_live_text(value: &str, char_limit: usize) -> String {
    let mut truncated = value.to_owned();
    trim_cli_chat_live_buffer(&mut truncated, char_limit);
    truncated
}

fn cli_chat_live_output_char_limit(render_width: usize) -> usize {
    let _ = render_width;
    CLI_CHAT_LIVE_OUTPUT_MAX_BUFFER_CHARS
}

fn cli_chat_live_line_count(value: &str) -> usize {
    if value.is_empty() {
        return 0;
    }

    let line_break_count = value.chars().filter(|ch| *ch == '\n').count();
    line_break_count.saturating_add(1)
}

fn push_cli_chat_live_output_lines(
    lines: &mut Vec<String>,
    label: &str,
    output: &CliChatLiveOutputView,
    max_lines: usize,
) {
    if output.text.trim().is_empty() {
        return;
    }

    lines.push(format!(
        "  ↳ {label} {} lines · {} bytes",
        output.total_lines, output.total_bytes
    ));

    let output_lines = output
        .text
        .lines()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>();
    let output_lines = output_lines.into_iter().rev().collect::<Vec<_>>();

    for output_line in output_lines {
        let rendered_line =
            truncate_cli_chat_live_text(output_line, CLI_CHAT_LIVE_OUTPUT_MAX_BUFFER_CHARS);
        lines.push(format!("    {rendered_line}"));
    }

    if output.truncated {
        lines.push("    … live output truncated".to_owned());
    }
}

pub(super) fn cli_chat_live_pending_tool_call_id(index: usize) -> String {
    format!("pending-stream-tool-{index}")
}

pub(super) fn ensure_cli_chat_live_tool_state<'a>(
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

pub(super) fn merge_cli_chat_live_pending_tool_state(
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

pub(super) fn update_cli_chat_live_tool_state(
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

pub(super) fn apply_cli_chat_live_tool_event(
    state: &mut CliChatLiveSurfaceState,
    event: &ConversationTurnToolEvent,
    render_width: usize,
) {
    let tool_state = ensure_cli_chat_live_tool_state(state, event.tool_call_id.as_str());
    let detail_char_limit = cli_chat_live_tool_args_char_limit(render_width);

    tool_state.name = Some(event.tool_name.clone());
    if let Some(request_summary) = event.request_summary.as_deref() {
        let truncated_summary =
            truncate_cli_chat_live_text(request_summary, CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS);
        tool_state.request_summary = Some(truncated_summary);
    }
    tool_state.status = event.state;
    tool_state.detail = event
        .detail
        .as_deref()
        .map(|detail| truncate_cli_chat_live_text(detail, detail_char_limit));
}

pub(super) fn apply_cli_chat_live_runtime_event(
    state: &mut CliChatLiveSurfaceState,
    event: &ConversationTurnRuntimeEvent,
    render_width: usize,
) {
    let tool_state = ensure_cli_chat_live_tool_state(state, event.tool_call_id.as_str());

    match &event.event {
        ToolRuntimeEvent::OutputDelta(delta) => {
            let output_char_limit = cli_chat_live_output_char_limit(render_width);
            let target_output = match delta.stream {
                ToolRuntimeStream::Stdout => &mut tool_state.stdout,
                ToolRuntimeStream::Stderr => &mut tool_state.stderr,
            };
            let chunk = delta.chunk.as_str();
            let fallback_line_count = cli_chat_live_line_count(chunk);
            let total_lines = if delta.total_lines == 0 {
                fallback_line_count
            } else {
                delta.total_lines
            };

            target_output.total_bytes = delta.total_bytes;
            target_output.total_lines = total_lines;
            target_output.truncated = delta.truncated;
            append_cli_chat_live_buffer(&mut target_output.text, chunk, output_char_limit);
        }
        ToolRuntimeEvent::FileChangePreview(file_change) => {
            let preview = file_change.preview.as_deref();
            let preview = preview.map(|preview| {
                let preview_limit = cli_chat_live_output_char_limit(render_width);
                truncate_cli_chat_live_text(preview, preview_limit)
            });
            let file_change_view = CliChatLiveFileChangeView {
                path: file_change.path.clone(),
                operation: file_change.kind,
                added_lines: file_change.added_lines,
                removed_lines: file_change.removed_lines,
                preview,
            };
            tool_state.file_change = Some(file_change_view);
        }
        ToolRuntimeEvent::CommandMetrics(metrics) => {
            tool_state.duration_ms = Some(metrics.duration_ms);
            tool_state.exit_code = metrics.exit_code;
        }
    }
}

pub(super) fn reconcile_cli_chat_live_tool_states_for_phase(
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

pub(super) fn build_cli_chat_live_surface_snapshot(
    state: &CliChatLiveSurfaceState,
) -> Option<CliChatLiveSurfaceSnapshot> {
    let phase_event = state.latest_phase_event.as_ref()?;
    let draft_preview = if state.draft_preview.trim().is_empty() {
        None
    } else {
        Some(state.draft_preview.clone())
    };
    let tools = build_cli_chat_live_tool_snapshots(&state.tool_states);

    Some(CliChatLiveSurfaceSnapshot {
        phase: phase_event.phase,
        provider_round: phase_event.provider_round,
        lane: phase_event.lane,
        tool_call_count: phase_event.tool_call_count,
        message_count: phase_event.message_count,
        estimated_tokens: phase_event.estimated_tokens,
        first_token_latency_ms: state.first_token_latency_ms,
        draft_preview,
        tools,
    })
}

fn build_cli_chat_live_tool_snapshots(
    tool_states: &BTreeMap<String, CliChatLiveToolState>,
) -> Vec<CliChatLiveToolSnapshot> {
    let mut ordered_states = tool_states.values().collect::<Vec<_>>();
    ordered_states.sort_by_key(|tool_state| tool_state.display_order);

    let mut snapshots = Vec::with_capacity(ordered_states.len());
    for tool_state in ordered_states {
        let snapshot = CliChatLiveToolSnapshot {
            tool_call_id: tool_state.tool_call_id.clone(),
            name: tool_state
                .name
                .as_deref()
                .map(crate::tools::user_visible_tool_name),
            request_summary: tool_state.request_summary.clone(),
            args: tool_state.args.clone(),
            status: tool_state.status,
            detail: tool_state.detail.clone(),
            stdout: tool_state.stdout.clone(),
            stderr: tool_state.stderr.clone(),
            file_change: tool_state.file_change.clone(),
            duration_ms: tool_state.duration_ms,
            exit_code: tool_state.exit_code,
        };
        snapshots.push(snapshot);
    }

    snapshots
}

pub(super) fn format_cli_chat_live_tool_activity_lines(
    tool_snapshots: &[CliChatLiveToolSnapshot],
) -> Vec<String> {
    let mut lines = Vec::new();

    for tool_snapshot in tool_snapshots {
        let name = tool_snapshot.name.as_deref().unwrap_or("pending");
        let tool_line = format_cli_chat_live_tool_headline(tool_snapshot, name);
        lines.push(tool_line);

        if let Some(primary_request_line) =
            format_cli_chat_live_primary_request_line(tool_snapshot, name)
        {
            lines.push(primary_request_line);
        }

        let request_preview = tool_snapshot
            .request_summary
            .as_deref()
            .map(format_cli_chat_live_structured_preview);
        let args_preview = (!tool_snapshot.args.is_empty())
            .then(|| format_cli_chat_live_structured_preview(tool_snapshot.args.as_str()));

        if let Some(request_preview) = request_preview.as_deref() {
            let request_line = if args_preview.as_deref() == Some(request_preview) {
                format!("  ↳ request {request_preview}")
            } else if tool_snapshot.request_summary.as_deref() == Some(request_preview) {
                format!("  ↳ {request_preview}")
            } else {
                format!("  ↳ request {request_preview}")
            };
            lines.push(request_line);
        }

        if let Some(args_preview) = args_preview.as_deref()
            && request_preview.as_deref() != Some(args_preview)
        {
            let args_line = format!("  ↳ args {args_preview}");
            lines.push(args_line);
        }

        push_cli_chat_live_output_lines(
            &mut lines,
            "stdout",
            &tool_snapshot.stdout,
            CLI_CHAT_LIVE_OUTPUT_RENDER_MAX_LINES,
        );
        push_cli_chat_live_output_lines(
            &mut lines,
            "stderr",
            &tool_snapshot.stderr,
            CLI_CHAT_LIVE_OUTPUT_RENDER_MAX_LINES,
        );

        if let Some(file_change) = tool_snapshot.file_change.as_ref() {
            let operation = match file_change.operation {
                ToolFileChangeKind::Create => "create",
                ToolFileChangeKind::Overwrite => "overwrite",
                ToolFileChangeKind::Edit => "edit",
            };
            let path = truncate_cli_chat_live_text(
                file_change.path.as_str(),
                CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS,
            );
            let summary_line = format!(
                "  ↳ file {operation} {path} (+{} / -{})",
                file_change.added_lines, file_change.removed_lines,
            );
            lines.push(summary_line);

            if let Some(preview) = file_change.preview.as_deref() {
                let preview_lines = preview.lines().take(CLI_CHAT_LIVE_DIFF_PREVIEW_MAX_LINES);
                for preview_line in preview_lines {
                    let preview_line = truncate_cli_chat_live_text(
                        preview_line,
                        CLI_CHAT_LIVE_OUTPUT_MAX_BUFFER_CHARS,
                    );
                    lines.push(format!("  {preview_line}"));
                }
            }
        }

        if let Some(duration_ms) = tool_snapshot.duration_ms {
            let metrics_line = if let Some(exit_code) = tool_snapshot.exit_code {
                format!("  ↳ metrics {duration_ms}ms · exit={exit_code}")
            } else {
                format!("  ↳ metrics {duration_ms}ms")
            };
            lines.push(metrics_line);
        }
    }

    lines
}

fn format_cli_chat_live_tool_headline(
    tool_snapshot: &CliChatLiveToolSnapshot,
    name: &str,
) -> String {
    let prefix = match tool_snapshot.status {
        ConversationTurnToolState::Running => "• Called",
        ConversationTurnToolState::Completed
        | ConversationTurnToolState::Failed
        | ConversationTurnToolState::Interrupted => "• Closed",
        ConversationTurnToolState::NeedsApproval => "• Approval",
        ConversationTurnToolState::Denied => "• Denied",
    };

    if let Some(detail) = tool_snapshot.detail.as_deref() {
        format!("{prefix} {name} · {detail}")
    } else {
        format!("{prefix} {name}")
    }
}

fn format_cli_chat_live_structured_preview(text: &str) -> String {
    compact_structured_preview(text, 3).unwrap_or_else(|| {
        truncate_cli_chat_live_text(text, CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS)
    })
}

fn format_cli_chat_live_primary_request_line(
    tool_snapshot: &CliChatLiveToolSnapshot,
    name: &str,
) -> Option<String> {
    let normalized_name = normalize_cli_chat_live_tool_name(name);

    if is_cli_chat_live_read_tool(normalized_name.as_str())
        && let Some(path) = cli_chat_live_read_request_display(tool_snapshot)
    {
        return Some(format!("  ↳ Read {path}"));
    }

    if is_cli_chat_live_run_tool(normalized_name.as_str())
        && let Some(command) = cli_chat_live_request_command(tool_snapshot)
    {
        let command =
            truncate_cli_chat_live_text(command.as_str(), CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS);
        return Some(format!("  ↳ Command {command}"));
    }

    if is_cli_chat_live_search_tool(normalized_name.as_str())
        && let Some(summary) = cli_chat_live_search_request_display(tool_snapshot)
    {
        return Some(format!("  ↳ Search {summary}"));
    }

    if is_cli_chat_live_list_tool(normalized_name.as_str())
        && let Some(path) = cli_chat_live_request_path(tool_snapshot)
    {
        let path =
            truncate_cli_chat_live_text(path.as_str(), CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS);
        return Some(format!("  ↳ List {path}"));
    }

    if is_cli_chat_live_glob_tool(normalized_name.as_str())
        && let Some(summary) = cli_chat_live_glob_request_display(tool_snapshot)
    {
        return Some(format!("  ↳ Glob {summary}"));
    }

    None
}

fn normalize_cli_chat_live_tool_name(name: &str) -> String {
    name.trim_matches(|ch: char| ch == '`' || ch == '"' || ch == '\'')
        .rsplit(['.', '/', ':'])
        .next()
        .unwrap_or(name)
        .to_owned()
}

fn is_cli_chat_live_read_tool(name: &str) -> bool {
    matches!(
        name,
        "read" | "read_file" | "read-file" | "readfile" | "open_file" | "open-file" | "cat"
    )
}

fn is_cli_chat_live_run_tool(name: &str) -> bool {
    matches!(
        name,
        "bash" | "shell" | "sh" | "exec_command" | "run_command" | "terminal" | "cmd"
    )
}

fn is_cli_chat_live_search_tool(name: &str) -> bool {
    matches!(
        name,
        "search" | "grep" | "ripgrep" | "rg" | "find" | "find_text"
    )
}

fn is_cli_chat_live_list_tool(name: &str) -> bool {
    matches!(
        name,
        "list" | "ls" | "list_directory" | "list_dir" | "read_dir" | "dir"
    )
}

fn is_cli_chat_live_glob_tool(name: &str) -> bool {
    matches!(name, "glob" | "find_files" | "find_file" | "walk")
}

fn cli_chat_live_read_request_display(tool_snapshot: &CliChatLiveToolSnapshot) -> Option<String> {
    let path = cli_chat_live_request_path(tool_snapshot)?;
    let mut display = path;
    if let Some(offset) = cli_chat_live_request_number(tool_snapshot, "offset") {
        display.push_str(
            cli_chat_live_read_line_range(
                offset,
                cli_chat_live_request_number(tool_snapshot, "limit"),
            )
            .as_str(),
        );
    }
    Some(display)
}

fn cli_chat_live_read_line_range(offset: u64, limit: Option<u64>) -> String {
    let start = offset.max(1);
    match limit.and_then(|limit| limit.checked_sub(1)) {
        Some(limit_tail) if limit_tail > 0 => format!(":{start}-{}", start + limit_tail),
        _ => format!(":{start}"),
    }
}

fn cli_chat_live_request_command(tool_snapshot: &CliChatLiveToolSnapshot) -> Option<String> {
    cli_chat_live_request_string_field(tool_snapshot, &["cmd", "command", "script"])
}

fn cli_chat_live_search_request_display(tool_snapshot: &CliChatLiveToolSnapshot) -> Option<String> {
    let query =
        cli_chat_live_request_string_field(tool_snapshot, &["query", "pattern", "needle", "text"])?;
    let query = truncate_cli_chat_live_text(query.as_str(), 48);
    let path = cli_chat_live_request_path(tool_snapshot)
        .map(|path| truncate_cli_chat_live_text(path.as_str(), 40));

    Some(if let Some(path) = path {
        format!("\"{query}\" in {path}")
    } else {
        format!("\"{query}\"")
    })
}

fn cli_chat_live_glob_request_display(tool_snapshot: &CliChatLiveToolSnapshot) -> Option<String> {
    let pattern = cli_chat_live_request_string_field(
        tool_snapshot,
        &["glob", "pattern", "query", "pathspec"],
    )?;
    let pattern = truncate_cli_chat_live_text(pattern.as_str(), 48);
    let path = cli_chat_live_request_path(tool_snapshot)
        .map(|path| truncate_cli_chat_live_text(path.as_str(), 40));

    Some(if let Some(path) = path {
        format!("{pattern} in {path}")
    } else {
        pattern
    })
}

fn cli_chat_live_request_path(tool_snapshot: &CliChatLiveToolSnapshot) -> Option<String> {
    cli_chat_live_request_string_field(
        tool_snapshot,
        &["path", "file_path", "absolute_path", "source", "url"],
    )
}

fn cli_chat_live_request_string_field(
    tool_snapshot: &CliChatLiveToolSnapshot,
    keys: &[&str],
) -> Option<String> {
    cli_chat_live_request_value(tool_snapshot)
        .and_then(|value| cli_chat_live_first_string_field(value, keys, 0))
}

fn cli_chat_live_request_number(tool_snapshot: &CliChatLiveToolSnapshot, key: &str) -> Option<u64> {
    cli_chat_live_request_value(tool_snapshot)
        .and_then(|value| cli_chat_live_find_u64_field(value, key, 0))
}

fn cli_chat_live_request_value(tool_snapshot: &CliChatLiveToolSnapshot) -> Option<Value> {
    for candidate in [
        (!tool_snapshot.args.is_empty()).then_some(tool_snapshot.args.as_str()),
        tool_snapshot.request_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(value) = serde_json::from_str::<Value>(candidate.trim()) {
            return Some(value);
        }
    }
    None
}

fn cli_chat_live_first_string_field(value: Value, keys: &[&str], depth: usize) -> Option<String> {
    if depth > 3 {
        return None;
    }

    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(text) = object.get(*key).and_then(Value::as_str)
                    && !text.trim().is_empty()
                {
                    return Some(text.trim().to_owned());
                }
            }
            object
                .into_values()
                .find_map(|value| cli_chat_live_first_string_field(value, keys, depth + 1))
        }
        Value::Array(items) => items
            .into_iter()
            .find_map(|value| cli_chat_live_first_string_field(value, keys, depth + 1)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn cli_chat_live_find_u64_field(value: Value, key: &str, depth: usize) -> Option<u64> {
    if depth > 3 {
        return None;
    }

    match value {
        Value::Object(object) => object
            .get(key)
            .and_then(cli_chat_live_value_as_u64)
            .or_else(|| {
                object
                    .into_values()
                    .find_map(|value| cli_chat_live_find_u64_field(value, key, depth + 1))
            }),
        Value::Array(items) => items
            .into_iter()
            .find_map(|value| cli_chat_live_find_u64_field(value, key, depth + 1)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn cli_chat_live_value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

pub(super) fn render_cli_chat_live_surface_lines_with_width(
    snapshot: &CliChatLiveSurfaceSnapshot,
    width: usize,
) -> Vec<String> {
    let body_width = cli_chat_card_inner_width(width);
    let message_spec = build_cli_chat_live_surface_message_spec(snapshot, body_width);
    let body_lines = render_tui_message_body_spec(&message_spec, body_width);
    let title = build_cli_chat_live_surface_card_title(snapshot);
    render_cli_chat_card_lines(title.as_str(), &body_lines, width)
}

pub(super) fn render_cli_chat_live_compact_lines_with_width(
    snapshot: &CliChatLiveSurfaceSnapshot,
    width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    let wrap_width = width.saturating_sub(2).max(1);

    if let Some(preview) = snapshot.draft_preview.as_deref() {
        lines.extend(render_live_preview_lines(preview, wrap_width));
    }

    if !snapshot.tools.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        for raw_line in format_cli_chat_live_tool_activity_lines(snapshot.tools.as_slice()) {
            for wrapped in
                crate::presentation::render_wrapped_display_line(raw_line.as_str(), wrap_width)
            {
                lines.push(wrapped);
            }
        }
    }

    lines
}

fn split_live_preview_text(preview: &str) -> (Option<String>, Option<String>) {
    let open_tag = "<think>";
    let close_tag = "</think>";
    let lower = preview.to_ascii_lowercase();
    let mut visible = String::new();
    let mut thinking = String::new();
    let mut idx = 0usize;
    let mut in_think = false;

    while idx < preview.len() {
        let remaining = &lower[idx..];
        if remaining.starts_with(open_tag) {
            in_think = true;
            idx += open_tag.len();
            continue;
        }
        if remaining.starts_with(close_tag) {
            in_think = false;
            idx += close_tag.len();
            continue;
        }
        if open_tag.starts_with(remaining) || close_tag.starts_with(remaining) {
            break;
        }

        let Some(ch) = preview[idx..].chars().next() else {
            break;
        };
        if in_think {
            thinking.push(ch);
        } else {
            visible.push(ch);
        }
        idx += ch.len_utf8();
    }

    let thinking = thinking.trim().to_owned();
    let visible = visible.trim().to_owned();
    (
        (!thinking.is_empty()).then_some(thinking),
        (!visible.is_empty()).then_some(visible),
    )
}

fn render_live_preview_lines(preview: &str, wrap_width: usize) -> Vec<String> {
    let (thinking_preview, visible_preview) = split_live_preview_text(preview);
    let mut lines = Vec::new();
    let wrap_width = wrap_width.max(1);

    if let Some(thinking_preview) = thinking_preview.as_deref() {
        lines.extend(render_live_preview_segment_lines(
            thinking_preview,
            wrap_width,
        ));
    }

    if thinking_preview.is_some() && visible_preview.is_some() && !lines.is_empty() {
        lines.push(String::new());
    }

    if let Some(visible_preview) = visible_preview.as_deref() {
        lines.extend(render_live_preview_segment_lines(
            visible_preview,
            wrap_width,
        ));
    }

    lines
}

fn render_live_preview_segment_lines(segment: &str, wrap_width: usize) -> Vec<String> {
    let normalized = sanitize_live_preview_text(segment);
    if let Some(structured_lines) =
        render_live_preview_structured_lines(normalized.as_str(), wrap_width)
    {
        return structured_lines;
    }
    let mut rendered = Vec::new();

    let normalized_lines = normalized.lines().collect::<Vec<_>>();
    for line in normalized_lines {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            if rendered
                .last()
                .map(|line: &String| !line.trim().is_empty())
                .unwrap_or(true)
            {
                rendered.push(String::new());
            }
            continue;
        }

        if let Some(split_bullets) = split_live_preview_inline_bullet_runs(trimmed) {
            for bullet_line in split_bullets {
                rendered.extend(crate::presentation::render_wrapped_display_line(
                    bullet_line.as_str(),
                    wrap_width,
                ));
            }
            continue;
        }

        rendered.extend(crate::presentation::render_wrapped_display_line(
            trimmed, wrap_width,
        ));
    }

    trim_outer_blank_lines(&mut rendered);
    rendered
}

fn render_live_preview_structured_lines(segment: &str, wrap_width: usize) -> Option<Vec<String>> {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return Some(Vec::new());
    }

    if let Some(diff_body) = live_preview_diff_body(trimmed) {
        let mut rendered = Vec::new();
        for plain in live_preview_render_raw_diff_lines(diff_body.as_str()) {
            for wrapped in wrap_live_preview_diff_line(plain.as_str(), wrap_width) {
                rendered.push(wrapped);
            }
        }
        trim_outer_blank_lines(&mut rendered);
        return Some(rendered);
    }

    if let Some(diff_body) = live_preview_provisional_diff_body(trimmed) {
        let diff_lines = if diff_body.trim().is_empty() {
            vec!["  diff preview…".to_owned()]
        } else {
            live_preview_render_raw_diff_lines(diff_body.as_str())
        };
        let mut rendered = Vec::new();
        for plain in diff_lines {
            for wrapped in wrap_live_preview_diff_line(plain.as_str(), wrap_width) {
                rendered.push(wrapped);
            }
        }
        trim_outer_blank_lines(&mut rendered);
        return Some(rendered);
    }

    if live_preview_contains_markdown_table(trimmed) {
        let mut rendered = markdown::render_markdown_to_lines_with_width(trimmed, Some(wrap_width))
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        trim_outer_blank_lines(&mut rendered);
        return Some(rendered);
    }

    if let Some(rendered) = render_live_preview_provisional_table(trimmed, wrap_width) {
        return Some(rendered);
    }

    if live_preview_contains_structured_markdown(trimmed) {
        let mut rendered = markdown::render_markdown_to_lines_with_width(trimmed, Some(wrap_width))
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        trim_outer_blank_lines(&mut rendered);
        return Some(rendered);
    }

    None
}
fn live_preview_provisional_diff_body(segment: &str) -> Option<String> {
    let mut lines = segment.lines();
    let first = lines.next()?.trim();
    let lower = first.to_ascii_lowercase();
    let is_partial_diff_fence = first.starts_with("```")
        && first != "```"
        && ("```diff".starts_with(lower.as_str()) || "```patch".starts_with(lower.as_str()));
    if is_partial_diff_fence {
        return Some(lines.collect::<Vec<_>>().join("\n"));
    }

    let mut plus_line_count = 0usize;
    let mut minus_line_count = 0usize;
    let mut structural_line_count = 0usize;
    for line in segment.lines().map(str::trim_start) {
        if line.starts_with("+ ") {
            plus_line_count = plus_line_count.saturating_add(1);
        } else if line.starts_with("- ") {
            minus_line_count = minus_line_count.saturating_add(1);
        } else if line.starts_with("@@")
            || line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
        {
            structural_line_count = structural_line_count.saturating_add(1);
        }
    }

    let has_diff_pair = plus_line_count > 0 && minus_line_count > 0;
    let has_structured_diff = structural_line_count > 0
        && plus_line_count
            .saturating_add(minus_line_count)
            .saturating_add(structural_line_count)
            >= 2;
    (has_diff_pair || has_structured_diff).then(|| segment.to_owned())
}

fn live_preview_diff_body(segment: &str) -> Option<String> {
    let mut lines = segment.lines();
    let fence = lines.next()?.trim();
    if !matches!(fence, "```diff" | "```patch") {
        return None;
    }

    let mut body = lines.collect::<Vec<_>>();
    if body.last().is_some_and(|line| line.trim() == "```") {
        body.pop();
    }
    Some(body.join("\n"))
}

fn render_live_preview_provisional_table(segment: &str, wrap_width: usize) -> Option<Vec<String>> {
    let mut rows = Vec::new();
    let mut saw_table_like_line = false;

    for line in segment
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if live_preview_is_markdown_table_separator(line) {
            saw_table_like_line = true;
            continue;
        }
        if !live_preview_is_provisional_table_row(line) {
            return None;
        }
        saw_table_like_line = true;
        rows.push(parse_live_preview_table_cells(line));
    }

    if !saw_table_like_line || rows.is_empty() {
        return None;
    }

    let markdown_table = provisional_table_rows_to_markdown(rows.as_slice());
    let mut rendered = markdown::render_markdown_to_lines_with_width(
        markdown_table.as_str(),
        Some(wrap_width.max(1)),
    )
    .into_iter()
    .map(|line| {
        line.spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<String>()
    })
    .collect::<Vec<_>>();
    trim_outer_blank_lines(&mut rendered);
    Some(rendered)
}

fn live_preview_is_provisional_table_row(line: &str) -> bool {
    line.starts_with('|') && line.matches('|').count() >= 2
}

fn parse_live_preview_table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect::<Vec<_>>()
}

fn provisional_table_rows_to_markdown(rows: &[Vec<String>]) -> String {
    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let mut normalized_rows = rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            row.resize(column_count, String::new());
            row
        })
        .collect::<Vec<_>>();

    if normalized_rows.is_empty() {
        normalized_rows.push(vec![String::new(); column_count]);
    }

    let mut lines = Vec::with_capacity(normalized_rows.len().saturating_add(1));
    if let Some(header) = normalized_rows.first() {
        lines.push(format_provisional_markdown_row(header.as_slice()));
    }
    lines.push(format_provisional_markdown_separator(column_count));
    for row in normalized_rows.iter().skip(1) {
        lines.push(format_provisional_markdown_row(row.as_slice()));
    }
    lines.join("\n")
}

fn format_provisional_markdown_row(row: &[String]) -> String {
    let cells = row
        .iter()
        .map(|cell| escape_provisional_markdown_cell(cell))
        .collect::<Vec<_>>()
        .join(" | ");
    format!("| {cells} |")
}

fn format_provisional_markdown_separator(column_count: usize) -> String {
    let columns = std::iter::repeat_n("---", column_count)
        .collect::<Vec<_>>()
        .join(" | ");
    format!("| {columns} |")
}

fn escape_provisional_markdown_cell(cell: &str) -> String {
    cell.replace('\\', "\\\\")
        .replace('|', "\\|")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn live_preview_render_raw_diff_lines(diff: &str) -> Vec<String> {
    let lines = diff
        .lines()
        .map(|line| {
            if line.starts_with('+')
                || line.starts_with('-')
                || line.starts_with("@@")
                || line.starts_with("diff ")
                || line.starts_with("index ")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
            {
                line.to_owned()
            } else {
                format!("  {line}")
            }
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        vec!["  (empty diff)".to_owned()]
    } else {
        lines
    }
}

fn wrap_live_preview_diff_line(line: &str, wrap_width: usize) -> Vec<String> {
    for prefix in ["+ ", "- ", "@@ ", "+++ ", "--- ", "diff ", "index "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let continuation = " ".repeat(crate::presentation::display_width(prefix));
            return wrap_with_prefix(prefix, continuation.as_str(), rest, wrap_width);
        }
    }

    crate::presentation::render_wrapped_display_line(line, wrap_width)
}

fn wrap_with_prefix(
    prefix: &str,
    continuation_prefix: &str,
    body: &str,
    wrap_width: usize,
) -> Vec<String> {
    let prefix_width = crate::presentation::display_width(prefix);
    let continuation_width = crate::presentation::display_width(continuation_prefix);
    let first_width = wrap_width.saturating_sub(prefix_width).max(1);
    let continuation_body_width = wrap_width.saturating_sub(continuation_width).max(1);

    let wrapped_body = crate::presentation::render_wrapped_display_line(body, first_width);
    let mut rendered = Vec::new();
    for (index, line) in wrapped_body.into_iter().enumerate() {
        if index == 0 {
            rendered.push(format!("{prefix}{line}"));
        } else {
            for continuation_line in crate::presentation::render_wrapped_display_line(
                line.as_str(),
                continuation_body_width,
            ) {
                rendered.push(format!("{continuation_prefix}{continuation_line}"));
            }
        }
    }
    rendered
}

fn live_preview_contains_markdown_table(segment: &str) -> bool {
    let lines = segment.lines().map(str::trim).collect::<Vec<_>>();
    lines.len() >= 2
        && lines.windows(2).any(|pair| match pair {
            [first, second] => {
                live_preview_is_markdown_table_row(first)
                    && live_preview_is_markdown_table_separator(second)
            }
            _ => false,
        })
}

fn live_preview_contains_structured_markdown(segment: &str) -> bool {
    let lines = segment.lines().map(str::trim).collect::<Vec<_>>();
    if lines.is_empty() {
        return false;
    }

    lines.iter().any(|line| {
        line.starts_with("```")
            || line.starts_with("# ")
            || line.starts_with("## ")
            || line.starts_with("### ")
            || line.starts_with("> ")
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("1. ")
            || line.starts_with("2. ")
            || line.starts_with("3. ")
    })
}

fn live_preview_is_markdown_table_row(line: &str) -> bool {
    line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 3
}

fn live_preview_is_markdown_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if !live_preview_is_markdown_table_row(trimmed) {
        return false;
    }

    trimmed.trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
    })
}

fn sanitize_live_preview_text(segment: &str) -> String {
    let normalized = segment.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = trim_unstable_preview_suffix(normalized.as_str());
    collapse_preview_blank_runs(trimmed.as_str())
}

fn trim_unstable_preview_suffix(segment: &str) -> String {
    if segment.is_empty() || segment.ends_with('\n') {
        return segment.to_owned();
    }

    let trimmed = segment.trim_end_matches([' ', '\t']);
    if trimmed.is_empty() {
        return String::new();
    }

    let lower = trimmed.to_ascii_lowercase();
    if "<think>".starts_with(lower.as_str())
        || "</think>".starts_with(lower.as_str())
        || lower.ends_with('<')
        || lower.ends_with("</")
    {
        return String::new();
    }

    if trimmed.ends_with("```") && trimmed.matches("```").count() % 2 == 1 {
        return trimmed
            .rsplit_once('\n')
            .map(|(head, _)| head.to_owned())
            .unwrap_or_default();
    }

    trimmed.to_owned()
}

fn collapse_preview_blank_runs(segment: &str) -> String {
    let mut normalized = Vec::new();
    let mut last_was_blank = false;
    for line in segment.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && last_was_blank {
            continue;
        }
        last_was_blank = is_blank;
        normalized.push(line);
    }
    normalized.join("\n")
}

fn split_live_preview_inline_bullet_runs(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if trimmed.matches("• ").count() < 2 {
        return None;
    }

    let items = trimmed
        .split("• ")
        .filter_map(|segment| {
            let segment = segment.trim();
            (!segment.is_empty()).then(|| format!("• {segment}"))
        })
        .collect::<Vec<_>>();

    (items.len() >= 2).then_some(items)
}

fn live_preview_is_cjk(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch)
        || ('\u{3040}'..='\u{30FF}').contains(&ch)
        || ('\u{AC00}'..='\u{D7AF}').contains(&ch)
}

fn trim_outer_blank_lines(lines: &mut Vec<String>) {
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

fn build_cli_chat_live_surface_message_spec(
    snapshot: &CliChatLiveSurfaceSnapshot,
    body_width: usize,
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

    if let Some(preview_section) = build_cli_chat_live_preview_section(snapshot, body_width) {
        sections.push(preview_section);
    }

    if let Some(tool_section) = build_cli_chat_live_tool_section(snapshot) {
        sections.push(tool_section);
    }

    TuiMessageSpec {
        role: config::CLI_COMMAND_NAME.to_owned(),
        caption: Some("live".to_owned()),
        sections,
        footer_lines: vec![
            "Streaming turn state · /status runtime · /compact checkpoint".to_owned(),
        ],
    }
}

fn build_cli_chat_live_surface_card_title(snapshot: &CliChatLiveSurfaceSnapshot) -> String {
    let mut segments = vec![build_cli_chat_message_card_title(
        config::CLI_COMMAND_NAME,
        Some("live"),
    )];

    if let Some(provider_round) = snapshot.provider_round {
        segments.push(format!("round {provider_round}"));
    }

    if let Some(message_count) = snapshot.message_count {
        segments.push(format!("{message_count} msgs"));
    }

    if let Some(estimated_tokens) = snapshot.estimated_tokens {
        segments.push(format!("~{estimated_tokens} tok"));
    }

    if let Some(first_token_latency_ms) = snapshot.first_token_latency_ms {
        segments.push(format!("ttft {first_token_latency_ms}ms"));
    }

    segments.join(" · ")
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
            if let Some(first_token_latency_ms) = snapshot.first_token_latency_ms {
                return format!(
                    "Provider round {provider_round} started streaming after {first_token_latency_ms} ms."
                );
            }

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
            if let Some(first_token_latency_ms) = snapshot.first_token_latency_ms {
                return format!(
                    "Follow-up provider round {provider_round} started streaming after {first_token_latency_ms} ms."
                );
            }

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
            if let Some(first_token_latency_ms) = snapshot.first_token_latency_ms {
                return format!("first token in {first_token_latency_ms} ms");
            }

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

    if let Some(first_token_latency_ms) = snapshot.first_token_latency_ms {
        items.push(TuiKeyValueSpec::Plain {
            key: "first token".to_owned(),
            value: format!("{first_token_latency_ms} ms"),
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
    body_width: usize,
) -> Option<TuiSectionSpec> {
    let preview = snapshot.draft_preview.as_ref()?;
    let preview_lines = render_live_preview_lines(preview, body_width);

    if preview_lines.is_empty() {
        return None;
    }

    if let Some(language) = live_preview_preformatted_language(preview) {
        return Some(TuiSectionSpec::Preformatted {
            title: Some("draft preview".to_owned()),
            language: Some(language.to_owned()),
            lines: preview_lines,
        });
    }

    Some(TuiSectionSpec::Narrative {
        title: Some("draft preview".to_owned()),
        lines: preview_lines,
    })
}

fn live_preview_preformatted_language(preview: &str) -> Option<&'static str> {
    let (thinking_preview, visible_preview) = split_live_preview_text(preview);
    let mut segments = [thinking_preview.as_deref(), visible_preview.as_deref()]
        .into_iter()
        .flatten();

    let segment = segments.next()?;
    if segments.next().is_some() {
        return None;
    }

    let sanitized = sanitize_live_preview_text(segment);
    live_preview_diff_body(sanitized.trim()).map(|_| "diff")
}

fn build_cli_chat_live_tool_section(
    snapshot: &CliChatLiveSurfaceSnapshot,
) -> Option<TuiSectionSpec> {
    if snapshot.tools.is_empty() {
        return None;
    }

    let lines = format_cli_chat_live_tool_activity_lines(snapshot.tools.as_slice());

    Some(TuiSectionSpec::Narrative {
        title: Some("tool activity".to_owned()),
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CliChatLiveFileChangeView, CliChatLiveOutputView, CliChatLiveSurfaceSink,
        CliChatLiveSurfaceSnapshot, CliChatLiveToolSnapshot,
        build_cli_chat_live_compact_observer_controller,
        render_cli_chat_live_compact_lines_with_width,
        render_cli_chat_live_surface_lines_with_width, render_live_preview_segment_lines,
    };
    use crate::conversation::{
        ConversationTurnPhase, ConversationTurnPhaseEvent, ConversationTurnToolState, ExecutionLane,
    };
    use crate::tools::runtime_events::ToolFileChangeKind;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn assert_uniform_display_width(lines: &[String]) {
        let Some(first_width) = lines
            .first()
            .map(|line| crate::presentation::display_width(line))
        else {
            return;
        };

        for line in lines {
            assert_eq!(
                crate::presentation::display_width(line),
                first_width,
                "table line has a different display width: {line:?}"
            );
        }
    }

    fn empty_output() -> CliChatLiveOutputView {
        CliChatLiveOutputView {
            text: String::new(),
            total_bytes: 0,
            total_lines: 0,
            truncated: false,
        }
    }

    #[test]
    fn compact_render_shows_preview_without_card_chrome() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(4),
            estimated_tokens: Some(1200),
            first_token_latency_ms: None,
            draft_preview: Some("Hello there\nHow are you?".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
        let joined = lines.join("\n");

        assert!(joined.contains("Hello there"));
        assert!(joined.contains("How are you?"));
        assert!(!joined.contains("╭─"));
        assert!(!joined.contains("turn pipeline"));
    }

    #[test]
    fn compact_render_includes_tool_activity_summary_without_card_chrome() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Safe),
            tool_call_count: 1,
            message_count: Some(6),
            estimated_tokens: Some(1800),
            first_token_latency_ms: None,
            draft_preview: None,
            tools: vec![CliChatLiveToolSnapshot {
                tool_call_id: "call-1".to_owned(),
                name: Some("read_file".to_owned()),
                request_summary: Some("Read src/main.rs".to_owned()),
                args: "{\"path\":\"src/main.rs\"}".to_owned(),
                status: ConversationTurnToolState::Running,
                detail: Some("working".to_owned()),
                stdout: empty_output(),
                stderr: empty_output(),
                file_change: None,
                duration_ms: Some(12),
                exit_code: None,
            }],
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 60);
        let joined = lines.join("\n");

        assert!(joined.contains("• Called read_file · working"));
        assert!(joined.contains("↳ Read src/main.rs"));
        assert!(joined.contains("↳ args path=src/main.rs"));
        assert!(joined.contains("↳ metrics 12ms"));
        assert!(!joined.contains("╭─"));
        assert!(!joined.contains("tool activity]"));
    }

    #[test]
    fn compact_render_compacts_structured_request_and_args_previews() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 1,
            message_count: Some(3),
            estimated_tokens: Some(900),
            first_token_latency_ms: None,
            draft_preview: None,
            tools: vec![CliChatLiveToolSnapshot {
                tool_call_id: "call-2".to_owned(),
                name: Some("search".to_owned()),
                request_summary: Some(
                    "{\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
                ),
                args: "{\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
                status: ConversationTurnToolState::Running,
                detail: None,
                stdout: empty_output(),
                stderr: empty_output(),
                file_change: None,
                duration_ms: None,
                exit_code: None,
            }],
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 64);
        let joined = lines.join("\n");

        assert!(joined.contains("• Called search"));
        assert!(joined.contains("↳ request") || joined.contains("query=rust"));
        assert!(!joined.contains("↳ args query=rust"));
        assert!(joined.contains("limit=5"));
    }

    #[test]
    fn compact_render_promotes_command_request_into_primary_preview_line() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 1,
            message_count: Some(3),
            estimated_tokens: Some(900),
            first_token_latency_ms: None,
            draft_preview: None,
            tools: vec![CliChatLiveToolSnapshot {
                tool_call_id: "call-bash".to_owned(),
                name: Some("bash".to_owned()),
                request_summary: None,
                args: "{\"cmd\":\"cargo test --workspace --all-features\"}".to_owned(),
                status: ConversationTurnToolState::Running,
                detail: Some("working".to_owned()),
                stdout: empty_output(),
                stderr: empty_output(),
                file_change: None,
                duration_ms: None,
                exit_code: None,
            }],
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 72);
        let joined = lines.join("\n");

        assert!(joined.contains("• Called bash · working"));
        assert!(joined.contains("↳ Command cargo test --workspace --all-features"));
        assert!(joined.contains("↳ args cmd=cargo test --workspace --all-features"));
    }

    #[test]
    fn compact_render_promotes_search_request_into_primary_preview_line() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 1,
            message_count: Some(3),
            estimated_tokens: Some(900),
            first_token_latency_ms: None,
            draft_preview: None,
            tools: vec![CliChatLiveToolSnapshot {
                tool_call_id: "call-search".to_owned(),
                name: Some("grep".to_owned()),
                request_summary: None,
                args: "{\"query\":\"稳定|wenjian|robust|stable\",\"path\":\"~/chat\"}".to_owned(),
                status: ConversationTurnToolState::Running,
                detail: Some("working".to_owned()),
                stdout: empty_output(),
                stderr: empty_output(),
                file_change: None,
                duration_ms: None,
                exit_code: None,
            }],
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);
        let joined = lines.join("\n");

        assert!(joined.contains("• Called grep · working"));
        assert!(joined.contains("↳ Search \"稳定|wenjian|robust|stable\" in ~/chat"));
    }

    #[test]
    fn compact_render_promotes_glob_request_into_primary_preview_line() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 1,
            message_count: Some(3),
            estimated_tokens: Some(900),
            first_token_latency_ms: None,
            draft_preview: None,
            tools: vec![CliChatLiveToolSnapshot {
                tool_call_id: "call-glob".to_owned(),
                name: Some("find_files".to_owned()),
                request_summary: None,
                args: "{\"glob\":\"src/**/*.rs\",\"path\":\"~/chat\"}".to_owned(),
                status: ConversationTurnToolState::Running,
                detail: Some("working".to_owned()),
                stdout: empty_output(),
                stderr: empty_output(),
                file_change: None,
                duration_ms: None,
                exit_code: None,
            }],
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);
        let joined = lines.join("\n");

        assert!(joined.contains("• Called find_files · working"));
        assert!(joined.contains("↳ Glob src/**/*.rs in ~/chat"));
    }

    #[test]
    fn compact_render_compacts_stderr_and_file_children() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 1,
            message_count: Some(3),
            estimated_tokens: Some(900),
            first_token_latency_ms: None,
            draft_preview: None,
            tools: vec![CliChatLiveToolSnapshot {
                tool_call_id: "call-3".to_owned(),
                name: Some("exec".to_owned()),
                request_summary: None,
                args: String::new(),
                status: ConversationTurnToolState::Completed,
                detail: Some("ok".to_owned()),
                stdout: empty_output(),
                stderr: CliChatLiveOutputView {
                    text: "permission denied".to_owned(),
                    total_bytes: 17,
                    total_lines: 1,
                    truncated: false,
                },
                file_change: Some(CliChatLiveFileChangeView {
                    path: "src/lib.rs".to_owned(),
                    operation: ToolFileChangeKind::Edit,
                    added_lines: 2,
                    removed_lines: 1,
                    preview: None,
                }),
                duration_ms: Some(42),
                exit_code: Some(0),
            }],
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 64);
        let joined = lines.join("\n");

        assert!(joined.contains("• Closed exec · ok"));
        assert!(joined.contains("↳ stderr 1 lines · 17 bytes"));
        assert!(joined.contains("permission denied"));
        assert!(joined.contains("↳ file edit src/lib.rs (+2 / -1)"));
        assert!(joined.contains("↳ metrics 42ms · exit=0"));
    }

    #[test]
    fn compact_render_surfaces_approval_and_denied_status_lines() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Safe),
            tool_call_count: 2,
            message_count: Some(5),
            estimated_tokens: Some(700),
            first_token_latency_ms: None,
            draft_preview: None,
            tools: vec![
                CliChatLiveToolSnapshot {
                    tool_call_id: "call-a".to_owned(),
                    name: Some("search".to_owned()),
                    request_summary: None,
                    args: String::new(),
                    status: ConversationTurnToolState::NeedsApproval,
                    detail: Some("operator confirmation required".to_owned()),
                    stdout: empty_output(),
                    stderr: empty_output(),
                    file_change: None,
                    duration_ms: None,
                    exit_code: None,
                },
                CliChatLiveToolSnapshot {
                    tool_call_id: "call-b".to_owned(),
                    name: Some("search".to_owned()),
                    request_summary: None,
                    args: String::new(),
                    status: ConversationTurnToolState::Denied,
                    detail: Some("blocked".to_owned()),
                    stdout: empty_output(),
                    stderr: empty_output(),
                    file_change: None,
                    duration_ms: None,
                    exit_code: None,
                },
            ],
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 64);
        let joined = lines.join("\n");

        assert!(joined.contains("• Approval search · operator confirmation required"));
        assert!(joined.contains("• Denied search · blocked"));
    }

    #[test]
    fn compact_render_splits_think_blocks_into_reasoning_and_visible_reply() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(2),
            estimated_tokens: Some(512),
            first_token_latency_ms: None,
            draft_preview: Some(
                "<think>quiet reasoning\nsecond line</think>Hello there".to_owned(),
            ),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
        let joined = lines.join("\n");

        assert!(joined.contains("quiet reasoning"));
        assert!(joined.contains("second line"));
        assert!(joined.contains("Hello there"));
        assert!(!joined.contains("<think>"));
        assert!(!joined.contains("</think>"));
    }

    #[test]
    fn compact_render_collapses_outer_and_repeated_blank_lines() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(2),
            estimated_tokens: Some(512),
            first_token_latency_ms: None,
            draft_preview: Some(
                "\n\n<think>reasoning line</think>\n\n\nvisible reply\n\n".to_owned(),
            ),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
        assert_eq!(
            lines,
            vec![
                "reasoning line".to_owned(),
                String::new(),
                "visible reply".to_owned()
            ]
        );
    }

    #[test]
    fn compact_render_hides_partial_think_tag_prefixes() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(128),
            first_token_latency_ms: None,
            draft_preview: Some("<thi".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);

        assert!(lines.is_empty());
    }

    #[test]
    fn compact_render_keeps_incomplete_trailing_paragraph_literal_while_streaming() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("hello\nworld from stream".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);

        assert_eq!(
            lines,
            vec!["hello".to_owned(), "world from stream".to_owned()]
        );
    }

    #[test]
    fn compact_render_drops_trailing_unclosed_code_fence_suffix() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("hello world\n```".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);

        assert_eq!(lines, vec!["hello world".to_owned()]);
    }

    #[test]
    fn compact_render_keeps_visible_text_when_partial_closing_think_tag_arrives() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("visible answer</t".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);

        assert_eq!(lines, vec!["visible answer".to_owned()]);
    }

    #[test]
    fn compact_render_structures_markdown_tables_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some(
                "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |"
                    .to_owned(),
            ),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
        let joined = lines.join("\n");

        assert!(joined.contains("┌"));
        assert!(joined.contains("覆盖率"));
        assert!(joined.contains("220ms"));
        assert!(!joined.contains("| --- |"));
    }

    #[test]
    fn compact_render_structures_provisional_markdown_tables_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("| Name | Value |\n| A | 1 |\n| B | 2 |".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 32);

        assert_eq!(
            lines,
            vec![
                "┌──────┬───────┐".to_owned(),
                "│ Name │ Value │".to_owned(),
                "├──────┼───────┤".to_owned(),
                "│ A    │ 1     │".to_owned(),
                "│ B    │ 2     │".to_owned(),
                "└──────┴───────┘".to_owned(),
            ]
        );
        assert_uniform_display_width(lines.as_slice());
    }

    #[test]
    fn surface_render_structures_markdown_tables_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some(
                "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |"
                    .to_owned(),
            ),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, 72);
        let joined = lines.join("\n");

        assert!(joined.contains("draft preview"));
        assert!(joined.contains("┌"));
        assert!(joined.contains("覆盖率"));
        assert!(joined.contains("220ms"));
        assert!(!joined.contains("| --- |"));
    }

    #[test]
    fn compact_render_structures_fenced_diff_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("```diff\n- old value\n+ new value\n```".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
        let joined = lines.join("\n");

        assert!(joined.contains("- old value"));
        assert!(joined.contains("+ new value"));
        assert!(!joined.contains("```diff"));
    }

    #[test]
    fn compact_render_structures_provisional_diff_fence_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("```di\n- old value\n+ new value".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
        let joined = lines.join("\n");

        assert!(joined.contains("- old value"), "{joined}");
        assert!(joined.contains("+ new value"), "{joined}");
        assert!(!joined.contains("```di"), "{joined}");
    }

    #[test]
    fn surface_render_structures_fenced_diff_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("```diff\n- old value\n+ new value\n```".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, 72);
        let joined = lines.join("\n");

        assert!(joined.contains("draft preview"), "{joined}");
        assert!(joined.contains("- old value"), "{joined}");
        assert!(joined.contains("+ new value"), "{joined}");
        assert!(!joined.contains("```diff"), "{joined}");
    }

    #[test]
    fn compact_render_structures_fenced_code_block_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("```bash\nnpm install\nnpm test\n```".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
        let joined = lines.join("\n");

        assert!(joined.contains("```bash"));
        assert!(joined.contains("npm install"));
        assert!(joined.contains("npm test"));
        assert!(joined.contains("```"));
    }

    #[test]
    fn compact_render_structures_markdown_list_in_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Fast),
            tool_call_count: 0,
            message_count: Some(1),
            estimated_tokens: Some(256),
            first_token_latency_ms: None,
            draft_preview: Some("## 本周进展\n- 修复崩溃\n- 提升性能".to_owned()),
            tools: Vec::new(),
        };

        let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
        let joined = lines.join("\n");

        assert!(joined.contains("## 本周进展"));
        assert!(joined.contains("• 修复崩溃"));
        assert!(joined.contains("• 提升性能"));
    }

    #[test]
    fn preview_emit_waits_for_a_stable_initial_boundary() {
        let mut state = super::CliChatLiveSurfaceState {
            draft_preview: "hel".to_owned(),
            total_text_chars_seen: 3,
            ..Default::default()
        };

        assert!(!super::should_emit_cli_chat_live_preview(
            &state,
            40,
            Some(3)
        ));

        state.draft_preview.push(' ');
        state.total_text_chars_seen = 4;

        assert!(super::should_emit_cli_chat_live_preview(
            &state,
            40,
            Some(4)
        ));
    }

    #[test]
    fn preview_emit_allows_a_readable_initial_phrase() {
        let state = super::CliChatLiveSurfaceState {
            draft_preview: "Draft response".to_owned(),
            total_text_chars_seen: "Draft response".chars().count(),
            ..Default::default()
        };

        assert!(super::should_emit_cli_chat_live_preview(
            &state,
            72,
            Some(42)
        ));
    }

    #[test]
    fn preview_emit_forces_progress_after_large_unstable_burst() {
        let state = super::CliChatLiveSurfaceState {
            last_preview_emit_chars_seen: 8,
            draft_preview: "averylongunstablesuffixwithoutbreaks".to_owned(),
            total_text_chars_seen: 24,
            ..Default::default()
        };

        assert!(super::should_emit_cli_chat_live_preview(
            &state,
            8,
            Some(24)
        ));
    }

    #[test]
    fn preview_emit_uses_visual_line_pressure_for_wrapped_cjk_text() {
        let state = super::CliChatLiveSurfaceState {
            draft_preview: "渲染表格边界".to_owned(),
            total_text_chars_seen: 5,
            ..Default::default()
        };

        assert!(super::should_emit_cli_chat_live_preview(&state, 8, Some(5)));
    }

    #[test]
    fn preview_emit_mode_enters_catch_up_when_visual_backlog_grows() {
        let mut state = super::CliChatLiveSurfaceState::default();
        state.draft_preview =
            "line one wraps quickly\nline two wraps quickly\nline three wraps quickly".to_owned();
        state.total_text_chars_seen = state.draft_preview.chars().count();
        state.last_preview_emit_chars_seen = 8;
        state.last_preview_emit_visual_line_count = 1;

        assert_eq!(
            super::cli_chat_live_preview_emit_mode(&state, 18),
            super::CliChatLivePreviewEmitMode::CatchUp
        );
    }

    #[test]
    fn preview_emit_mode_stays_smooth_for_small_stable_updates() {
        let mut state = super::CliChatLiveSurfaceState::default();
        state.draft_preview = "hello world ".to_owned();
        state.total_text_chars_seen = state.draft_preview.chars().count();
        state.last_preview_emit_chars_seen = 8;
        state.last_preview_emit_visual_line_count = 1;

        assert_eq!(
            super::cli_chat_live_preview_emit_mode(&state, 80),
            super::CliChatLivePreviewEmitMode::Smooth
        );
    }

    #[test]
    fn preview_emit_cadence_slows_smooth_mode() {
        let mut state = super::CliChatLiveSurfaceState::default();
        state.draft_preview =
            "hello world this is a stable preview chunk with just enough size ".to_owned();
        state.total_text_chars_seen = state.draft_preview.chars().count();
        state.last_preview_emit_chars_seen = 8;
        state.last_preview_emit_visual_line_count = 1;
        state.last_preview_emit_elapsed_ms = Some(100);

        assert!(!super::should_emit_cli_chat_live_preview(
            &state,
            80,
            Some(120)
        ));
        assert!(super::should_emit_cli_chat_live_preview(
            &state,
            80,
            Some(140)
        ));
    }

    #[test]
    fn preview_emit_cadence_keeps_catch_up_faster_than_smooth() {
        let mut state = super::CliChatLiveSurfaceState::default();
        state.draft_preview =
            "line one wraps quickly\nline two wraps quickly\nline three wraps quickly".to_owned();
        state.total_text_chars_seen = state.draft_preview.chars().count();
        state.last_preview_emit_chars_seen = 8;
        state.last_preview_emit_visual_line_count = 1;
        state.last_preview_emit_elapsed_ms = Some(100);

        assert!(super::should_emit_cli_chat_live_preview(
            &state,
            18,
            Some(118)
        ));
    }

    #[test]
    fn delta_commit_boundary_detects_newline_and_structural_tokens() {
        assert!(super::cli_chat_live_delta_has_commit_boundary(
            "line done\n"
        ));
        assert!(super::cli_chat_live_delta_has_commit_boundary("<think>"));
        assert!(super::cli_chat_live_delta_has_commit_boundary("</think>"));
        assert!(super::cli_chat_live_delta_has_commit_boundary("```rust"));
        assert!(!super::cli_chat_live_delta_has_commit_boundary(
            "plain delta"
        ));
    }

    #[test]
    fn compact_observer_rerenders_preview_when_width_changes() {
        let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
        let render_sink: CliChatLiveSurfaceSink = {
            let captured_batches = Arc::clone(&captured_batches);
            Arc::new(move |lines| {
                let mut batches = captured_batches
                    .lock()
                    .expect("captured batches lock should not be poisoned");
                batches.push(lines);
            })
        };
        let render_width = Arc::new(AtomicUsize::new(32));
        let (observer, rerender) =
            build_cli_chat_live_compact_observer_controller(Arc::clone(&render_width), render_sink);

        observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
            1,
            3,
            Some(96),
        ));
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "text_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: Some("alpha beta gamma delta epsilon".to_owned()),
                tool_call: None,
            },
            index: None,
            elapsed_ms: Some(42),
        });

        render_width.store(12, Ordering::Relaxed);
        rerender();

        let batches = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned");
        let last_batch = batches.last().expect("rerender batch");
        assert!(last_batch.len() > 1);
        assert!(last_batch.iter().any(|line| line.contains("alpha beta")));
    }

    #[test]
    fn compact_observer_commits_preview_immediately_on_newline_boundary() {
        let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
        let render_sink: CliChatLiveSurfaceSink = {
            let captured_batches = Arc::clone(&captured_batches);
            Arc::new(move |lines| {
                let mut batches = captured_batches
                    .lock()
                    .expect("captured batches lock should not be poisoned");
                batches.push(lines);
            })
        };
        let render_width = Arc::new(AtomicUsize::new(80));
        let (observer, _) =
            build_cli_chat_live_compact_observer_controller(Arc::clone(&render_width), render_sink);

        observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
            1,
            3,
            Some(32),
        ));
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "text_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: Some("ok\n".to_owned()),
                tool_call: None,
            },
            index: None,
            elapsed_ms: Some(8),
        });

        let batches = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned");
        let last_batch = batches.last().expect("newline-triggered batch");
        assert!(last_batch.iter().any(|line| line.contains("ok")));
    }

    #[test]
    fn compact_observer_skips_rerender_when_width_change_keeps_same_lines() {
        let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
        let render_sink: CliChatLiveSurfaceSink = {
            let captured_batches = Arc::clone(&captured_batches);
            Arc::new(move |lines| {
                let mut batches = captured_batches
                    .lock()
                    .expect("captured batches lock should not be poisoned");
                batches.push(lines);
            })
        };
        let render_width = Arc::new(AtomicUsize::new(80));
        let (observer, rerender) =
            build_cli_chat_live_compact_observer_controller(Arc::clone(&render_width), render_sink);

        observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
            1,
            3,
            Some(64),
        ));
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "text_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: Some("short line ".to_owned()),
                tool_call: None,
            },
            index: None,
            elapsed_ms: Some(10),
        });

        let batch_count_before = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned")
            .len();

        render_width.store(79, Ordering::Relaxed);
        rerender();

        let batch_count_after = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned")
            .len();

        assert_eq!(batch_count_after, batch_count_before);
    }

    #[test]
    fn live_preview_keeps_command_lines_split_after_label() {
        let lines = render_live_preview_segment_lines(
            "Command:\ncargo test --workspace --all-features",
            80,
        );

        assert_eq!(
            lines,
            vec![
                "Command:".to_owned(),
                "cargo test --workspace --all-features".to_owned(),
            ]
        );
    }

    #[test]
    fn live_preview_keeps_path_lines_split_after_label() {
        let lines = render_live_preview_segment_lines("Path:\n~/chat/.omx/state.json", 80);

        assert_eq!(
            lines,
            vec!["Path:".to_owned(), "~/chat/.omx/state.json".to_owned(),]
        );
    }

    #[test]
    fn live_preview_keeps_logfmt_lines_out_of_paragraph_reflow() {
        let lines = render_live_preview_segment_lines(
            "prefix\n2026-04-25T11:02:58.547678Z WARN Loong.tools: tool execution failed requested_tool_name=file.read payload_kind=object duration_ms=0",
            96,
        );

        assert_eq!(lines.first().map(String::as_str), Some("prefix"));
        assert!(lines.iter().any(|line| line.contains("WARN Loong.tools:")));
        assert!(!lines[0].contains("WARN Loong.tools:"));
    }

    #[test]
    fn live_preview_preserves_code_like_lines_without_markdown_fence() {
        let lines = render_live_preview_segment_lines(
            "import \"strings\"\nconst (\n    openAIToolCallTypeCustom = \"custom_tool_call\"\n)\nfunc RequiresOpenAIWSV2Continuation(reqBody map[string]any) bool {\n    return false\n}",
            96,
        );

        assert!(lines.iter().any(|line| line == "import \"strings\""));
        assert!(lines.iter().any(|line| line.contains("const (")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("func RequiresOpenAIWSV2Continuation"))
        );
    }

    #[test]
    fn preview_emit_stride_is_more_responsive_on_narrow_widths() {
        assert_eq!(super::cli_chat_live_preview_emit_stride(4), 8);
        assert_eq!(super::cli_chat_live_preview_emit_stride(12), 12);
        assert_eq!(super::cli_chat_live_preview_emit_stride(20), 20);
        assert_eq!(super::cli_chat_live_preview_emit_stride(80), 48);
    }

    #[test]
    fn preview_buffer_limit_stays_large_even_for_narrow_widths() {
        assert_eq!(
            super::cli_chat_live_preview_char_limit(12),
            super::CLI_CHAT_LIVE_PREVIEW_MAX_BUFFER_CHARS
        );
        assert_eq!(
            super::cli_chat_live_tool_args_char_limit(12),
            super::CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS
        );
        assert_eq!(
            super::cli_chat_live_output_char_limit(12),
            super::CLI_CHAT_LIVE_OUTPUT_MAX_BUFFER_CHARS
        );
    }
}
