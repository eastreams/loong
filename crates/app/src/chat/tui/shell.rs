use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::{FutureExt as _, StreamExt};
use loongclaw_contracts::ToolCoreRequest;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::Style;
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::mpsc;

use crate::CliResult;
use crate::acp::AcpConversationTurnOptions;
use crate::conversation::{
    ConversationRuntimeBinding, ConversationTurnObserverHandle, DefaultConversationRuntime,
    ProviderErrorMode, parse_conversation_event, resolve_context_engine,
    resolve_context_engine_selection,
};
use crate::session::presentation::{
    DelegateAgentPresentation, SessionPresentationLocale, localized_root_thread_label,
    root_thread_search_terms,
};

use super::boot::{TuiBootFlow, TuiBootScreen, TuiBootTransition};
use super::commands::{self, ParsedSlashCommand, SlashCommand};
use super::dialog::ClarifyDialog;
use super::events::UiEvent;
use super::focus::{FocusLayer, FocusStack};
use super::history::{self, PaneView};
use super::input::InputView;
use super::layout;
use super::message::Message;
use super::observer::build_tui_observer;
use super::render::{self, ShellView};
use super::spinner::SpinnerView;
use super::state;
use super::stats;
use super::status_bar::StatusBarView;
use super::theme::Palette;

// ---------------------------------------------------------------------------
// View trait impls — bridge concrete state types into the render layer
// ---------------------------------------------------------------------------

impl PaneView for state::Pane {
    fn messages(&self) -> &[Message] {
        &self.messages
    }
    fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }
    fn streaming_active(&self) -> bool {
        self.streaming_active
    }
    fn transcript_cursor_line(&self, total_lines: usize) -> Option<usize> {
        state::Pane::transcript_cursor_line(self, total_lines)
    }
    fn transcript_selection_range(&self, total_lines: usize) -> Option<(usize, usize)> {
        state::Pane::transcript_selection_range(self, total_lines)
    }
}

impl SpinnerView for state::Pane {
    fn agent_running(&self) -> bool {
        self.agent_running
    }
    fn spinner_frame(&self) -> usize {
        self.spinner_frame
    }
    fn dots_frame(&self) -> usize {
        self.dots_frame
    }
    fn loop_state(&self) -> &str {
        &self.loop_state
    }
    fn loop_action(&self) -> &str {
        &self.loop_action
    }
    fn loop_iteration(&self) -> u32 {
        self.loop_iteration
    }
    fn status_message(&self) -> Option<(&str, &Instant)> {
        self.status_message.as_ref().map(|(s, i)| (s.as_str(), i))
    }
}

impl StatusBarView for state::Pane {
    fn model(&self) -> &str {
        &self.model
    }
    fn input_tokens(&self) -> u32 {
        self.input_tokens
    }
    fn output_tokens(&self) -> u32 {
        self.output_tokens
    }
    fn context_length(&self) -> u32 {
        self.context_length
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    fn session_display_label(&self) -> Option<&str> {
        self.session_display_label.as_deref()
    }
    fn busy_input_mode(&self) -> state::BusyInputMode {
        self.busy_input_mode()
    }
    fn pending_submission_count(&self) -> usize {
        self.pending_submission_count()
    }
    fn running_task_count(&self) -> Option<usize> {
        self.composer_suggestion_context.running_tasks
    }
    fn overdue_task_count(&self) -> Option<usize> {
        self.composer_suggestion_context.overdue_tasks
    }
    fn pending_approval_count(&self) -> Option<usize> {
        self.composer_suggestion_context.pending_approvals
    }
    fn attention_approval_count(&self) -> Option<usize> {
        self.composer_suggestion_context.attention_approvals
    }
    fn visible_session_count(&self) -> Option<usize> {
        self.composer_suggestion_context.visible_sessions
    }
    fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }
    fn transcript_selection_line_count(&self) -> usize {
        self.transcript_selection_line_count_hint()
    }
    fn status_message(&self) -> Option<(&str, &Instant)> {
        self.status_message.as_ref().map(|(s, i)| (s.as_str(), i))
    }
}

impl InputView for state::Pane {
    fn agent_running(&self) -> bool {
        self.agent_running
    }
    fn pending_submission_count(&self) -> usize {
        self.pending_submission_count()
    }
    fn busy_input_mode(&self) -> state::BusyInputMode {
        self.busy_input_mode()
    }
    fn transcript_selection_line_count(&self) -> usize {
        self.transcript_selection_line_count_hint()
    }
    fn input_hint(&self) -> Option<&str> {
        self.input_hint_override.as_deref()
    }
    fn input_placeholder(&self) -> Option<String> {
        composer_placeholder(self)
    }
}

const STARTER_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "Explain this workspace's layered kernel design",
    "Inspect installed skills with /skills",
    "Review the current worktree and call out the riskiest diff",
    "Check effective tool policy with /permissions",
    "Trace active child threads with /subagents",
    "Review approval hotspots with /approvals attention",
    "Summarize the crate graph and the sharp edges in this repo",
    "Use /compact before the session context gets too heavy",
];

const ACTIVE_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "Summarize what changed in this thread before we continue",
    "Turn the latest idea into a minimal patch plan",
    "Inspect the next blocked child thread with /subagents",
    "Resolve the next queued approval from /approvals",
    "Check whether this session is over-permitted with /permissions",
    "Switch across active threads with /subagents",
    "Tighten the current patch and call out regression risk",
    "Compare the transcript story against the actual diff",
];

const COMPOSER_PLACEHOLDER_ROTATION_SECONDS: u64 = 45;

const DIRTY_WORKTREE_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "Review the current worktree and call out the riskiest diff",
    "Open /diff status before the next edit",
    "Compare the transcript story against the actual diff",
];

const HOT_CONTEXT_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "Context is getting heavy; run /compact before the next long turn",
    "Use /compact to compress this session before pushing more context",
];

const SESSION_RESUME_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "There are other visible threads; inspect them with /resume",
    "Switch across active subagent threads with /subagents",
];

const RUNNING_TASK_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "Delegate work is active; open /subagents to watch the child threads",
    "Jump into a running child thread with /subagents",
];

const OVERDUE_TASK_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "A child thread looks overdue; inspect it with /subagents",
    "Open /subagents and jump into the blocked thread",
];

const PENDING_APPROVAL_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "There are queued approvals; inspect them with /approvals",
    "Review pending approval requests before the next risky action",
];

const ATTENTION_APPROVAL_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "An approval needs attention; inspect it with /approvals attention",
    "Resolve the next risky approval from /approvals attention",
];

const EXPLICIT_POLICY_COMPOSER_PLACEHOLDERS: &[&str] = &[
    "This session has an explicit tool policy; inspect it with /permissions",
    "Check whether the current session is over- or under-permitted with /permissions",
];

fn choose_placeholder(
    placeholders: &[&str],
    session_id: &str,
    message_count: usize,
) -> Option<String> {
    let rotation_bucket = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / COMPOSER_PLACEHOLDER_ROTATION_SECONDS)
        .unwrap_or(0);
    let session_seed = session_id.bytes().fold(0_u64, |acc, byte| {
        acc.wrapping_mul(131).wrapping_add(u64::from(byte))
    });
    let index = ((session_seed ^ message_count as u64).wrapping_add(rotation_bucket)
        % placeholders.len() as u64) as usize;

    placeholders
        .get(index)
        .map(|placeholder| (*placeholder).to_owned())
}

fn composer_placeholder(pane: &state::Pane) -> Option<String> {
    if pane.agent_running || pane.input_hint_override.is_some() {
        return None;
    }

    let context = &pane.composer_suggestion_context;
    let placeholders = if pane.context_percent() >= 0.72 {
        HOT_CONTEXT_COMPOSER_PLACEHOLDERS
    } else if context.attention_approvals.unwrap_or(0) > 0 {
        ATTENTION_APPROVAL_COMPOSER_PLACEHOLDERS
    } else if context.pending_approvals.unwrap_or(0) > 0 {
        PENDING_APPROVAL_COMPOSER_PLACEHOLDERS
    } else if context.overdue_tasks.unwrap_or(0) > 0 {
        OVERDUE_TASK_COMPOSER_PLACEHOLDERS
    } else if context.running_tasks.unwrap_or(0) > 0 {
        RUNNING_TASK_COMPOSER_PLACEHOLDERS
    } else if context.worktree_dirty == Some(true) {
        DIRTY_WORKTREE_COMPOSER_PLACEHOLDERS
    } else if context.has_explicit_permission_policy == Some(true) {
        EXPLICIT_POLICY_COMPOSER_PLACEHOLDERS
    } else if context.visible_sessions.unwrap_or(0) > 1 {
        SESSION_RESUME_COMPOSER_PLACEHOLDERS
    } else if pane.messages.is_empty() {
        STARTER_COMPOSER_PLACEHOLDERS
    } else {
        ACTIVE_COMPOSER_PLACEHOLDERS
    };

    choose_placeholder(placeholders, pane.session_id.as_str(), pane.messages.len())
}

impl ShellView for state::Shell {
    type Pane = state::Pane;

    fn pane(&self) -> &state::Pane {
        &self.pane
    }
    fn show_thinking(&self) -> bool {
        self.show_thinking
    }
    fn focus(&self) -> &FocusStack {
        &self.focus
    }
    fn clarify_dialog(&self) -> Option<&ClarifyDialog> {
        self.pane.clarify_dialog.as_ref()
    }
    fn tool_inspector(&self) -> Option<render::ToolInspectorView<'_>> {
        let active_tool_inspector = self.pane.active_tool_inspector()?;
        let tool_call = active_tool_inspector.tool_call;

        Some(render::ToolInspectorView {
            tool_id: tool_call.tool_id,
            tool_name: tool_call.tool_name,
            args_preview: tool_call.args_preview,
            status: tool_call.status,
            scroll_offset: active_tool_inspector.scroll_offset,
            position: active_tool_inspector.position,
            total: active_tool_inspector.total,
        })
    }
    fn stats_overlay(&self) -> Option<render::StatsOverlayView<'_>> {
        let stats_overlay = self.stats_overlay.as_ref()?;
        Some(render::StatsOverlayView {
            snapshot: &stats_overlay.snapshot,
            active_tab: stats_overlay.active_tab,
            date_range: stats_overlay.date_range,
            list_scroll_offset: stats_overlay.list_scroll_offset,
            copy_status: stats_overlay.copy_status.as_deref(),
        })
    }
    fn session_picker(&self) -> Option<render::SessionPickerView<'_>> {
        let session_picker = self.session_picker.as_ref()?;
        Some(render::SessionPickerView {
            picker: session_picker,
            current_session_id: self.pane.session_id.as_str(),
        })
    }
    fn slash_command_selection(&self) -> usize {
        self.slash_command_selection
    }
    fn slash_palette_entries(&self, draft_prefix: &str) -> Vec<render::SlashPaletteEntry> {
        slash_palette_entries(self, draft_prefix)
    }
}

// ---------------------------------------------------------------------------
// RAII terminal guard
// ---------------------------------------------------------------------------

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalGuard {
    fn enter() -> CliResult<Self> {
        enable_raw_mode().map_err(|error| format!("failed to enable raw mode: {error}"))?;

        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(format!("failed to enter alternate screen: {error}"));
        }

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
                return Err(format!("failed to initialize TUI terminal: {error}"));
            }
        };

        if let Err(error) = terminal.hide_cursor() {
            let _ = disable_raw_mode();
            let _ = execute!(
                terminal.backend_mut(),
                DisableMouseCapture,
                LeaveAlternateScreen
            );
            return Err(format!("failed to hide TUI cursor: {error}"));
        }

        Ok(Self { terminal })
    }

    fn draw(
        &mut self,
        shell: &state::Shell,
        textarea: &tui_textarea::TextArea<'_>,
        palette: &Palette,
    ) -> CliResult<()> {
        self.terminal
            .draw(|frame| render::draw(frame, shell, textarea, palette))
            .map(|_| ())
            .map_err(|error| format!("failed to draw TUI frame: {error}"))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

// ---------------------------------------------------------------------------
// Streaming-tracking observer wrapper
// ---------------------------------------------------------------------------

/// Wraps a `ConversationTurnObserver` to track whether streaming tokens
/// were delivered, so the shell can send a fallback reply for non-streaming
/// providers.
struct TrackingObserver {
    inner: ConversationTurnObserverHandle,
    streamed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl crate::conversation::ConversationTurnObserver for TrackingObserver {
    fn on_phase(&self, event: crate::conversation::ConversationTurnPhaseEvent) {
        self.inner.on_phase(event);
    }

    fn on_tool(&self, event: crate::conversation::ConversationTurnToolEvent) {
        self.inner.on_tool(event);
    }

    fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
        if event.event_type == "text_delta" {
            self.streamed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.inner.on_streaming_token(event);
    }
}

// ---------------------------------------------------------------------------
// Turn runner
// ---------------------------------------------------------------------------

async fn run_turn(
    runtime: &super::runtime::TuiRuntime,
    input: &str,
    observer_handle: Option<ConversationTurnObserverHandle>,
) -> CliResult<String> {
    let turn_config = runtime
        .config
        .reload_provider_runtime_state_from_path(runtime.resolved_path.as_path())?;
    let acp_options = AcpConversationTurnOptions::automatic();
    runtime
        .turn_coordinator
        .handle_turn_with_address_and_acp_options_and_observer(
            &turn_config,
            &runtime.session_address,
            input,
            ProviderErrorMode::InlineMessage,
            &acp_options,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
            observer_handle,
        )
        .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResumeAction {
    List,
    Inspect(String),
    Switch(String),
}

fn parse_resume_action(args: &str) -> ResumeAction {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return ResumeAction::List;
    }

    if let Some(session_id) = trimmed.strip_prefix("inspect ") {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            return ResumeAction::Inspect(session_id.to_owned());
        }
    }

    if let Some(session_id) = trimmed.strip_prefix("switch ") {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            return ResumeAction::Switch(session_id.to_owned());
        }
    }

    ResumeAction::Switch(trimmed.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SubagentAction {
    List,
    Switch(String),
}

fn parse_subagent_action(args: &str) -> SubagentAction {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return SubagentAction::List;
    }

    if let Some(session_id) = trimmed.strip_prefix("switch ") {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            return SubagentAction::Switch(session_id.to_owned());
        }
    }

    SubagentAction::Switch(trimmed.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ApprovalResolveDecision {
    ApproveOnce,
    ApproveAlways,
    Deny,
}

impl ApprovalResolveDecision {
    fn as_payload_value(&self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::ApproveAlways => "approve_always",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ApprovalsAction {
    List {
        filter: String,
    },
    Inspect {
        approval_request_id: String,
    },
    Resolve {
        approval_request_id: String,
        decision: ApprovalResolveDecision,
        session_consent_mode: Option<String>,
    },
}

fn parse_approvals_action(args: &str) -> Result<ApprovalsAction, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(ApprovalsAction::List {
            filter: String::new(),
        });
    }

    let mut parts = trimmed.split_whitespace();
    let first = parts.next().unwrap_or_default();
    if first.eq_ignore_ascii_case("resolve") {
        let approval_request_id = parts
            .next()
            .ok_or_else(|| {
                "usage: `/approvals resolve <request-id> <approve-once|approve-always|deny> [auto|full]`"
                    .to_owned()
            })?
            .trim()
            .to_owned();
        let decision_raw = parts.next().ok_or_else(|| {
            "usage: `/approvals resolve <request-id> <approve-once|approve-always|deny> [auto|full]`"
                .to_owned()
        })?;
        let decision = match decision_raw.to_ascii_lowercase().as_str() {
            "approve-once" | "approve_once" => ApprovalResolveDecision::ApproveOnce,
            "approve-always" | "approve_always" => ApprovalResolveDecision::ApproveAlways,
            "deny" => ApprovalResolveDecision::Deny,
            other => {
                return Err(format!(
                    "unsupported approval decision `{other}`; use `approve-once`, `approve-always`, or `deny`"
                ));
            }
        };
        let session_consent_mode = parts.next().map(str::to_ascii_lowercase);
        if let Some(mode) = session_consent_mode.as_deref()
            && mode != "auto"
            && mode != "full"
        {
            return Err(format!(
                "unsupported session consent mode `{mode}`; use `auto` or `full`"
            ));
        }
        if parts.next().is_some() {
            return Err(
                "usage: `/approvals resolve <request-id> <approve-once|approve-always|deny> [auto|full]`"
                    .to_owned(),
            );
        }

        return Ok(ApprovalsAction::Resolve {
            approval_request_id,
            decision,
            session_consent_mode,
        });
    }

    let known_filters = [
        "all",
        "pending",
        "attention",
        "approved",
        "executing",
        "executed",
        "denied",
        "expired",
        "cancelled",
    ];
    if known_filters.contains(&first.to_ascii_lowercase().as_str()) {
        return Ok(ApprovalsAction::List {
            filter: first.to_owned(),
        });
    }

    Ok(ApprovalsAction::Inspect {
        approval_request_id: trimmed.to_owned(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelReasoningChoice {
    Auto,
    Explicit(crate::config::ReasoningEffort),
}

impl ModelReasoningChoice {
    fn display_label(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Explicit(effort) => effort.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelAction {
    Status,
    Switch {
        selector: String,
        reasoning: Option<ModelReasoningChoice>,
    },
}

fn parse_reasoning_choice(raw: &str) -> Option<ModelReasoningChoice> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ModelReasoningChoice::Auto),
        "none" => Some(ModelReasoningChoice::Explicit(
            crate::config::ReasoningEffort::None,
        )),
        "minimal" => Some(ModelReasoningChoice::Explicit(
            crate::config::ReasoningEffort::Minimal,
        )),
        "low" => Some(ModelReasoningChoice::Explicit(
            crate::config::ReasoningEffort::Low,
        )),
        "medium" => Some(ModelReasoningChoice::Explicit(
            crate::config::ReasoningEffort::Medium,
        )),
        "high" => Some(ModelReasoningChoice::Explicit(
            crate::config::ReasoningEffort::High,
        )),
        "xhigh" | "max" => Some(ModelReasoningChoice::Explicit(
            crate::config::ReasoningEffort::Xhigh,
        )),
        _ => None,
    }
}

fn parse_model_action(args: &str) -> Result<ModelAction, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(ModelAction::Status);
    }

    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [selector] => Ok(ModelAction::Switch {
            selector: selector.trim().to_owned(),
            reasoning: None,
        }),
        [selector, reasoning] => {
            let reasoning = parse_reasoning_choice(reasoning).ok_or_else(|| {
                format!(
                    "unsupported reasoning level `{reasoning}`; use `auto`, `none`, `minimal`, `low`, `medium`, `high`, or `xhigh`"
                )
            })?;
            Ok(ModelAction::Switch {
                selector: selector.trim().to_owned(),
                reasoning: Some(reasoning),
            })
        }
        _ => Err(
            "usage: `/model [selector]` or `/model <selector> <auto|none|minimal|low|medium|high|xhigh>`"
                .to_owned(),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusyInputModeAction {
    Status,
    Set(state::BusyInputMode),
    Toggle,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RenameAction {
    Status,
    Set(String),
}

fn parse_rename_action(args: &str) -> Result<RenameAction, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(RenameAction::Status);
    }

    Ok(RenameAction::Set(trimmed.to_owned()))
}

fn parse_busy_input_mode_action(args: &str) -> Result<BusyInputModeAction, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(BusyInputModeAction::Status);
    }

    let normalized = trimmed.to_ascii_lowercase();
    match normalized.as_str() {
        "queue" => Ok(BusyInputModeAction::Set(state::BusyInputMode::Queue)),
        "steer" => Ok(BusyInputModeAction::Set(state::BusyInputMode::Steer)),
        "toggle" | "cycle" => Ok(BusyInputModeAction::Toggle),
        "clear" => Ok(BusyInputModeAction::Clear),
        _ => Err("usage: `/mode [queue|steer|toggle|clear]`".to_owned()),
    }
}

fn is_async_slash_request(request: &ParsedSlashCommand) -> bool {
    matches!(request.command, SlashCommand::Compact)
        || matches!(
            parse_approvals_action(request.args.as_str()),
            Ok(ApprovalsAction::Resolve { .. })
        )
}

fn is_runtime_slash_request(request: &ParsedSlashCommand) -> bool {
    matches!(request.command, SlashCommand::New)
        || matches!(request.command, SlashCommand::Rename)
            && matches!(
                parse_rename_action(request.args.as_str()),
                Ok(RenameAction::Set(_))
            )
        || matches!(request.command, SlashCommand::Resume)
            && matches!(
                parse_resume_action(request.args.as_str()),
                ResumeAction::Switch(_)
            )
        || matches!(request.command, SlashCommand::Subagents)
            && matches!(
                parse_subagent_action(request.args.as_str()),
                SubagentAction::Switch(_)
            )
        || matches!(request.command, SlashCommand::Model)
            && matches!(
                parse_model_action(request.args.as_str()),
                Ok(ModelAction::Switch { .. })
            )
}

async fn run_compact_command(
    runtime: &super::runtime::TuiRuntime,
) -> CliResult<(String, Vec<String>)> {
    let config = runtime
        .config
        .reload_provider_runtime_state_from_path(runtime.resolved_path.as_path())?;
    let selection = resolve_context_engine_selection(&config);
    let context_engine = resolve_context_engine(Some(selection.id.as_str()))?;
    let workspace_root = config
        .tools
        .file_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|_| config.tools.resolved_file_root());
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let compact_diagnostics = crate::memory::run_compact_stage(
        runtime.session_id.as_str(),
        workspace_root.as_deref(),
        &memory_config,
    )
    .await?;

    let mut lines = vec![
        format!("- session: {}", runtime.session_id),
        format!(
            "- context engine: {} ({})",
            selection.id,
            selection.source.as_str()
        ),
        format!("- memory stage: {}", compact_diagnostics.outcome.as_str()),
    ];

    if let Some(workspace_root) = workspace_root.as_ref() {
        lines.push(format!("- workspace root: {}", workspace_root.display()));
    } else {
        lines.push("- workspace root: unavailable".to_owned());
    }

    if compact_diagnostics.fallback_activated {
        lines.push("- durable memory flush: fallback activated".to_owned());
    }

    if let Some(message) = compact_diagnostics.message.as_deref() {
        lines.push(format!("- memory note: {message}"));
    }

    if matches!(
        compact_diagnostics.outcome,
        crate::memory::StageOutcome::Fallback
    ) {
        if config.conversation.compaction_fail_open() {
            lines.push("- result: skipped context rewrite because fail-open is enabled".to_owned());
            return Ok(("context compaction".to_owned(), lines));
        }

        return Err(format!(
            "pre-compaction durable memory flush failed: {}",
            compact_diagnostics
                .message
                .as_deref()
                .unwrap_or("compact stage fallback without error detail")
        ));
    }

    context_engine
        .compact_context(
            &config,
            runtime.session_id.as_str(),
            &[],
            &runtime.kernel_ctx,
        )
        .await?;
    lines.push("- result: context compaction completed".to_owned());

    Ok(("context compaction".to_owned(), lines))
}

async fn run_approval_resolve_command(
    runtime: &super::runtime::TuiRuntime,
    approval_request_id: &str,
    decision: ApprovalResolveDecision,
    session_consent_mode: Option<&str>,
) -> CliResult<(String, Vec<String>)> {
    let config = runtime
        .config
        .reload_provider_runtime_state_from_path(runtime.resolved_path.as_path())?;
    let conversation_runtime = DefaultConversationRuntime::from_config_or_env(&config)?;
    let mut payload = json!({
        "approval_request_id": approval_request_id,
        "decision": decision.as_payload_value(),
    });
    if let Some(mode) = session_consent_mode
        && let Some(object) = payload.as_object_mut()
    {
        object.insert(
            "session_consent_mode".to_owned(),
            Value::String(mode.to_owned()),
        );
    }

    let outcome = crate::conversation::execute_approval_tool_with_runtime_support(
        &config,
        &conversation_runtime,
        runtime.session_id.as_str(),
        ToolCoreRequest {
            tool_name: "approval_request_resolve".to_owned(),
            payload,
        },
        ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
    )
    .await?;

    let approval_request = outcome
        .payload
        .get("approval_request")
        .cloned()
        .unwrap_or(Value::Null);
    let mut lines = approval_request_lines(&approval_request);
    if let Some(mode) = session_consent_mode {
        lines.push(format!("- session consent mode: {mode}"));
    }
    if let Some(resumed_tool_output) = outcome.payload.get("resumed_tool_output") {
        let status = resumed_tool_output
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        lines.push(format!("- replayed tool result: {status}"));
        let tool_payload_preview = json_preview(resumed_tool_output.get("payload"), 120);
        lines.push(format!("  payload: {tool_payload_preview}"));
    } else {
        lines.push("- replayed tool result: not applicable".to_owned());
    }

    Ok(("approval resolution".to_owned(), lines))
}

fn prepare_async_slash_command_future(
    runtime: std::sync::Arc<super::runtime::TuiRuntime>,
    request: ParsedSlashCommand,
    tx: mpsc::UnboundedSender<UiEvent>,
) -> Pin<Box<dyn std::future::Future<Output = ()>>> {
    Box::pin(async move {
        let result = if matches!(request.command, SlashCommand::Compact) {
            if request.args.trim().is_empty() {
                run_compact_command(runtime.as_ref()).await
            } else {
                Err(format!(
                    "`/compact` does not take arguments, received `{}`",
                    request.args.trim()
                ))
            }
        } else if matches!(request.command, SlashCommand::Approvals) {
            match parse_approvals_action(request.args.as_str()) {
                Ok(ApprovalsAction::Resolve {
                    approval_request_id,
                    decision,
                    session_consent_mode,
                }) => {
                    run_approval_resolve_command(
                        runtime.as_ref(),
                        approval_request_id.as_str(),
                        decision,
                        session_consent_mode.as_deref(),
                    )
                    .await
                }
                Ok(_) => Err(
                    "approval async dispatch requires `resolve`; use `/approvals` or `/approvals <id>` for read-only views".to_owned(),
                ),
                Err(error) => Err(error),
            }
        } else {
            Err(format!(
                "async slash command dispatch is not implemented for `{:?}`",
                request.command
            ))
        };

        match result {
            Ok((title, lines)) => {
                let _ = tx.send(UiEvent::Surface { title, lines });
            }
            Err(error) => {
                let _ = tx.send(UiEvent::TurnError(error));
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Event application
// ---------------------------------------------------------------------------

fn apply_ui_event(shell: &mut state::Shell, event: UiEvent) {
    match event {
        UiEvent::Tick => {
            shell.pane.tick_animations();
        }
        UiEvent::Terminal(_) => {}
        UiEvent::Token {
            content,
            is_thinking,
        } => {
            shell.pane.append_token(&content, is_thinking);
        }
        UiEvent::ToolStart {
            tool_id,
            tool_name,
            args_preview,
        } => {
            shell
                .pane
                .start_tool_call(&tool_id, &tool_name, &args_preview);
        }
        UiEvent::ToolArgsDelta { tool_id, chunk } => {
            shell.pane.append_tool_call_args(&tool_id, &chunk);
        }
        UiEvent::ToolDone {
            tool_id,
            success,
            output,
            duration_ms,
        } => {
            shell
                .pane
                .complete_tool_call(&tool_id, success, &output, duration_ms);
        }
        UiEvent::PhaseChange {
            phase,
            iteration,
            action,
        } => {
            shell.pane.loop_state = phase;
            shell.pane.loop_action = action;
            shell.pane.loop_iteration = iteration;
        }
        UiEvent::ResponseDone {
            input_tokens,
            output_tokens,
        } => {
            shell.pane.finalize_response(input_tokens, output_tokens);
            refresh_composer_suggestion_context(shell);
            maybe_emit_subagent_lifecycle_surface(shell);
        }
        UiEvent::ClarifyRequest { question, choices } => {
            shell.pane.clarify_dialog = Some(ClarifyDialog::new(question, choices));
            shell.focus.push(FocusLayer::ClarifyDialog);
        }
        UiEvent::Surface { title, lines } => {
            append_surface_message(shell, &title, lines.as_slice());
            shell
                .pane
                .set_status(format!("{title} added to transcript"));
            refresh_composer_suggestion_context(shell);
            maybe_emit_subagent_lifecycle_surface(shell);
        }
        UiEvent::TurnError(msg) => {
            shell.pane.agent_running = false;
            shell.pane.add_system_message(&format!("Error: {msg}"));
            refresh_composer_suggestion_context(shell);
            maybe_emit_subagent_lifecycle_surface(shell);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryNavigationAction {
    ScrollLineUp,
    ScrollLineDown,
    ScrollHalfPageUp,
    ScrollHalfPageDown,
    ScrollPageUp,
    ScrollPageDown,
    JumpTop,
    JumpLatest,
}

fn textarea_is_empty(textarea: &tui_textarea::TextArea<'_>) -> bool {
    let lines = textarea.lines();
    let has_non_empty_line = lines.iter().any(|line| !line.is_empty());
    !has_non_empty_line
}

#[allow(clippy::wildcard_enum_match_arm)]
fn history_navigation_action(
    key: KeyEvent,
    composer_is_empty: bool,
) -> Option<HistoryNavigationAction> {
    match key.code {
        KeyCode::Up if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineUp)
        }
        KeyCode::Down if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineDown)
        }
        KeyCode::PageUp => Some(HistoryNavigationAction::ScrollPageUp),
        KeyCode::PageDown => Some(HistoryNavigationAction::ScrollPageDown),
        KeyCode::Home if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::JumpTop)
        }
        KeyCode::End if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::JumpLatest)
        }
        _ => None,
    }
}

#[allow(clippy::wildcard_enum_match_arm)]
fn transcript_navigation_action(key: KeyEvent) -> Option<HistoryNavigationAction> {
    match key.code {
        KeyCode::Up => Some(HistoryNavigationAction::ScrollLineUp),
        KeyCode::Down => Some(HistoryNavigationAction::ScrollLineDown),
        KeyCode::Char('k') if key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineUp)
        }
        KeyCode::Char('j') if key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineDown)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(HistoryNavigationAction::ScrollHalfPageUp)
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(HistoryNavigationAction::ScrollHalfPageDown)
        }
        KeyCode::PageUp => Some(HistoryNavigationAction::ScrollPageUp),
        KeyCode::PageDown => Some(HistoryNavigationAction::ScrollPageDown),
        KeyCode::Home => Some(HistoryNavigationAction::JumpTop),
        KeyCode::End => Some(HistoryNavigationAction::JumpLatest),
        KeyCode::Char('g') if key.modifiers.is_empty() => Some(HistoryNavigationAction::JumpTop),
        KeyCode::Char('G') => Some(HistoryNavigationAction::JumpLatest),
        _ => None,
    }
}

fn history_page_step(textarea: &tui_textarea::TextArea<'_>) -> u16 {
    let terminal_size = crossterm::terminal::size();
    let (width, height) = terminal_size.unwrap_or((80, 24));

    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let input_height = textarea.lines().len() as u16 + 2;
    let shell_areas = layout::compute(area, input_height);
    let history_height = shell_areas.history.height;
    let page_step = history_height.saturating_sub(1);

    page_step.max(1)
}

fn history_half_page_step(textarea: &tui_textarea::TextArea<'_>) -> u16 {
    let page_step = history_page_step(textarea);
    let half_page_step = page_step / 2;

    half_page_step.max(1)
}

fn terminal_shell_areas(textarea: &tui_textarea::TextArea<'_>) -> layout::ShellAreas {
    let terminal_size = crossterm::terminal::size();
    let (width, height) = terminal_size.unwrap_or((80, 24));
    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let input_height = textarea.lines().len() as u16 + 2;

    layout::compute(area, input_height)
}

fn session_picker_visible_rows() -> usize {
    let terminal_size = crossterm::terminal::size();
    let (width, height) = terminal_size.unwrap_or((80, 24));
    if width < 56 || height < 14 {
        return 1;
    }

    let popup_height = {
        let max_height = height.saturating_sub(2);
        let preferred_height = height.saturating_mul(3) / 4;
        preferred_height.max(14).min(max_height)
    };
    let inner_height = popup_height.saturating_sub(2);
    let list_height = inner_height.saturating_sub(2);

    usize::from(list_height.max(1))
}

fn apply_history_navigation(
    shell: &mut state::Shell,
    textarea: &tui_textarea::TextArea<'_>,
    action: HistoryNavigationAction,
) {
    match action {
        HistoryNavigationAction::ScrollLineUp => {
            let next_offset = shell.pane.scroll_offset.saturating_add(1);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollLineDown => {
            let next_offset = shell.pane.scroll_offset.saturating_sub(1);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollHalfPageUp => {
            let half_page_step = history_half_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_add(half_page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollHalfPageDown => {
            let half_page_step = history_half_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_sub(half_page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollPageUp => {
            let page_step = history_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_add(page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollPageDown => {
            let page_step = history_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_sub(page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::JumpTop => {
            shell.pane.scroll_offset = u16::MAX;
            shell.pane.set_status("Viewing oldest output".to_owned());
        }
        HistoryNavigationAction::JumpLatest => {
            shell.pane.scroll_offset = 0;
            shell.pane.set_status("Jumped to latest output".to_owned());
        }
    }
}

fn point_in_rect(area: ratatui::layout::Rect, column: u16, row: u16) -> bool {
    let within_x = column >= area.x && column < area.x.saturating_add(area.width);
    let within_y = row >= area.y && row < area.y.saturating_add(area.height);

    within_x && within_y
}

fn command_palette_entries(prefix: &str) -> Vec<render::SlashPaletteEntry> {
    commands::completions(prefix)
        .into_iter()
        .map(|spec| {
            let label = match spec.argument_hint {
                Some(argument_hint) => format!("{} {}", spec.name, argument_hint),
                None => spec.name.to_owned(),
            };
            let immediate = commands::parse(spec.name).is_some_and(|request| {
                !is_async_slash_request(&request)
                    && !is_runtime_slash_request(&request)
                    && spec.argument_hint.is_none()
            });

            render::SlashPaletteEntry {
                replacement: spec.name.to_owned(),
                label,
                meta: spec.category.to_owned(),
                detail: spec.help.to_owned(),
                immediate,
                submit_on_select: false,
            }
        })
        .collect()
}

fn matches_candidate_query(candidate: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    candidate.to_ascii_lowercase().contains(query)
}

fn session_presentation_locale() -> SessionPresentationLocale {
    SessionPresentationLocale::detect_from_env()
}

fn visible_session_primary_label(
    session: &state::VisibleSessionSuggestion,
    locale: SessionPresentationLocale,
) -> String {
    if let Some(presentation) = session.agent_presentation.as_ref() {
        return presentation.primary_label(locale);
    }

    if let Some(label) = session.label.as_deref() {
        return label.to_owned();
    }

    session.session_id.clone()
}

fn subagent_session_primary_label(
    session: &state::VisibleSessionSuggestion,
    locale: SessionPresentationLocale,
) -> String {
    if session.kind == "root" {
        if let Some(label) = session.label.as_deref() {
            let trimmed_label = label.trim();
            if !trimmed_label.is_empty() {
                return trimmed_label.to_owned();
            }
        }
        return localized_root_thread_label(locale).to_owned();
    }

    visible_session_primary_label(session, locale)
}

fn visible_session_meta_label(session: &state::VisibleSessionSuggestion) -> &'static str {
    match session.kind.as_str() {
        "root" => "Primary",
        "delegate_child" => "Subagent",
        _ => "Session",
    }
}

fn visible_session_kind_detail_label(session: &state::VisibleSessionSuggestion) -> &'static str {
    match session.kind.as_str() {
        "root" => "thread",
        "delegate_child" => "subagent",
        _ => "session",
    }
}

fn visible_session_status_detail(session: &state::VisibleSessionSuggestion) -> String {
    let status_label = session
        .task_phase
        .as_deref()
        .unwrap_or(session.state.as_str());
    let kind_label = visible_session_kind_detail_label(session);
    format!("{status_label} · {kind_label}")
}

fn visible_session_provider_label(
    session: &state::VisibleSessionSuggestion,
    locale: SessionPresentationLocale,
) -> Option<String> {
    session
        .agent_presentation
        .as_ref()
        .and_then(|presentation| presentation.provider_label(locale))
}

fn visible_session_matches_query(session: &state::VisibleSessionSuggestion, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let session_id_matches = matches_candidate_query(session.session_id.as_str(), query);
    if session_id_matches {
        return true;
    }

    let label_matches = session
        .label
        .as_deref()
        .is_some_and(|label| matches_candidate_query(label, query));
    if label_matches {
        return true;
    }

    let phase_matches = session
        .task_phase
        .as_deref()
        .is_some_and(|phase| matches_candidate_query(phase, query));
    if phase_matches {
        return true;
    }

    if session.kind == "root" {
        let alias_matches = root_thread_search_terms()
            .iter()
            .any(|term| matches_candidate_query(term, query));
        if alias_matches {
            return true;
        }
    }

    let Some(presentation) = session.agent_presentation.as_ref() else {
        return false;
    };

    let zh_hans_matches = matches_candidate_query(presentation.names.zh_hans.as_str(), query);
    let zh_hant_matches = matches_candidate_query(presentation.names.zh_hant.as_str(), query);
    let en_matches = matches_candidate_query(presentation.names.en.as_str(), query);
    let ja_matches = matches_candidate_query(presentation.names.ja.as_str(), query);
    let role_matches = matches_candidate_query(presentation.role_id.as_str(), query);
    let model_matches = presentation
        .model
        .as_deref()
        .is_some_and(|model| matches_candidate_query(model, query));
    let reasoning_matches = presentation
        .reasoning_effort
        .as_deref()
        .is_some_and(|reasoning| matches_candidate_query(reasoning, query));

    zh_hans_matches
        || zh_hant_matches
        || en_matches
        || ja_matches
        || role_matches
        || model_matches
        || reasoning_matches
}

fn resume_palette_entries(shell: &state::Shell, args: &str) -> Vec<render::SlashPaletteEntry> {
    let context = &shell.pane.composer_suggestion_context;
    let action = parse_resume_action(args);
    let raw_query = match &action {
        ResumeAction::List => "",
        ResumeAction::Inspect(query) | ResumeAction::Switch(query) => query.as_str(),
    };
    let query = raw_query.trim().to_ascii_lowercase();
    let inspect_mode = matches!(action, ResumeAction::Inspect(_));
    let locale = session_presentation_locale();

    context
        .visible_session_suggestions
        .iter()
        .filter(|session| visible_session_matches_query(session, query.as_str()))
        .take(6)
        .map(|session| {
            let replacement = if inspect_mode {
                format!("/resume inspect {}", session.session_id)
            } else {
                format!("/resume {}", session.session_id)
            };
            let mut detail = visible_session_status_detail(session);
            if let Some(presentation) = session.agent_presentation.as_ref() {
                detail.push_str(&format!(" · {}", presentation.primary_label(locale)));
                if let Some(provider_label) = visible_session_provider_label(session, locale) {
                    detail.push_str(&format!(" · {provider_label}"));
                }
            }
            if let Some(label) = session.label.as_deref() {
                detail.push_str(&format!(" · {label}"));
            }
            render::SlashPaletteEntry {
                label: visible_session_primary_label(session, locale),
                replacement,
                meta: if inspect_mode {
                    "Resume Inspect".to_owned()
                } else {
                    "Resume Switch".to_owned()
                },
                detail,
                immediate: false,
                submit_on_select: true,
            }
        })
        .collect()
}

fn subagents_palette_entries(shell: &state::Shell, args: &str) -> Vec<render::SlashPaletteEntry> {
    let query = args.trim().to_ascii_lowercase();
    let locale = session_presentation_locale();

    shell
        .pane
        .composer_suggestion_context
        .visible_session_suggestions
        .iter()
        .filter(|session| session.kind == "root" || session.kind == "delegate_child")
        .filter(|session| {
            visible_session_matches_query(session, query.as_str())
                || matches_candidate_query(
                    subagent_session_primary_label(session, locale).as_str(),
                    query.as_str(),
                )
        })
        .take(6)
        .map(|session| {
            let mut detail_parts = vec![visible_session_status_detail(session)];
            if session.overdue {
                detail_parts.push("overdue".to_owned());
            }
            if let Some(provider_label) = visible_session_provider_label(session, locale) {
                detail_parts.push(provider_label);
            }
            if let Some(label) = session.label.as_deref() {
                detail_parts.push(label.to_owned());
            }
            if session.attention_approval_count > 0 {
                detail_parts.push(format!("APR! {}", session.attention_approval_count));
            }
            let remaining_pending = session
                .pending_approval_count
                .saturating_sub(session.attention_approval_count);
            if remaining_pending > 0 {
                detail_parts.push(format!("APR {remaining_pending}"));
            }

            let detail = detail_parts.join(" · ");
            let replacement = format!("/subagents {}", session.session_id);

            render::SlashPaletteEntry {
                replacement,
                label: subagent_session_primary_label(session, locale),
                meta: visible_session_meta_label(session).to_owned(),
                detail,
                immediate: false,
                submit_on_select: true,
            }
        })
        .collect()
}

fn approval_decision_palette_entries(
    approval_request_id: &str,
    decision_query: &str,
) -> Vec<render::SlashPaletteEntry> {
    [
        ("approve-once", "Resolve once and replay the blocked tool"),
        (
            "approve-always",
            "Persist a reusable grant and replay the blocked tool",
        ),
        (
            "deny",
            "Deny the request without replaying the blocked tool",
        ),
    ]
    .into_iter()
    .filter(|(decision, _)| matches_candidate_query(decision, decision_query))
    .map(|(decision, detail)| render::SlashPaletteEntry {
        replacement: format!("/approvals resolve {approval_request_id} {decision}"),
        label: format!("/approvals resolve {approval_request_id} {decision}"),
        meta: "Approval Resolve".to_owned(),
        detail: detail.to_owned(),
        immediate: false,
        submit_on_select: true,
    })
    .collect()
}

fn approval_mode_palette_entries(
    approval_request_id: &str,
    mode_query: &str,
) -> Vec<render::SlashPaletteEntry> {
    [
        ("auto", "Set this root session to auto consent"),
        ("full", "Set this root session to full consent"),
    ]
    .into_iter()
    .filter(|(mode, _)| matches_candidate_query(mode, mode_query))
    .map(|(mode, detail)| render::SlashPaletteEntry {
        replacement: format!("/approvals resolve {approval_request_id} approve-once {mode}"),
        label: format!("/approvals resolve {approval_request_id} approve-once {mode}"),
        meta: "Approval Mode".to_owned(),
        detail: detail.to_owned(),
        immediate: false,
        submit_on_select: true,
    })
    .collect()
}

fn approvals_palette_entries(shell: &state::Shell, args: &str) -> Vec<render::SlashPaletteEntry> {
    let context = &shell.pane.composer_suggestion_context;
    if args.trim().to_ascii_lowercase().starts_with("resolve") {
        let parts = args.split_whitespace().collect::<Vec<_>>();
        return match parts.as_slice() {
            ["resolve"] | ["resolve", ""] => context
                .approval_request_suggestions
                .iter()
                .take(6)
                .map(|approval| render::SlashPaletteEntry {
                    replacement: format!(
                        "/approvals resolve {} approve-once",
                        approval.approval_request_id
                    ),
                    label: format!("/approvals resolve {}", approval.approval_request_id),
                    meta: "Approval Resolve".to_owned(),
                    detail: format!(
                        "{} · {} · {}",
                        approval.status, approval.tool_name, approval.session_id
                    ),
                    immediate: false,
                    submit_on_select: false,
                })
                .collect(),
            ["resolve", approval_request_id] => {
                approval_decision_palette_entries(approval_request_id, "")
            }
            ["resolve", approval_request_id, decision_query] => {
                let normalized = decision_query.to_ascii_lowercase();
                if normalized == "approve-once" || normalized == "approve_once" {
                    approval_mode_palette_entries(approval_request_id, "")
                } else {
                    approval_decision_palette_entries(approval_request_id, normalized.as_str())
                }
            }
            ["resolve", approval_request_id, decision, mode_query] => {
                let normalized = decision.to_ascii_lowercase();
                if normalized == "approve-once" || normalized == "approve_once" {
                    approval_mode_palette_entries(approval_request_id, mode_query)
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };
    }

    match parse_approvals_action(args) {
        Ok(ApprovalsAction::List { filter }) => {
            let filter = filter.to_ascii_lowercase();
            let filter_entries = [
                ("pending", "Show pending approval requests"),
                ("attention", "Show approval requests that need attention"),
                ("all", "Show approval requests across all visible states"),
            ]
            .into_iter()
            .filter(|(name, _)| matches_candidate_query(name, filter.as_str()))
            .map(|(name, detail)| render::SlashPaletteEntry {
                replacement: format!("/approvals {name}"),
                label: format!("/approvals {name}"),
                meta: "Approval Filter".to_owned(),
                detail: detail.to_owned(),
                immediate: false,
                submit_on_select: true,
            });

            let approval_entries = context
                .approval_request_suggestions
                .iter()
                .filter(|approval| {
                    matches_candidate_query(approval.approval_request_id.as_str(), filter.as_str())
                        || matches_candidate_query(approval.tool_name.as_str(), filter.as_str())
                        || matches_candidate_query(approval.session_id.as_str(), filter.as_str())
                })
                .map(|approval| render::SlashPaletteEntry {
                    replacement: format!("/approvals {}", approval.approval_request_id),
                    label: format!("/approvals {}", approval.approval_request_id),
                    meta: "Approval".to_owned(),
                    detail: format!(
                        "{} · {} · {}{}",
                        approval.status,
                        approval.tool_name,
                        approval.session_id,
                        if approval.needs_attention {
                            " · attention"
                        } else {
                            ""
                        }
                    ),
                    immediate: false,
                    submit_on_select: true,
                });

            filter_entries.chain(approval_entries).take(6).collect()
        }
        Ok(ApprovalsAction::Inspect {
            approval_request_id,
        }) => context
            .approval_request_suggestions
            .iter()
            .filter(|approval| {
                matches_candidate_query(
                    approval.approval_request_id.as_str(),
                    approval_request_id.to_ascii_lowercase().as_str(),
                ) || matches_candidate_query(
                    approval.tool_name.as_str(),
                    approval_request_id.to_ascii_lowercase().as_str(),
                )
            })
            .take(6)
            .map(|approval| render::SlashPaletteEntry {
                replacement: format!("/approvals {}", approval.approval_request_id),
                label: format!("/approvals {}", approval.approval_request_id),
                meta: "Approval".to_owned(),
                detail: format!(
                    "{} · {} · {}{}",
                    approval.status,
                    approval.tool_name,
                    approval.session_id,
                    if approval.needs_attention {
                        " · attention"
                    } else {
                        ""
                    }
                ),
                immediate: false,
                submit_on_select: true,
            })
            .collect(),
        Ok(ApprovalsAction::Resolve { .. }) | Err(_) => Vec::new(),
    }
}

fn tasks_palette_entries(shell: &state::Shell, args: &str) -> Vec<render::SlashPaletteEntry> {
    let query = args.trim().to_ascii_lowercase();
    let locale = session_presentation_locale();
    let filter_entries = [
        ("running", "Show active delegate task sessions"),
        ("overdue", "Show overdue delegate task sessions"),
        ("queued", "Show queued delegate task sessions"),
        ("failed", "Show failed delegate task sessions"),
        ("completed", "Show completed delegate task sessions"),
    ]
    .into_iter()
    .filter(|(name, _)| matches_candidate_query(name, query.as_str()))
    .map(|(name, detail)| render::SlashPaletteEntry {
        replacement: format!("/tasks {name}"),
        label: format!("/tasks {name}"),
        meta: "Task Filter".to_owned(),
        detail: detail.to_owned(),
        immediate: false,
        submit_on_select: true,
    });

    let session_entries = shell
        .pane
        .composer_suggestion_context
        .visible_session_suggestions
        .iter()
        .filter(|session| session.kind == "delegate_child")
        .filter(|session| {
            visible_session_matches_query(session, query.as_str())
                || (query == "overdue" && session.overdue)
        })
        .map(|session| {
            let mut detail = visible_session_status_detail(session);
            if session.overdue {
                detail.push_str(" · overdue");
            }
            if let Some(presentation) = session.agent_presentation.as_ref() {
                detail.push_str(&format!(" · {}", presentation.primary_label(locale)));
                if let Some(provider_label) = visible_session_provider_label(session, locale) {
                    detail.push_str(&format!(" · {provider_label}"));
                }
            }
            if let Some(label) = session.label.as_deref() {
                detail.push_str(&format!(" · {label}"));
            }
            render::SlashPaletteEntry {
                replacement: format!("/tasks {}", session.session_id),
                label: visible_session_primary_label(session, locale),
                meta: "Task Session".to_owned(),
                detail,
                immediate: false,
                submit_on_select: true,
            }
        });

    filter_entries.chain(session_entries).take(6).collect()
}

fn permissions_palette_entries(shell: &state::Shell, args: &str) -> Vec<render::SlashPaletteEntry> {
    let query = args.trim().to_ascii_lowercase();
    let locale = session_presentation_locale();

    shell
        .pane
        .composer_suggestion_context
        .visible_session_suggestions
        .iter()
        .filter(|session| visible_session_matches_query(session, query.as_str()))
        .take(6)
        .map(|session| {
            let mut detail = visible_session_status_detail(session);
            if let Some(presentation) = session.agent_presentation.as_ref() {
                detail.push_str(&format!(" · {}", presentation.primary_label(locale)));
                if let Some(provider_label) = visible_session_provider_label(session, locale) {
                    detail.push_str(&format!(" · {provider_label}"));
                }
            }
            if let Some(label) = session.label.as_deref() {
                detail.push_str(&format!(" · {label}"));
            }
            render::SlashPaletteEntry {
                replacement: format!("/permissions {}", session.session_id),
                label: visible_session_primary_label(session, locale),
                meta: "Permissions".to_owned(),
                detail,
                immediate: false,
                submit_on_select: true,
            }
        })
        .collect()
}

fn model_effort_palette_entries(
    suggestion: &state::ModelSelectionSuggestion,
    effort_query: &str,
) -> Vec<render::SlashPaletteEntry> {
    let query = effort_query.trim().to_ascii_lowercase();
    let current_effort = suggestion.current_reasoning_effort.as_deref();

    let auto_entry = render::SlashPaletteEntry {
        replacement: format!("/model {} auto", suggestion.profile_id),
        label: format!("/model {} auto", suggestion.selector),
        meta: "Reasoning".to_owned(),
        detail: if current_effort.is_none() {
            "Use the provider default reasoning effort (current)".to_owned()
        } else {
            "Use the provider default reasoning effort".to_owned()
        },
        immediate: false,
        submit_on_select: true,
    };

    let effort_entries = suggestion
        .reasoning_efforts
        .iter()
        .filter(|effort| matches_candidate_query(effort, query.as_str()))
        .map(|effort| render::SlashPaletteEntry {
            replacement: format!("/model {} {}", suggestion.profile_id, effort),
            label: format!("/model {} {}", suggestion.selector, effort),
            meta: "Reasoning".to_owned(),
            detail: if current_effort == Some(effort.as_str()) {
                "Apply this reasoning effort (current)".to_owned()
            } else {
                "Apply this reasoning effort".to_owned()
            },
            immediate: false,
            submit_on_select: true,
        });

    matches_candidate_query("auto", query.as_str())
        .then_some(auto_entry)
        .into_iter()
        .chain(effort_entries)
        .take(7)
        .collect()
}

fn model_palette_entries(shell: &state::Shell, args: &str) -> Vec<render::SlashPaletteEntry> {
    let suggestions = &shell
        .pane
        .composer_suggestion_context
        .model_selection_suggestions;
    match parse_model_action(args) {
        Ok(ModelAction::Status) => suggestions
            .iter()
            .take(8)
            .map(|suggestion| {
                let detail = format!(
                    "{} · {}{}{}",
                    suggestion.model,
                    suggestion.kind,
                    if suggestion.active { " · current" } else { "" },
                    if suggestion.reasoning_efforts.is_empty() {
                        ""
                    } else {
                        " · reasoning"
                    }
                );
                let requires_reasoning_step = suggestion.reasoning_efforts.len() > 1;
                render::SlashPaletteEntry {
                    replacement: format!("/model {}", suggestion.profile_id),
                    label: format!("/model {}", suggestion.selector),
                    meta: "Model".to_owned(),
                    detail,
                    immediate: false,
                    submit_on_select: !requires_reasoning_step,
                }
            })
            .collect(),
        Ok(ModelAction::Switch {
            selector,
            reasoning,
        }) => {
            if reasoning.is_some() {
                return Vec::new();
            }

            let query = selector.trim().to_ascii_lowercase();
            let exact = suggestions.iter().find(|suggestion| {
                suggestion.selector.eq_ignore_ascii_case(selector.as_str())
                    || suggestion
                        .profile_id
                        .eq_ignore_ascii_case(selector.as_str())
                    || suggestion.model.eq_ignore_ascii_case(selector.as_str())
            });
            if let Some(suggestion) = exact
                && suggestion.reasoning_efforts.len() > 1
            {
                let entries = model_effort_palette_entries(suggestion, "");
                if !entries.is_empty() {
                    return entries;
                }
            }

            suggestions
                .iter()
                .filter(|suggestion| {
                    matches_candidate_query(suggestion.selector.as_str(), query.as_str())
                        || matches_candidate_query(suggestion.profile_id.as_str(), query.as_str())
                        || matches_candidate_query(suggestion.model.as_str(), query.as_str())
                        || matches_candidate_query(suggestion.kind.as_str(), query.as_str())
                })
                .take(8)
                .map(|suggestion| {
                    let detail = format!(
                        "{} · {}{}{}",
                        suggestion.model,
                        suggestion.kind,
                        if suggestion.active { " · current" } else { "" },
                        if suggestion.reasoning_efforts.is_empty() {
                            ""
                        } else {
                            " · reasoning"
                        }
                    );
                    let requires_reasoning_step = suggestion.reasoning_efforts.len() > 1;
                    render::SlashPaletteEntry {
                        replacement: format!("/model {}", suggestion.profile_id),
                        label: format!("/model {}", suggestion.selector),
                        meta: "Model".to_owned(),
                        detail,
                        immediate: false,
                        submit_on_select: !requires_reasoning_step,
                    }
                })
                .collect()
        }
        Err(_) => {
            let parts = args.split_whitespace().collect::<Vec<_>>();
            if let [selector, reasoning_query, ..] = parts.as_slice()
                && let Some(suggestion) = suggestions.iter().find(|suggestion| {
                    suggestion.selector.eq_ignore_ascii_case(selector)
                        || suggestion.profile_id.eq_ignore_ascii_case(selector)
                        || suggestion.model.eq_ignore_ascii_case(selector)
                })
            {
                return model_effort_palette_entries(suggestion, reasoning_query);
            }
            Vec::new()
        }
    }
}

fn mode_palette_entries(shell: &state::Shell, args: &str) -> Vec<render::SlashPaletteEntry> {
    let query = args.trim().to_ascii_lowercase();
    let busy_input_mode = shell.pane.busy_input_mode();
    let pending_submission_count = shell.pane.pending_submission_count();
    let can_clear = pending_submission_count > 0;
    let mode_entries = [
        (
            "queue",
            "Queue",
            if busy_input_mode == state::BusyInputMode::Queue {
                "Queue follow-up messages in FIFO order (current)"
            } else {
                "Queue follow-up messages in FIFO order"
            },
        ),
        (
            "steer",
            "Steer",
            if busy_input_mode == state::BusyInputMode::Steer {
                "Replace the pending steer message and interrupt at the next tool boundary (current)"
            } else {
                "Replace the pending steer message and interrupt at the next tool boundary"
            },
        ),
        ("toggle", "Mode", "Cycle between queue and steer modes"),
    ];

    let mut entries = mode_entries
        .into_iter()
        .filter(|(command, _meta, detail)| {
            matches_candidate_query(command, query.as_str())
                || matches_candidate_query(detail, query.as_str())
        })
        .map(|(command, meta, detail)| render::SlashPaletteEntry {
            replacement: format!("/mode {command}"),
            label: format!("/mode {command}"),
            meta: meta.to_owned(),
            detail: detail.to_owned(),
            immediate: false,
            submit_on_select: false,
        })
        .collect::<Vec<_>>();

    let clear_matches = matches_candidate_query("clear", query.as_str())
        || matches_candidate_query("clear pending submissions", query.as_str());
    if can_clear && clear_matches {
        let clear_entry = render::SlashPaletteEntry {
            replacement: "/mode clear".to_owned(),
            label: "/mode clear".to_owned(),
            meta: "Mode".to_owned(),
            detail: format!("Clear {pending_submission_count} pending submission(s)"),
            immediate: false,
            submit_on_select: false,
        };
        entries.push(clear_entry);
    }

    entries
}

fn slash_palette_entries(
    shell: &state::Shell,
    draft_prefix: &str,
) -> Vec<render::SlashPaletteEntry> {
    if !draft_prefix.starts_with('/') {
        return Vec::new();
    }

    if let Some(parsed) = commands::parse(draft_prefix) {
        if matches!(parsed.command, SlashCommand::Resume) {
            let entries = resume_palette_entries(shell, parsed.args.as_str());
            if !entries.is_empty() {
                return entries;
            }
        }
        if matches!(parsed.command, SlashCommand::Subagents) {
            let entries = subagents_palette_entries(shell, parsed.args.as_str());
            if !entries.is_empty() {
                return entries;
            }
        }
        if matches!(parsed.command, SlashCommand::Approvals) {
            let entries = approvals_palette_entries(shell, parsed.args.as_str());
            if !entries.is_empty() {
                return entries;
            }
        }
        if matches!(parsed.command, SlashCommand::Model) {
            let entries = model_palette_entries(shell, parsed.args.as_str());
            if !entries.is_empty() {
                return entries;
            }
        }
        if matches!(parsed.command, SlashCommand::Mode) {
            let entries = mode_palette_entries(shell, parsed.args.as_str());
            if !entries.is_empty() {
                return entries;
            }
        }
        if matches!(parsed.command, SlashCommand::Tasks) {
            let entries = tasks_palette_entries(shell, parsed.args.as_str());
            if !entries.is_empty() {
                return entries;
            }
        }
        if matches!(parsed.command, SlashCommand::Permissions) {
            let entries = permissions_palette_entries(shell, parsed.args.as_str());
            if !entries.is_empty() {
                return entries;
            }
        }
    }

    command_palette_entries(draft_prefix)
}

fn slash_command_matches(
    shell: &state::Shell,
    textarea: &tui_textarea::TextArea<'_>,
) -> Vec<render::SlashPaletteEntry> {
    let draft_text = textarea.lines().join("\n");
    slash_palette_entries(shell, draft_text.trim())
}

fn slash_command_palette_area(
    shell: &state::Shell,
    textarea: &tui_textarea::TextArea<'_>,
) -> Option<ratatui::layout::Rect> {
    let matches = slash_command_matches(shell, textarea);
    if matches.is_empty() {
        return None;
    }

    let shell_areas = terminal_shell_areas(textarea);
    let input_area = shell_areas.input;
    let popup_height = matches.len().min(5) as u16 + 2;
    let popup_width = input_area.width.clamp(28, 72);
    let popup_x = input_area.x;
    let popup_y = input_area.y.saturating_sub(popup_height.saturating_sub(1));

    Some(ratatui::layout::Rect::new(
        popup_x,
        popup_y,
        popup_width,
        popup_height,
    ))
}

fn apply_selected_slash_command(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
) -> Option<String> {
    let matches = slash_command_matches(shell, textarea);
    if matches.is_empty() {
        return None;
    }

    let selected_index = shell.slash_command_selection % matches.len();
    let selected_entry = matches.get(selected_index);
    let selected_entry = selected_entry?;

    textarea.select_all();
    textarea.delete_str(usize::MAX);
    if selected_entry.immediate {
        let parsed_command = commands::parse(selected_entry.replacement.as_str());
        let parsed_command = parsed_command?;
        handle_slash_command(shell, parsed_command);
        Some(String::new())
    } else if selected_entry.submit_on_select {
        Some(selected_entry.replacement.clone())
    } else {
        textarea.insert_str(selected_entry.replacement.as_str());
        Some(String::new())
    }
}

fn cycle_slash_command_selection(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    direction: i8,
) -> bool {
    let matches = slash_command_matches(shell, textarea);
    if matches.is_empty() {
        return false;
    }

    let selected_index = shell.slash_command_selection % matches.len();
    let next_index = if direction >= 0 {
        (selected_index + 1) % matches.len()
    } else if selected_index == 0 {
        matches.len().saturating_sub(1)
    } else {
        selected_index.saturating_sub(1)
    };

    shell.slash_command_selection = next_index;

    true
}

fn slash_command_index_at_mouse_row(
    shell: &state::Shell,
    textarea: &tui_textarea::TextArea<'_>,
    mouse_row: u16,
) -> Option<usize> {
    let palette_area = slash_command_palette_area(shell, textarea)?;
    let inner_top = palette_area.y.saturating_add(1);
    let inner_bottom = palette_area
        .y
        .saturating_add(palette_area.height.saturating_sub(1));
    if mouse_row < inner_top || mouse_row >= inner_bottom {
        return None;
    }

    Some(usize::from(mouse_row.saturating_sub(inner_top)))
}

fn transcript_plain_lines(shell: &state::Shell) -> Vec<String> {
    let render_width = terminal_render_width();

    history::transcript_plain_lines(shell.pane(), render_width, shell.show_thinking)
}

fn transcript_line_count(shell: &state::Shell) -> usize {
    let plain_lines = transcript_plain_lines(shell);

    plain_lines.len()
}

fn focus_layer_label(layer: FocusLayer) -> &'static str {
    match layer {
        FocusLayer::Composer => "compose",
        FocusLayer::Transcript => "review",
        FocusLayer::Help => "help",
        FocusLayer::SessionPicker => "session-picker",
        FocusLayer::StatsOverlay => "stats",
        FocusLayer::ToolInspector => "tool",
        FocusLayer::ClarifyDialog => "question",
    }
}

fn append_surface_message(shell: &mut state::Shell, title: &str, lines: &[String]) {
    let mut message_lines = Vec::with_capacity(lines.len().saturating_add(2));
    message_lines.push(title.to_owned());
    message_lines.push(String::new());
    message_lines.extend(lines.iter().cloned());
    shell.pane.add_surface_lines(message_lines.as_slice());
    shell.pane.scroll_offset = 0;
}

fn append_surface_event_message(shell: &mut state::Shell, title: &str, lines: &[String]) {
    let message =
        crate::chat::tui::message::Message::surface_event(title.to_owned(), lines.to_vec());
    shell.pane.messages.push(message);
    shell.pane.scroll_offset = 0;
}

fn model_status_lines(config: &crate::config::LoongClawConfig) -> Vec<String> {
    let active_provider_id = config.active_provider_id().unwrap_or("(none)");
    let model = config
        .provider
        .resolved_model()
        .unwrap_or_else(|| "(unknown)".to_owned());
    let reasoning_effort = config
        .provider
        .reasoning_effort
        .map(|effort| effort.as_str().to_owned())
        .unwrap_or_else(|| "auto".to_owned());
    let mut lines = vec![
        format!("- active provider: {active_provider_id}"),
        format!("- model: {model}"),
        format!("- provider kind: {}", config.provider.kind.as_str()),
        format!("- wire api: {}", config.provider.wire_api.as_str()),
        format!("- reasoning effort: {reasoning_effort}"),
    ];
    let allowed_efforts = provider_reasoning_effort_options(&config.provider)
        .into_iter()
        .map(|effort| effort.as_str().to_owned())
        .collect::<Vec<_>>();
    if allowed_efforts.is_empty() {
        lines.push("- reasoning levels: not supported for this provider".to_owned());
    } else {
        lines.push(format!(
            "- reasoning levels: auto, {}",
            allowed_efforts.join(", ")
        ));
    }
    lines
}

fn show_model_surface(shell: &mut state::Shell, args: &str) {
    match parse_model_action(args) {
        Ok(ModelAction::Status) => {
            let Some(config) = shell.runtime_config.as_ref() else {
                shell.pane.add_system_message(
                    "Model view is unavailable before the chat runtime is initialized.",
                );
                return;
            };
            let lines = model_status_lines(config);
            append_surface_message(shell, "model status", lines.as_slice());
            shell
                .pane
                .set_status("Model details added to transcript".to_owned());
        }
        Ok(ModelAction::Switch { selector, .. }) => {
            shell.pane.add_system_message(&format!(
                "Use `/model {selector}` from the composer submit path to switch models."
            ));
        }
        Err(error) => shell.pane.add_system_message(&error),
    }
}

fn show_rename_surface(shell: &mut state::Shell, args: &str) {
    match parse_rename_action(args) {
        Ok(RenameAction::Status) => {
            let outcome = match execute_shell_app_tool(shell, "session_status", json!({})) {
                Ok(outcome) => outcome,
                Err(error) => {
                    let message = format!("Unable to inspect the current session label: {error}");
                    shell.pane.add_system_message(&message);
                    return;
                }
            };
            let session = outcome
                .payload
                .get("session")
                .cloned()
                .unwrap_or(Value::Null);
            let session_id = session
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("(unknown)");
            let label = session
                .get("label")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("(unset)");
            let lines = vec![
                format!("- session: {session_id}"),
                format!("- label: {label}"),
                "- rename with `/rename <label>`".to_owned(),
            ];
            append_surface_message(shell, "session label", lines.as_slice());
            shell
                .pane
                .set_status("Session label added to transcript".to_owned());
        }
        Ok(RenameAction::Set(label)) => {
            let message = format!(
                "Use `/rename {label}` from the composer submit path to rename the current session."
            );
            shell.pane.add_system_message(&message);
        }
        Err(error) => shell.pane.add_system_message(&error),
    }
}

fn busy_input_mode_lines(shell: &state::Shell) -> Vec<String> {
    let busy_input_mode = shell.pane.busy_input_mode();
    let mode_label = busy_input_mode.label().to_ascii_lowercase();
    let pending_submission_count = shell.pane.pending_submission_count();
    let queue_depth = shell.pane.queued_messages.len();
    let steer_armed = shell.pane.steer_message.is_some();
    let queued_preview = shell
        .pane
        .queued_messages
        .iter()
        .take(2)
        .map(|message| trim_message_preview(message.text.as_str()))
        .collect::<Vec<_>>();
    let steer_preview = shell
        .pane
        .steer_message
        .as_ref()
        .map(|message| trim_message_preview(message.text.as_str()));

    let mut lines = vec![
        format!("- response mode: {mode_label}"),
        format!("- pending submissions: {pending_submission_count}"),
        format!("- queued messages: {queue_depth}"),
        format!("- steer armed: {}", if steer_armed { "yes" } else { "no" }),
        "- switch with `/mode queue` or `/mode steer`".to_owned(),
        "- cycle with `Ctrl+G` while composing".to_owned(),
    ];

    if !queued_preview.is_empty() {
        let queued_preview_label = queued_preview.join(" | ");
        lines.push(format!("- queue preview: {queued_preview_label}"));
    }

    if let Some(steer_preview) = steer_preview {
        lines.push(format!("- steer preview: {steer_preview}"));
    }

    lines
}

fn trim_message_preview(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = compact.trim().to_owned();
    let max_chars = 48_usize;
    let char_count = compact.chars().count();
    if char_count <= max_chars {
        return compact;
    }

    let truncated = compact
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    format!("{truncated}…")
}

fn show_busy_input_mode_surface(shell: &mut state::Shell, args: &str) {
    match parse_busy_input_mode_action(args) {
        Ok(BusyInputModeAction::Status) => {
            let lines = busy_input_mode_lines(shell);
            append_surface_message(shell, "busy input mode", lines.as_slice());
            shell
                .pane
                .set_status("Busy input mode added to transcript".to_owned());
        }
        Ok(BusyInputModeAction::Set(mode)) => {
            shell.pane.set_busy_input_mode(mode);
            let label = mode.label().to_ascii_lowercase();
            shell.pane.set_status(format!("Busy input mode: {label}"));
        }
        Ok(BusyInputModeAction::Toggle) => {
            shell.pane.cycle_busy_input_mode();
            let next_mode = shell.pane.busy_input_mode();
            let label = next_mode.label().to_ascii_lowercase();
            shell.pane.set_status(format!("Busy input mode: {label}"));
        }
        Ok(BusyInputModeAction::Clear) => {
            shell.pane.clear_pending_submissions();
            shell
                .pane
                .set_status("Pending submissions cleared".to_owned());
        }
        Err(error) => {
            shell.pane.add_system_message(&error);
        }
    }
}

fn open_stats_overlay(shell: &mut state::Shell, args: &str) {
    let options = match stats::parse_stats_open_options(args) {
        Ok(options) => options,
        Err(error) => {
            shell.pane.add_system_message(&error);
            return;
        }
    };
    let Some(config) = shell.runtime_config.as_ref() else {
        shell.pane.add_system_message(
            "Stats view is unavailable before the chat runtime is initialized.",
        );
        return;
    };
    let current_session_id = shell.pane.session_id.clone();
    let snapshot = match stats::load_stats_snapshot(config, current_session_id.as_str()) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to load stats: {error}"));
            return;
        }
    };

    shell.stats_overlay = Some(state::StatsOverlayState::new(
        snapshot,
        options.tab,
        options.date_range,
    ));
    if !shell.focus.has(FocusLayer::StatsOverlay) {
        shell.focus.push(FocusLayer::StatsOverlay);
    }
    shell.pane.set_status("Stats overlay opened".to_owned());
}

fn close_stats_overlay(shell: &mut state::Shell) {
    shell.stats_overlay = None;
    if shell.focus.top() == FocusLayer::StatsOverlay {
        shell.focus.pop();
    }
}

fn open_resume_picker(shell: &mut state::Shell) {
    let visibility_scope_session_id = session_visibility_scope_session_id(shell)
        .unwrap_or_else(|_| shell.pane.session_id.clone());
    let outcome = match execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "sessions_list",
        json!({
            "limit": session_surface_limit(shell),
            "include_delegate_lifecycle": true,
        }),
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            let message = format!("Unable to load resume candidates: {error}");
            shell.pane.add_system_message(&message);
            return;
        }
    };

    let approval_outcome = execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "approval_requests_list",
        json!({ "limit": 8 }),
    )
    .ok();
    let mut sessions = parse_visible_session_suggestions(&outcome);
    let approvals = approval_outcome
        .as_ref()
        .map(parse_approval_request_suggestions)
        .unwrap_or_default();
    annotate_session_approval_counts(sessions.as_mut_slice(), approvals.as_slice());

    open_session_picker(shell, state::SessionPickerMode::Resume, sessions);
}

fn open_subagent_picker(shell: &mut state::Shell) {
    let visibility_scope_session_id = session_visibility_scope_session_id(shell)
        .unwrap_or_else(|_| shell.pane.session_id.clone());
    let outcome = match execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "sessions_list",
        json!({
            "limit": session_surface_limit(shell),
            "include_delegate_lifecycle": true,
        }),
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            let message = format!("Unable to load subagent candidates: {error}");
            shell.pane.add_system_message(&message);
            return;
        }
    };

    let approval_outcome = execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "approval_requests_list",
        json!({ "limit": 8 }),
    )
    .ok();
    let mut sessions = parse_visible_session_suggestions(&outcome);
    let approvals = approval_outcome
        .as_ref()
        .map(parse_approval_request_suggestions)
        .unwrap_or_default();
    annotate_session_approval_counts(sessions.as_mut_slice(), approvals.as_slice());
    sessions.retain(|session| session.kind == "root" || session.kind == "delegate_child");
    sessions.sort_by_key(|session| {
        (
            usize::from(session.kind != "root"),
            session.session_id.clone(),
        )
    });

    open_session_picker(shell, state::SessionPickerMode::Subagents, sessions);
}

fn open_session_picker(
    shell: &mut state::Shell,
    mode: state::SessionPickerMode,
    sessions: Vec<state::VisibleSessionSuggestion>,
) {
    let picker = state::SessionPickerState::new(mode, sessions);
    shell.session_picker = Some(picker);
    if !shell.focus.has(FocusLayer::SessionPicker) {
        shell.focus.push(FocusLayer::SessionPicker);
    }
    shell.pane.set_status(mode.open_status_message().to_owned());
}

fn close_session_picker(shell: &mut state::Shell) {
    shell.session_picker = None;
    if shell.focus.top() == FocusLayer::SessionPicker {
        shell.focus.pop();
    }
}

fn stats_overlay_entry_count(stats_overlay: &state::StatsOverlayState) -> usize {
    let range_view = stats_overlay.snapshot.range_view(stats_overlay.date_range);
    match stats_overlay.active_tab {
        stats::StatsTab::Overview => 0,
        stats::StatsTab::Models => range_view.model_totals.len(),
        stats::StatsTab::Sessions => range_view.session_rows.len(),
    }
}

fn clamp_stats_overlay_list_offset(stats_overlay: &mut state::StatsOverlayState) {
    let entry_count = stats_overlay_entry_count(stats_overlay);
    let max_offset = entry_count.saturating_sub(1);
    if stats_overlay.list_scroll_offset > max_offset {
        stats_overlay.list_scroll_offset = max_offset;
    }
}

fn show_session_surface(shell: &mut state::Shell) {
    let total_lines = transcript_line_count(shell);
    let selected_lines = shell.pane.transcript_selection_line_count(total_lines);
    let tool_calls = shell.pane.tool_call_count();
    let focus_label = focus_layer_label(shell.focus.top());
    let busy_input_mode = shell.pane.busy_input_mode();
    let mode_label = busy_input_mode.label().to_ascii_lowercase();
    let pending_submission_count = shell.pane.pending_submission_count();
    let lines = vec![
        format!("- session: {}", shell.pane.session_id),
        format!("- focus: {focus_label}"),
        format!("- response mode: {mode_label}"),
        format!("- pending submissions: {pending_submission_count}"),
        format!("- messages: {}", shell.pane.messages.len()),
        format!("- transcript lines: {total_lines}"),
        format!("- selected lines: {selected_lines}"),
        format!("- tool calls: {tool_calls}"),
    ];

    append_surface_message(shell, "session status", lines.as_slice());
    shell
        .pane
        .set_status("Session details added to transcript".to_owned());
}

fn show_runtime_status_surface(shell: &mut state::Shell) {
    let model = if shell.pane.model.is_empty() {
        "(unknown)".to_owned()
    } else {
        shell.pane.model.clone()
    };
    let total_tokens = shell.pane.total_tokens();
    let context_percent = shell.pane.context_percent() * 100.0;
    let tool_calls = shell.pane.tool_call_count();
    let focus_label = focus_layer_label(shell.focus.top());
    let thinking_label = if shell.show_thinking {
        "visible"
    } else {
        "hidden"
    };
    let busy_input_mode = shell.pane.busy_input_mode();
    let mode_label = busy_input_mode.label().to_ascii_lowercase();
    let pending_submission_count = shell.pane.pending_submission_count();
    let lines = vec![
        format!("- session: {}", shell.pane.session_id),
        format!("- model: {model}"),
        format!("- tokens: {total_tokens}"),
        format!("- context usage: {context_percent:.1}%"),
        format!("- tool calls: {tool_calls}"),
        format!("- focus: {focus_label}"),
        format!("- busy input mode: {mode_label}"),
        format!("- pending submissions: {pending_submission_count}"),
        format!("- thinking blocks: {thinking_label}"),
    ];

    append_surface_message(shell, "runtime status", lines.as_slice());
    shell
        .pane
        .set_status("Runtime status added to transcript".to_owned());
}

fn show_context_surface(shell: &mut state::Shell) {
    let model = if shell.pane.model.is_empty() {
        "(unknown)".to_owned()
    } else {
        shell.pane.model.clone()
    };
    let input_tokens = shell.pane.input_tokens;
    let output_tokens = shell.pane.output_tokens;
    let total_tokens = shell.pane.total_tokens();
    let context_length = shell.pane.context_length;
    let context_label = if context_length == 0 {
        "unknown".to_owned()
    } else {
        context_length.to_string()
    };
    let context_percent = shell.pane.context_percent() * 100.0;
    let lines = vec![
        format!("- model: {model}"),
        format!("- input tokens: {input_tokens}"),
        format!("- output tokens: {output_tokens}"),
        format!("- total tokens: {total_tokens}"),
        format!("- context window: {context_label}"),
        format!("- context usage: {context_percent:.1}%"),
    ];

    append_surface_message(shell, "context status", lines.as_slice());
    shell
        .pane
        .set_status("Context details added to transcript".to_owned());
}

fn tool_runtime_config_from_shell(
    shell: &state::Shell,
) -> Option<crate::tools::runtime_config::ToolRuntimeConfig> {
    let config = shell.runtime_config.as_ref()?;
    let config_path = shell.runtime_config_path.as_deref();

    Some(
        crate::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, config_path),
    )
}

fn execute_shell_app_tool(
    shell: &state::Shell,
    tool_name: &str,
    payload: Value,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    execute_shell_app_tool_for_session(shell, shell.pane.session_id.as_str(), tool_name, payload)
}

fn execute_shell_app_tool_for_session(
    shell: &state::Shell,
    session_id: &str,
    tool_name: &str,
    payload: Value,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    let config = shell.runtime_config.as_ref().ok_or_else(|| {
        "App tool surface is unavailable before the chat runtime is initialized.".to_owned()
    })?;
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::tools::execute_app_tool_with_config(
        ToolCoreRequest {
            tool_name: tool_name.to_owned(),
            payload,
        },
        session_id,
        &memory_config,
        &config.tools,
    )
}

fn session_repository_for_shell(
    shell: &state::Shell,
) -> Result<crate::session::repository::SessionRepository, String> {
    let config = shell.runtime_config.as_ref().ok_or_else(|| {
        "Session repository is unavailable before the chat runtime is initialized.".to_owned()
    })?;
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::session::repository::SessionRepository::new(&memory_config)
}

fn session_visibility_scope_session_id(shell: &state::Shell) -> Result<String, String> {
    let repo = session_repository_for_shell(shell)?;
    let current_session_id = shell.pane.session_id.as_str();
    let scope_session_id = repo
        .lineage_root_session_id(current_session_id)?
        .unwrap_or_else(|| current_session_id.to_owned());
    Ok(scope_session_id)
}

fn execute_shell_core_tool(
    shell: &state::Shell,
    tool_name: &str,
    payload: Value,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    let runtime_config = tool_runtime_config_from_shell(shell).ok_or_else(|| {
        "Core tool surface is unavailable before the chat runtime is initialized.".to_owned()
    })?;

    crate::tools::execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: tool_name.to_owned(),
            payload,
        },
        &runtime_config,
    )
}

fn provider_supports_reasoning_effort(provider: &crate::config::ProviderConfig) -> bool {
    matches!(
        provider.kind.protocol_family(),
        crate::config::ProviderProtocolFamily::OpenAiChatCompletions
    )
}

fn provider_reasoning_effort_options(
    provider: &crate::config::ProviderConfig,
) -> Vec<crate::config::ReasoningEffort> {
    if !provider_supports_reasoning_effort(provider) {
        return Vec::new();
    }

    provider
        .kind
        .allowed_reasoning_efforts()
        .map(|allowed| allowed.to_vec())
        .unwrap_or_else(|| {
            vec![
                crate::config::ReasoningEffort::None,
                crate::config::ReasoningEffort::Minimal,
                crate::config::ReasoningEffort::Low,
                crate::config::ReasoningEffort::Medium,
                crate::config::ReasoningEffort::High,
                crate::config::ReasoningEffort::Xhigh,
            ]
        })
}

fn validated_reasoning_effort_for_provider(
    provider: &crate::config::ProviderConfig,
    provider_label: &str,
    reasoning: ModelReasoningChoice,
) -> Result<Option<crate::config::ReasoningEffort>, String> {
    match reasoning {
        ModelReasoningChoice::Auto => Ok(None),
        ModelReasoningChoice::Explicit(effort) => {
            let allowed_efforts = provider_reasoning_effort_options(provider);
            if allowed_efforts.is_empty() {
                return Err(format!(
                    "provider `{provider_label}` does not support reasoning effort overrides"
                ));
            }
            if !allowed_efforts.contains(&effort) {
                let supported = allowed_efforts
                    .iter()
                    .map(|effort| effort.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "provider `{provider_label}` does not support reasoning effort `{}`; choose `auto`, or one of: {supported}",
                    effort.as_str()
                ));
            }
            Ok(Some(effort))
        }
    }
}

fn persist_model_selection_to_config(
    config_path: &Path,
    selector: &str,
    reasoning: Option<ModelReasoningChoice>,
) -> Result<(crate::config::LoongClawConfig, Option<String>, String), String> {
    let path_string = config_path.to_string_lossy().to_string();
    let (_, mut loaded) = crate::config::load(Some(path_string.as_str()))?;
    let previous_active_provider = loaded.active_provider_id().map(str::to_owned);
    let selected_profile_id = loaded.switch_active_provider(selector)?;

    if let Some(choice) = reasoning {
        let updated_reasoning_effort = {
            let selected_provider = loaded
                .providers
                .get(&selected_profile_id)
                .map(|profile| &profile.provider)
                .unwrap_or(&loaded.provider);
            validated_reasoning_effort_for_provider(
                selected_provider,
                selected_profile_id.as_str(),
                choice,
            )?
        };

        loaded.provider.reasoning_effort = updated_reasoning_effort;
        if let Some(profile) = loaded.providers.get_mut(&selected_profile_id) {
            profile.provider.reasoning_effort = updated_reasoning_effort;
        }
    }
    crate::config::write(Some(path_string.as_str()), &loaded, true)?;
    Ok((loaded, previous_active_provider, selected_profile_id))
}

fn worktree_root_for_suggestions(shell: &state::Shell) -> Option<PathBuf> {
    if let Some(config) = shell.runtime_config.as_ref()
        && config
            .tools
            .file_root
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return Some(config.tools.resolved_file_root());
    }

    std::env::current_dir().ok()
}

fn matched_count_from_outcome(outcome: &loongclaw_contracts::ToolCoreOutcome) -> Option<usize> {
    outcome
        .payload
        .get("matched_count")
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
}

fn parse_visible_session_suggestions(
    outcome: &loongclaw_contracts::ToolCoreOutcome,
) -> Vec<state::VisibleSessionSuggestion> {
    outcome
        .payload
        .get("sessions")
        .and_then(Value::as_array)
        .map(|sessions| {
            sessions
                .iter()
                .filter_map(|session| {
                    Some(state::VisibleSessionSuggestion {
                        session_id: session.get("session_id")?.as_str()?.to_owned(),
                        label: session
                            .get("label")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        agent_presentation: parse_agent_presentation(
                            session.get("agent_presentation"),
                        ),
                        state: session
                            .get("state")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        kind: session
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        task_phase: session
                            .get("delegate_lifecycle")
                            .and_then(|value| value.get("phase"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        overdue: session
                            .get("delegate_lifecycle")
                            .and_then(|value| value.get("staleness"))
                            .and_then(|value| value.get("state"))
                            .and_then(Value::as_str)
                            == Some("overdue"),
                        pending_approval_count: 0,
                        attention_approval_count: 0,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_agent_presentation(value: Option<&Value>) -> Option<DelegateAgentPresentation> {
    let value = value?.clone();
    serde_json::from_value(value).ok()
}

fn annotate_session_approval_counts(
    sessions: &mut [state::VisibleSessionSuggestion],
    approvals: &[state::ApprovalRequestSuggestion],
) {
    for session in sessions.iter_mut() {
        session.pending_approval_count = 0;
        session.attention_approval_count = 0;
    }

    for approval in approvals {
        let maybe_session = sessions
            .iter_mut()
            .find(|session| session.session_id == approval.session_id);
        let Some(session) = maybe_session else {
            continue;
        };

        session.pending_approval_count = session.pending_approval_count.saturating_add(1);
        if approval.needs_attention {
            session.attention_approval_count = session.attention_approval_count.saturating_add(1);
        }
    }
}

fn parse_approval_request_suggestions(
    outcome: &loongclaw_contracts::ToolCoreOutcome,
) -> Vec<state::ApprovalRequestSuggestion> {
    outcome
        .payload
        .get("requests")
        .and_then(Value::as_array)
        .map(|requests| {
            requests
                .iter()
                .filter_map(|request| {
                    Some(state::ApprovalRequestSuggestion {
                        approval_request_id: request
                            .get("approval_request_id")?
                            .as_str()?
                            .to_owned(),
                        tool_name: request
                            .get("tool_name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        status: request
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        session_id: request
                            .get("session_id")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        needs_attention: request
                            .get("attention")
                            .and_then(|value| value.get("needs_attention"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_model_selection_suggestions(
    config: &crate::config::LoongClawConfig,
) -> Vec<state::ModelSelectionSuggestion> {
    if config.providers.is_empty() {
        let reasoning_efforts = provider_reasoning_effort_options(&config.provider)
            .into_iter()
            .map(|effort| effort.as_str().to_owned())
            .collect::<Vec<_>>();
        return vec![state::ModelSelectionSuggestion {
            selector: config
                .provider
                .resolved_model()
                .unwrap_or_else(|| "auto".to_owned()),
            profile_id: config.active_provider_id().unwrap_or("default").to_owned(),
            kind: config.provider.kind.as_str().to_owned(),
            model: config.provider.model.clone(),
            active: true,
            reasoning_efforts,
            current_reasoning_effort: config
                .provider
                .reasoning_effort
                .map(|effort| effort.as_str().to_owned()),
        }];
    }

    let active_provider_id = config.active_provider_id().map(str::to_owned);
    config
        .providers
        .iter()
        .map(|(profile_id, profile)| {
            let selector = config
                .preferred_provider_selector(profile_id)
                .unwrap_or_else(|| profile_id.clone());
            let reasoning_efforts = provider_reasoning_effort_options(&profile.provider)
                .into_iter()
                .map(|effort| effort.as_str().to_owned())
                .collect::<Vec<_>>();
            state::ModelSelectionSuggestion {
                selector,
                profile_id: profile_id.clone(),
                kind: profile.provider.kind.as_str().to_owned(),
                model: profile.provider.model.clone(),
                active: active_provider_id.as_deref() == Some(profile_id.as_str()),
                reasoning_efforts,
                current_reasoning_effort: profile
                    .provider
                    .reasoning_effort
                    .map(|effort| effort.as_str().to_owned()),
            }
        })
        .collect()
}

fn refresh_composer_suggestion_context(shell: &mut state::Shell) {
    let visibility_scope_session_id = session_visibility_scope_session_id(shell)
        .unwrap_or_else(|_| shell.pane.session_id.clone());
    let model_selection_suggestions = shell
        .runtime_config
        .as_ref()
        .map(parse_model_selection_suggestions)
        .unwrap_or_default();
    let worktree_dirty = worktree_root_for_suggestions(shell).and_then(|root| {
        git_output(&["status", "--short"], root.as_path())
            .ok()
            .map(|status| !status.trim().is_empty())
    });

    let visible_session_outcome = execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "sessions_list",
        json!({
            "limit": session_surface_limit(shell),
            "include_delegate_lifecycle": true,
        }),
    )
    .ok();
    let visible_sessions = visible_session_outcome
        .as_ref()
        .and_then(matched_count_from_outcome);
    let mut visible_session_suggestions = visible_session_outcome
        .as_ref()
        .map(parse_visible_session_suggestions)
        .unwrap_or_default();

    let running_tasks = execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "sessions_list",
        json!({
            "limit": 1,
            "kind": "delegate_child",
            "state": "running",
            "include_delegate_lifecycle": true,
        }),
    )
    .ok()
    .and_then(|outcome| matched_count_from_outcome(&outcome));

    let overdue_tasks = execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "sessions_list",
        json!({
            "limit": 1,
            "kind": "delegate_child",
            "overdue_only": true,
            "include_delegate_lifecycle": true,
        }),
    )
    .ok()
    .and_then(|outcome| matched_count_from_outcome(&outcome));

    let approval_outcome = execute_shell_app_tool_for_session(
        shell,
        visibility_scope_session_id.as_str(),
        "approval_requests_list",
        json!({ "limit": 8 }),
    )
    .ok();
    let (pending_approvals, attention_approvals) = match approval_outcome.as_ref() {
        Some(outcome) => (
            matched_count_from_outcome(outcome),
            outcome
                .payload
                .get("attention_summary")
                .and_then(|value| value.get("needs_attention_count"))
                .and_then(Value::as_u64)
                .and_then(|count| usize::try_from(count).ok()),
        ),
        None => (None, None),
    };
    let approval_request_suggestions = approval_outcome
        .as_ref()
        .map(parse_approval_request_suggestions)
        .unwrap_or_default();
    annotate_session_approval_counts(
        visible_session_suggestions.as_mut_slice(),
        approval_request_suggestions.as_slice(),
    );

    let has_explicit_permission_policy =
        execute_shell_app_tool(shell, "session_tool_policy_status", json!({}))
            .ok()
            .and_then(|outcome| {
                outcome
                    .payload
                    .get("policy")
                    .and_then(|value| value.get("has_policy"))
                    .and_then(Value::as_bool)
            });

    shell.pane.composer_suggestion_context = state::ComposerSuggestionContext {
        worktree_dirty,
        visible_sessions,
        visible_session_suggestions,
        model_selection_suggestions,
        running_tasks,
        overdue_tasks,
        pending_approvals,
        attention_approvals,
        approval_request_suggestions,
        has_explicit_permission_policy,
    };
    shell.pane.session_display_label = current_session_display_label(shell);
}

fn current_session_display_label(shell: &state::Shell) -> Option<String> {
    let locale = session_presentation_locale();
    let current_session_id = shell.pane.session_id.as_str();
    let visible_sessions = &shell
        .pane
        .composer_suggestion_context
        .visible_session_suggestions;
    let current_session = visible_sessions
        .iter()
        .find(|session| session.session_id == current_session_id)?;

    Some(subagent_session_primary_label(current_session, locale))
}

fn capture_subagent_surface_snapshot(shell: &state::Shell) -> state::SubagentSurfaceSnapshot {
    let visible_delegate_sessions = shell
        .pane
        .composer_suggestion_context
        .visible_session_suggestions
        .iter()
        .filter(|session| session.kind == "delegate_child")
        .cloned()
        .collect::<Vec<_>>();
    let active_delegate_session_ids = visible_delegate_sessions
        .iter()
        .filter(|session| matches!(session.state.as_str(), "ready" | "running"))
        .map(|session| session.session_id.clone())
        .collect::<Vec<_>>();

    state::SubagentSurfaceSnapshot {
        active_delegate_session_ids,
        visible_delegate_sessions,
    }
}

fn reset_subagent_surface_tracking(shell: &mut state::Shell) {
    shell.subagent_surface_snapshot = capture_subagent_surface_snapshot(shell);
    shell.last_context_poll_at = Instant::now();
}

fn should_poll_background_context(shell: &state::Shell) -> bool {
    if shell.runtime_config.is_none() {
        return false;
    }

    if shell.session_picker.is_some() {
        return true;
    }

    let has_visible_delegate_sessions = shell
        .pane
        .composer_suggestion_context
        .visible_session_suggestions
        .iter()
        .any(|session| session.kind == "delegate_child");
    if has_visible_delegate_sessions {
        return true;
    }

    !shell
        .subagent_surface_snapshot
        .active_delegate_session_ids
        .is_empty()
}

fn maybe_poll_background_context(shell: &mut state::Shell) {
    if !should_poll_background_context(shell) {
        return;
    }

    let elapsed_millis = shell.last_context_poll_at.elapsed().as_millis();
    if elapsed_millis < 900 {
        return;
    }

    refresh_composer_suggestion_context(shell);
    maybe_emit_subagent_lifecycle_surface(shell);
    shell.last_context_poll_at = Instant::now();
    shell.dirty = true;
}

fn maybe_emit_subagent_lifecycle_surface(shell: &mut state::Shell) {
    let previous_snapshot = shell.subagent_surface_snapshot.clone();
    let current_snapshot = capture_subagent_surface_snapshot(shell);

    let previous_active_ids = previous_snapshot.active_delegate_session_ids.as_slice();
    let current_active_ids = current_snapshot.active_delegate_session_ids.as_slice();
    if previous_active_ids == current_active_ids {
        shell.subagent_surface_snapshot = current_snapshot;
        return;
    }

    let previous_had_active = !previous_active_ids.is_empty();
    let current_has_active = !current_active_ids.is_empty();

    if !previous_had_active && current_has_active {
        let waiting_lines =
            build_waiting_subagent_lines(current_snapshot.visible_delegate_sessions.as_slice());
        if !waiting_lines.is_empty() {
            let waiting_title = if current_active_ids.len() == 1 {
                "Waiting for 1 subagent".to_owned()
            } else {
                format!("Waiting for {} subagents", current_active_ids.len())
            };
            append_surface_event_message(shell, waiting_title.as_str(), waiting_lines.as_slice());
        }
    } else if previous_had_active && !current_has_active {
        let finished_lines = build_finished_subagent_lines(
            shell,
            previous_snapshot.active_delegate_session_ids.as_slice(),
        );
        if !finished_lines.is_empty() {
            let finished_title = if previous_active_ids.len() == 1 {
                "Finished waiting on 1 subagent".to_owned()
            } else {
                format!(
                    "Finished waiting on {} subagents",
                    previous_active_ids.len()
                )
            };
            append_surface_event_message(shell, finished_title.as_str(), finished_lines.as_slice());
        }
    }

    shell.subagent_surface_snapshot = current_snapshot;
}

fn build_waiting_subagent_lines(sessions: &[state::VisibleSessionSuggestion]) -> Vec<String> {
    let locale = session_presentation_locale();
    sessions
        .iter()
        .filter(|session| matches!(session.state.as_str(), "ready" | "running"))
        .map(|session| {
            let mut parts = vec![subagent_session_primary_label(session, locale)];
            if let Some(provider_label) = visible_session_provider_label(session, locale) {
                parts.push(provider_label);
            }
            format!("  └ {}", parts.join(" · "))
        })
        .collect()
}

fn build_finished_subagent_lines(shell: &state::Shell, session_ids: &[String]) -> Vec<String> {
    let locale = session_presentation_locale();
    let visible_sessions = &shell
        .pane
        .composer_suggestion_context
        .visible_session_suggestions;

    session_ids
        .iter()
        .filter_map(|session_id| {
            let maybe_session = visible_sessions
                .iter()
                .find(|session| session.session_id == *session_id);
            let session = maybe_session?;
            let label = subagent_session_primary_label(session, locale);
            let status = terminal_subagent_status_label(session.state.as_str());
            let summary = load_delegate_terminal_summary(shell, session_id.as_str());
            let mut parts = vec![label];
            if let Some(provider_label) = visible_session_provider_label(session, locale) {
                parts.push(provider_label);
            }
            parts.push(status.to_owned());
            if let Some(summary) = summary {
                parts.push(summary);
            }
            let line = format!("  └ {}", parts.join(" · "));
            Some(line)
        })
        .collect()
}

fn terminal_subagent_status_label(state: &str) -> &'static str {
    match state {
        "completed" => "Completed",
        "failed" => "Failed",
        "timed_out" => "Timed out",
        "running" => "Running",
        "ready" => "Queued",
        _ => "Updated",
    }
}

fn load_delegate_terminal_summary(shell: &state::Shell, session_id: &str) -> Option<String> {
    let config = shell.runtime_config.as_ref()?;
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = session_repository_for_shell(shell).ok()?;
    let session_summary = repo
        .load_session_summary_with_legacy_fallback(session_id)
        .ok()
        .flatten()?;
    let turn_count = session_summary.turn_count;
    if turn_count == 0 {
        return session_summary
            .last_error
            .as_deref()
            .map(trim_message_preview);
    }

    let window_limit = turn_count.min(12);
    let turns = crate::memory::window_direct(session_id, window_limit, &memory_config).ok()?;
    for turn in turns.iter().rev() {
        if turn.role != "assistant" {
            continue;
        }
        if parse_conversation_event(turn.content.as_str()).is_some() {
            continue;
        }
        let trimmed = turn.content.trim();
        if trimmed.is_empty() {
            continue;
        }
        return Some(trim_message_preview(trimmed));
    }

    session_summary
        .last_error
        .as_deref()
        .map(trim_message_preview)
}

fn session_surface_limit(shell: &state::Shell) -> usize {
    shell
        .runtime_config
        .as_ref()
        .map(|config| config.tools.sessions.list_limit.clamp(1, 8))
        .unwrap_or(8)
}

fn format_unix_timestamp(timestamp: i64) -> String {
    OffsetDateTime::from_unix_timestamp(timestamp)
        .ok()
        .and_then(|value| value.format(&Rfc3339).ok())
        .unwrap_or_else(|| timestamp.to_string())
}

fn summarize_string_items(items: &[String], limit: usize) -> String {
    if items.is_empty() {
        return "(none)".to_owned();
    }

    let preview: Vec<String> = items.iter().take(limit).cloned().collect();
    let remaining = items.len().saturating_sub(preview.len());
    let mut summary = preview.join(", ");
    if remaining > 0 {
        summary.push_str(&format!(", +{remaining} more"));
    }
    summary
}

fn json_string_items(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn json_value_present(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Null) | None => false,
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

fn json_preview(value: Option<&Value>, max_chars: usize) -> String {
    let Some(value) = value else {
        return "(none)".to_owned();
    };
    if matches!(value, Value::Null) {
        return "(none)".to_owned();
    }

    let raw = serde_json::to_string(value).unwrap_or_else(|_| "<unrenderable>".to_owned());
    let preview: String = raw.chars().take(max_chars).collect();
    if raw.chars().count() > max_chars {
        format!("{preview}...")
    } else {
        preview
    }
}

fn push_session_summary_lines(lines: &mut Vec<String>, session: &Value, current_session_id: &str) {
    let locale = session_presentation_locale();
    let session_id = session
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("(unknown)");
    let state = session
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let kind = session
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut summary = format!(
        "- {session_id}{} · {state} · {kind}",
        if session_id == current_session_id {
            " (current)"
        } else {
            ""
        }
    );

    if let Some(turn_count) = session.get("turn_count").and_then(Value::as_u64) {
        summary.push_str(&format!(" · turns {turn_count}"));
    }

    if let Some(delegate_lifecycle) = session.get("delegate_lifecycle") {
        if let Some(phase) = delegate_lifecycle.get("phase").and_then(Value::as_str) {
            summary.push_str(&format!(" · task {phase}"));
        }
        if delegate_lifecycle
            .get("staleness")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            == Some("overdue")
        {
            summary.push_str(" · overdue");
        }
    }

    lines.push(summary);

    if let Some(agent_line) = session_agent_summary_line(session, locale) {
        lines.push(format!("  agent: {agent_line}"));
    }

    if let Some(label) = session
        .get("label")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("  label: {label}"));
    }

    if let Some(updated_at) = session.get("updated_at").and_then(Value::as_i64) {
        lines.push(format!("  updated: {}", format_unix_timestamp(updated_at)));
    }

    if let Some(last_error) = session
        .get("last_error")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("  error: {last_error}"));
    }
}

fn session_inspection_lines(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let locale = session_presentation_locale();
    let session = payload.get("session").cloned().unwrap_or(Value::Null);
    let session_id = session
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("(unknown)");
    let state = session
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let kind = session
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    lines.push(format!("- session: {session_id}"));
    lines.push(format!("- state: {state}"));
    lines.push(format!("- kind: {kind}"));

    if let Some(parent_session_id) = session
        .get("parent_session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("- parent: {parent_session_id}"));
    }

    if let Some(label) = session
        .get("label")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("- label: {label}"));
    }

    if let Some(agent_line) = payload
        .get("agent_presentation")
        .and_then(|value| agent_presentation_summary_line(value, locale))
    {
        lines.push(format!("- agent: {agent_line}"));
    }

    if let Some(updated_at) = session.get("updated_at").and_then(Value::as_i64) {
        lines.push(format!("- updated: {}", format_unix_timestamp(updated_at)));
    }

    if let Some(last_error) = session
        .get("last_error")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("- last error: {last_error}"));
    }

    if let Some(terminal_outcome_state) = payload
        .get("terminal_outcome_state")
        .and_then(Value::as_str)
    {
        lines.push(format!("- terminal outcome: {terminal_outcome_state}"));
    }

    if let Some(missing_reason) = payload
        .get("terminal_outcome_missing_reason")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("- recovery hint: {missing_reason}"));
    }

    if let Some(delegate_lifecycle) = payload.get("delegate_lifecycle") {
        if let Some(phase) = delegate_lifecycle.get("phase").and_then(Value::as_str) {
            let mode = delegate_lifecycle
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            lines.push(format!("- delegate lifecycle: {phase} ({mode})"));
        }
        if let Some(timeout_seconds) = delegate_lifecycle
            .get("timeout_seconds")
            .and_then(Value::as_u64)
        {
            lines.push(format!("- timeout seconds: {timeout_seconds}"));
        }
        if let Some(staleness_state) = delegate_lifecycle
            .get("staleness")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
        {
            lines.push(format!("- staleness: {staleness_state}"));
        }
    }

    let recent_event_kinds = payload
        .get("recent_events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .take(3)
                .filter_map(|event| event.get("event_kind").and_then(Value::as_str))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !recent_event_kinds.is_empty() {
        lines.push(format!(
            "- recent events: {}",
            recent_event_kinds.join(", ")
        ));
    }

    lines
}

fn session_agent_summary_line(
    session: &Value,
    locale: SessionPresentationLocale,
) -> Option<String> {
    let presentation = session.get("agent_presentation")?;
    agent_presentation_summary_line(presentation, locale)
}

fn agent_presentation_summary_line(
    value: &Value,
    locale: SessionPresentationLocale,
) -> Option<String> {
    let presentation = parse_agent_presentation(Some(value))?;
    let mut parts = vec![presentation.primary_label(locale)];
    if let Some(provider_label) = presentation.provider_label(locale) {
        parts.push(provider_label);
    }
    Some(parts.join(" · "))
}

fn session_history_messages(payload: &Value) -> Vec<Message> {
    payload
        .get("turns")
        .and_then(Value::as_array)
        .map(|turns| {
            turns
                .iter()
                .filter_map(|turn| {
                    let role = turn.get("role").and_then(Value::as_str)?;
                    let content = turn
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let message = match role {
                        "user" => Message::user(content),
                        "assistant" => Message {
                            role: super::message::Role::Assistant,
                            parts: vec![super::message::MessagePart::Text(content.to_owned())],
                        },
                        _ => Message::system(content),
                    };
                    Some(message)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn activate_resumed_session(
    shell: &mut state::Shell,
    runtime: &super::runtime::TuiRuntime,
    transcript: Vec<Message>,
    system_message: &str,
) {
    shell.runtime_config = Some(runtime.config.clone());
    shell.runtime_config_path = Some(runtime.resolved_path.clone());
    shell.session_picker = None;
    shell.pane.session_id = runtime.session_id.clone();
    shell.pane.model = runtime.model_label.clone();
    shell.pane.context_length = state::context_length_for_model(&runtime.model_label);
    shell.pane.input_tokens = 0;
    shell.pane.output_tokens = 0;
    shell.pane.streaming_active = false;
    shell.pane.agent_running = false;
    shell.pane.loop_state.clear();
    shell.pane.loop_action.clear();
    shell.pane.loop_iteration = 0;
    shell.pane.status_message = None;
    shell.pane.clear_pending_submissions();
    shell.pane.tool_inspector = None;
    shell.pane.transcript_review.cursor_line = 0;
    shell.pane.transcript_review.anchor_line = None;
    shell.pane.clear_input_hint_override();
    shell.focus.focus_composer();
    shell.pane.messages = transcript;
    shell.pane.add_system_message(system_message);
    shell.pane.scroll_offset = 0;
    refresh_composer_suggestion_context(shell);
    reset_subagent_surface_tracking(shell);
}

fn approval_request_lines(request: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let approval_request_id = request
        .get("approval_request_id")
        .and_then(Value::as_str)
        .unwrap_or("(unknown)");
    let tool_name = request
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let session_id = request
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let status = request
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    lines.push(format!(
        "- {approval_request_id} · {status} · {tool_name} · {session_id}"
    ));

    if let Some(reason) = request
        .get("reason")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            request
                .get("governance_snapshot")
                .and_then(|value| value.get("reason"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
    {
        lines.push(format!("  reason: {reason}"));
    }

    if let Some(rule_id) = request
        .get("rule_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            request
                .get("governance_snapshot")
                .and_then(|value| value.get("rule_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
    {
        lines.push(format!("  rule: {rule_id}"));
    }

    if let Some(requested_at) = request.get("requested_at").and_then(Value::as_i64) {
        lines.push(format!(
            "  requested: {}",
            format_unix_timestamp(requested_at)
        ));
    }

    if let Some(last_error) = request
        .get("last_error")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("  error: {last_error}"));
    }

    if let Some(attention) = request.get("attention") {
        let needs_attention = attention
            .get("needs_attention")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if needs_attention {
            let severity = attention
                .get("highest_escalation_level")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let action = attention
                .get("primary_action")
                .and_then(Value::as_str)
                .unwrap_or("review");
            lines.push(format!("  attention: {severity} · {action}"));
        }
    }

    lines
}

fn show_resume_surface(shell: &mut state::Shell, args: &str) {
    match parse_resume_action(args) {
        ResumeAction::List => {
            open_resume_picker(shell);
        }
        ResumeAction::Inspect(target_session_id) => {
            let visibility_scope_session_id = session_visibility_scope_session_id(shell)
                .unwrap_or_else(|_| shell.pane.session_id.clone());
            let outcome = match execute_shell_app_tool_for_session(
                shell,
                visibility_scope_session_id.as_str(),
                "session_status",
                json!({ "session_id": target_session_id }),
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    shell.pane.add_system_message(&format!(
                        "Unable to inspect resume target `{target_session_id}`: {error}"
                    ));
                    return;
                }
            };

            let lines = session_inspection_lines(&outcome.payload);
            append_surface_message(shell, "resume target", lines.as_slice());
            shell.pane.set_status(format!(
                "Resume target `{target_session_id}` added to transcript"
            ));
        }
        ResumeAction::Switch(target_session_id) => {
            shell.pane.add_system_message(&format!(
                "Use `/resume {target_session_id}` from the composer submit path to switch sessions."
            ));
        }
    }
}

fn show_subagents_surface(shell: &mut state::Shell, args: &str) {
    match parse_subagent_action(args) {
        SubagentAction::List => {
            open_subagent_picker(shell);
        }
        SubagentAction::Switch(target_session_id) => {
            shell.pane.add_system_message(&format!(
                "Use `/subagents {target_session_id}` from the composer submit path to switch threads."
            ));
        }
    }
}

fn show_tasks_surface(shell: &mut state::Shell, args: &str) {
    let raw = args.trim();
    let filter = raw.to_ascii_lowercase();
    let known_filters = [
        "",
        "all",
        "overdue",
        "queued",
        "running",
        "failed",
        "completed",
        "timed_out",
    ];

    if !known_filters.contains(&filter.as_str()) {
        let outcome =
            match execute_shell_app_tool(shell, "session_status", json!({ "session_id": raw })) {
                Ok(outcome) => outcome,
                Err(error) => {
                    shell.pane.add_system_message(&format!(
                        "Unable to inspect task session `{raw}`: {error}"
                    ));
                    return;
                }
            };
        let lines = session_inspection_lines(&outcome.payload);
        append_surface_message(shell, "task session", lines.as_slice());
        shell
            .pane
            .set_status(format!("Task session `{raw}` added to transcript"));
        return;
    }

    let mut payload = json!({
        "limit": session_surface_limit(shell),
        "kind": "delegate_child",
        "include_delegate_lifecycle": true,
    });
    if let Some(object) = payload.as_object_mut() {
        match filter.as_str() {
            "overdue" => {
                object.insert("overdue_only".to_owned(), Value::Bool(true));
            }
            "queued" => {
                object.insert("state".to_owned(), Value::String("ready".to_owned()));
            }
            "running" | "failed" | "completed" | "timed_out" => {
                object.insert("state".to_owned(), Value::String(filter.clone()));
            }
            "" | "all" => {}
            _ => {}
        }
    }

    let outcome = match execute_shell_app_tool(shell, "sessions_list", payload) {
        Ok(outcome) => outcome,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to list task sessions: {error}"));
            return;
        }
    };

    let matched_count = outcome
        .payload
        .get("matched_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let returned_count = outcome
        .payload
        .get("returned_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let scope_label = if filter.is_empty() {
        "all"
    } else {
        filter.as_str()
    };
    let mut lines = vec![
        format!("- scope: {scope_label}"),
        format!("- matched tasks: {returned_count}/{matched_count}"),
    ];
    let sessions = outcome
        .payload
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if sessions.is_empty() {
        lines.push("- no delegate task sessions matched".to_owned());
    } else {
        for session in &sessions {
            push_session_summary_lines(&mut lines, session, shell.pane.session_id.as_str());
        }
    }

    append_surface_message(shell, "delegate tasks", lines.as_slice());
    shell
        .pane
        .set_status("Delegate tasks added to transcript".to_owned());
}

fn show_approvals_surface(shell: &mut state::Shell, args: &str) {
    let raw = args.trim();
    match parse_approvals_action(raw) {
        Ok(ApprovalsAction::Inspect {
            approval_request_id,
        }) => {
            let outcome = match execute_shell_app_tool(
                shell,
                "approval_request_status",
                json!({ "approval_request_id": approval_request_id }),
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    shell.pane.add_system_message(&format!(
                        "Unable to inspect approval request `{approval_request_id}`: {error}"
                    ));
                    return;
                }
            };
            let approval_request = outcome
                .payload
                .get("approval_request")
                .cloned()
                .unwrap_or(Value::Null);
            let lines = approval_request_lines(&approval_request);
            append_surface_message(shell, "approval request", lines.as_slice());
            shell.pane.set_status(format!(
                "Approval request `{approval_request_id}` added to transcript"
            ));
        }
        Ok(ApprovalsAction::List { filter }) => {
            let filter = filter.to_ascii_lowercase();
            let mut payload = json!({ "limit": session_surface_limit(shell) });
            if let Some(object) = payload.as_object_mut() {
                match filter.as_str() {
                    "" | "pending" => {
                        object.insert("status".to_owned(), Value::String("pending".to_owned()));
                    }
                    "attention" => {
                        object.insert(
                            "grant_attention".to_owned(),
                            Value::String("needs_attention".to_owned()),
                        );
                    }
                    "all" => {}
                    "approved" | "executing" | "executed" | "denied" | "expired" | "cancelled" => {
                        object.insert("status".to_owned(), Value::String(filter.clone()));
                    }
                    _ => {}
                }
            }

            let outcome = match execute_shell_app_tool(shell, "approval_requests_list", payload) {
                Ok(outcome) => outcome,
                Err(error) => {
                    shell
                        .pane
                        .add_system_message(&format!("Unable to list approval requests: {error}"));
                    return;
                }
            };

            let matched_count = outcome
                .payload
                .get("matched_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let returned_count = outcome
                .payload
                .get("returned_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let needs_attention_count = outcome
                .payload
                .get("attention_summary")
                .and_then(|value| value.get("needs_attention_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let scope_label = if filter.is_empty() {
                "pending"
            } else {
                filter.as_str()
            };
            let mut lines = vec![
                format!("- scope: {scope_label}"),
                format!("- matched requests: {returned_count}/{matched_count}"),
                format!("- needs attention: {needs_attention_count}"),
            ];
            let requests = outcome
                .payload
                .get("requests")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if requests.is_empty() {
                lines.push("- no approval requests matched".to_owned());
            } else {
                for request in &requests {
                    lines.extend(approval_request_lines(request));
                }
                lines.push(
                    "- resolve with `/approvals resolve <request-id> <approve-once|approve-always|deny>`"
                        .to_owned(),
                );
            }

            append_surface_message(shell, "approval requests", lines.as_slice());
            shell
                .pane
                .set_status("Approval requests added to transcript".to_owned());
        }
        Ok(ApprovalsAction::Resolve {
            approval_request_id,
            ..
        }) => {
            shell.pane.add_system_message(&format!(
                "Use `/approvals resolve {approval_request_id} ...` from the composer submit path to resolve this request."
            ));
        }
        Err(error) => {
            shell.pane.add_system_message(&error);
        }
    }
}

fn session_history_limit(shell: &state::Shell) -> usize {
    shell
        .runtime_config
        .as_ref()
        .map(|config| config.tools.sessions.history_limit.max(1))
        .unwrap_or(50)
}

fn switch_resumed_session(
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    target_session_id: &str,
) -> Result<(), String> {
    let visibility_scope_session_id = session_visibility_scope_session_id(shell)?;
    switch_session_with_scope(
        owned_runtime,
        shell,
        target_session_id,
        Some(visibility_scope_session_id.as_str()),
    )
}

fn switch_subagent_session(
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    target_session_id: &str,
) -> Result<(), String> {
    let visibility_scope_session_id = session_visibility_scope_session_id(shell)?;
    switch_session_with_scope(
        owned_runtime,
        shell,
        target_session_id,
        Some(visibility_scope_session_id.as_str()),
    )
}

fn switch_session_with_scope(
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    target_session_id: &str,
    visibility_scope_session_id: Option<&str>,
) -> Result<(), String> {
    let current_runtime = resolve_active_runtime(owned_runtime.as_ref())
        .ok_or_else(|| "active TUI runtime is unavailable".to_owned())?;
    let target_session_id = target_session_id.trim();
    if target_session_id.is_empty() {
        return Err("resume target session id cannot be empty".to_owned());
    }
    if target_session_id == current_runtime.session_id {
        shell
            .pane
            .set_status(format!("Already on session `{target_session_id}`"));
        return Ok(());
    }

    let scoped_session_id = visibility_scope_session_id.unwrap_or(shell.pane.session_id.as_str());
    let status_outcome = execute_shell_app_tool_for_session(
        shell,
        scoped_session_id,
        "session_status",
        json!({ "session_id": target_session_id }),
    )?;
    let history_outcome = execute_shell_app_tool_for_session(
        shell,
        scoped_session_id,
        "sessions_history",
        json!({
            "session_id": target_session_id,
            "limit": session_history_limit(shell),
        }),
    )?;
    let next_runtime = std::sync::Arc::new(current_runtime.switched_session(target_session_id));
    let transcript = session_history_messages(&history_outcome.payload);
    let session = status_outcome
        .payload
        .get("session")
        .cloned()
        .unwrap_or(Value::Null);
    let state = session
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let label_suffix = session
        .get("label")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|label| format!(" · {label}"))
        .unwrap_or_default();
    let system_message = format!("Resumed session `{target_session_id}` ({state}{label_suffix}).");

    activate_resumed_session(shell, next_runtime.as_ref(), transcript, &system_message);
    shell
        .pane
        .set_status(format!("Switched to session `{target_session_id}`"));
    *owned_runtime = Some(next_runtime);
    Ok(())
}

fn trimmed_optional_session_label(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn next_root_session_id() -> String {
    static ROOT_SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

    let elapsed_since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp_millis = elapsed_since_epoch.as_millis();
    let sequence = ROOT_SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);

    format!("root-{timestamp_millis:x}-{sequence:x}")
}

fn create_new_root_session_record(
    repo: &crate::session::repository::SessionRepository,
    label: Option<&str>,
) -> Result<String, String> {
    let label = label.map(str::to_owned);

    for _attempt in 0..4 {
        let session_id = next_root_session_id();
        let record = crate::session::repository::NewSessionRecord {
            session_id: session_id.clone(),
            kind: crate::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: label.clone(),
            state: crate::session::repository::SessionState::Ready,
        };
        let create_result = repo.create_session(record);

        match create_result {
            Ok(_) => return Ok(session_id),
            Err(error) if error.contains("UNIQUE constraint failed") => continue,
            Err(error) => return Err(error),
        }
    }

    Err("failed to allocate a unique root session id".to_owned())
}

fn start_new_session(
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    label: &str,
) -> Result<(), String> {
    let current_runtime = resolve_active_runtime(owned_runtime.as_ref())
        .ok_or_else(|| "active TUI runtime is unavailable".to_owned())?;
    let config = shell
        .runtime_config
        .as_ref()
        .ok_or_else(|| "session creation requires an initialized runtime config".to_owned())?;
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)?;
    let label = trimmed_optional_session_label(label);
    let session_id = create_new_root_session_record(&repo, label.as_deref())?;
    let next_runtime = std::sync::Arc::new(current_runtime.switched_session(session_id.as_str()));
    let label_suffix = label
        .as_deref()
        .map(|value| format!(" · {value}"))
        .unwrap_or_default();
    let system_message = format!("Started new session `{session_id}`{label_suffix}.");
    let transcript = Vec::new();

    activate_resumed_session(shell, next_runtime.as_ref(), transcript, &system_message);
    shell
        .pane
        .set_status(format!("Started new session `{session_id}`"));
    *owned_runtime = Some(next_runtime);
    Ok(())
}

fn apply_model_runtime_config(
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    updated_config: crate::config::LoongClawConfig,
) -> Result<(), String> {
    let current_runtime = resolve_active_runtime(owned_runtime.as_ref())
        .ok_or_else(|| "active TUI runtime is unavailable".to_owned())?;
    let next_runtime =
        std::sync::Arc::new(current_runtime.with_provider_runtime_config(updated_config.clone()));
    shell.runtime_config = Some(updated_config);
    shell.runtime_config_path = Some(next_runtime.resolved_path.clone());
    shell.pane.model = next_runtime.model_label.clone();
    shell.pane.context_length = state::context_length_for_model(&next_runtime.model_label);
    refresh_composer_suggestion_context(shell);
    *owned_runtime = Some(next_runtime);
    Ok(())
}

fn refresh_model_runtime_environment(
    owned_runtime: &Option<std::sync::Arc<super::runtime::TuiRuntime>>,
) -> Result<(), String> {
    let active_runtime = resolve_active_runtime(owned_runtime.as_ref())
        .ok_or_else(|| "active TUI runtime is unavailable".to_owned())?;
    let active_config = &active_runtime.config;
    let resolved_path = &active_runtime.resolved_path;
    crate::runtime_env::initialize_runtime_environment(active_config, Some(resolved_path));
    Ok(())
}

fn switch_model_selection(
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    selector: &str,
    reasoning: Option<ModelReasoningChoice>,
) -> Result<(), String> {
    let target_selector = selector.trim();
    if target_selector.is_empty() {
        return Err("model selector cannot be empty".to_owned());
    }
    let config_path = shell
        .runtime_config_path
        .clone()
        .ok_or_else(|| "model switching requires a resolved runtime config path".to_owned())?;
    let (updated_config, previous_active_provider, selected_profile_id) =
        persist_model_selection_to_config(config_path.as_path(), target_selector, reasoning)?;
    apply_model_runtime_config(owned_runtime, shell, updated_config.clone())?;

    let mut lines = model_status_lines(&updated_config);
    if let Some(previous_active_provider) = previous_active_provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("- previous provider: {previous_active_provider}"));
    }
    lines.push(format!("- selector: {target_selector}"));
    if !selected_profile_id.eq_ignore_ascii_case(target_selector) {
        lines.push(format!("- resolved profile: {selected_profile_id}"));
    }

    append_surface_message(shell, "model status", lines.as_slice());
    shell
        .pane
        .set_status(format!("Switched model with selector `{target_selector}`"));
    Ok(())
}

fn show_permissions_surface(shell: &mut state::Shell, args: &str) {
    let target_session_id = args.trim();
    let payload = if target_session_id.is_empty() {
        json!({})
    } else {
        json!({ "session_id": target_session_id })
    };
    let outcome = match execute_shell_app_tool(shell, "session_tool_policy_status", payload) {
        Ok(outcome) => outcome,
        Err(error) => {
            let label = if target_session_id.is_empty() {
                "current session".to_owned()
            } else {
                format!("session `{target_session_id}`")
            };
            shell.pane.add_system_message(&format!(
                "Unable to inspect permissions for {label}: {error}"
            ));
            return;
        }
    };

    let policy = outcome
        .payload
        .get("policy")
        .cloned()
        .unwrap_or(Value::Null);
    let target_session_id = outcome
        .payload
        .get("target_session_id")
        .and_then(Value::as_str)
        .unwrap_or(shell.pane.session_id.as_str());
    let base_tool_ids = json_string_items(policy.get("base_tool_ids"));
    let effective_tool_ids = json_string_items(policy.get("effective_tool_ids"));
    let requested_tool_ids = json_string_items(policy.get("requested_tool_ids"));
    let mut lines = vec![
        format!("- session: {target_session_id}"),
        format!(
            "- explicit policy: {}",
            if policy
                .get("has_policy")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "yes"
            } else {
                "no"
            }
        ),
        format!("- base tools: {}", base_tool_ids.len()),
        format!("- effective tools: {}", effective_tool_ids.len()),
        format!(
            "- requested tools: {}",
            summarize_string_items(requested_tool_ids.as_slice(), 6)
        ),
        format!(
            "- effective tool preview: {}",
            summarize_string_items(effective_tool_ids.as_slice(), 6)
        ),
    ];

    if let Some(updated_at) = policy.get("updated_at").and_then(Value::as_i64) {
        lines.push(format!("- updated: {}", format_unix_timestamp(updated_at)));
    }

    lines.push(format!(
        "- requested narrowing: {}",
        if json_value_present(policy.get("requested_runtime_narrowing")) {
            json_preview(policy.get("requested_runtime_narrowing"), 96)
        } else {
            "(none)".to_owned()
        }
    ));
    lines.push(format!(
        "- delegate narrowing: {}",
        if json_value_present(policy.get("delegate_runtime_narrowing")) {
            json_preview(policy.get("delegate_runtime_narrowing"), 96)
        } else {
            "(none)".to_owned()
        }
    ));
    lines.push(format!(
        "- effective narrowing: {}",
        if json_value_present(policy.get("effective_runtime_narrowing")) {
            json_preview(policy.get("effective_runtime_narrowing"), 96)
        } else {
            "(none)".to_owned()
        }
    ));

    append_surface_message(shell, "session permissions", lines.as_slice());
    shell
        .pane
        .set_status("Session permissions added to transcript".to_owned());
}

fn render_tool_status_label(status: &super::message::ToolStatus) -> String {
    match status {
        super::message::ToolStatus::Running { started } => {
            format!("running ({:.1}s)", started.elapsed().as_secs_f32())
        }
        super::message::ToolStatus::Done {
            success,
            duration_ms,
            ..
        } => {
            let result = if *success { "done" } else { "failed" };
            format!("{result} ({}ms)", duration_ms)
        }
    }
}

fn show_tools_surface(shell: &mut state::Shell) {
    let tool_call_count = shell.pane.tool_call_count();
    let recent_tool_calls = shell.pane.recent_tool_calls(3);
    let mut lines = vec![format!("- tool calls recorded: {tool_call_count}")];

    if recent_tool_calls.is_empty() {
        lines.push("- no tool calls yet".to_owned());
    } else {
        for tool_call in recent_tool_calls {
            let args_preview = if tool_call.args_preview.is_empty() {
                "(no args)".to_owned()
            } else {
                tool_call.args_preview.to_owned()
            };
            let args_preview = args_preview.replace('\n', " ");
            lines.push(format!(
                "- {} · {} · {}",
                tool_call.tool_name,
                render_tool_status_label(tool_call.status),
                args_preview
            ));
        }
        lines.push("- use Ctrl+O or `/tools open` for full latest tool details".to_owned());
    }

    append_surface_message(shell, "tool activity", lines.as_slice());
    shell
        .pane
        .set_status("Tool activity added to transcript".to_owned());
}

fn show_skills_surface(shell: &mut state::Shell, args: &str) {
    let Some(runtime_config) = tool_runtime_config_from_shell(shell) else {
        shell.pane.add_system_message(
            "Skills view is unavailable before the chat runtime is initialized.",
        );
        return;
    };

    let skill_id = args.trim();
    if skill_id.is_empty() {
        let outcome = match crate::tools::external_skills_operator_list_with_config(&runtime_config)
        {
            Ok(outcome) => outcome,
            Err(error) => {
                shell
                    .pane
                    .add_system_message(&format!("Unable to list skills: {error}"));
                return;
            }
        };

        let mut lines = Vec::new();
        if let Some(skills) = outcome
            .payload
            .get("skills")
            .and_then(|value| value.as_array())
        {
            lines.push(format!("- active skills: {}", skills.len()));
            for skill in skills.iter().take(8) {
                let skill_id = skill
                    .get("skill_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("(unknown)");
                let summary = skill
                    .get("summary")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                lines.push(format!("- {skill_id}: {summary}"));
            }
        }
        if let Some(shadowed) = outcome
            .payload
            .get("shadowed_skills")
            .and_then(|value| value.as_array())
        {
            lines.push(format!("- shadowed skills: {}", shadowed.len()));
        }

        append_surface_message(shell, "skills status", lines.as_slice());
        shell
            .pane
            .set_status("Skills status added to transcript".to_owned());
        return;
    }

    let outcome =
        match crate::tools::external_skills_operator_inspect_with_config(skill_id, &runtime_config)
        {
            Ok(outcome) => outcome,
            Err(error) => {
                shell
                    .pane
                    .add_system_message(&format!("Unable to inspect skill `{skill_id}`: {error}"));
                return;
            }
        };

    let mut lines = Vec::new();
    if let Some(skill) = outcome.payload.get("skill") {
        if let Some(skill_id) = skill.get("skill_id").and_then(|value| value.as_str()) {
            lines.push(format!("- skill: {skill_id}"));
        }
        if let Some(scope) = skill.get("scope").and_then(|value| value.as_str()) {
            lines.push(format!("- scope: {scope}"));
        }
        if let Some(summary) = skill.get("summary").and_then(|value| value.as_str()) {
            lines.push(format!("- summary: {summary}"));
        }
    }
    if let Some(preview) = outcome
        .payload
        .get("instructions_preview")
        .and_then(|value| value.as_str())
    {
        lines.push("- preview:".to_owned());
        lines.extend(preview.lines().map(|line| format!("  {line}")));
    }

    append_surface_message(shell, "skill details", lines.as_slice());
    shell
        .pane
        .set_status(format!("Skill `{skill_id}` details added to transcript"));
}

fn show_commands_surface(shell: &mut state::Shell) {
    let mut lines = Vec::new();

    for spec in commands::discoverable_command_specs() {
        let mut command_label = spec.name.to_owned();
        if let Some(argument_hint) = spec.argument_hint {
            command_label.push(' ');
            command_label.push_str(argument_hint);
        }
        lines.push(format!(
            "- {command_label} [{}]: {}",
            spec.category, spec.help
        ));
        if !spec.aliases.is_empty() {
            lines.push(format!("  aliases: {}", spec.aliases.join(", ")));
        }
    }

    append_surface_message(shell, "command catalog", lines.as_slice());
    shell
        .pane
        .set_status("Command catalog added to transcript".to_owned());
}

fn default_transcript_export_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(format!("loongclaw-transcript-{timestamp}.txt"))
}

fn resolve_transcript_export_path(args: &str) -> PathBuf {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return default_transcript_export_path();
    }

    PathBuf::from(trimmed)
}

fn latest_copyable_output(shell: &state::Shell) -> Option<String> {
    let message = shell
        .pane
        .messages
        .iter()
        .rev()
        .find(|message| message.role != super::message::Role::User)?;
    let mut lines = Vec::new();

    match message.role {
        super::message::Role::Assistant => {
            for part in &message.parts {
                match part {
                    super::message::MessagePart::Text(text) => {
                        lines.extend(text.lines().map(ToOwned::to_owned));
                    }
                    super::message::MessagePart::ThinkBlock(_)
                    | super::message::MessagePart::ToolCall { .. }
                    | super::message::MessagePart::SurfaceEvent { .. } => {}
                }
            }
        }
        super::message::Role::System | super::message::Role::Surface => {
            for part in &message.parts {
                match part {
                    super::message::MessagePart::Text(text) => {
                        lines.extend(text.lines().map(ToOwned::to_owned));
                    }
                    super::message::MessagePart::SurfaceEvent { title, lines: body } => {
                        lines.push(title.clone());
                        lines.extend(body.iter().cloned());
                    }
                    super::message::MessagePart::ThinkBlock(_)
                    | super::message::MessagePart::ToolCall { .. } => {}
                }
            }
        }
        super::message::Role::User => {}
    }

    if lines.is_empty() {
        return None;
    }

    Some(lines.join("\n"))
}

enum ExportTarget {
    Latest,
    Transcript,
}

fn resolve_export_request(args: &str) -> (ExportTarget, PathBuf) {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return (ExportTarget::Transcript, default_transcript_export_path());
    }

    let mut parts = trimmed.split_whitespace();
    let Some(first) = parts.next() else {
        return (ExportTarget::Transcript, default_transcript_export_path());
    };
    let remainder = parts.collect::<Vec<_>>().join(" ");

    match first.to_ascii_lowercase().as_str() {
        "latest" => {
            let path = if remainder.trim().is_empty() {
                resolve_transcript_export_path("loongclaw-latest-output.txt")
            } else {
                resolve_transcript_export_path(remainder.as_str())
            };
            (ExportTarget::Latest, path)
        }
        "transcript" => {
            let path = if remainder.trim().is_empty() {
                default_transcript_export_path()
            } else {
                resolve_transcript_export_path(remainder.as_str())
            };
            (ExportTarget::Transcript, path)
        }
        _ => (
            ExportTarget::Transcript,
            resolve_transcript_export_path(trimmed),
        ),
    }
}

fn export_transcript(shell: &mut state::Shell, args: &str) {
    let (target, export_path) = resolve_export_request(args);
    let export_body = match target {
        ExportTarget::Latest => latest_copyable_output(shell),
        ExportTarget::Transcript => {
            let plain_lines = transcript_plain_lines(shell);
            (!plain_lines.is_empty()).then(|| plain_lines.join("\n"))
        }
    };

    let Some(export_body) = export_body else {
        shell.pane.set_status("Nothing to export".to_owned());
        return;
    };

    match fs::write(&export_path, export_body) {
        Ok(()) => {
            let export_target = match target {
                ExportTarget::Latest => "latest output",
                ExportTarget::Transcript => "transcript",
            };
            let lines = vec![
                format!("- path: {}", export_path.display()),
                format!("- target: {export_target}"),
            ];
            append_surface_message(shell, "export status", lines.as_slice());
            shell
                .pane
                .set_status(format!("Exported to {}", export_path.display()));
        }
        Err(error) => {
            shell.pane.set_status(format!("Export failed: {error}"));
        }
    }
}

const DIFF_OUTPUT_LINE_LIMIT: usize = 120;

fn git_output(args: &[&str], cwd: &std::path::Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("failed to run `git {}`: {error}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            return Err(format!(
                "`git {}` exited with status {}",
                args.join(" "),
                output.status
            ));
        }
        return Err(stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn normalize_diff_mode(args: &str) -> Result<&str, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("status") {
        return Ok("status");
    }
    if trimmed.eq_ignore_ascii_case("full") {
        return Ok("full");
    }

    Err(format!(
        "unsupported diff mode `{trimmed}`; use `status` or `full`"
    ))
}

fn build_diff_surface_lines(cwd: &std::path::Path, mode: &str) -> Result<Vec<String>, String> {
    let status = git_output(&["status", "--short"], cwd)?;
    let diff = if mode == "full" {
        git_output(
            &["diff", "--no-ext-diff", "--stat", "--patch", "--no-color"],
            cwd,
        )?
    } else {
        git_output(&["diff", "--no-ext-diff", "--stat", "--no-color"], cwd)?
    };

    let mut lines = vec![format!("- cwd: {}", cwd.display())];

    if status.trim().is_empty() && diff.trim().is_empty() {
        lines.push("- working tree clean".to_owned());
        return Ok(lines);
    }

    if !status.trim().is_empty() {
        lines.push("- status:".to_owned());
        lines.extend(status.lines().map(|line| format!("  {line}")));
    }

    if !diff.trim().is_empty() {
        lines.push(format!("- diff ({mode}):"));
        let diff_lines = diff.lines().collect::<Vec<_>>();
        let truncated = diff_lines.len() > DIFF_OUTPUT_LINE_LIMIT;
        lines.extend(
            diff_lines
                .iter()
                .take(DIFF_OUTPUT_LINE_LIMIT)
                .map(|line| format!("  {line}")),
        );
        if truncated {
            lines.push(format!(
                "  ... truncated after {} lines",
                DIFF_OUTPUT_LINE_LIMIT
            ));
        }
    }

    Ok(lines)
}

fn show_diff_surface(shell: &mut state::Shell, args: &str) {
    let mode = match normalize_diff_mode(args) {
        Ok(mode) => mode,
        Err(error) => {
            shell.pane.add_system_message(&error);
            return;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let lines = match build_diff_surface_lines(cwd.as_path(), mode) {
        Ok(lines) => lines,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to show diff: {error}"));
            return;
        }
    };

    append_surface_message(shell, "working tree diff", lines.as_slice());
    shell.pane.set_status("Diff added to transcript".to_owned());
}

enum CopyMode {
    Latest,
    Selection,
    Transcript,
}

fn resolve_copy_mode(args: &str) -> Result<CopyMode, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("latest") {
        return Ok(CopyMode::Latest);
    }
    if trimmed.eq_ignore_ascii_case("selection") {
        return Ok(CopyMode::Selection);
    }
    if trimmed.eq_ignore_ascii_case("transcript") || trimmed.eq_ignore_ascii_case("all") {
        return Ok(CopyMode::Transcript);
    }

    Err(format!(
        "unsupported copy mode `{trimmed}`; use `latest`, `selection`, or `transcript`"
    ))
}

fn apply_transcript_navigation(
    shell: &mut state::Shell,
    textarea: &tui_textarea::TextArea<'_>,
    action: HistoryNavigationAction,
    extend_selection: bool,
) {
    let total_lines = transcript_line_count(shell);

    if extend_selection {
        shell.pane.begin_transcript_selection();
    }

    apply_history_navigation(shell, textarea, action);

    match action {
        HistoryNavigationAction::ScrollLineUp => {
            shell.pane.move_transcript_cursor_up(1, total_lines);
        }
        HistoryNavigationAction::ScrollLineDown => {
            shell.pane.move_transcript_cursor_down(1, total_lines);
        }
        HistoryNavigationAction::ScrollHalfPageUp => {
            let step = usize::from(history_half_page_step(textarea));
            shell.pane.move_transcript_cursor_up(step, total_lines);
        }
        HistoryNavigationAction::ScrollHalfPageDown => {
            let step = usize::from(history_half_page_step(textarea));
            shell.pane.move_transcript_cursor_down(step, total_lines);
        }
        HistoryNavigationAction::ScrollPageUp => {
            let step = usize::from(history_page_step(textarea));
            shell.pane.move_transcript_cursor_up(step, total_lines);
        }
        HistoryNavigationAction::ScrollPageDown => {
            let step = usize::from(history_page_step(textarea));
            shell.pane.move_transcript_cursor_down(step, total_lines);
        }
        HistoryNavigationAction::JumpTop => {
            shell.pane.jump_transcript_cursor_top(total_lines);
        }
        HistoryNavigationAction::JumpLatest => {
            shell.pane.jump_transcript_cursor_latest(total_lines);
        }
    }
}

fn open_transcript_review(shell: &mut state::Shell) {
    let total_lines = transcript_line_count(shell);

    shell.pane.set_transcript_cursor_to_latest(total_lines);
    shell.focus.focus_transcript();
    shell.pane.set_status("Transcript review mode".to_owned());
}

fn transcript_cursor_tool_call_index(shell: &state::Shell) -> Option<usize> {
    let total_lines = transcript_line_count(shell);
    let cursor_line = shell.pane.transcript_cursor_line(total_lines)?;
    let render_width = terminal_render_width();
    let hit_target = history::transcript_hit_target_at_plain_line(
        &shell.pane,
        render_width,
        cursor_line,
        shell.show_thinking,
    )?;

    match hit_target {
        history::TranscriptHitTarget::ToolCallLine {
            tool_call_index, ..
        } => Some(tool_call_index),
        history::TranscriptHitTarget::PlainLine(_) => None,
    }
}

fn open_tool_inspector_from_transcript_cursor(shell: &mut state::Shell) -> bool {
    let tool_call_index = match transcript_cursor_tool_call_index(shell) {
        Some(tool_call_index) => tool_call_index,
        None => return false,
    };
    let opened = shell.pane.open_tool_inspector_for_index(tool_call_index);
    if !opened {
        return false;
    }

    if !shell.focus.has(FocusLayer::ToolInspector) {
        shell.focus.push(FocusLayer::ToolInspector);
    }

    true
}

fn close_transcript_review(shell: &mut state::Shell) {
    shell.pane.clear_transcript_selection();
    shell.focus.focus_composer();

    shell.pane.set_status("Back to composer".to_owned());
}

fn toggle_transcript_review(shell: &mut state::Shell) {
    if shell.focus.top() == FocusLayer::Transcript {
        close_transcript_review(shell);
        return;
    }

    open_transcript_review(shell);
}

fn tool_inspector_scroll_step() -> u16 {
    let terminal_size = crossterm::terminal::size();
    let (_, height) = terminal_size.unwrap_or((80, 24));
    let available_height = height.saturating_sub(8);
    let scroll_step = available_height / 2;

    scroll_step.max(1)
}

fn open_tool_inspector(shell: &mut state::Shell) {
    let opened = shell.pane.open_latest_tool_inspector();
    if opened {
        if !shell.focus.has(FocusLayer::ToolInspector) {
            shell.focus.push(FocusLayer::ToolInspector);
        }
    } else {
        shell.pane.set_status("No tool details available".into());
    }
}

fn close_tool_inspector(shell: &mut state::Shell) {
    shell.pane.close_tool_inspector();
    if shell.focus.top() == FocusLayer::ToolInspector {
        shell.focus.pop();
    }
}

fn build_osc52_copy_sequence(text: &str) -> String {
    let encoded_text = BASE64_STANDARD.encode(text.as_bytes());

    format!("\u{1b}]52;c;{encoded_text}\u{7}")
}

fn copy_text_to_terminal_clipboard(text: &str) -> CliResult<()> {
    let copy_sequence = build_osc52_copy_sequence(text);
    let mut stdout = io::stdout();

    stdout
        .write_all(copy_sequence.as_bytes())
        .map_err(|error| format!("failed to write clipboard escape sequence: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("failed to flush clipboard escape sequence: {error}"))?;

    Ok(())
}

fn copy_transcript_selection(shell: &mut state::Shell, mode: CopyMode) {
    let plain_lines = transcript_plain_lines(shell);
    let copied_text = match mode {
        CopyMode::Latest => latest_copyable_output(shell),
        CopyMode::Selection => shell.pane.transcript_copy_text(plain_lines.as_slice()),
        CopyMode::Transcript => Some(plain_lines.join("\n")),
    };

    let Some(copied_text) = copied_text else {
        shell.pane.set_status("Nothing to copy".to_owned());
        return;
    };

    let copied_line_count = match mode {
        CopyMode::Latest => copied_text.lines().count(),
        CopyMode::Selection => shell
            .pane
            .transcript_selection_line_count(plain_lines.len()),
        CopyMode::Transcript => plain_lines.len(),
    };
    let effective_line_count = copied_line_count.max(1);
    let line_label = if effective_line_count == 1 {
        "line"
    } else {
        "lines"
    };

    match copy_text_to_terminal_clipboard(copied_text.as_str()) {
        Ok(()) => {
            let copy_scope = match mode {
                CopyMode::Latest => "latest output",
                CopyMode::Selection => "selection",
                CopyMode::Transcript => "transcript",
            };
            shell.pane.set_status(format!(
                "Copied {copy_scope}: {effective_line_count} {line_label}"
            ));
        }
        Err(error) => {
            shell.pane.set_status(format!("Copy failed: {error}"));
        }
    }
}

fn scroll_stats_overlay_up(shell: &mut state::Shell, amount: usize) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    let current_offset = stats_overlay.list_scroll_offset;
    let next_offset = current_offset.saturating_sub(amount);
    stats_overlay.list_scroll_offset = next_offset;
}

fn scroll_stats_overlay_down(shell: &mut state::Shell, amount: usize) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    let current_offset = stats_overlay.list_scroll_offset;
    let next_offset = current_offset.saturating_add(amount);
    stats_overlay.list_scroll_offset = next_offset;
    clamp_stats_overlay_list_offset(stats_overlay);
}

fn jump_stats_overlay_top(shell: &mut state::Shell) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    stats_overlay.list_scroll_offset = 0;
}

fn jump_stats_overlay_bottom(shell: &mut state::Shell) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    let entry_count = stats_overlay_entry_count(stats_overlay);
    let last_index = entry_count.saturating_sub(1);
    stats_overlay.list_scroll_offset = last_index;
    clamp_stats_overlay_list_offset(stats_overlay);
}

fn apply_terminal_event(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    event: Event,
    tx: &mpsc::UnboundedSender<UiEvent>,
    submit_text: &mut Option<String>,
) {
    if let Event::Mouse(mouse_event) = event {
        apply_mouse_event(shell, textarea, mouse_event, submit_text);
        return;
    }

    let Event::Key(key) = event else {
        return;
    };

    match shell.focus.top() {
        FocusLayer::ClarifyDialog => {
            if let Some(ref mut dialog) = shell.pane.clarify_dialog {
                #[allow(clippy::wildcard_enum_match_arm)]
                match key.code {
                    KeyCode::Enter => {
                        let response = dialog.response();
                        shell.pane.clarify_dialog = None;
                        shell.focus.pop();
                        let _ = tx.send(UiEvent::Token {
                            content: format!("\n[user chose: {response}]\n"),
                            is_thinking: false,
                        });
                    }
                    KeyCode::Esc => {
                        shell.pane.clarify_dialog = None;
                        shell.focus.pop();
                    }
                    KeyCode::Up => dialog.select_up(),
                    KeyCode::Down => dialog.select_down(),
                    KeyCode::Left => dialog.move_cursor_left(),
                    KeyCode::Right => dialog.move_cursor_right(),
                    KeyCode::Backspace => dialog.delete_back(),
                    KeyCode::Char(ch) => dialog.insert_char(ch),
                    _ => {}
                }
            }
            return;
        }
        FocusLayer::SessionPicker => {
            enum SessionPickerAction {
                Close,
                Submit(String),
                UpdateQuery(String),
                MoveUp,
                MoveDown,
                None,
            }

            let action = {
                let session_picker = match shell.session_picker.as_mut() {
                    Some(session_picker) => session_picker,
                    None => return,
                };
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => SessionPickerAction::Close,
                    KeyCode::Enter => {
                        let filtered_indices = session_picker.filtered_indices();
                        let selected_index = session_picker.selected_index;
                        let selected_session_id = filtered_indices
                            .get(selected_index)
                            .and_then(|session_index| session_picker.sessions.get(*session_index))
                            .map(|session| session.session_id.clone());
                        match selected_session_id {
                            Some(session_id) => SessionPickerAction::Submit(session_id),
                            None => SessionPickerAction::None,
                        }
                    }
                    KeyCode::Up => SessionPickerAction::MoveUp,
                    KeyCode::Down => SessionPickerAction::MoveDown,
                    KeyCode::Backspace => {
                        let mut query = session_picker.query.clone();
                        query.pop();
                        SessionPickerAction::UpdateQuery(query)
                    }
                    KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL) => {
                        let mut query = session_picker.query.clone();
                        query.push(ch);
                        SessionPickerAction::UpdateQuery(query)
                    }
                    KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Tab
                    | KeyCode::BackTab
                    | KeyCode::Delete
                    | KeyCode::Insert
                    | KeyCode::F(_)
                    | KeyCode::Char(_)
                    | KeyCode::Null
                    | KeyCode::CapsLock
                    | KeyCode::ScrollLock
                    | KeyCode::NumLock
                    | KeyCode::PrintScreen
                    | KeyCode::Pause
                    | KeyCode::Menu
                    | KeyCode::KeypadBegin
                    | KeyCode::Media(_)
                    | KeyCode::Modifier(_) => SessionPickerAction::None,
                }
            };

            match action {
                SessionPickerAction::Close => {
                    close_session_picker(shell);
                }
                SessionPickerAction::Submit(session_id) => {
                    if shell.pane.agent_running {
                        shell.pane.add_system_message(
                            "Cannot switch threads while a response is already in progress.",
                        );
                    } else {
                        let submit_command = shell
                            .session_picker
                            .as_ref()
                            .map(|picker| picker.mode.submit_command(session_id.as_str()))
                            .unwrap_or_else(|| format!("/resume {session_id}"));
                        let status = shell
                            .session_picker
                            .as_ref()
                            .map(|picker| picker.mode.switching_status_message().to_owned())
                            .unwrap_or_else(|| "Switching sessions...".to_owned());
                        close_session_picker(shell);
                        *submit_text = Some(submit_command);
                        shell.pane.set_status(status);
                    }
                }
                SessionPickerAction::UpdateQuery(query) => {
                    if let Some(session_picker) = shell.session_picker.as_mut() {
                        session_picker.query = query;
                        session_picker.selected_index = 0;
                        session_picker.list_scroll_offset = 0;
                        session_picker.clamp_selection(session_picker_visible_rows());
                    }
                }
                SessionPickerAction::MoveUp => {
                    if let Some(session_picker) = shell.session_picker.as_mut() {
                        let filtered_count = session_picker.filtered_indices().len();
                        if filtered_count > 0 {
                            if session_picker.selected_index == 0 {
                                session_picker.selected_index = filtered_count.saturating_sub(1);
                            } else {
                                session_picker.selected_index =
                                    session_picker.selected_index.saturating_sub(1);
                            }
                            session_picker.clamp_selection(session_picker_visible_rows());
                        }
                    }
                }
                SessionPickerAction::MoveDown => {
                    if let Some(session_picker) = shell.session_picker.as_mut() {
                        let filtered_count = session_picker.filtered_indices().len();
                        if filtered_count > 0 {
                            let next_index = session_picker.selected_index.saturating_add(1);
                            session_picker.selected_index = next_index % filtered_count;
                            session_picker.clamp_selection(session_picker_visible_rows());
                        }
                    }
                }
                SessionPickerAction::None => {}
            }
            return;
        }
        FocusLayer::StatsOverlay => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                close_stats_overlay(shell);
                return;
            }

            if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    let copied_text = stats::render_copy_text(
                        &stats_overlay.snapshot,
                        stats_overlay.active_tab,
                        stats_overlay.date_range,
                    );
                    let copy_result = copy_text_to_terminal_clipboard(copied_text.as_str());
                    match copy_result {
                        Ok(()) => {
                            stats_overlay.copy_status = Some("copied".to_owned());
                            shell.pane.set_status("Stats copied".to_owned());
                        }
                        Err(error) => {
                            stats_overlay.copy_status = Some("copy failed".to_owned());
                            shell.pane.set_status(format!("Copy failed: {error}"));
                        }
                    }
                }
                return;
            }

            if key.code == KeyCode::Tab || key.code == KeyCode::Right {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    stats_overlay.active_tab = stats_overlay.active_tab.next();
                    stats_overlay.list_scroll_offset = 0;
                }
                return;
            }

            if key.code == KeyCode::BackTab || key.code == KeyCode::Left {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    stats_overlay.active_tab = stats_overlay.active_tab.previous();
                    stats_overlay.list_scroll_offset = 0;
                }
                return;
            }

            if key.code == KeyCode::Char('r') && key.modifiers.is_empty() {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    stats_overlay.date_range = stats_overlay.date_range.next();
                    stats_overlay.list_scroll_offset = 0;
                }
                return;
            }

            if key.code == KeyCode::Up {
                scroll_stats_overlay_up(shell, 1);
                return;
            }

            if key.code == KeyCode::Down {
                scroll_stats_overlay_down(shell, 1);
                return;
            }

            if key.code == KeyCode::PageUp {
                scroll_stats_overlay_up(shell, 5);
                return;
            }

            if key.code == KeyCode::PageDown {
                scroll_stats_overlay_down(shell, 5);
                return;
            }

            if key.code == KeyCode::Home {
                jump_stats_overlay_top(shell);
                return;
            }

            if key.code == KeyCode::End {
                jump_stats_overlay_bottom(shell);
                return;
            }

            return;
        }
        FocusLayer::ToolInspector => {
            let scroll_step = tool_inspector_scroll_step();

            #[allow(clippy::wildcard_enum_match_arm)]
            #[allow(clippy::wildcard_enum_match_arm)]
            #[allow(clippy::wildcard_enum_match_arm)]
            match key.code {
                KeyCode::Esc => {
                    close_tool_inspector(shell);
                }
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = shell.pane.open_latest_tool_inspector();
                }
                KeyCode::Up => {
                    let moved = shell.pane.select_previous_tool_inspector_entry();
                    if !moved {
                        shell.pane.scroll_tool_inspector_up(1);
                    }
                }
                KeyCode::Down => {
                    let moved = shell.pane.select_next_tool_inspector_entry();
                    if !moved {
                        shell.pane.scroll_tool_inspector_down(1);
                    }
                }
                KeyCode::PageUp => {
                    shell.pane.scroll_tool_inspector_up(scroll_step);
                }
                KeyCode::PageDown => {
                    shell.pane.scroll_tool_inspector_down(scroll_step);
                }
                KeyCode::Home => {
                    let _ = shell.pane.select_first_tool_inspector_entry();
                }
                KeyCode::End => {
                    let _ = shell.pane.select_last_tool_inspector_entry();
                }
                _ => {}
            }
            return;
        }
        FocusLayer::Help => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                shell.focus.pop();
            }
            return;
        }
        FocusLayer::Transcript => {
            let navigation_action = transcript_navigation_action(key);
            if let Some(action) = navigation_action {
                let extend_selection = key.modifiers.contains(KeyModifiers::SHIFT);
                apply_transcript_navigation(shell, textarea, action, extend_selection);
                return;
            }

            match key.code {
                KeyCode::Esc => {
                    if shell.pane.clear_transcript_selection() {
                        shell.pane.set_status("Selection cleared".to_owned());
                        return;
                    }
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Enter => {
                    let opened_tool_inspector = open_tool_inspector_from_transcript_cursor(shell);
                    if opened_tool_inspector {
                        return;
                    }
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Tab if key.modifiers.is_empty() => {
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Char('q') if key.modifiers.is_empty() => {
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    open_tool_inspector(shell);
                    return;
                }
                KeyCode::Char('v') if key.modifiers.is_empty() => {
                    let selection_active = shell.pane.toggle_transcript_selection();
                    if selection_active {
                        shell.pane.set_status("Selection started".to_owned());
                    } else {
                        shell.pane.set_status("Selection cleared".to_owned());
                    }
                    return;
                }
                KeyCode::Char('y') if key.modifiers.is_empty() => {
                    copy_transcript_selection(shell, CopyMode::Selection);
                    return;
                }
                KeyCode::Backspace
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Delete
                | KeyCode::Insert
                | KeyCode::F(_)
                | KeyCode::Char(_)
                | KeyCode::Null
                | KeyCode::CapsLock
                | KeyCode::ScrollLock
                | KeyCode::NumLock
                | KeyCode::PrintScreen
                | KeyCode::Pause
                | KeyCode::Menu
                | KeyCode::KeypadBegin
                | KeyCode::Media(_)
                | KeyCode::Modifier(_) => {
                    shell.pane.set_status(
                        "Transcript is focused. Press Tab or click the input box.".to_owned(),
                    );
                    return;
                }
            }
        }
        FocusLayer::Composer => {
            // Fall through to global shortcuts + textarea below
        }
    }

    // --- Global shortcuts ---------------------------------------------
    let composer_has_slash_matches = !slash_command_matches(shell, textarea).is_empty();
    if composer_has_slash_matches {
        #[allow(clippy::wildcard_enum_match_arm)]
        match key.code {
            KeyCode::Down | KeyCode::Tab if key.modifiers.is_empty() => {
                let moved = cycle_slash_command_selection(shell, textarea, 1);
                if moved {
                    return;
                }
            }
            KeyCode::Up | KeyCode::BackTab if key.modifiers.is_empty() => {
                let moved = cycle_slash_command_selection(shell, textarea, -1);
                if moved {
                    return;
                }
            }
            _ => {}
        }
    }

    let composer_is_empty = textarea_is_empty(textarea);
    let navigation_action = history_navigation_action(key, composer_is_empty);
    if let Some(action) = navigation_action {
        apply_history_navigation(shell, textarea, action);
        return;
    }

    if key.code == KeyCode::Tab && key.modifiers.is_empty() {
        toggle_transcript_review(shell);
        return;
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if shell.pane.agent_running {
                shell.interrupt_requested = true;
                shell.pane.set_status("Interrupting response...".to_owned());
            } else if shell.pane.has_pending_submissions() {
                let pending_submission_count = shell.pane.pending_submission_count();
                shell.pane.clear_pending_submissions();
                shell.pane.set_status(format!(
                    "Cleared {pending_submission_count} pending submission(s)"
                ));
            } else {
                shell.running = false;
            }
            return;
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            shell.show_thinking = !shell.show_thinking;
            let label = if shell.show_thinking { "on" } else { "off" };
            shell.pane.set_status(format!("Thinking display: {label}"));
            return;
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_tool_inspector(shell);
            return;
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_transcript_review(shell);
            return;
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            shell.pane.cycle_busy_input_mode();
            let busy_input_mode = shell.pane.busy_input_mode();
            let label = busy_input_mode.label().to_ascii_lowercase();
            shell.pane.set_status(format!("Busy input mode: {label}"));
            return;
        }
        _ => {}
    }

    if shell.focus.top() != FocusLayer::Composer {
        return;
    }

    // --- Escape to clear staged message --------------------------------
    if key.code == KeyCode::Esc && shell.pane.agent_running && shell.pane.has_pending_submissions()
    {
        shell.pane.clear_pending_submissions();
        shell.pane.set_status("Pending submissions cleared".into());
        return;
    }

    // --- Enter to submit (or stage if agent is running) ---------------
    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
        textarea.insert_newline();
        return;
    }

    if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
        let text: String = textarea.lines().join("\n");
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        // Slash commands are handled immediately regardless of agent state.
        if let Some(request) = commands::parse(trimmed) {
            if matches!(request.command, SlashCommand::Unknown(_)) {
                let palette_selection = apply_selected_slash_command(shell, textarea);
                if let Some(selection_submit) = palette_selection {
                    shell.slash_command_selection = 0;
                    if !selection_submit.is_empty() {
                        *submit_text = Some(selection_submit);
                    }
                    return;
                }
            }
            textarea.select_all();
            textarea.delete_str(usize::MAX);
            if is_async_slash_request(&request) || is_runtime_slash_request(&request) {
                if shell.pane.agent_running {
                    let message = if matches!(request.command, SlashCommand::New) {
                        "Cannot start a new session while a response is already in progress."
                    } else if matches!(request.command, SlashCommand::Rename) {
                        "Cannot rename the current session while a response is already in progress."
                    } else if matches!(request.command, SlashCommand::Resume) {
                        "Cannot switch sessions while a response is already in progress."
                    } else if matches!(request.command, SlashCommand::Subagents) {
                        "Cannot switch subagent threads while a response is already in progress."
                    } else if matches!(request.command, SlashCommand::Model) {
                        "Cannot switch models while a response is already in progress."
                    } else {
                        "Cannot run this command while a response is already in progress."
                    };
                    shell.pane.add_system_message(message);
                } else {
                    *submit_text = Some(trimmed.to_owned());
                    let status = if matches!(request.command, SlashCommand::New) {
                        "Starting new session..."
                    } else if matches!(request.command, SlashCommand::Rename) {
                        "Renaming session..."
                    } else if matches!(request.command, SlashCommand::Resume) {
                        "Switching sessions..."
                    } else if matches!(request.command, SlashCommand::Subagents) {
                        "Switching subagent thread..."
                    } else if matches!(request.command, SlashCommand::Model) {
                        "Switching model..."
                    } else if matches!(request.command, SlashCommand::Approvals) {
                        "Resolving approval request..."
                    } else {
                        "Running context compaction..."
                    };
                    shell.pane.set_status(status.to_owned());
                }
            } else {
                handle_slash_command(shell, request);
            }
            shell.slash_command_selection = 0;
            return;
        }

        textarea.select_all();
        textarea.delete_str(usize::MAX);

        if shell.pane.agent_running {
            queue_busy_submission(shell, trimmed);
        } else {
            // Agent is idle — submit immediately.
            shell.pane.add_user_message(trimmed);
            shell.pane.scroll_offset = 0;
            *submit_text = Some(trimmed.to_owned());
        }
        return;
    }

    // --- Everything else goes to the textarea -------------------------
    // Map crossterm key events manually to avoid version-mismatch issues
    // between the app's crossterm and tui-textarea's crossterm dependency.
    #[allow(clippy::wildcard_enum_match_arm)]
    match key.code {
        KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL) => {
            textarea.insert_char(ch);
            shell.slash_command_selection = 0;
        }
        KeyCode::Backspace => {
            textarea.delete_char();
            shell.slash_command_selection = 0;
        }
        KeyCode::Left => {
            textarea.move_cursor(tui_textarea::CursorMove::Back);
        }
        KeyCode::Right => {
            textarea.move_cursor(tui_textarea::CursorMove::Forward);
        }
        KeyCode::Up => {
            textarea.move_cursor(tui_textarea::CursorMove::Up);
        }
        KeyCode::Down => {
            textarea.move_cursor(tui_textarea::CursorMove::Down);
        }
        KeyCode::Home => {
            textarea.move_cursor(tui_textarea::CursorMove::Head);
        }
        KeyCode::End => {
            textarea.move_cursor(tui_textarea::CursorMove::End);
        }
        _ => {}
    }
}

fn apply_mouse_event(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    mouse_event: MouseEvent,
    submit_text: &mut Option<String>,
) {
    let shell_areas = terminal_shell_areas(textarea);
    let column = mouse_event.column;
    let row = mouse_event.row;
    let slash_palette_area = slash_command_palette_area(shell, textarea);

    if shell.focus.top() == FocusLayer::ToolInspector {
        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                shell.pane.scroll_tool_inspector_up(3);
            }
            MouseEventKind::ScrollDown => {
                shell.pane.scroll_tool_inspector_down(3);
            }
            MouseEventKind::Down(_)
            | MouseEventKind::Up(_)
            | MouseEventKind::Drag(_)
            | MouseEventKind::Moved
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {}
        }
        return;
    }

    if shell.focus.top() == FocusLayer::SessionPicker {
        return;
    }

    if shell.focus.top() == FocusLayer::StatsOverlay {
        return;
    }

    let in_history = point_in_rect(shell_areas.history, column, row);
    let in_input = point_in_rect(shell_areas.input, column, row);

    match mouse_event.kind {
        MouseEventKind::ScrollUp => {
            if in_history {
                shell.pane.scroll_offset = shell.pane.scroll_offset.saturating_add(3);
            }
        }
        MouseEventKind::ScrollDown => {
            if in_history {
                shell.pane.scroll_offset = shell.pane.scroll_offset.saturating_sub(3);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(palette_area) = slash_palette_area
                && point_in_rect(palette_area, column, row)
            {
                let command_index = slash_command_index_at_mouse_row(shell, textarea, row);
                let Some(command_index) = command_index else {
                    return;
                };
                shell.slash_command_selection = command_index;
                let palette_selection = apply_selected_slash_command(shell, textarea);
                if let Some(selection_submit) = palette_selection {
                    shell.slash_command_selection = 0;
                    if !selection_submit.is_empty() {
                        *submit_text = Some(selection_submit);
                    }
                }
                return;
            }

            if in_input {
                shell.focus.focus_composer();
                return;
            }

            if in_history {
                let viewport_row = row.saturating_sub(shell_areas.history.y);
                let hit_target = history::viewport_hit_target_at(
                    &shell.pane,
                    shell_areas.history.width,
                    shell_areas.history.height,
                    viewport_row,
                    shell.show_thinking,
                );

                let Some(hit_target) = hit_target else {
                    return;
                };
                let line_index = match hit_target {
                    history::TranscriptHitTarget::PlainLine(plain_line_index) => plain_line_index,
                    history::TranscriptHitTarget::ToolCallLine {
                        plain_line_index,
                        tool_call_index,
                    } => {
                        let opened = shell.pane.open_tool_inspector_for_index(tool_call_index);
                        if opened {
                            if !shell.focus.has(FocusLayer::ToolInspector) {
                                shell.focus.push(FocusLayer::ToolInspector);
                            }
                            return;
                        }
                        plain_line_index
                    }
                };

                shell.focus.focus_transcript();
                shell.pane.transcript_review.cursor_line = line_index;
                shell.pane.transcript_review.anchor_line = Some(line_index);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if !in_history {
                return;
            }

            let viewport_row = row.saturating_sub(shell_areas.history.y);
            let line_index = history::viewport_plain_line_at(
                &shell.pane,
                shell_areas.history.width,
                shell_areas.history.height,
                viewport_row,
                shell.show_thinking,
            );

            let Some(line_index) = line_index else {
                return;
            };

            shell.focus.focus_transcript();
            if shell.pane.transcript_review.anchor_line.is_none() {
                shell.pane.transcript_review.anchor_line = Some(line_index);
            }
            shell.pane.transcript_review.cursor_line = line_index;
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if shell.focus.top() == FocusLayer::Transcript
                && shell.pane.transcript_review.anchor_line.is_some()
            {
                shell.pane.set_status("Mouse selection updated".to_owned());
            }
        }
        MouseEventKind::Down(_)
        | MouseEventKind::Up(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::Moved
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => {}
    }
}

fn handle_slash_command(shell: &mut state::Shell, request: ParsedSlashCommand) {
    match request.command {
        SlashCommand::Exit => {
            shell.running = false;
        }
        SlashCommand::Commands => {
            show_commands_surface(shell);
        }
        SlashCommand::Compact => {
            shell.pane.add_system_message(
                "Context compaction runs asynchronously. Submit `/compact` from the composer.",
            );
        }
        SlashCommand::New => {
            shell.pane.add_system_message(
                "Use `/new` from the composer submit path to start a fresh session.",
            );
        }
        SlashCommand::Rename => {
            show_rename_surface(shell, request.args.as_str());
        }
        SlashCommand::Resume => {
            show_resume_surface(shell, request.args.as_str());
        }
        SlashCommand::Subagents => {
            show_subagents_surface(shell, request.args.as_str());
        }
        SlashCommand::Tasks => {
            show_tasks_surface(shell, request.args.as_str());
        }
        SlashCommand::Approvals => {
            show_approvals_surface(shell, request.args.as_str());
        }
        SlashCommand::Permissions => {
            show_permissions_surface(shell, request.args.as_str());
        }
        SlashCommand::Clear => {
            shell.pane.messages.clear();
            shell.pane.add_system_message("Conversation cleared.");
        }
        SlashCommand::Help => {
            if shell.focus.has(FocusLayer::Help) {
                shell.focus.pop();
            } else {
                shell.focus.push(FocusLayer::Help);
            }
            // Help is rendered as an overlay — no transcript message needed.
        }
        SlashCommand::Export => {
            export_transcript(shell, request.args.as_str());
        }
        SlashCommand::Diff => {
            show_diff_surface(shell, request.args.as_str());
        }
        SlashCommand::Model => {
            show_model_surface(shell, request.args.as_str());
        }
        SlashCommand::Mode => {
            show_busy_input_mode_surface(shell, request.args.as_str());
        }
        SlashCommand::Stats => {
            open_stats_overlay(shell, request.args.as_str());
        }
        SlashCommand::Session => {
            show_session_surface(shell);
        }
        SlashCommand::Status => {
            show_runtime_status_surface(shell);
        }
        SlashCommand::Context => {
            show_context_surface(shell);
        }
        SlashCommand::Skills => {
            show_skills_surface(shell, request.args.as_str());
        }
        SlashCommand::Review => {
            toggle_transcript_review(shell);
        }
        SlashCommand::Tools => {
            if request.args.trim().eq_ignore_ascii_case("open") {
                open_tool_inspector(shell);
            } else {
                show_tools_surface(shell);
            }
        }
        SlashCommand::Thinking => {
            match request.args.trim().to_ascii_lowercase().as_str() {
                "" | "toggle" => {
                    shell.show_thinking = !shell.show_thinking;
                }
                "on" => {
                    shell.show_thinking = true;
                }
                "off" => {
                    shell.show_thinking = false;
                }
                other => {
                    shell.pane.add_system_message(&format!(
                        "Unknown thinking mode: {other}. Use `on`, `off`, or `toggle`."
                    ));
                    return;
                }
            }
            let label = if shell.show_thinking { "on" } else { "off" };
            shell.pane.set_status(format!("Thinking display: {label}"));
        }
        SlashCommand::Latest => {
            shell.pane.scroll_offset = 0;
            shell.pane.set_status("Jumped to latest output".to_owned());
        }
        SlashCommand::Top => {
            shell.pane.scroll_offset = u16::MAX;
            shell.pane.set_status("Viewing oldest output".to_owned());
        }
        SlashCommand::Copy => match resolve_copy_mode(request.args.as_str()) {
            Ok(mode) => copy_transcript_selection(shell, mode),
            Err(error) => shell.pane.add_system_message(&error),
        },
        SlashCommand::Unknown(name) => {
            shell
                .pane
                .add_system_message(&format!("Unknown command: {name}"));
        }
    }
}

fn terminal_render_width() -> usize {
    match crossterm::terminal::size() {
        Ok((width, _)) => usize::from(width.max(40)),
        Err(_) => 80,
    }
}

fn replace_textarea_contents(textarea: &mut tui_textarea::TextArea<'_>, value: &str) {
    textarea.select_all();
    textarea.delete_str(usize::MAX);

    if !value.is_empty() {
        textarea.insert_str(value);
    }
}

fn take_textarea_submission(textarea: &mut tui_textarea::TextArea<'_>) -> String {
    let text = textarea.lines().join("\n");
    textarea.select_all();
    textarea.delete_str(usize::MAX);
    text
}

fn apply_boot_screen(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    screen: &TuiBootScreen,
) {
    shell.pane.show_surface_lines(&screen.lines);
    shell.pane.input_hint_override = Some(screen.prompt_hint.clone());
    shell.pane.agent_running = false;
    shell.pane.scroll_offset = u16::MAX;
    replace_textarea_contents(textarea, &screen.initial_value);
}

fn activate_chat_surface(
    shell: &mut state::Shell,
    runtime: &super::runtime::TuiRuntime,
    system_message: Option<String>,
) {
    shell.pane.messages.clear();
    shell.runtime_config = Some(runtime.config.clone());
    shell.runtime_config_path = Some(runtime.resolved_path.clone());
    shell.pane.model = runtime.model_label.clone();
    shell.pane.context_length = state::context_length_for_model(&runtime.model_label);
    shell.pane.clear_input_hint_override();

    if let Some(system_message) = system_message {
        shell.pane.add_system_message(&system_message);
    }

    shell
        .pane
        .add_system_message("Welcome to LoongClaw TUI. Type a message and press Enter.");
    refresh_composer_suggestion_context(shell);
    reset_subagent_surface_tracking(shell);
}

fn handle_boot_key_event(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    key: KeyEvent,
    tx: &mpsc::UnboundedSender<UiEvent>,
    boot_escape_submit: Option<&str>,
    submit_text: &mut Option<String>,
) {
    let is_ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    if is_ctrl_c {
        shell.running = false;
        return;
    }

    let is_escape = key.code == KeyCode::Esc;
    if is_escape {
        *submit_text = boot_escape_submit.map(str::to_owned);
        return;
    }

    let is_submit = key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT);
    if is_submit {
        let text = take_textarea_submission(textarea);
        *submit_text = Some(text);
        return;
    }

    let forwarded_event = Event::Key(key);
    apply_terminal_event(shell, textarea, forwarded_event, tx, submit_text);
}

fn apply_boot_transition(
    transition: TuiBootTransition,
    boot_flow: &mut Option<Box<dyn TuiBootFlow>>,
    boot_escape_submit: &mut Option<String>,
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    config_path: Option<&str>,
    session_hint: Option<&str>,
) -> CliResult<()> {
    match transition {
        TuiBootTransition::Screen(screen) => {
            *boot_escape_submit = screen.escape_submit.clone();
            apply_boot_screen(shell, textarea, &screen);
        }
        TuiBootTransition::StartChat { system_message } => {
            if owned_runtime.is_none() {
                let runtime = super::runtime::initialize(config_path, session_hint)?;
                let shared_runtime = std::sync::Arc::new(runtime);
                *owned_runtime = Some(shared_runtime);
            }

            let active_runtime = resolve_active_runtime(owned_runtime.as_ref());
            let Some(runtime) = active_runtime else {
                return Err("failed to initialize TUI runtime after boot flow".to_owned());
            };

            *boot_flow = None;
            *boot_escape_submit = None;
            activate_chat_surface(shell, runtime.as_ref(), system_message);
            replace_textarea_contents(textarea, "");
        }
        TuiBootTransition::Exit => {
            shell.running = false;
        }
    }

    Ok(())
}

async fn submit_boot_flow_input(
    boot_flow: &mut Option<Box<dyn TuiBootFlow>>,
    boot_escape_submit: &mut Option<String>,
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    config_path: Option<&str>,
    session_hint: Option<&str>,
    text: &str,
) -> CliResult<()> {
    let width = terminal_render_width();
    let input = text.to_owned();

    let Some(flow) = boot_flow.as_mut() else {
        return Err("internal TUI state error: boot flow missing during submit".to_owned());
    };

    let transition = flow.submit(input, width).await?;

    apply_boot_transition(
        transition,
        boot_flow,
        boot_escape_submit,
        owned_runtime,
        shell,
        textarea,
        config_path,
        session_hint,
    )?;

    Ok(())
}

fn resolve_active_runtime(
    owned_runtime: Option<&std::sync::Arc<super::runtime::TuiRuntime>>,
) -> Option<std::sync::Arc<super::runtime::TuiRuntime>> {
    owned_runtime.cloned()
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub(super) async fn run(
    runtime: &super::runtime::TuiRuntime,
    palette_hint: super::terminal::PaletteHint,
) -> CliResult<()> {
    run_inner(Some(runtime.clone()), None, None, None, None, palette_hint).await
}

pub(super) async fn run_lazy(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    boot_flow: Option<Box<dyn TuiBootFlow>>,
    initial_system_message: Option<String>,
    palette_hint: super::terminal::PaletteHint,
) -> CliResult<()> {
    run_inner(
        None,
        config_path,
        session_hint,
        boot_flow,
        initial_system_message,
        palette_hint,
    )
    .await
}

fn prepare_chat_turn_future(
    runtime: std::sync::Arc<super::runtime::TuiRuntime>,
    text: String,
    tx: mpsc::UnboundedSender<UiEvent>,
) -> Pin<Box<dyn std::future::Future<Output = ()>>> {
    let obs = build_tui_observer(tx.clone());
    let tx2 = tx;
    let streamed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let streamed_flag = streamed.clone();
    let tracking_obs = TrackingObserver {
        inner: obs,
        streamed: streamed_flag,
    };
    let tracking_handle: crate::conversation::ConversationTurnObserverHandle =
        std::sync::Arc::new(tracking_obs);

    Box::pin(async move {
        let result = run_turn(runtime.as_ref(), text.as_str(), Some(tracking_handle)).await;
        match result {
            Ok(reply) => {
                if !streamed.load(std::sync::atomic::Ordering::Relaxed) && !reply.is_empty() {
                    let _ = tx2.send(UiEvent::Token {
                        content: reply,
                        is_thinking: false,
                    });
                    let _ = tx2.send(UiEvent::ResponseDone {
                        input_tokens: 0,
                        output_tokens: 0,
                    });
                }
            }
            Err(error) => {
                let _ = tx2.send(UiEvent::TurnError(error));
            }
        }
    })
}

fn process_submitted_chat_text(
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    text: &str,
    tx: &mpsc::UnboundedSender<UiEvent>,
    turn_future: &mut Pin<Box<dyn std::future::Future<Output = ()> + '_>>,
    turn_active: &mut bool,
) -> CliResult<()> {
    let Some(runtime) = resolve_active_runtime(owned_runtime.as_ref()) else {
        return Ok(());
    };

    if let Some(request) = commands::parse(text) {
        if is_runtime_slash_request(&request) {
            if matches!(request.command, SlashCommand::New) {
                match start_new_session(owned_runtime, shell, request.args.as_str()) {
                    Ok(()) => {
                        *turn_active = false;
                        *turn_future = Box::pin(std::future::pending());
                    }
                    Err(error) => {
                        shell
                            .pane
                            .add_system_message(&format!("Unable to start a new session: {error}"));
                    }
                }
            } else if matches!(request.command, SlashCommand::Rename) {
                match parse_rename_action(request.args.as_str()) {
                    Ok(RenameAction::Set(label)) => {
                        let payload = json!({ "label": label });
                        let outcome = execute_shell_app_tool(shell, "session_rename", payload);
                        match outcome {
                            Ok(outcome) => {
                                if let Some(inspection) = outcome.payload.get("inspection") {
                                    let session =
                                        inspection.get("session").cloned().unwrap_or(Value::Null);
                                    let session_id = session
                                        .get("session_id")
                                        .and_then(Value::as_str)
                                        .unwrap_or("(unknown)");
                                    let label = session
                                        .get("label")
                                        .and_then(Value::as_str)
                                        .filter(|value| !value.trim().is_empty())
                                        .unwrap_or("(unset)");
                                    let lines = vec![
                                        format!("- session: {session_id}"),
                                        format!("- label: {label}"),
                                    ];
                                    append_surface_message(
                                        shell,
                                        "session label",
                                        lines.as_slice(),
                                    );
                                }
                                refresh_composer_suggestion_context(shell);
                                shell.pane.set_status("Renamed current session".to_owned());
                                *turn_active = false;
                                *turn_future = Box::pin(std::future::pending());
                            }
                            Err(error) => {
                                let message =
                                    format!("Unable to rename the current session: {error}");
                                shell.pane.add_system_message(&message);
                            }
                        }
                    }
                    Ok(RenameAction::Status) => {}
                    Err(error) => shell.pane.add_system_message(&error),
                }
            } else if matches!(request.command, SlashCommand::Resume) {
                if let ResumeAction::Switch(target_session_id) =
                    parse_resume_action(request.args.as_str())
                {
                    match switch_resumed_session(owned_runtime, shell, target_session_id.as_str()) {
                        Ok(()) => {
                            *turn_active = false;
                            *turn_future = Box::pin(std::future::pending());
                        }
                        Err(error) => {
                            shell.pane.add_system_message(&format!(
                                "Unable to resume session `{target_session_id}`: {error}"
                            ));
                        }
                    }
                }
            } else if matches!(request.command, SlashCommand::Subagents) {
                if let SubagentAction::Switch(target_session_id) =
                    parse_subagent_action(request.args.as_str())
                {
                    match switch_subagent_session(owned_runtime, shell, target_session_id.as_str())
                    {
                        Ok(()) => {
                            *turn_active = false;
                            *turn_future = Box::pin(std::future::pending());
                        }
                        Err(error) => {
                            shell.pane.add_system_message(&format!(
                                "Unable to switch subagent thread `{target_session_id}`: {error}"
                            ));
                        }
                    }
                }
            } else if matches!(request.command, SlashCommand::Model) {
                match parse_model_action(request.args.as_str()) {
                    Ok(ModelAction::Switch {
                        selector,
                        reasoning,
                    }) => {
                        match switch_model_selection(
                            owned_runtime,
                            shell,
                            selector.as_str(),
                            reasoning,
                        ) {
                            Ok(()) => {
                                if let Err(error) = refresh_model_runtime_environment(owned_runtime)
                                {
                                    shell.pane.add_system_message(&format!(
                                        "Model switched, but runtime environment refresh failed: {error}"
                                    ));
                                }
                                *turn_active = false;
                                *turn_future = Box::pin(std::future::pending());
                            }
                            Err(error) => {
                                shell.pane.add_system_message(&format!(
                                    "Unable to switch model `{selector}`: {error}"
                                ));
                            }
                        }
                    }
                    Ok(ModelAction::Status) => {}
                    Err(error) => shell.pane.add_system_message(&error),
                }
            }
            return Ok(());
        }

        *turn_future = if is_async_slash_request(&request) {
            prepare_async_slash_command_future(runtime, request, tx.clone())
        } else {
            prepare_chat_turn_future(runtime, text.to_string(), tx.clone())
        };
    } else {
        *turn_future = prepare_chat_turn_future(runtime, text.to_string(), tx.clone());
    }

    *turn_active = true;
    shell.pane.agent_running = true;
    Ok(())
}

fn queue_busy_submission(shell: &mut state::Shell, text: &str) {
    let queued_text = text.to_owned();
    let busy_input_mode = shell.pane.busy_input_mode();
    shell.pane.enqueue_busy_submission(queued_text);
    let pending_submission_count = shell.pane.pending_submission_count();
    let status = match busy_input_mode {
        state::BusyInputMode::Queue => {
            format!("Queued {pending_submission_count} message(s)")
        }
        state::BusyInputMode::Steer => "Steer message armed".to_owned(),
    };
    shell.pane.set_status(status);
}

fn submit_pending_message(shell: &mut state::Shell, submit_text: &mut Option<String>) {
    let pending_submission = shell.pane.take_next_pending_submission();
    let Some(pending_submission) = pending_submission else {
        return;
    };

    let pending_text = pending_submission.text;
    let pending_mode = pending_submission.mode;
    let status = match pending_mode {
        state::BusyInputMode::Queue => "Sending queued message...".to_owned(),
        state::BusyInputMode::Steer => "Sending steer message...".to_owned(),
    };

    shell.pane.add_user_message(pending_text.as_str());
    shell.pane.scroll_offset = 0;
    shell.pane.set_status(status);
    *submit_text = Some(pending_text);
}

fn apply_interrupt_request(
    shell: &mut state::Shell,
    turn_future: &mut Pin<Box<dyn std::future::Future<Output = ()> + '_>>,
    turn_active: &mut bool,
) {
    if !shell.interrupt_requested {
        return;
    }
    shell.interrupt_requested = false;

    if !*turn_active {
        return;
    }

    *turn_active = false;
    *turn_future = Box::pin(std::future::pending());
    shell.pane.agent_running = false;
    shell
        .pane
        .add_system_message("Interrupted the in-flight turn.");

    let pending_submission_count = shell.pane.pending_submission_count();
    let status = if pending_submission_count > 0 {
        format!("Response interrupted; {pending_submission_count} pending submission(s) kept")
    } else {
        "Response interrupted".to_owned()
    };
    shell.pane.set_status(status);
}

fn interrupt_for_steer_boundary(
    shell: &mut state::Shell,
    turn_future: &mut Pin<Box<dyn std::future::Future<Output = ()> + '_>>,
    turn_active: &mut bool,
    submit_text: &mut Option<String>,
) {
    if !*turn_active {
        return;
    }

    let running_tool_call_count = shell.pane.running_tool_call_count();
    if running_tool_call_count > 0 {
        return;
    }

    let steer_submission = shell.pane.take_steer_submission();
    let Some(steer_submission) = steer_submission else {
        return;
    };

    *turn_active = false;
    *turn_future = Box::pin(std::future::pending());
    shell.pane.agent_running = false;
    shell.pane.add_system_message(
        "Steer mode interrupted the in-flight turn after the latest tool completed.",
    );

    let steer_text = steer_submission.text;
    shell.pane.add_user_message(steer_text.as_str());
    shell.pane.scroll_offset = 0;
    shell.pane.set_status("Steering conversation...".to_owned());
    *submit_text = Some(steer_text);
}

async fn run_inner(
    initial_runtime: Option<super::runtime::TuiRuntime>,
    config_path: Option<&str>,
    session_hint: Option<&str>,
    mut boot_flow: Option<Box<dyn TuiBootFlow>>,
    initial_system_message: Option<String>,
    palette_hint: super::terminal::PaletteHint,
) -> CliResult<()> {
    let mut guard = TerminalGuard::enter()?;

    let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();

    let mut textarea = tui_textarea::TextArea::default();
    textarea.set_cursor_line_style(Style::default());

    let session_id = initial_runtime
        .as_ref()
        .map(|runtime| runtime.session_id.as_str())
        .or_else(|| {
            session_hint
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("default");
    let mut shell = state::Shell::new(session_id);

    let mut owned_runtime = initial_runtime.map(std::sync::Arc::new);
    if boot_flow.is_none() {
        if let Some(runtime) = resolve_active_runtime(owned_runtime.as_ref()) {
            activate_chat_surface(&mut shell, runtime.as_ref(), initial_system_message.clone());
        } else {
            let runtime = super::runtime::initialize(config_path, session_hint)?;
            activate_chat_surface(&mut shell, &runtime, initial_system_message.clone());
            owned_runtime = Some(std::sync::Arc::new(runtime));
        }
    }

    let palette = match palette_hint {
        super::terminal::PaletteHint::Dark => Palette::dark(),
        super::terminal::PaletteHint::Light => Palette::light(),
        super::terminal::PaletteHint::Plain => Palette::plain(),
    };

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut crossterm_events = EventStream::new();

    let mut turn_future: Pin<Box<dyn std::future::Future<Output = ()> + '_>> =
        Box::pin(std::future::pending());
    let mut turn_active = false;
    let mut boot_escape_submit: Option<String> = None;

    if let Some(flow) = boot_flow.as_mut() {
        let width = terminal_render_width();
        let screen = flow.begin(width)?;
        boot_escape_submit = screen.escape_submit.clone();
        apply_boot_screen(&mut shell, &mut textarea, &screen);
    }

    if shell.dirty {
        // Render a deterministic first frame before the async event loop starts
        // so PTY clients observe a stable fullscreen surface immediately.
        shell.pane.tick_animations();
        guard.draw(&shell, &textarea, &palette)?;
        shell.dirty = false;
    }

    loop {
        // ── Phase 1: Drain all pending events (non-blocking) ──────────

        let mut submit_text: Option<String> = None;

        // Drain observer channel
        while let Ok(event) = rx.try_recv() {
            let tool_done_boundary = matches!(event, UiEvent::ToolDone { .. });
            apply_ui_event(&mut shell, event);
            shell.dirty = true;
            if tool_done_boundary {
                interrupt_for_steer_boundary(
                    &mut shell,
                    &mut turn_future,
                    &mut turn_active,
                    &mut submit_text,
                );
            }
        }

        // Drain crossterm terminal events
        {
            while let Some(maybe_event) = crossterm_events.next().now_or_never().flatten() {
                if let Ok(event) = maybe_event {
                    let mut submit_text_drain: Option<String> = None;
                    if boot_flow.is_some() {
                        if let Event::Key(key) = event {
                            let boot_escape_submit = boot_escape_submit.as_deref();
                            handle_boot_key_event(
                                &mut shell,
                                &mut textarea,
                                key,
                                &tx,
                                boot_escape_submit,
                                &mut submit_text_drain,
                            );
                        }
                    } else {
                        apply_terminal_event(
                            &mut shell,
                            &mut textarea,
                            event,
                            &tx,
                            &mut submit_text_drain,
                        );
                    }
                    shell.dirty = true;
                    apply_interrupt_request(&mut shell, &mut turn_future, &mut turn_active);

                    if submit_text_drain.is_some() {
                        submit_text = submit_text_drain;
                    }
                }
            }
        }

        // Check turn completion (non-blocking)
        if turn_active {
            let waker = futures_util::task::noop_waker();
            let mut cx = std::task::Context::from_waker(&waker);
            if turn_future.as_mut().poll(&mut cx).is_ready() {
                turn_active = false;
                turn_future = Box::pin(std::future::pending());
                shell.pane.agent_running = false;
                shell.dirty = true;
                submit_pending_message(&mut shell, &mut submit_text);
            }
        }

        // Submit turn if drain phase produced one
        if let Some(ref text) = submit_text.take() {
            if boot_flow.is_some() {
                submit_boot_flow_input(
                    &mut boot_flow,
                    &mut boot_escape_submit,
                    &mut owned_runtime,
                    &mut shell,
                    &mut textarea,
                    config_path,
                    session_hint,
                    text,
                )
                .await?;
                shell.dirty = true;
            } else {
                process_submitted_chat_text(
                    &mut owned_runtime,
                    &mut shell,
                    text,
                    &tx,
                    &mut turn_future,
                    &mut turn_active,
                )?;
            }
        }

        // ── Phase 2: Render (only when dirty) ─────────────────────────
        if shell.dirty {
            shell.pane.tick_animations();
            guard.draw(&shell, &textarea, &palette)?;
            shell.dirty = false;
        }

        if !shell.running {
            break;
        }

        // ── Phase 3: Sleep until next event or tick ───────────────────
        let mut submit_text: Option<String> = None;

        tokio::select! {
            biased;

            Some(event) = rx.recv() => {
                let tool_done_boundary = matches!(event, UiEvent::ToolDone { .. });
                apply_ui_event(&mut shell, event);
                shell.dirty = true;
                if tool_done_boundary {
                    interrupt_for_steer_boundary(
                        &mut shell,
                        &mut turn_future,
                        &mut turn_active,
                        &mut submit_text,
                    );
                }
            }

            maybe_event = crossterm_events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    if boot_flow.is_some() {
                        if let Event::Key(key) = event {
                            let boot_escape_submit = boot_escape_submit.as_deref();
                            handle_boot_key_event(
                                &mut shell,
                                &mut textarea,
                                key,
                                &tx,
                                boot_escape_submit,
                                &mut submit_text,
                            );
                        }
                    } else {
                        apply_terminal_event(
                            &mut shell,
                            &mut textarea,
                            event,
                            &tx,
                            &mut submit_text,
                        );
                    }
                    shell.dirty = true;
                    apply_interrupt_request(&mut shell, &mut turn_future, &mut turn_active);
                }
            }

            _ = &mut turn_future, if turn_active => {
                turn_active = false;
                turn_future = Box::pin(std::future::pending());
                shell.pane.agent_running = false;
                shell.dirty = true;
                submit_pending_message(&mut shell, &mut submit_text);
            }

            _ = tick.tick() => {
                if shell.pane.needs_periodic_redraw() {
                    shell.dirty = true;
                }
                maybe_poll_background_context(&mut shell);
            }
        }

        // Submit turn after select! releases borrows
        if let Some(ref text) = submit_text.take() {
            if boot_flow.is_some() {
                submit_boot_flow_input(
                    &mut boot_flow,
                    &mut boot_escape_submit,
                    &mut owned_runtime,
                    &mut shell,
                    &mut textarea,
                    config_path,
                    session_hint,
                    text,
                )
                .await?;
                shell.dirty = true;
            } else {
                process_submitted_chat_text(
                    &mut owned_runtime,
                    &mut shell,
                    text,
                    &tx,
                    &mut turn_future,
                    &mut turn_active,
                )?;
            }
        }
    }

    drop(guard);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::tui::message::{MessagePart, Role};
    use crate::chat::tui::runtime;
    #[cfg(feature = "memory-sqlite")]
    use crate::memory::append_turn_direct;

    use crossterm::event::{KeyEventKind, KeyEventState};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn plain_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    struct CurrentDirGuard {
        original: PathBuf,
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.original).expect("restore current dir");
        }
    }

    fn set_current_dir_for_test(path: &Path) -> CurrentDirGuard {
        let original = std::env::current_dir().expect("read current dir");
        std::env::set_current_dir(path).expect("set current dir");
        CurrentDirGuard { original }
    }

    #[cfg(feature = "memory-sqlite")]
    fn shell_runtime_config_for_test(temp_root: &Path) -> crate::config::LoongClawConfig {
        let mut config = crate::config::LoongClawConfig::default();
        config.memory.sqlite_path = temp_root.join("memory.sqlite3").display().to_string();
        config.tools.sessions.enabled = true;
        config.tools.sessions.allow_mutation = true;
        config.tools.sessions.list_limit = 16;
        config.tools.file_root = Some(temp_root.display().to_string());
        config
    }

    #[cfg(feature = "memory-sqlite")]
    fn shell_memory_test_lock() -> &'static std::sync::Mutex<()> {
        static SHELL_MEMORY_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
            std::sync::OnceLock::new();
        SHELL_MEMORY_TEST_LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[cfg(feature = "memory-sqlite")]
    fn attach_shell_runtime_config(
        shell: &mut state::Shell,
        config: &crate::config::LoongClawConfig,
        temp_root: &Path,
    ) {
        let config_path = temp_root.join("loongclaw.toml");
        fs::write(
            &config_path,
            crate::config::render(config).expect("render shell test config"),
        )
        .expect("write config path");
        shell.runtime_config = Some(config.clone());
        shell.runtime_config_path = Some(config_path);
    }

    #[cfg(feature = "memory-sqlite")]
    fn scoped_test_id(temp_root: &Path, base: &str) -> String {
        let suffix = temp_root
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "temp".to_owned());
        format!("{base}-{suffix}")
    }

    #[cfg(feature = "memory-sqlite")]
    fn session_repo_for_config(
        config: &crate::config::LoongClawConfig,
    ) -> crate::session::repository::SessionRepository {
        let sqlite_path = config.memory.sqlite_path.trim();
        if !sqlite_path.is_empty() {
            let sqlite_path = std::path::Path::new(sqlite_path);
            let _ = crate::memory::drop_cached_sqlite_runtime(sqlite_path);
            let _ = fs::remove_file(sqlite_path);
        }
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        crate::session::repository::SessionRepository::new(&memory_config).expect("session repo")
    }

    #[cfg(feature = "memory-sqlite")]
    fn ensure_root_session(repo: &crate::session::repository::SessionRepository, session_id: &str) {
        repo.ensure_session(crate::session::repository::NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: crate::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some("Root session".to_owned()),
            state: crate::session::repository::SessionState::Ready,
        })
        .expect("ensure root session");
    }

    fn latest_message_text(shell: &state::Shell) -> String {
        shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.clone()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.clone()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or_default()
    }

    fn sample_stats_snapshot_for_test() -> stats::StatsSnapshot {
        let today = chrono::Utc::now().date_naive();
        let earlier = today - chrono::Duration::days(1);
        let mut earlier_models = std::collections::BTreeMap::new();
        earlier_models.insert(
            "gpt-5".to_owned(),
            stats::ModelTokenAccumulator {
                input_tokens: 120,
                output_tokens: 80,
            },
        );
        let mut current_models = std::collections::BTreeMap::new();
        current_models.insert(
            "o4-mini".to_owned(),
            stats::ModelTokenAccumulator {
                input_tokens: 140,
                output_tokens: 110,
            },
        );

        stats::StatsSnapshot {
            visible_sessions: 2,
            root_sessions: 1,
            delegate_sessions: 1,
            running_delegate_sessions: 1,
            pending_approvals: 1,
            usage_event_count: 2,
            first_activity_date: Some(earlier),
            last_activity_date: Some(today),
            longest_session: Some(stats::SessionDurationStat {
                session_id: "sess-root".to_owned(),
                label: Some("Root".to_owned()),
                duration_seconds: 3600,
            }),
            session_rows: vec![stats::StatsSessionRow {
                session_id: "sess-root".to_owned(),
                label: Some("Root".to_owned()),
                agent_presentation: None,
                kind: "root".to_owned(),
                state: "ready".to_owned(),
                turn_count: 2,
                duration_seconds: 3600,
                last_activity_date: Some(today),
                current: true,
            }],
            active_dates: vec![earlier, today],
            daily_points: vec![
                stats::DailyTokenPoint {
                    date: earlier,
                    total_input_tokens: 120,
                    total_output_tokens: 80,
                    total_tokens: 200,
                    model_tokens: earlier_models,
                },
                stats::DailyTokenPoint {
                    date: today,
                    total_input_tokens: 140,
                    total_output_tokens: 110,
                    total_tokens: 250,
                    model_tokens: current_models,
                },
            ],
        }
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn end_key_routes_to_history_when_composer_is_empty() {
        let key = plain_key(KeyCode::End);
        let action = history_navigation_action(key, true);

        assert_eq!(action, Some(HistoryNavigationAction::JumpLatest));
    }

    #[test]
    fn end_key_stays_with_input_when_composer_has_text() {
        let key = plain_key(KeyCode::End);
        let action = history_navigation_action(key, false);

        assert_eq!(action, None);
    }

    #[test]
    fn home_key_routes_to_history_when_composer_is_empty() {
        let key = plain_key(KeyCode::Home);
        let action = history_navigation_action(key, true);

        assert_eq!(action, Some(HistoryNavigationAction::JumpTop));
    }

    #[test]
    fn up_key_scrolls_history_when_composer_is_empty() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let up_event = Event::Key(plain_key(KeyCode::Up));

        apply_terminal_event(&mut shell, &mut textarea, up_event, &tx, &mut submit_text);

        assert_eq!(
            shell.pane.scroll_offset, 1,
            "Up should scroll transcript when composer is empty"
        );
    }

    #[test]
    fn down_key_scrolls_history_toward_latest_when_composer_is_empty() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let down_event = Event::Key(plain_key(KeyCode::Down));

        shell.pane.scroll_offset = 2;

        apply_terminal_event(&mut shell, &mut textarea, down_event, &tx, &mut submit_text);

        assert_eq!(
            shell.pane.scroll_offset, 1,
            "Down should move transcript toward latest output when composer is empty"
        );
    }

    #[test]
    fn shift_enter_inserts_newline_in_composer() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));

        textarea.insert_str("hello");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let lines = textarea.lines();

        assert_eq!(lines.len(), 2, "Shift+Enter should create a new line");
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], "");
        assert!(
            submit_text.is_none(),
            "Shift+Enter should not submit composer contents"
        );
    }

    #[test]
    fn tab_switches_primary_focus_to_transcript() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let tab_event = Event::Key(plain_key(KeyCode::Tab));

        apply_terminal_event(&mut shell, &mut textarea, tab_event, &tx, &mut submit_text);

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
    }

    #[test]
    fn transcript_focus_scrolls_even_when_composer_has_text() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let tab_event = Event::Key(plain_key(KeyCode::Tab));
        let up_event = Event::Key(plain_key(KeyCode::Up));

        textarea.insert_str("draft reply");

        apply_terminal_event(&mut shell, &mut textarea, tab_event, &tx, &mut submit_text);
        apply_terminal_event(&mut shell, &mut textarea, up_event, &tx, &mut submit_text);

        let draft_text = textarea.lines().join("\n");

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
        assert_eq!(shell.pane.scroll_offset, 1);
        assert_eq!(draft_text, "draft reply");
    }

    #[test]
    fn typing_while_transcript_focused_keeps_focus_and_draft_unchanged() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let tab_event = Event::Key(plain_key(KeyCode::Tab));
        let char_event = Event::Key(plain_key(KeyCode::Char('!')));

        textarea.insert_str("draft");

        apply_terminal_event(&mut shell, &mut textarea, tab_event, &tx, &mut submit_text);
        apply_terminal_event(&mut shell, &mut textarea, char_event, &tx, &mut submit_text);

        let draft_text = textarea.lines().join("\n");

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
        assert_eq!(draft_text, "draft");
    }

    #[test]
    fn ctrl_o_without_tool_calls_sets_status_message() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let open_event = Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));

        apply_terminal_event(&mut shell, &mut textarea, open_event, &tx, &mut submit_text);

        let status_message = shell
            .pane
            .status_message
            .as_ref()
            .map(|(msg, _)| msg.as_str());

        assert_eq!(status_message, Some("No tool details available"));
    }

    #[test]
    fn ctrl_o_with_tool_calls_opens_tool_inspector() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let open_event = Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));

        shell.pane.start_tool_call("tool-1", "shell", "ls -la");
        shell.pane.complete_tool_call("tool-1", true, "file-a", 12);

        apply_terminal_event(&mut shell, &mut textarea, open_event, &tx, &mut submit_text);

        let selected_tool_id = shell
            .pane
            .tool_inspector
            .as_ref()
            .map(|state| state.selected_tool_id.as_str());

        assert_eq!(shell.focus.top(), FocusLayer::ToolInspector);
        assert_eq!(selected_tool_id, Some("tool-1"));
    }

    #[test]
    fn ctrl_r_enables_history_navigation_with_non_empty_composer() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let review_event = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let up_event = Event::Key(plain_key(KeyCode::Up));

        textarea.insert_str("draft message");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            review_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(&mut shell, &mut textarea, up_event, &tx, &mut submit_text);

        assert_eq!(
            shell.pane.scroll_offset, 1,
            "review mode should allow transcript scrolling even when the composer has text"
        );
        assert_eq!(
            textarea.lines().join("\n"),
            "draft message",
            "review mode should not mutate composer contents while navigating transcript"
        );
    }

    #[test]
    fn esc_closes_tool_inspector_focus() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let open_event = Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        let close_event = Event::Key(plain_key(KeyCode::Esc));

        shell.pane.start_tool_call("tool-1", "shell", "ls -la");
        shell.pane.complete_tool_call("tool-1", true, "file-a", 12);

        apply_terminal_event(&mut shell, &mut textarea, open_event, &tx, &mut submit_text);
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            close_event,
            &tx,
            &mut submit_text,
        );

        assert_eq!(shell.focus.top(), FocusLayer::Composer);
        assert!(shell.pane.tool_inspector.is_none());
    }

    #[test]
    fn shift_up_in_transcript_focus_starts_selection() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let review_event = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let shift_up_event = Event::Key(modified_key(KeyCode::Up, KeyModifiers::SHIFT));

        shell.pane.add_system_message("line 1");
        shell.pane.add_system_message("line 2");
        shell.pane.add_system_message("line 3");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            review_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            shift_up_event,
            &tx,
            &mut submit_text,
        );

        let total_lines = transcript_line_count(&shell);

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
        assert_eq!(
            shell.pane.transcript_selection_range(total_lines),
            Some((4, 5))
        );
    }

    #[test]
    fn enter_on_tool_line_opens_matching_tool_inspector() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let review_event = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        shell
            .pane
            .start_tool_call("tool-1", "shell.exec", "git status --short");
        shell
            .pane
            .complete_tool_call("tool-1", true, "diff --git a/file b/file", 12);

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            review_event,
            &tx,
            &mut submit_text,
        );

        let width = 80_usize;
        let total_lines = transcript_line_count(&shell);
        let tool_line_index = (0..total_lines)
            .find(|line_index| {
                matches!(
                    history::transcript_hit_target_at_plain_line(
                        &shell.pane,
                        width,
                        *line_index,
                        shell.show_thinking,
                    ),
                    Some(history::TranscriptHitTarget::ToolCallLine { .. })
                )
            })
            .expect("tool line should exist");
        shell.pane.transcript_review.cursor_line = tool_line_index;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let selected_tool_id = shell
            .pane
            .tool_inspector
            .as_ref()
            .map(|tool_inspector| tool_inspector.selected_tool_id.as_str());

        assert_eq!(shell.focus.top(), FocusLayer::ToolInspector);
        assert_eq!(selected_tool_id, Some("tool-1"));
    }

    #[test]
    fn esc_clears_transcript_selection_before_returning_to_composer() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let review_event = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let selection_event = Event::Key(plain_key(KeyCode::Char('v')));
        let esc_event = Event::Key(plain_key(KeyCode::Esc));

        shell.pane.add_system_message("line 1");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            review_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            selection_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(&mut shell, &mut textarea, esc_event, &tx, &mut submit_text);

        let total_lines = transcript_line_count(&shell);

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
        assert_eq!(shell.pane.transcript_selection_range(total_lines), None);
    }

    #[test]
    fn osc52_copy_sequence_encodes_clipboard_payload() {
        let sequence = build_osc52_copy_sequence("hello");

        assert!(sequence.starts_with("\u{1b}]52;c;"));
        assert!(sequence.ends_with('\u{7}'));
        assert!(sequence.contains("aGVsbG8="));
    }

    #[test]
    fn enter_with_partial_slash_command_executes_selected_completion() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        textarea.insert_str("/rev");
        shell.slash_command_selection = 0;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
    }

    #[test]
    fn enter_with_compact_submits_async_slash_command() {
        let mut shell = state::Shell::new("sess-compact");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        textarea.insert_str("/compact");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let status = shell
            .pane
            .status_message
            .as_ref()
            .map(|(message, _)| message.as_str());

        assert_eq!(submit_text.as_deref(), Some("/compact"));
        assert_eq!(textarea.lines(), vec![""]);
        assert_eq!(status, Some("Running context compaction..."));
    }

    #[test]
    fn enter_with_new_submits_runtime_switch_request() {
        let mut shell = state::Shell::new("sess-new");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        textarea.insert_str("/new fresh start");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let status = shell
            .pane
            .status_message
            .as_ref()
            .map(|(message, _)| message.as_str());

        assert_eq!(submit_text.as_deref(), Some("/new fresh start"));
        assert_eq!(status, Some("Starting new session..."));
    }

    #[test]
    fn enter_with_rename_submits_runtime_update_request() {
        let mut shell = state::Shell::new("sess-rename");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        textarea.insert_str("/rename Session Prime");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let status = shell
            .pane
            .status_message
            .as_ref()
            .map(|(message, _)| message.as_str());

        assert_eq!(submit_text.as_deref(), Some("/rename Session Prime"));
        assert_eq!(status, Some("Renaming session..."));
    }

    #[test]
    fn enter_with_resume_submits_runtime_switch_request() {
        let mut shell = state::Shell::new("sess-resume");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        textarea.insert_str("/resume child-session");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let status = shell
            .pane
            .status_message
            .as_ref()
            .map(|(message, _)| message.as_str());

        assert_eq!(submit_text.as_deref(), Some("/resume child-session"));
        assert_eq!(status, Some("Switching sessions..."));
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn new_command_creates_root_session_switches_runtime_and_persists_label() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        let runtime = runtime::TuiRuntime {
            resolved_path: temp_dir.path().join("loongclaw.toml"),
            config: config.clone(),
            session_id: root_session_id.clone(),
            session_address: crate::conversation::ConversationSessionAddress::from_session_id(
                root_session_id.clone(),
            ),
            turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
            kernel_ctx: crate::context::bootstrap_kernel_context_with_config(
                "tui-new-session-test",
                crate::context::DEFAULT_TOKEN_TTL_S,
                &config,
            )
            .expect("bootstrap kernel context"),
            model_label: config
                .provider
                .resolved_model()
                .unwrap_or_else(|| "auto".to_owned()),
        };
        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        shell.pane.add_user_message("old history");
        let mut owned_runtime = Some(std::sync::Arc::new(runtime));
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut turn_future: Pin<Box<dyn std::future::Future<Output = ()>>> =
            Box::pin(std::future::pending());
        let mut turn_active = false;

        process_submitted_chat_text(
            &mut owned_runtime,
            &mut shell,
            "/new Session Alpha",
            &tx,
            &mut turn_future,
            &mut turn_active,
        )
        .expect("process new command");

        let new_session_id = shell.pane.session_id.clone();
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let repo = crate::session::repository::SessionRepository::new(&memory_config)
            .expect("session repo");
        let stored_session = repo
            .load_session(new_session_id.as_str())
            .expect("load session")
            .expect("created session");

        assert_ne!(new_session_id, root_session_id);
        assert_eq!(
            stored_session.kind,
            crate::session::repository::SessionKind::Root
        );
        assert_eq!(stored_session.parent_session_id, None);
        assert_eq!(stored_session.label.as_deref(), Some("Session Alpha"));
        assert_eq!(
            owned_runtime
                .as_ref()
                .map(|runtime| runtime.session_id.as_str()),
            Some(new_session_id.as_str())
        );
        assert!(
            latest_message_text(&shell).contains("Started new session"),
            "new command should reset transcript and append a system note"
        );
        assert_eq!(shell.pane.messages.len(), 1);
        assert!(!turn_active);
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn rename_command_updates_current_session_label() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        let runtime = runtime::TuiRuntime {
            resolved_path: temp_dir.path().join("loongclaw.toml"),
            config: config.clone(),
            session_id: root_session_id.clone(),
            session_address: crate::conversation::ConversationSessionAddress::from_session_id(
                root_session_id.clone(),
            ),
            turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
            kernel_ctx: crate::context::bootstrap_kernel_context_with_config(
                "tui-rename-session-test",
                crate::context::DEFAULT_TOKEN_TTL_S,
                &config,
            )
            .expect("bootstrap kernel context"),
            model_label: config
                .provider
                .resolved_model()
                .unwrap_or_else(|| "auto".to_owned()),
        };
        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let repo = crate::session::repository::SessionRepository::new(&memory_config)
            .expect("session repo");
        repo.ensure_session(crate::session::repository::NewSessionRecord {
            session_id: root_session_id.clone(),
            kind: crate::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some("Before".to_owned()),
            state: crate::session::repository::SessionState::Ready,
        })
        .expect("seed root session");
        let mut owned_runtime = Some(std::sync::Arc::new(runtime));
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut turn_future: Pin<Box<dyn std::future::Future<Output = ()>>> =
            Box::pin(std::future::pending());
        let mut turn_active = false;

        process_submitted_chat_text(
            &mut owned_runtime,
            &mut shell,
            "/rename Session Prime",
            &tx,
            &mut turn_future,
            &mut turn_active,
        )
        .expect("process rename command");

        let stored_session = repo
            .load_session(root_session_id.as_str())
            .expect("load session")
            .expect("session exists");

        assert_eq!(stored_session.label.as_deref(), Some("Session Prime"));
        assert!(
            latest_message_text(&shell).contains("Session Prime"),
            "rename command should append the updated label"
        );
        assert!(!turn_active);
    }

    #[test]
    fn selecting_resume_palette_candidate_fills_composer() {
        let mut shell = state::Shell::new("sess-resume");
        let mut textarea = tui_textarea::TextArea::default();
        textarea.insert_str("/resume child");
        shell
            .pane
            .composer_suggestion_context
            .visible_session_suggestions = vec![state::VisibleSessionSuggestion {
            session_id: "child-session".to_owned(),
            label: Some("Child work".to_owned()),
            agent_presentation: None,
            state: "completed".to_owned(),
            kind: "delegate_child".to_owned(),
            task_phase: Some("completed".to_owned()),
            overdue: false,
            pending_approval_count: 0,
            attention_approval_count: 0,
        }];

        let applied = apply_selected_slash_command(&mut shell, &mut textarea);

        assert_eq!(applied.as_deref(), Some("/resume child-session"));
        assert_eq!(textarea.lines().join("\n"), "");
        assert!(shell.pane.messages.is_empty());
    }

    #[test]
    fn compact_command_is_rejected_while_agent_is_running() {
        let mut shell = state::Shell::new("sess-compact");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        shell.pane.agent_running = true;
        textarea.insert_str("/compact");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let rendered_system_message = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");

        assert!(submit_text.is_none());
        assert!(rendered_system_message.contains("already in progress"));
    }

    #[test]
    fn approvals_resolve_action_is_async() {
        let request = ParsedSlashCommand {
            command: SlashCommand::Approvals,
            args: "resolve apr_123 approve-once auto".to_owned(),
        };

        assert!(is_async_slash_request(&request));
    }

    #[test]
    fn approvals_resolve_palette_suggests_request_ids() {
        let mut shell = state::Shell::new("sess-approvals");
        shell
            .pane
            .composer_suggestion_context
            .approval_request_suggestions = vec![state::ApprovalRequestSuggestion {
            approval_request_id: "apr_123".to_owned(),
            tool_name: "delegate_async".to_owned(),
            status: "pending".to_owned(),
            session_id: "root-session".to_owned(),
            needs_attention: true,
        }];

        let entries = slash_palette_entries(&shell, "/approvals resolve");

        assert!(
            entries
                .iter()
                .any(|entry| entry.replacement.contains("apr_123")),
            "resolve palette should suggest approval ids: {entries:?}"
        );
    }

    #[test]
    fn model_palette_suggests_model_candidates() {
        let mut shell = state::Shell::new("sess-model");
        shell
            .pane
            .composer_suggestion_context
            .model_selection_suggestions = vec![state::ModelSelectionSuggestion {
            selector: "openai-reasoning".to_owned(),
            profile_id: "openai-reasoning".to_owned(),
            kind: "openai".to_owned(),
            model: "o4-mini".to_owned(),
            active: true,
            reasoning_efforts: vec!["none".to_owned(), "low".to_owned(), "medium".to_owned()],
            current_reasoning_effort: Some("medium".to_owned()),
        }];

        let entries = slash_palette_entries(&shell, "/model openai");

        assert!(
            entries
                .iter()
                .any(|entry| entry.replacement == "/model openai-reasoning"),
            "model palette should surface configured model selectors: {entries:?}"
        );
    }

    #[test]
    fn model_palette_suggests_reasoning_efforts_for_selected_model() {
        let mut shell = state::Shell::new("sess-model");
        shell
            .pane
            .composer_suggestion_context
            .model_selection_suggestions = vec![state::ModelSelectionSuggestion {
            selector: "openai-reasoning".to_owned(),
            profile_id: "openai-reasoning".to_owned(),
            kind: "openai".to_owned(),
            model: "o4-mini".to_owned(),
            active: true,
            reasoning_efforts: vec!["none".to_owned(), "low".to_owned(), "medium".to_owned()],
            current_reasoning_effort: Some("medium".to_owned()),
        }];

        let entries = slash_palette_entries(&shell, "/model openai-reasoning");

        assert!(
            entries
                .iter()
                .any(|entry| entry.replacement == "/model openai-reasoning medium"),
            "model palette should surface reasoning efforts after selecting a reasoning-capable model: {entries:?}"
        );
    }

    #[test]
    fn enter_with_model_submits_runtime_switch_request() {
        let mut shell = state::Shell::new("sess-model");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(plain_key(KeyCode::Enter));

        textarea.insert_str("/model openai-reasoning");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let status = shell
            .pane
            .status_message
            .as_ref()
            .map(|(message, _)| message.as_str());

        assert_eq!(submit_text.as_deref(), Some("/model openai-reasoning"));
        assert_eq!(status, Some("Switching model..."));
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn switch_model_selection_updates_runtime_and_reasoning_effort() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("loongclaw.toml");
        let mut config = crate::config::LoongClawConfig::default();
        let mut openai_main =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai_main.model = "gpt-5".to_owned();
        config.set_active_provider_profile(
            "openai-main",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai_main.clone(),
            },
        );
        let mut openai_reasoning =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai_reasoning.model = "o4-mini".to_owned();
        config.providers.insert(
            "openai-reasoning".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: false,
                provider: openai_reasoning,
            },
        );
        config.provider = openai_main;
        config.active_provider = Some("openai-main".to_owned());
        std::fs::write(
            &config_path,
            crate::config::render(&config).expect("render config"),
        )
        .expect("write config");

        let runtime = runtime::TuiRuntime {
            resolved_path: config_path.clone(),
            config: config.clone(),
            session_id: scoped_test_id(temp_dir.path(), "ops-root"),
            session_address: crate::conversation::ConversationSessionAddress::from_session_id(
                scoped_test_id(temp_dir.path(), "ops-root"),
            ),
            turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
            kernel_ctx: crate::context::bootstrap_kernel_context_with_config(
                "tui-model-switch-test",
                crate::context::DEFAULT_TOKEN_TTL_S,
                &config,
            )
            .expect("bootstrap kernel context"),
            model_label: config
                .provider
                .resolved_model()
                .unwrap_or_else(|| "auto".to_owned()),
        };

        let mut shell = state::Shell::new(scoped_test_id(temp_dir.path(), "ops-root").as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        let mut owned_runtime = Some(std::sync::Arc::new(runtime));
        switch_model_selection(
            &mut owned_runtime,
            &mut shell,
            "openai-reasoning",
            Some(ModelReasoningChoice::Explicit(
                crate::config::ReasoningEffort::Low,
            )),
        )
        .expect("switch model");

        let reloaded =
            crate::config::load(Some(config_path.to_str().expect("utf8 path"))).expect("reload");
        let reloaded = reloaded.1;

        assert_eq!(reloaded.active_provider_id(), Some("openai-reasoning"));
        assert_eq!(
            reloaded.provider.reasoning_effort,
            Some(crate::config::ReasoningEffort::Low)
        );
        assert_eq!(shell.pane.model, "o4-mini");
        assert_eq!(
            owned_runtime
                .as_ref()
                .map(|runtime| runtime.config.provider.reasoning_effort),
            Some(Some(crate::config::ReasoningEffort::Low))
        );
        assert!(
            latest_message_text(&shell).contains("openai-reasoning"),
            "switch should append model status surface"
        );
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn switch_model_selection_accepts_model_selector_and_auto_clears_reasoning_effort() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("loongclaw.toml");
        let mut config = crate::config::LoongClawConfig::default();
        let mut openai_main =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai_main.model = "gpt-5".to_owned();
        config.set_active_provider_profile(
            "openai-main",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai_main.clone(),
            },
        );
        let mut openai_reasoning =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai_reasoning.model = "o4-mini".to_owned();
        openai_reasoning.reasoning_effort = Some(crate::config::ReasoningEffort::High);
        config.providers.insert(
            "openai-reasoning".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: false,
                provider: openai_reasoning,
            },
        );
        config.provider = openai_main;
        config.active_provider = Some("openai-main".to_owned());
        std::fs::write(
            &config_path,
            crate::config::render(&config).expect("render config"),
        )
        .expect("write config");

        let runtime = runtime::TuiRuntime {
            resolved_path: config_path.clone(),
            config: config.clone(),
            session_id: scoped_test_id(temp_dir.path(), "ops-root"),
            session_address: crate::conversation::ConversationSessionAddress::from_session_id(
                scoped_test_id(temp_dir.path(), "ops-root"),
            ),
            turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
            kernel_ctx: crate::context::bootstrap_kernel_context_with_config(
                "tui-model-switch-auto-test",
                crate::context::DEFAULT_TOKEN_TTL_S,
                &config,
            )
            .expect("bootstrap kernel context"),
            model_label: config
                .provider
                .resolved_model()
                .unwrap_or_else(|| "auto".to_owned()),
        };

        let mut shell = state::Shell::new(scoped_test_id(temp_dir.path(), "ops-root").as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        let mut owned_runtime = Some(std::sync::Arc::new(runtime));
        switch_model_selection(
            &mut owned_runtime,
            &mut shell,
            "o4-mini",
            Some(ModelReasoningChoice::Auto),
        )
        .expect("switch model");

        let reloaded =
            crate::config::load(Some(config_path.to_str().expect("utf8 path"))).expect("reload");
        let reloaded = reloaded.1;

        assert_eq!(reloaded.active_provider_id(), Some("openai-reasoning"));
        assert_eq!(reloaded.provider.reasoning_effort, None);
        assert_eq!(
            reloaded
                .providers
                .get("openai-reasoning")
                .and_then(|profile| profile.provider.reasoning_effort),
            None
        );
        assert!(
            latest_message_text(&shell).contains("resolved profile: openai-reasoning"),
            "switch should surface the resolved profile when the selector is the model alias"
        );
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn switch_model_selection_rejects_reasoning_effort_for_unsupported_provider() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("loongclaw.toml");
        let mut config = crate::config::LoongClawConfig::default();
        let mut openai_main =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai_main.model = "gpt-5".to_owned();
        config.set_active_provider_profile(
            "openai-main",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai_main.clone(),
            },
        );
        let mut anthropic =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Anthropic);
        anthropic.model = "claude-sonnet-4-20250514".to_owned();
        config.providers.insert(
            "anthropic-main".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: anthropic,
            },
        );
        config.provider = openai_main;
        config.active_provider = Some("openai-main".to_owned());
        std::fs::write(
            &config_path,
            crate::config::render(&config).expect("render config"),
        )
        .expect("write config");

        let runtime = runtime::TuiRuntime {
            resolved_path: config_path.clone(),
            config: config.clone(),
            session_id: scoped_test_id(temp_dir.path(), "ops-root"),
            session_address: crate::conversation::ConversationSessionAddress::from_session_id(
                scoped_test_id(temp_dir.path(), "ops-root"),
            ),
            turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
            kernel_ctx: crate::context::bootstrap_kernel_context_with_config(
                "tui-model-switch-unsupported-test",
                crate::context::DEFAULT_TOKEN_TTL_S,
                &config,
            )
            .expect("bootstrap kernel context"),
            model_label: config
                .provider
                .resolved_model()
                .unwrap_or_else(|| "auto".to_owned()),
        };

        let mut shell = state::Shell::new(scoped_test_id(temp_dir.path(), "ops-root").as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        let mut owned_runtime = Some(std::sync::Arc::new(runtime));
        let error = switch_model_selection(
            &mut owned_runtime,
            &mut shell,
            "anthropic",
            Some(ModelReasoningChoice::Explicit(
                crate::config::ReasoningEffort::Low,
            )),
        )
        .expect_err("unsupported providers should reject explicit reasoning efforts");

        let reloaded =
            crate::config::load(Some(config_path.to_str().expect("utf8 path"))).expect("reload");
        let reloaded = reloaded.1;

        assert!(
            error.contains("does not support reasoning effort overrides"),
            "unexpected error: {error}"
        );
        assert_eq!(reloaded.active_provider_id(), Some("openai-main"));
        assert_eq!(
            owned_runtime
                .as_ref()
                .and_then(|runtime| runtime.config.active_provider_id().map(str::to_owned))
                .as_deref(),
            Some("openai-main")
        );
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn stats_command_opens_overlay() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let repo = session_repo_for_config(&config);
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        ensure_root_session(&repo, root_session_id.as_str());
        let usage_event = crate::memory::build_conversation_event_content(
            crate::conversation::analytics::TURN_USAGE_EVENT_NAME,
            json!({
                "model": "gpt-5",
                "input_tokens": 120,
                "output_tokens": 80,
            }),
        );
        crate::memory::replace_session_turns_direct(
            root_session_id.as_str(),
            &[crate::memory::WindowTurn {
                role: "assistant".to_owned(),
                content: usage_event,
                ts: Some(chrono::Utc::now().timestamp()),
            }],
            &memory_config,
        )
        .expect("seed usage event");

        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Stats,
                args: "models 30d".to_owned(),
            },
        );

        assert_eq!(shell.focus.top(), FocusLayer::StatsOverlay);
        assert!(shell.stats_overlay.is_some());
        assert_eq!(
            shell.stats_overlay.as_ref().map(|state| state.active_tab),
            Some(stats::StatsTab::Models)
        );
        assert_eq!(
            shell.stats_overlay.as_ref().map(|state| state.date_range),
            Some(stats::StatsDateRange::Last30Days)
        );
    }

    #[test]
    fn stats_overlay_keyboard_cycles_tab_and_range() {
        let mut shell = state::Shell::new("sess-stats");
        shell.stats_overlay = Some(state::StatsOverlayState::new(
            sample_stats_snapshot_for_test(),
            stats::StatsTab::Overview,
            stats::StatsDateRange::All,
        ));
        shell.focus.push(FocusLayer::StatsOverlay);
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Tab)),
            &tx,
            &mut submit_text,
        );
        assert_eq!(
            shell.stats_overlay.as_ref().map(|state| state.active_tab),
            Some(stats::StatsTab::Models)
        );

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Char('r'))),
            &tx,
            &mut submit_text,
        );
        assert_eq!(
            shell.stats_overlay.as_ref().map(|state| state.date_range),
            Some(stats::StatsDateRange::Last7Days)
        );

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Esc)),
            &tx,
            &mut submit_text,
        );
        assert!(shell.stats_overlay.is_none());
        assert_eq!(shell.focus.top(), FocusLayer::Composer);
    }

    #[test]
    fn stats_overlay_sessions_tab_scrolls_with_down_key() {
        let mut shell = state::Shell::new("sess-stats-scroll");
        let mut snapshot = sample_stats_snapshot_for_test();
        for index in 0..12 {
            let session_row = stats::StatsSessionRow {
                session_id: format!("sess-{index}"),
                label: Some(format!("Session {index}")),
                agent_presentation: None,
                kind: "delegate_child".to_owned(),
                state: "completed".to_owned(),
                turn_count: index + 1,
                duration_seconds: 60,
                last_activity_date: snapshot.last_activity_date,
                current: false,
            };
            snapshot.session_rows.push(session_row);
        }
        shell.stats_overlay = Some(state::StatsOverlayState::new(
            snapshot,
            stats::StatsTab::Sessions,
            stats::StatsDateRange::All,
        ));
        shell.focus.push(FocusLayer::StatsOverlay);
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Down)),
            &tx,
            &mut submit_text,
        );

        assert_eq!(
            shell
                .stats_overlay
                .as_ref()
                .map(|stats_overlay| stats_overlay.list_scroll_offset),
            Some(1)
        );
    }

    #[test]
    fn mode_palette_suggests_busy_input_modes() {
        let shell = state::Shell::new("sess-mode");
        let entries = slash_palette_entries(&shell, "/mode");

        assert!(
            entries
                .iter()
                .any(|entry| entry.replacement == "/mode queue"),
            "mode palette should include queue: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.replacement == "/mode steer"),
            "mode palette should include steer: {entries:?}"
        );
    }

    #[test]
    fn ctrl_g_cycles_busy_input_mode() {
        let mut shell = state::Shell::new("sess-mode");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(modified_key(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            &tx,
            &mut submit_text,
        );

        assert_eq!(shell.pane.busy_input_mode(), state::BusyInputMode::Steer);
    }

    #[test]
    fn ctrl_c_while_running_requests_interrupt_without_exiting() {
        let mut shell = state::Shell::new("sess-interrupt");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        shell.pane.agent_running = true;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            &tx,
            &mut submit_text,
        );

        assert!(shell.running);
        assert!(shell.interrupt_requested);
        assert_eq!(
            shell
                .pane
                .status_message
                .as_ref()
                .map(|(message, _)| message.as_str()),
            Some("Interrupting response...")
        );
    }

    #[test]
    fn ctrl_c_when_idle_clears_pending_submissions_before_exiting() {
        let mut shell = state::Shell::new("sess-cancel-pending");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        shell.pane.enqueue_busy_submission("queued".to_owned());
        shell.pane.enqueue_busy_submission("next".to_owned());

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            &tx,
            &mut submit_text,
        );

        assert!(shell.running);
        assert!(!shell.interrupt_requested);
        assert!(!shell.pane.has_pending_submissions());
        assert_eq!(
            shell
                .pane
                .status_message
                .as_ref()
                .map(|(message, _)| message.as_str()),
            Some("Cleared 2 pending submission(s)")
        );
    }

    #[test]
    fn ctrl_c_when_idle_and_no_pending_exits_shell() {
        let mut shell = state::Shell::new("sess-exit");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(modified_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            &tx,
            &mut submit_text,
        );

        assert!(!shell.running);
        assert!(!shell.interrupt_requested);
    }

    #[test]
    fn mode_clear_command_drops_pending_submissions() {
        let mut shell = state::Shell::new("sess-mode-clear");
        shell.pane.set_busy_input_mode(state::BusyInputMode::Queue);
        shell.pane.enqueue_busy_submission("queued".to_owned());
        shell.pane.set_busy_input_mode(state::BusyInputMode::Steer);
        shell.pane.enqueue_busy_submission("steer".to_owned());

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Mode,
                args: "clear".to_owned(),
            },
        );

        assert!(!shell.pane.has_pending_submissions());
    }

    #[test]
    fn enter_while_running_in_queue_mode_appends_pending_queue() {
        let mut shell = state::Shell::new("sess-queue");
        shell.pane.agent_running = true;
        shell.pane.set_busy_input_mode(state::BusyInputMode::Queue);
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        textarea.insert_str("first queued");
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Enter)),
            &tx,
            &mut submit_text,
        );

        textarea.insert_str("second queued");
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Enter)),
            &tx,
            &mut submit_text,
        );

        assert_eq!(shell.pane.pending_submission_count(), 2);
        assert_eq!(shell.pane.queued_messages.len(), 2);
        assert!(shell.pane.steer_message.is_none());
        assert!(submit_text.is_none());
    }

    #[test]
    fn enter_while_running_in_steer_mode_replaces_pending_steer() {
        let mut shell = state::Shell::new("sess-steer");
        shell.pane.agent_running = true;
        shell.pane.set_busy_input_mode(state::BusyInputMode::Steer);
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        textarea.insert_str("first steer");
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Enter)),
            &tx,
            &mut submit_text,
        );

        textarea.insert_str("second steer");
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Enter)),
            &tx,
            &mut submit_text,
        );

        assert_eq!(shell.pane.pending_submission_count(), 1);
        assert_eq!(shell.pane.queued_messages.len(), 0);
        assert_eq!(
            shell
                .pane
                .steer_message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("second steer")
        );
        assert!(submit_text.is_none());
    }

    #[test]
    fn steer_boundary_interrupts_after_tool_completion() {
        let mut shell = state::Shell::new("sess-steer-boundary");
        shell.pane.agent_running = true;
        shell.pane.set_busy_input_mode(state::BusyInputMode::Steer);
        shell.pane.enqueue_busy_submission("follow-up".to_owned());
        shell.pane.start_tool_call("tool-1", "shell.exec", "ls");
        shell.pane.complete_tool_call("tool-1", true, "ok", 10);

        let mut turn_future: Pin<Box<dyn std::future::Future<Output = ()>>> =
            Box::pin(std::future::pending());
        let mut turn_active = true;
        let mut submit_text: Option<String> = None;

        interrupt_for_steer_boundary(
            &mut shell,
            &mut turn_future,
            &mut turn_active,
            &mut submit_text,
        );

        assert!(!turn_active);
        assert!(!shell.pane.agent_running);
        assert_eq!(submit_text.as_deref(), Some("follow-up"));
    }

    #[test]
    fn ctrl_c_interrupt_request_stops_active_turn_and_keeps_pending_submissions() {
        let mut shell = state::Shell::new("sess-interrupt-active");
        shell.pane.agent_running = true;
        shell.pane.enqueue_busy_submission("follow-up".to_owned());
        shell.interrupt_requested = true;

        let mut turn_future: Pin<Box<dyn std::future::Future<Output = ()>>> =
            Box::pin(std::future::pending());
        let mut turn_active = true;

        apply_interrupt_request(&mut shell, &mut turn_future, &mut turn_active);

        assert!(!turn_active);
        assert!(!shell.pane.agent_running);
        assert!(!shell.interrupt_requested);
        assert!(shell.pane.has_pending_submissions());
        assert_eq!(
            shell
                .pane
                .status_message
                .as_ref()
                .map(|(message, _)| message.as_str()),
            Some("Response interrupted; 1 pending submission(s) kept")
        );
        assert!(latest_message_text(&shell).contains("Interrupted the in-flight turn."));
    }

    #[test]
    fn tasks_palette_suggests_delegate_sessions() {
        let mut shell = state::Shell::new("sess-tasks");
        shell
            .pane
            .composer_suggestion_context
            .visible_session_suggestions = vec![state::VisibleSessionSuggestion {
            session_id: "delegate:task-7".to_owned(),
            label: Some("Background review".to_owned()),
            agent_presentation: None,
            state: "running".to_owned(),
            kind: "delegate_child".to_owned(),
            task_phase: Some("running".to_owned()),
            overdue: false,
            pending_approval_count: 0,
            attention_approval_count: 0,
        }];

        let entries = slash_palette_entries(&shell, "/tasks run");

        assert!(
            entries
                .iter()
                .any(|entry| entry.replacement.contains("delegate:task-7")),
            "tasks palette should surface delegate sessions: {entries:?}"
        );
    }

    #[test]
    fn permissions_palette_suggests_visible_sessions() {
        let mut shell = state::Shell::new("sess-permissions");
        shell
            .pane
            .composer_suggestion_context
            .visible_session_suggestions = vec![state::VisibleSessionSuggestion {
            session_id: "child-session".to_owned(),
            label: Some("Child work".to_owned()),
            agent_presentation: None,
            state: "completed".to_owned(),
            kind: "delegate_child".to_owned(),
            task_phase: Some("completed".to_owned()),
            overdue: false,
            pending_approval_count: 0,
            attention_approval_count: 0,
        }];

        let entries = slash_palette_entries(&shell, "/permissions child");

        assert!(
            entries
                .iter()
                .any(|entry| entry.replacement == "/permissions child-session"),
            "permissions palette should surface visible sessions: {entries:?}"
        );
    }

    #[test]
    fn subagents_palette_matches_root_aliases_and_humanizes_meta() {
        let mut shell = state::Shell::new("sess-subagents");
        shell
            .pane
            .composer_suggestion_context
            .visible_session_suggestions = vec![state::VisibleSessionSuggestion {
            session_id: "root-session".to_owned(),
            label: Some("Main thread".to_owned()),
            agent_presentation: None,
            state: "running".to_owned(),
            kind: "root".to_owned(),
            task_phase: None,
            overdue: false,
            pending_approval_count: 0,
            attention_approval_count: 0,
        }];

        let entries = slash_palette_entries(&shell, "/subagents root");
        let root_entry = entries
            .iter()
            .find(|entry| entry.replacement == "/subagents root-session")
            .expect("root entry");

        assert_eq!(root_entry.meta, "Primary");
        assert_eq!(root_entry.label, "Main thread");
        assert!(
            root_entry.detail.contains("thread"),
            "root entry should use human thread wording: {root_entry:?}"
        );
    }

    #[test]
    fn tab_cycles_slash_command_palette_selection() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let tab_event = Event::Key(plain_key(KeyCode::Tab));

        textarea.insert_str("/t");

        apply_terminal_event(&mut shell, &mut textarea, tab_event, &tx, &mut submit_text);

        assert_eq!(shell.slash_command_selection, 1);
    }

    #[test]
    fn mouse_click_on_slash_palette_executes_command() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        textarea.insert_str("/rev");

        let palette_area = slash_command_palette_area(&shell, &textarea).expect("palette area");
        let click_row = palette_area.y.saturating_add(1);
        let click_col = palette_area.x.saturating_add(2);
        let click_event = Event::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            click_col,
            click_row,
        ));

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            click_event,
            &tx,
            &mut submit_text,
        );

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
    }

    #[test]
    fn mouse_click_on_tool_line_opens_matching_tool_inspector() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        shell
            .pane
            .start_tool_call("tool-1", "shell.exec", "git status --short");
        shell
            .pane
            .complete_tool_call("tool-1", true, "diff --git a/file b/file", 12);

        let shell_areas = terminal_shell_areas(&textarea);
        let click_row = shell_areas.history.y.saturating_add(1);
        let click_col = shell_areas.history.x.saturating_add(1);
        let click_event = Event::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            click_col,
            click_row,
        ));

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            click_event,
            &tx,
            &mut submit_text,
        );

        let selected_tool_id = shell
            .pane
            .tool_inspector
            .as_ref()
            .map(|tool_inspector| tool_inspector.selected_tool_id.as_str());

        assert_eq!(shell.focus.top(), FocusLayer::ToolInspector);
        assert_eq!(selected_tool_id, Some("tool-1"));
    }

    #[test]
    fn status_command_appends_runtime_surface_message() {
        let mut shell = state::Shell::new("sess-7");
        shell.pane.model = "gpt-5".to_owned();
        shell.pane.input_tokens = 120;
        shell.pane.output_tokens = 80;
        shell.pane.context_length = 1000;

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Status,
                args: String::new(),
            },
        );

        let rendered_surface = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");

        assert!(rendered_surface.contains("runtime status"));
        assert!(rendered_surface.contains("- session: sess-7"));
        assert!(rendered_surface.contains("- model: gpt-5"));
        assert!(rendered_surface.contains("- tokens: 200"));
    }

    #[test]
    fn surface_ui_event_appends_surface_message_and_status() {
        let mut shell = state::Shell::new("sess-surface");

        apply_ui_event(
            &mut shell,
            UiEvent::Surface {
                title: "context compaction".to_owned(),
                lines: vec![
                    "- session: sess-surface".to_owned(),
                    "- result: context compaction completed".to_owned(),
                ],
            },
        );

        let rendered_surface = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");
        let status = shell
            .pane
            .status_message
            .as_ref()
            .map(|(message, _)| message.as_str());

        assert!(rendered_surface.contains("context compaction"));
        assert!(rendered_surface.contains("- session: sess-surface"));
        assert_eq!(status, Some("context compaction added to transcript"));
    }

    #[test]
    fn composer_placeholder_prefers_compact_when_context_is_hot() {
        let mut pane = state::Pane::new("sess-hot");
        pane.context_length = 100;
        pane.input_tokens = 45;
        pane.output_tokens = 35;

        let placeholder = composer_placeholder(&pane).expect("placeholder");

        assert!(
            placeholder.contains("/compact"),
            "high context usage should steer toward compaction: {placeholder}"
        );
    }

    #[test]
    fn composer_placeholder_prefers_attention_approvals_over_generic_prompts() {
        let mut pane = state::Pane::new("sess-approvals");
        pane.composer_suggestion_context.attention_approvals = Some(2);
        pane.composer_suggestion_context.pending_approvals = Some(3);
        pane.messages
            .push(crate::chat::tui::message::Message::system("already active"));

        let placeholder = composer_placeholder(&pane).expect("placeholder");

        assert!(
            placeholder.contains("/approvals attention"),
            "attention-heavy approvals should drive the prompt: {placeholder}"
        );
    }

    #[test]
    fn composer_placeholder_prefers_overdue_tasks_before_dirty_worktree() {
        let mut pane = state::Pane::new("sess-tasks");
        pane.composer_suggestion_context.overdue_tasks = Some(1);
        pane.composer_suggestion_context.worktree_dirty = Some(true);
        pane.messages
            .push(crate::chat::tui::message::Message::system("already active"));

        let placeholder = composer_placeholder(&pane).expect("placeholder");

        assert!(
            placeholder.contains("/subagents"),
            "overdue delegate work should outrank generic dirty-worktree advice: {placeholder}"
        );
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn resume_command_lists_visible_sessions() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        let delegate_session_id = scoped_test_id(temp_dir.path(), "delegate-task-1");
        ensure_root_session(&repo, root_session_id.as_str());
        repo.create_session_with_event(crate::session::repository::CreateSessionWithEventRequest {
            session: crate::session::repository::NewSessionRecord {
                session_id: delegate_session_id.clone(),
                kind: crate::session::repository::SessionKind::DelegateChild,
                parent_session_id: Some(root_session_id.clone()),
                label: Some("Research PR".to_owned()),
                state: crate::session::repository::SessionState::Ready,
            },
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some(root_session_id.clone()),
            event_payload_json: json!({ "task": "Research PR" }),
        })
        .expect("create delegate child");

        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Resume,
                args: String::new(),
            },
        );

        let session_picker = shell.session_picker.as_ref().expect("session picker");

        assert_eq!(shell.focus.top(), FocusLayer::SessionPicker);
        assert!(
            session_picker
                .sessions
                .iter()
                .any(|session| session.session_id == root_session_id)
        );
        assert!(
            session_picker
                .sessions
                .iter()
                .any(|session| session.session_id == delegate_session_id)
        );
    }

    #[test]
    fn session_picker_enter_submits_selected_session_switch() {
        let mut shell = state::Shell::new("root-session");
        shell.session_picker = Some(state::SessionPickerState::new(
            state::SessionPickerMode::Resume,
            vec![
                state::VisibleSessionSuggestion {
                    session_id: "root-session".to_owned(),
                    label: Some("Root".to_owned()),
                    agent_presentation: None,
                    state: "ready".to_owned(),
                    kind: "root".to_owned(),
                    task_phase: None,
                    overdue: false,
                    pending_approval_count: 0,
                    attention_approval_count: 0,
                },
                state::VisibleSessionSuggestion {
                    session_id: "child-session".to_owned(),
                    label: Some("Child".to_owned()),
                    agent_presentation: None,
                    state: "completed".to_owned(),
                    kind: "delegate_child".to_owned(),
                    task_phase: Some("completed".to_owned()),
                    overdue: false,
                    pending_approval_count: 0,
                    attention_approval_count: 0,
                },
            ],
        ));
        if let Some(session_picker) = shell.session_picker.as_mut() {
            session_picker.selected_index = 1;
        }
        shell.focus.push(FocusLayer::SessionPicker);
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Enter)),
            &tx,
            &mut submit_text,
        );

        assert_eq!(submit_text.as_deref(), Some("/resume child-session"));
        assert!(shell.session_picker.is_none());
        assert_eq!(shell.focus.top(), FocusLayer::Composer);
    }

    #[test]
    fn session_picker_search_filters_visible_sessions() {
        let mut shell = state::Shell::new("root-session");
        shell.session_picker = Some(state::SessionPickerState::new(
            state::SessionPickerMode::Resume,
            vec![
                state::VisibleSessionSuggestion {
                    session_id: "alpha-session".to_owned(),
                    label: Some("Alpha".to_owned()),
                    agent_presentation: None,
                    state: "ready".to_owned(),
                    kind: "root".to_owned(),
                    task_phase: None,
                    overdue: false,
                    pending_approval_count: 0,
                    attention_approval_count: 0,
                },
                state::VisibleSessionSuggestion {
                    session_id: "beta-session".to_owned(),
                    label: Some("Beta".to_owned()),
                    agent_presentation: None,
                    state: "completed".to_owned(),
                    kind: "delegate_child".to_owned(),
                    task_phase: Some("completed".to_owned()),
                    overdue: false,
                    pending_approval_count: 0,
                    attention_approval_count: 0,
                },
            ],
        ));
        shell.focus.push(FocusLayer::SessionPicker);
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            Event::Key(plain_key(KeyCode::Char('b'))),
            &tx,
            &mut submit_text,
        );

        let session_picker = shell.session_picker.as_ref().expect("session picker");
        let filtered_indices = session_picker.filtered_indices();
        let filtered_session_id = filtered_indices
            .first()
            .and_then(|index| session_picker.sessions.get(*index))
            .map(|session| session.session_id.as_str());

        assert_eq!(session_picker.query, "b");
        assert_eq!(filtered_session_id, Some("beta-session"));
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn subagents_command_opens_session_picker_with_main_and_delegate_threads() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let root_session_id = scoped_test_id(temp_dir.path(), "subagent-root");
        let delegate_session_id = scoped_test_id(temp_dir.path(), "subagent-child");
        ensure_root_session(&repo, root_session_id.as_str());
        repo.create_session_with_event(crate::session::repository::CreateSessionWithEventRequest {
            session: crate::session::repository::NewSessionRecord {
                session_id: delegate_session_id.clone(),
                kind: crate::session::repository::SessionKind::DelegateChild,
                parent_session_id: Some(root_session_id.clone()),
                label: Some("Research branch".to_owned()),
                state: crate::session::repository::SessionState::Running,
            },
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some(root_session_id.clone()),
            event_payload_json: json!({
                "task": "research external references",
                "provider": {
                    "profile_id": "openai-reasoning",
                    "provider_kind": "openai",
                    "model": "gpt-5",
                    "reasoning_effort": "high"
                }
            }),
        })
        .expect("create delegate child");

        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Subagents,
                args: String::new(),
            },
        );

        let session_picker = shell.session_picker.as_ref().expect("session picker");

        assert_eq!(shell.focus.top(), FocusLayer::SessionPicker);
        assert_eq!(session_picker.mode, state::SessionPickerMode::Subagents);
        assert_eq!(
            session_picker
                .sessions
                .first()
                .map(|session| session.session_id.as_str()),
            Some(root_session_id.as_str())
        );
        assert!(
            session_picker
                .sessions
                .iter()
                .any(|session| session.session_id == delegate_session_id)
        );
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn switch_subagent_session_allows_switching_between_sibling_delegate_threads() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let root_session_id = scoped_test_id(temp_dir.path(), "subagent-root");
        let left_session_id = scoped_test_id(temp_dir.path(), "subagent-left");
        let right_session_id = scoped_test_id(temp_dir.path(), "subagent-right");

        ensure_root_session(&repo, root_session_id.as_str());
        for session_id in [left_session_id.as_str(), right_session_id.as_str()] {
            repo.create_session_with_event(
                crate::session::repository::CreateSessionWithEventRequest {
                    session: crate::session::repository::NewSessionRecord {
                        session_id: session_id.to_owned(),
                        kind: crate::session::repository::SessionKind::DelegateChild,
                        parent_session_id: Some(root_session_id.clone()),
                        label: Some(format!("Task {session_id}")),
                        state: crate::session::repository::SessionState::Completed,
                    },
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some(root_session_id.clone()),
                    event_payload_json: json!({ "task": session_id }),
                },
            )
            .expect("create delegate child");
        }

        append_turn_direct(
            left_session_id.as_str(),
            "user",
            "left user",
            &memory_config,
        )
        .expect("append left user");
        append_turn_direct(
            left_session_id.as_str(),
            "assistant",
            "left assistant",
            &memory_config,
        )
        .expect("append left assistant");
        append_turn_direct(
            right_session_id.as_str(),
            "user",
            "right user",
            &memory_config,
        )
        .expect("append right user");
        append_turn_direct(
            right_session_id.as_str(),
            "assistant",
            "right assistant",
            &memory_config,
        )
        .expect("append right assistant");

        let runtime = runtime::TuiRuntime {
            resolved_path: temp_dir.path().join("loongclaw.toml"),
            config: config.clone(),
            session_id: left_session_id.clone(),
            session_address: crate::conversation::ConversationSessionAddress::from_session_id(
                left_session_id.clone(),
            ),
            turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
            kernel_ctx: crate::context::bootstrap_kernel_context_with_config(
                "tui-subagent-switch-test",
                crate::context::DEFAULT_TOKEN_TTL_S,
                &config,
            )
            .expect("bootstrap kernel context"),
            model_label: config
                .provider
                .resolved_model()
                .unwrap_or_else(|| "auto".to_owned()),
        };
        let mut shell = state::Shell::new(left_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        let mut owned_runtime = Some(std::sync::Arc::new(runtime));

        switch_subagent_session(&mut owned_runtime, &mut shell, right_session_id.as_str())
            .expect("switch sibling subagent");

        assert_eq!(shell.pane.session_id, right_session_id);
        assert!(
            transcript_plain_lines(&shell)
                .join("\n")
                .contains("right assistant"),
            "expected switched transcript to show right sibling history"
        );
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn subagent_lifecycle_surface_reports_waiting_and_finished_states() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let root_session_id = scoped_test_id(temp_dir.path(), "surface-root");
        let child_session_id = scoped_test_id(temp_dir.path(), "surface-child");

        ensure_root_session(&repo, root_session_id.as_str());
        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        refresh_composer_suggestion_context(&mut shell);
        reset_subagent_surface_tracking(&mut shell);

        repo.create_session_with_event(crate::session::repository::CreateSessionWithEventRequest {
            session: crate::session::repository::NewSessionRecord {
                session_id: child_session_id.clone(),
                kind: crate::session::repository::SessionKind::DelegateChild,
                parent_session_id: Some(root_session_id.clone()),
                label: Some("Research branch".to_owned()),
                state: crate::session::repository::SessionState::Running,
            },
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some(root_session_id.clone()),
            event_payload_json: json!({
                "task": "research external references",
                "provider": {
                    "profile_id": "openai-reasoning",
                    "provider_kind": "openai",
                    "model": "gpt-5",
                    "reasoning_effort": "high"
                }
            }),
        })
        .expect("create running child");

        refresh_composer_suggestion_context(&mut shell);
        maybe_emit_subagent_lifecycle_surface(&mut shell);

        let waiting_surface = latest_message_text(&shell);
        let waiting_transcript = transcript_plain_lines(&shell).join("\n");
        assert!(waiting_surface.contains("Waiting for 1 subagent"));
        assert!(waiting_transcript.contains("gpt-5 · high"));

        append_turn_direct(
            child_session_id.as_str(),
            "assistant",
            "Finished the research without edits.",
            &memory_config,
        )
        .expect("append assistant summary");
        repo.finalize_session_terminal(
            child_session_id.as_str(),
            crate::session::repository::FinalizeSessionTerminalRequest {
                state: crate::session::repository::SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some(root_session_id),
                event_payload_json: json!({ "result": "ok" }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({ "child_session_id": child_session_id }),
            },
        )
        .expect("finalize child");

        refresh_composer_suggestion_context(&mut shell);
        maybe_emit_subagent_lifecycle_surface(&mut shell);

        let finished_surface = latest_message_text(&shell);
        let transcript = transcript_plain_lines(&shell).join("\n");
        assert!(finished_surface.contains("Finished waiting"));
        assert!(transcript.contains("Completed"));
        assert!(transcript.contains("Finished the research without edits"));
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn switch_resumed_session_restores_target_history() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        let child_session_id = scoped_test_id(temp_dir.path(), "child-session");
        ensure_root_session(&repo, root_session_id.as_str());
        repo.create_session_with_event(crate::session::repository::CreateSessionWithEventRequest {
            session: crate::session::repository::NewSessionRecord {
                session_id: child_session_id.clone(),
                kind: crate::session::repository::SessionKind::DelegateChild,
                parent_session_id: Some(root_session_id.clone()),
                label: Some("Child work".to_owned()),
                state: crate::session::repository::SessionState::Completed,
            },
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some(root_session_id.clone()),
            event_payload_json: json!({ "task": "Child work" }),
        })
        .expect("create child session");
        crate::memory::append_turn_direct(
            child_session_id.as_str(),
            "user",
            "hello child",
            &memory_config,
        )
        .expect("append child user turn");
        crate::memory::append_turn_direct(
            child_session_id.as_str(),
            "assistant",
            "child reply",
            &memory_config,
        )
        .expect("append child assistant turn");

        let config_path = temp_dir.path().join("loongclaw.toml");
        fs::write(&config_path, "# shell switch test\n").expect("write config file");
        let runtime = runtime::TuiRuntime {
            resolved_path: config_path,
            config: config.clone(),
            session_id: root_session_id.clone(),
            session_address: crate::conversation::ConversationSessionAddress::from_session_id(
                root_session_id.clone(),
            ),
            turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
            kernel_ctx: crate::context::bootstrap_kernel_context_with_config(
                "tui-switch-test",
                crate::context::DEFAULT_TOKEN_TTL_S,
                &config,
            )
            .expect("bootstrap kernel context"),
            model_label: "auto".to_owned(),
        };

        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());
        let mut owned_runtime = Some(std::sync::Arc::new(runtime));

        switch_resumed_session(&mut owned_runtime, &mut shell, child_session_id.as_str())
            .expect("switch session");

        assert_eq!(shell.pane.session_id, child_session_id.as_str());
        assert_eq!(
            owned_runtime
                .as_ref()
                .map(|runtime| runtime.session_id.as_str()),
            Some(child_session_id.as_str())
        );
        assert!(
            shell
                .pane
                .messages
                .iter()
                .any(|message| matches!(message.role, Role::User)
                    && matches!(message.parts.first(), Some(MessagePart::Text(text)) if text == "hello child")),
            "resumed transcript should include child user turn"
        );
        assert!(
            shell
                .pane
                .messages
                .iter()
                .any(|message| matches!(message.role, Role::Assistant)
                    && matches!(message.parts.first(), Some(MessagePart::Text(text)) if text == "child reply")),
            "resumed transcript should include child assistant turn"
        );
        assert!(
            latest_message_text(&shell)
                .contains(format!("Resumed session `{}`", child_session_id).as_str()),
            "switch should append a session resume system message"
        );
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn tasks_command_lists_delegate_sessions() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        let delegate_session_id = scoped_test_id(temp_dir.path(), "delegate-task-2");
        ensure_root_session(&repo, root_session_id.as_str());
        repo.create_session_with_event(crate::session::repository::CreateSessionWithEventRequest {
            session: crate::session::repository::NewSessionRecord {
                session_id: delegate_session_id.clone(),
                kind: crate::session::repository::SessionKind::DelegateChild,
                parent_session_id: Some(root_session_id.clone()),
                label: Some("Long-running task".to_owned()),
                state: crate::session::repository::SessionState::Running,
            },
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some(root_session_id.clone()),
            event_payload_json: json!({ "task": "Long-running task" }),
        })
        .expect("create running delegate child");

        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Tasks,
                args: "running".to_owned(),
            },
        );

        let rendered_surface = latest_message_text(&shell);

        assert!(rendered_surface.contains("delegate tasks"));
        assert!(rendered_surface.contains(delegate_session_id.as_str()));
        assert!(rendered_surface.contains("task running"));
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn approvals_command_lists_pending_requests() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        let approval_request_id = scoped_test_id(temp_dir.path(), "apr-123");
        ensure_root_session(&repo, root_session_id.as_str());
        repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
            approval_request_id: approval_request_id.clone(),
            session_id: root_session_id.clone(),
            turn_id: "turn-1".to_owned(),
            tool_call_id: "tool-call-1".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate_async",
                "payload": { "task": "Inspect issue" }
            }),
            governance_snapshot_json: json!({
                "reason": "approval required for delegate_async",
                "rule_id": "delegate_review"
            }),
        })
        .expect("create approval request");

        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Approvals,
                args: String::new(),
            },
        );

        let rendered_surface = latest_message_text(&shell);

        assert!(rendered_surface.contains("approval requests"));
        assert!(rendered_surface.contains(approval_request_id.as_str()));
        assert!(rendered_surface.contains("delegate_async"));
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn permissions_command_reports_session_tool_policy() {
        let _guard = shell_memory_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_dir = tempdir().expect("tempdir");
        let config = shell_runtime_config_for_test(temp_dir.path());
        let repo = session_repo_for_config(&config);
        let root_session_id = scoped_test_id(temp_dir.path(), "ops-root");
        ensure_root_session(&repo, root_session_id.as_str());
        repo.upsert_session_tool_policy(crate::session::repository::NewSessionToolPolicyRecord {
            session_id: root_session_id.clone(),
            requested_tool_ids: vec!["session_events".to_owned(), "session_status".to_owned()],
            runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
        })
        .expect("set session tool policy");

        let mut shell = state::Shell::new(root_session_id.as_str());
        attach_shell_runtime_config(&mut shell, &config, temp_dir.path());

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Permissions,
                args: String::new(),
            },
        );

        let rendered_surface = latest_message_text(&shell);

        assert!(rendered_surface.contains("session permissions"));
        assert!(rendered_surface.contains(format!("- session: {root_session_id}").as_str()));
        assert!(rendered_surface.contains("session_events"));
        assert!(rendered_surface.contains("session_status"));
    }

    #[test]
    fn session_command_appends_selection_and_tool_summary() {
        let mut shell = state::Shell::new("sess-9");
        shell.pane.add_user_message("hello");
        shell
            .pane
            .start_tool_call("tool-1", "shell.exec", "git status --short");
        shell.pane.complete_tool_call("tool-1", true, "clean", 5);
        shell.focus.focus_transcript();
        shell.pane.transcript_review.cursor_line = 1;
        shell.pane.transcript_review.anchor_line = Some(0);

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Session,
                args: String::new(),
            },
        );

        let rendered_surface = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");

        assert!(rendered_surface.contains("session status"));
        assert!(rendered_surface.contains("- session: sess-9"));
        assert!(rendered_surface.contains("- focus: review"));
        assert!(rendered_surface.contains("- tool calls: 1"));
    }

    #[test]
    fn context_command_appends_context_surface_message() {
        let mut shell = state::Shell::new("sess-ctx");
        shell.pane.model = "gpt-5".to_owned();
        shell.pane.input_tokens = 300;
        shell.pane.output_tokens = 200;
        shell.pane.context_length = 2000;

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Context,
                args: String::new(),
            },
        );

        let rendered_surface = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");

        assert!(rendered_surface.contains("context status"));
        assert!(rendered_surface.contains("- input tokens: 300"));
        assert!(rendered_surface.contains("- output tokens: 200"));
        assert!(rendered_surface.contains("- total tokens: 500"));
        assert!(rendered_surface.contains("- context usage: 25.0%"));
    }

    #[test]
    fn tools_command_appends_tool_activity_surface() {
        let mut shell = state::Shell::new("sess-tools");
        shell
            .pane
            .start_tool_call("tool-1", "shell.exec", "git status --short");
        shell.pane.complete_tool_call("tool-1", true, "clean", 5);

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Tools,
                args: String::new(),
            },
        );

        let rendered_surface = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");

        assert!(rendered_surface.contains("tool activity"));
        assert!(rendered_surface.contains("shell.exec"));
        assert!(rendered_surface.contains("git status --short"));
    }

    #[test]
    fn thinking_command_toggles_visibility_from_args() {
        let mut shell = state::Shell::new("sess-thinking");
        shell.show_thinking = true;

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Thinking,
                args: "off".to_owned(),
            },
        );
        assert!(!shell.show_thinking);

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Thinking,
                args: "toggle".to_owned(),
            },
        );
        assert!(shell.show_thinking);
    }

    #[test]
    fn commands_command_appends_catalog_surface_message() {
        let mut shell = state::Shell::new("sess-cmd");

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Commands,
                args: String::new(),
            },
        );

        let rendered_surface = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");

        assert!(rendered_surface.contains("command catalog"));
        assert!(rendered_surface.contains("/diff [status|full] [Status]"));
        assert!(rendered_surface.contains("/export [latest|transcript] [path] [General]"));
        assert!(rendered_surface.contains("/context [Status]"));
    }

    #[test]
    fn export_command_writes_transcript_to_requested_path() {
        let temp_dir = tempdir().expect("tempdir");
        let export_path = temp_dir.path().join("transcript.txt");
        let export_arg = export_path.display().to_string();
        let mut shell = state::Shell::new("sess-export");
        shell.pane.add_user_message("hello");
        shell.pane.add_system_message("world");

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Export,
                args: export_arg,
            },
        );

        let written = fs::read_to_string(&export_path).expect("exported transcript");
        assert!(written.contains("hello"));
        assert!(written.contains("world"));
    }

    #[test]
    fn copy_command_can_copy_full_transcript() {
        let mut shell = state::Shell::new("sess-copy");
        shell.pane.add_user_message("alpha");
        shell.pane.add_system_message("beta");

        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Copy,
                args: "transcript".to_owned(),
            },
        );

        let status = shell
            .pane
            .status_message
            .as_ref()
            .map(|(message, _)| message.as_str())
            .unwrap_or("");

        assert!(status.contains("Copied transcript"));
    }

    #[test]
    fn diff_command_appends_working_tree_surface_from_git_repo() {
        let temp_dir = tempdir().expect("tempdir");
        let repo_root = temp_dir.path();
        let _cwd_guard = set_current_dir_for_test(repo_root);

        Command::new("git")
            .args(["init"])
            .output()
            .expect("git init");
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("git config email");
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("git config name");
        fs::write(repo_root.join("demo.txt"), "alpha\n").expect("write initial file");
        Command::new("git")
            .args(["add", "demo.txt"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .output()
            .expect("git commit");
        fs::write(repo_root.join("demo.txt"), "alpha\nbeta\n").expect("write modified file");

        let mut shell = state::Shell::new("sess-diff");
        handle_slash_command(
            &mut shell,
            ParsedSlashCommand {
                command: SlashCommand::Diff,
                args: "status".to_owned(),
            },
        );

        let rendered_surface = shell
            .pane
            .messages
            .last()
            .and_then(|message| message.parts.first())
            .and_then(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                MessagePart::SurfaceEvent { title, .. } => Some(title.as_str()),
                MessagePart::ThinkBlock(_) | MessagePart::ToolCall { .. } => None,
            })
            .unwrap_or("");

        assert!(rendered_surface.contains("working tree diff"));
        assert!(rendered_surface.contains("demo.txt"));
    }
}
