use std::collections::BTreeMap;
#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::Capability;
use tokio::sync::Notify;

use crate::CliResult;
use crate::acp::{
    AcpConversationTurnOptions, AcpTurnEventSink, JsonlAcpTurnEventSink,
    resolve_acp_backend_selection,
};
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context_with_config};

mod cli_input;

use self::cli_input::ConcurrentCliInputReader;

use super::config::{self, ConversationConfig, LoongClawConfig};
#[cfg(test)]
use super::conversation::TurnCheckpointTailRepairRuntimeProbe;
use super::conversation::{
    ConversationRuntimeBinding, ConversationSessionAddress, ConversationTurnCoordinator,
    ConversationTurnObserver, ConversationTurnObserverHandle, ConversationTurnPhase,
    ConversationTurnPhaseEvent, ConversationTurnToolEvent, ConversationTurnToolState,
    ExecutionLane, ProviderErrorMode, parse_approval_prompt_view, resolve_context_engine_selection,
};
#[cfg(any(test, feature = "memory-sqlite"))]
use super::conversation::{
    FastLaneToolBatchEventSummary, FastLaneToolBatchSegmentSnapshot, SafeLaneEventSummary,
    SafeLaneFinalStatus,
};
#[cfg(any(test, feature = "memory-sqlite"))]
use super::conversation::{
    TurnCheckpointDiagnostics, TurnCheckpointEventSummary, TurnCheckpointFailureStep,
    TurnCheckpointProgressStatus, TurnCheckpointRecoveryAction, TurnCheckpointRecoveryAssessment,
    TurnCheckpointSessionState, TurnCheckpointStage, TurnCheckpointTailRepairOutcome,
    TurnCheckpointTailRepairReason, TurnCheckpointTailRepairStatus,
};
#[cfg(feature = "memory-sqlite")]
use super::conversation::{load_fast_lane_tool_batch_event_summary, load_safe_lane_event_summary};
#[cfg(any(test, feature = "memory-sqlite"))]
use super::memory;
#[cfg(feature = "memory-sqlite")]
use super::memory::runtime_config::MemoryRuntimeConfig;
use super::tui_surface::{
    TuiActionSpec, TuiCalloutTone, TuiChecklistItemSpec, TuiChecklistStatus, TuiChoiceSpec,
    TuiHeaderStyle, TuiKeyValueSpec, TuiMessageSpec, TuiScreenSpec, TuiSectionSpec,
    render_tui_message_spec, render_tui_screen_spec,
};

pub const DEFAULT_FIRST_PROMPT: &str = "Summarize this repository and suggest the best next step.";
const TEST_ONBOARD_EXECUTABLE_ENV: &str = "LOONGCLAW_TEST_ONBOARD_EXECUTABLE";
const CLI_CHAT_LIVE_PREVIEW_MIN_EMIT_CHARS: usize = 80;
const CLI_CHAT_LIVE_PREVIEW_MAX_EMIT_CHARS: usize = 240;
const CLI_CHAT_LIVE_PREVIEW_MIN_BUFFER_CHARS: usize = 320;
const CLI_CHAT_LIVE_PREVIEW_MAX_BUFFER_CHARS: usize = 4096;
const CLI_CHAT_LIVE_TOOL_ARGS_MIN_BUFFER_CHARS: usize = 160;
const CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS: usize = 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliChatOptions {
    pub acp_requested: bool,
    pub acp_event_stream: bool,
    pub acp_bootstrap_mcp_servers: Vec<String>,
    pub acp_working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ConcurrentCliHostOptions {
    pub resolved_path: PathBuf,
    pub config: LoongClawConfig,
    pub session_id: String,
    pub shutdown: ConcurrentCliShutdown,
    pub initialize_runtime_environment: bool,
}

#[derive(Debug, Clone)]
pub struct ConcurrentCliShutdown {
    requested: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Default for ConcurrentCliShutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrentCliShutdown {
    pub fn new() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn request_shutdown(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    pub async fn wait(&self) {
        if self.is_requested() {
            return;
        }

        loop {
            if self.is_requested() {
                return;
            }
            let notified = self.notify.notified();
            if self.is_requested() {
                return;
            }
            notified.await;
        }
    }
}

impl CliChatOptions {
    fn requests_explicit_acp(&self) -> bool {
        self.acp_requested
            || self.acp_event_stream
            || !self.acp_bootstrap_mcp_servers.is_empty()
            || self.acp_working_directory.is_some()
    }
}

fn append_onboard_target_args(
    command: &mut std::process::Command,
    config_path: Option<&str>,
    resolved_config_path: &Path,
) {
    if config_path.is_some() {
        command.arg("--output").arg(resolved_config_path);
    }
}

fn resolve_onboard_executable_path() -> CliResult<PathBuf> {
    if cfg!(debug_assertions)
        && let Some(executable_path) = std::env::var_os(TEST_ONBOARD_EXECUTABLE_ENV)
    {
        return Ok(PathBuf::from(executable_path));
    }

    std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))
}

fn build_onboard_command_for_executable(
    executable_path: PathBuf,
    config_path: Option<&str>,
    resolved_config_path: &Path,
) -> std::process::Command {
    let mut command = std::process::Command::new(executable_path);
    command.arg("onboard");
    append_onboard_target_args(&mut command, config_path, resolved_config_path);
    command
}

fn build_onboard_command(
    config_path: Option<&str>,
    resolved_config_path: &Path,
) -> CliResult<std::process::Command> {
    let executable_path = resolve_onboard_executable_path()?;
    Ok(build_onboard_command_for_executable(
        executable_path,
        config_path,
        resolved_config_path,
    ))
}

fn format_onboard_command_hint(config_path: Option<&str>, resolved_config_path: &Path) -> String {
    let mut command = String::from("loongclaw onboard");
    if config_path.is_some() {
        command.push_str(" --output ");
        command.push_str(&resolved_config_path.display().to_string());
    }
    command
}

struct CliTurnRuntime {
    resolved_path: PathBuf,
    config: LoongClawConfig,
    session_id: String,
    session_address: ConversationSessionAddress,
    turn_coordinator: ConversationTurnCoordinator,
    kernel_ctx: crate::KernelContext,
    explicit_acp_request: bool,
    effective_bootstrap_mcp_servers: Vec<String>,
    effective_working_directory: Option<PathBuf>,
    memory_label: String,
    #[cfg(feature = "memory-sqlite")]
    memory_config: MemoryRuntimeConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliSessionRequirement {
    AllowImplicitDefault,
    RequireExplicit,
}

enum CliChatLoopControl {
    Continue,
    Exit,
    AssistantText(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliChatStartupSummary {
    config_path: String,
    memory_label: String,
    session_id: String,
    context_engine_id: String,
    context_engine_source: String,
    acp_enabled: bool,
    dispatch_enabled: bool,
    conversation_routing: String,
    allowed_channels: Vec<String>,
    acp_backend_id: String,
    acp_backend_source: String,
    explicit_acp_request: bool,
    event_stream_enabled: bool,
    bootstrap_mcp_servers: Vec<String>,
    working_directory: Option<String>,
}

type CliChatLiveSurfaceSink = Arc<dyn Fn(Vec<String>) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliChatLiveSurfaceSnapshot {
    phase: ConversationTurnPhase,
    provider_round: Option<usize>,
    lane: Option<ExecutionLane>,
    tool_call_count: usize,
    message_count: Option<usize>,
    estimated_tokens: Option<usize>,
    draft_preview: Option<String>,
    tool_activity_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliChatLiveToolState {
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

struct CliChatLiveSurfaceObserver {
    render_width: usize,
    render_sink: CliChatLiveSurfaceSink,
    state: StdMutex<CliChatLiveSurfaceState>,
}

#[allow(clippy::print_stdout)] // CLI REPL output
pub async fn run_cli_chat(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    options: &CliChatOptions,
) -> CliResult<()> {
    let resolved_config_path = config_path
        .map(config::expand_path)
        .unwrap_or_else(config::default_config_path);
    let config_exists = resolved_config_path.try_exists().map_err(|error| {
        format!(
            "failed to access config path {}: {error}",
            resolved_config_path.display()
        )
    })?;

    if !config_exists {
        let onboard_hint = format_onboard_command_hint(config_path, &resolved_config_path);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines =
            render_cli_chat_missing_config_lines_with_width(&onboard_hint, render_width);

        print_rendered_cli_chat_lines(&rendered_lines);

        let mut input = String::new();
        let read = io::stdin()
            .read_line(&mut input)
            .map_err(|e| format!("read stdin failed: {e}"))?;
        let should_run_onboard = should_run_missing_config_onboard(read, &input);

        if should_run_onboard {
            let mut onboard = build_onboard_command(config_path, &resolved_config_path)?;

            let exit_status = onboard
                .spawn()
                .map_err(|e| format!("failed to spawn onboard: {e}"))?
                .wait()
                .map_err(|e| format!("failed to wait for onboard: {e}"))?;

            if !exit_status.success() {
                return Err(format!("onboard exited with code {:?}", exit_status.code()));
            }
        } else {
            let rendered_lines = render_cli_chat_missing_config_decline_lines_with_width(
                &onboard_hint,
                render_width,
            );

            print_rendered_cli_chat_lines(&rendered_lines);
        }
        return Ok(());
    }

    let runtime = initialize_cli_turn_runtime(config_path, session_hint, options, "cli-chat")?;
    print_cli_chat_startup(&runtime, options)?;
    print_turn_checkpoint_startup_health(&runtime).await;
    let acp_event_printer = options
        .acp_event_stream
        .then(|| JsonlAcpTurnEventSink::stderr_with_prefix("acp-event> "));

    loop {
        print!("you> ");
        io::stdout()
            .flush()
            .map_err(|error| format!("flush stdout failed: {error}"))?;
        let mut line = String::new();
        let read = io::stdin()
            .read_line(&mut line)
            .map_err(|error| format!("read stdin failed: {error}"))?;
        if read == 0 {
            println!();
            break;
        }
        match process_cli_chat_input(
            &runtime,
            line.trim(),
            options,
            acp_event_printer
                .as_ref()
                .map(|printer| printer as &dyn AcpTurnEventSink),
        )
        .await?
        {
            CliChatLoopControl::Continue => continue,
            CliChatLoopControl::Exit => break,
            CliChatLoopControl::AssistantText(assistant_text) => {
                let render_width = detect_cli_chat_render_width();
                let rendered_lines =
                    render_cli_chat_assistant_lines_with_width(&assistant_text, render_width);
                print_rendered_cli_chat_lines(&rendered_lines);
            }
        }
    }

    println!("bye.");
    Ok(())
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_cli_ask(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    message: &str,
    options: &CliChatOptions,
) -> CliResult<()> {
    let input = message.trim();
    if input.is_empty() {
        return Err("ask message must not be empty".to_owned());
    }

    let runtime = initialize_cli_turn_runtime(config_path, session_hint, options, "cli-ask")?;
    let acp_event_printer = options
        .acp_event_stream
        .then(|| JsonlAcpTurnEventSink::stderr_with_prefix("acp-event> "));
    let assistant_text = run_cli_turn(
        &runtime,
        input,
        acp_event_printer
            .as_ref()
            .map(|printer| printer as &dyn AcpTurnEventSink),
        false,
    )
    .await?;
    println!("{assistant_text}");
    Ok(())
}

pub fn run_concurrent_cli_host(options: &ConcurrentCliHostOptions) -> CliResult<()> {
    let chat_options = CliChatOptions::default();
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        options.resolved_path.clone(),
        options.config.clone(),
        Some(options.session_id.as_str()),
        &chat_options,
        "cli-chat-concurrent",
        CliSessionRequirement::RequireExplicit,
        options.initialize_runtime_environment,
    )?;
    print_cli_chat_startup(&runtime, &chat_options)?;

    let host_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize concurrent CLI host runtime: {error}"))?;

    host_runtime.block_on(async {
        print_turn_checkpoint_startup_health(&runtime).await;
        run_concurrent_cli_host_loop(&runtime, &chat_options, &options.shutdown).await
    })
}

fn initialize_cli_turn_runtime(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    kernel_scope: &'static str,
) -> CliResult<CliTurnRuntime> {
    let (resolved_path, config) = config::load(config_path)?;
    initialize_cli_turn_runtime_with_loaded_config(
        resolved_path,
        config,
        session_hint,
        options,
        kernel_scope,
        CliSessionRequirement::AllowImplicitDefault,
        true,
    )
}

fn initialize_cli_turn_runtime_with_loaded_config(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    kernel_scope: &'static str,
    session_requirement: CliSessionRequirement,
    initialize_runtime_environment: bool,
) -> CliResult<CliTurnRuntime> {
    if !config.cli.enabled {
        return Err("CLI channel is disabled by config.cli.enabled=false".to_owned());
    }

    let session_id = resolve_cli_session_id(session_hint, session_requirement)?;
    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));
    }
    let kernel_ctx =
        bootstrap_kernel_context_with_config(kernel_scope, DEFAULT_TOKEN_TTL_S, &config)?;
    let explicit_acp_request = options.requests_explicit_acp();
    let effective_bootstrap_mcp_servers = config
        .acp
        .dispatch
        .bootstrap_mcp_server_names_with_additions(&options.acp_bootstrap_mcp_servers)?;
    let effective_working_directory = options
        .acp_working_directory
        .clone()
        .or_else(|| config.acp.dispatch.resolved_working_directory());
    let session_address = ConversationSessionAddress::from_session_id(session_id.clone());

    #[cfg(feature = "memory-sqlite")]
    let (memory_config, memory_label) = {
        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        let sqlite_path = config.memory.resolved_sqlite_path();
        let initialized = memory::ensure_memory_db_ready(Some(sqlite_path), &memory_config)
            .map_err(|error| format!("failed to initialize sqlite memory: {error}"))?;
        (memory_config, initialized.display().to_string())
    };

    #[cfg(not(feature = "memory-sqlite"))]
    let memory_label = "disabled".to_owned();

    Ok(CliTurnRuntime {
        resolved_path,
        config,
        session_id,
        session_address,
        turn_coordinator: ConversationTurnCoordinator::new(),
        kernel_ctx,
        explicit_acp_request,
        effective_bootstrap_mcp_servers,
        effective_working_directory,
        memory_label,
        #[cfg(feature = "memory-sqlite")]
        memory_config,
    })
}

fn resolve_cli_session_id(
    session_hint: Option<&str>,
    session_requirement: CliSessionRequirement,
) -> CliResult<String> {
    match session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(session_id) => Ok(session_id.to_owned()),
        None => match session_requirement {
            CliSessionRequirement::AllowImplicitDefault => Ok("default".to_owned()),
            CliSessionRequirement::RequireExplicit => {
                Err("concurrent CLI host requires an explicit session id".to_owned())
            }
        },
    }
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_turn_checkpoint_startup_health(runtime: &CliTurnRuntime) {
    #[cfg(feature = "memory-sqlite")]
    let render_width = detect_cli_chat_render_width();

    #[cfg(feature = "memory-sqlite")]
    match runtime
        .turn_coordinator
        .load_turn_checkpoint_diagnostics(
            &runtime.config,
            &runtime.session_id,
            crate::conversation::ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await
    {
        Ok(diagnostics) => {
            if let Some(rendered_lines) = render_turn_checkpoint_startup_health_lines_with_width(
                &runtime.session_id,
                &diagnostics,
                render_width,
            ) {
                print_rendered_cli_chat_lines(&rendered_lines);
            }
        }
        Err(error) => {
            let rendered_lines = render_turn_checkpoint_health_error_lines_with_width(
                &runtime.session_id,
                &error,
                render_width,
            );

            print_rendered_cli_chat_lines(&rendered_lines);
        }
    }
}

#[allow(clippy::print_stdout)] // CLI output
async fn run_concurrent_cli_host_loop(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
    shutdown: &ConcurrentCliShutdown,
) -> CliResult<()> {
    if shutdown.is_requested() {
        println!("bye.");
        return Ok(());
    }

    let mut stdin_reader = ConcurrentCliInputReader::new()?;

    loop {
        if shutdown.is_requested() {
            break;
        }

        print!("you> ");
        io::stdout()
            .flush()
            .map_err(|error| format!("flush stdout failed: {error}"))?;

        let next_line = tokio::select! {
            _ = shutdown.wait() => {
                println!();
                None
            },
            line = stdin_reader.next_line() => Some(line?),
        };

        let Some(line) = next_line else {
            break;
        };
        let Some(line) = line else {
            println!();
            break;
        };

        match process_cli_chat_input(runtime, line.trim(), options, None).await? {
            CliChatLoopControl::Continue => continue,
            CliChatLoopControl::Exit => break,
            CliChatLoopControl::AssistantText(assistant_text) => {
                let render_width = detect_cli_chat_render_width();
                let rendered_lines =
                    render_cli_chat_assistant_lines_with_width(&assistant_text, render_width);
                print_rendered_cli_chat_lines(&rendered_lines);
            }
        }
    }

    println!("bye.");
    Ok(())
}

async fn process_cli_chat_input(
    runtime: &CliTurnRuntime,
    input: &str,
    _options: &CliChatOptions,
    event_sink: Option<&dyn AcpTurnEventSink>,
) -> CliResult<CliChatLoopControl> {
    if input.is_empty() {
        return Ok(CliChatLoopControl::Continue);
    }
    if is_exit_command(&runtime.config, input) {
        return Ok(CliChatLoopControl::Exit);
    }
    if input == "/help" {
        print_help();
        return Ok(CliChatLoopControl::Continue);
    }
    if input == "/history" {
        #[cfg(feature = "memory-sqlite")]
        print_history(
            &runtime.session_id,
            runtime.config.memory.sliding_window,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
            &runtime.memory_config,
        )
        .await?;
        #[cfg(not(feature = "memory-sqlite"))]
        print_history(
            &runtime.session_id,
            runtime.config.memory.sliding_window,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await?;
        return Ok(CliChatLoopControl::Continue);
    }
    if let Some(limit) = parse_fast_lane_summary_limit(input, runtime.config.memory.sliding_window)?
    {
        #[cfg(feature = "memory-sqlite")]
        print_fast_lane_summary(
            &runtime.session_id,
            limit,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
            &runtime.memory_config,
        )
        .await?;
        #[cfg(not(feature = "memory-sqlite"))]
        print_fast_lane_summary(
            &runtime.session_id,
            limit,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await?;
        return Ok(CliChatLoopControl::Continue);
    }
    if let Some(limit) = parse_safe_lane_summary_limit(input, runtime.config.memory.sliding_window)?
    {
        #[cfg(feature = "memory-sqlite")]
        print_safe_lane_summary(
            &runtime.session_id,
            limit,
            &runtime.config.conversation,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
            &runtime.memory_config,
        )
        .await?;
        #[cfg(not(feature = "memory-sqlite"))]
        print_safe_lane_summary(
            &runtime.session_id,
            limit,
            &runtime.config.conversation,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await?;
        return Ok(CliChatLoopControl::Continue);
    }
    if let Some(limit) =
        parse_turn_checkpoint_summary_limit(input, runtime.config.memory.sliding_window)?
    {
        #[cfg(feature = "memory-sqlite")]
        print_turn_checkpoint_summary(
            &runtime.turn_coordinator,
            &runtime.config,
            &runtime.session_id,
            limit,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
            &runtime.memory_config,
        )
        .await?;
        #[cfg(not(feature = "memory-sqlite"))]
        print_turn_checkpoint_summary(
            &runtime.turn_coordinator,
            &runtime.config,
            &runtime.session_id,
            limit,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await?;
        return Ok(CliChatLoopControl::Continue);
    }
    if is_turn_checkpoint_repair_command(input)? {
        print_turn_checkpoint_repair(
            &runtime.turn_coordinator,
            &runtime.config,
            &runtime.session_id,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await?;
        return Ok(CliChatLoopControl::Continue);
    }

    Ok(CliChatLoopControl::AssistantText(
        run_cli_turn(runtime, input, event_sink, true).await?,
    ))
}

#[allow(clippy::print_stdout)] // CLI output
fn print_cli_chat_startup(runtime: &CliTurnRuntime, options: &CliChatOptions) -> CliResult<()> {
    let summary = build_cli_chat_startup_summary(runtime, options)?;
    for line in render_cli_chat_startup_lines(&summary) {
        println!("{line}");
    }
    Ok(())
}

fn build_cli_chat_startup_summary(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
) -> CliResult<CliChatStartupSummary> {
    let context_engine_selection = resolve_context_engine_selection(&runtime.config);
    let acp_selection = resolve_acp_backend_selection(&runtime.config);
    Ok(CliChatStartupSummary {
        config_path: runtime.resolved_path.display().to_string(),
        memory_label: runtime.memory_label.clone(),
        session_id: runtime.session_id.clone(),
        context_engine_id: context_engine_selection.id.to_owned(),
        context_engine_source: context_engine_selection.source.as_str().to_owned(),
        acp_enabled: runtime.config.acp.enabled,
        dispatch_enabled: runtime.config.acp.dispatch_enabled(),
        conversation_routing: runtime
            .config
            .acp
            .dispatch
            .conversation_routing
            .as_str()
            .to_owned(),
        allowed_channels: runtime.config.acp.dispatch.allowed_channel_ids()?,
        acp_backend_id: acp_selection.id.to_owned(),
        acp_backend_source: acp_selection.source.as_str().to_owned(),
        explicit_acp_request: runtime.explicit_acp_request,
        event_stream_enabled: options.acp_event_stream,
        bootstrap_mcp_servers: runtime.effective_bootstrap_mcp_servers.clone(),
        working_directory: runtime
            .effective_working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
    })
}

fn render_cli_chat_startup_lines(summary: &CliChatStartupSummary) -> Vec<String> {
    let render_width = detect_cli_chat_render_width();
    render_cli_chat_startup_lines_with_width(summary, render_width)
}

fn should_run_missing_config_onboard(read: usize, input: &str) -> bool {
    if read == 0 {
        return false;
    }

    let normalized_input = input.trim().to_ascii_lowercase();

    if normalized_input.is_empty() {
        return true;
    }

    matches!(normalized_input.as_str(), "y" | "yes")
}

fn render_cli_chat_missing_config_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_missing_config_screen_spec(onboard_hint);
    render_tui_screen_spec(&screen_spec, width, false)
}

fn build_cli_chat_missing_config_screen_spec(onboard_hint: &str) -> TuiScreenSpec {
    let intro_lines = vec![
        "Welcome to LoongClaw!".to_owned(),
        "No configuration found for interactive chat.".to_owned(),
    ];
    let sections = vec![TuiSectionSpec::ActionGroup {
        title: Some("setup command".to_owned()),
        inline_title_when_wide: true,
        items: vec![TuiActionSpec {
            label: "start setup".to_owned(),
            command: onboard_hint.to_owned(),
        }],
    }];
    let choices = vec![
        TuiChoiceSpec {
            key: "y".to_owned(),
            label: "run setup wizard".to_owned(),
            detail_lines: vec!["Create a config now and return to interactive chat.".to_owned()],
            recommended: true,
        },
        TuiChoiceSpec {
            key: "n".to_owned(),
            label: "skip for now".to_owned(),
            detail_lines: vec!["Exit chat now and keep the setup command for later.".to_owned()],
            recommended: false,
        },
    ];
    let footer_lines = vec!["Press Enter to accept y.".to_owned()];

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("setup required".to_owned()),
        progress_line: None,
        intro_lines,
        sections,
        choices,
        footer_lines,
    }
}

fn render_cli_chat_missing_config_decline_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_missing_config_decline_message_spec(onboard_hint);
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_missing_config_decline_message_spec(onboard_hint: &str) -> TuiMessageSpec {
    let setup_hint = format!("You can run '{onboard_hint}' later to get started.");
    let sections = vec![
        TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("setup skipped".to_owned()),
            lines: vec![setup_hint],
        },
        TuiSectionSpec::ActionGroup {
            title: Some("start later".to_owned()),
            inline_title_when_wide: true,
            items: vec![TuiActionSpec {
                label: "setup command".to_owned(),
                command: onboard_hint.to_owned(),
            }],
        },
    ];

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("setup required".to_owned()),
        sections,
        footer_lines: Vec::new(),
    }
}

fn render_cli_chat_startup_lines_with_width(
    summary: &CliChatStartupSummary,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_startup_screen_spec(summary);
    render_tui_screen_spec(&screen_spec, width, false)
}

fn detect_cli_chat_render_width() -> usize {
    crate::presentation::detect_render_width()
}

#[allow(clippy::print_stdout)] // CLI output
fn print_rendered_cli_chat_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

fn build_cli_chat_startup_screen_spec(summary: &CliChatStartupSummary) -> TuiScreenSpec {
    let allowed_channels = if summary.allowed_channels.is_empty() {
        "-".to_owned()
    } else {
        summary.allowed_channels.join(",")
    };
    let runtime_line = format!(
        "ACP enabled={} dispatch_enabled={} routing={} backend={} ({}) allowed_channels={allowed_channels}",
        summary.acp_enabled,
        summary.dispatch_enabled,
        summary.conversation_routing,
        summary.acp_backend_id,
        summary.acp_backend_source,
    );
    let mut sections = vec![
        TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: true,
            items: vec![TuiActionSpec {
                label: "first prompt".to_owned(),
                command: DEFAULT_FIRST_PROMPT.to_owned(),
            }],
        },
        TuiSectionSpec::Narrative {
            title: None,
            lines: vec!["- type your request, or use /help for commands".to_owned()],
        },
        TuiSectionSpec::KeyValues {
            title: Some("session details".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "session".to_owned(),
                    value: summary.session_id.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "config".to_owned(),
                    value: summary.config_path.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "memory".to_owned(),
                    value: summary.memory_label.clone(),
                },
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("runtime details".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "context engine".to_owned(),
                    value: format!(
                        "{} ({})",
                        summary.context_engine_id, summary.context_engine_source
                    ),
                },
                TuiKeyValueSpec::Plain {
                    key: "acp".to_owned(),
                    value: runtime_line,
                },
            ],
        },
    ];

    if summary.explicit_acp_request
        || summary.event_stream_enabled
        || !summary.bootstrap_mcp_servers.is_empty()
        || summary.working_directory.is_some()
    {
        let bootstrap_label = if summary.bootstrap_mcp_servers.is_empty() {
            "-".to_owned()
        } else {
            summary.bootstrap_mcp_servers.join(",")
        };
        let cwd_label = summary.working_directory.as_deref().unwrap_or("-");
        let override_lines = vec![
            format!("explicit request: {}", summary.explicit_acp_request),
            format!("event stream: {}", summary.event_stream_enabled),
            format!("bootstrap MCP servers: {bootstrap_label}"),
            format!("working directory: {cwd_label}"),
        ];
        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("acp overrides".to_owned()),
            lines: override_lines,
        });
    }

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("chat ready".to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: Vec::new(),
    }
}

fn render_cli_chat_help_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = build_cli_chat_help_message_spec();
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_help_message_spec() -> TuiMessageSpec {
    let command_items = vec![
        TuiKeyValueSpec::Plain {
            key: "/help".to_owned(),
            value: "show chat commands".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/history".to_owned(),
            value: "print the current session sliding window".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/fast_lane_summary [limit]".to_owned(),
            value: "summarize fast-lane batch execution events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/safe_lane_summary [limit]".to_owned(),
            value: "summarize safe-lane runtime events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/turn_checkpoint_summary [limit]".to_owned(),
            value: "summarize durable turn finalization state".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/turn_checkpoint_repair".to_owned(),
            value: "repair durable turn finalization tail when safe".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/exit".to_owned(),
            value: "quit chat".to_owned(),
        },
    ];
    let note_lines = vec![
        "Type any non-command text to send a normal assistant turn.".to_owned(),
        "Use /history to inspect the active memory window when a reply feels off.".to_owned(),
    ];

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("commands".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("slash commands".to_owned()),
                items: command_items,
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("usage notes".to_owned()),
                lines: note_lines,
            },
        ],
        footer_lines: Vec::new(),
    }
}

fn render_cli_chat_history_lines_with_width(
    session_id: &str,
    limit: usize,
    history_lines: &[String],
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_history_message_spec(session_id, limit, history_lines);
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_history_message_spec(
    session_id: &str,
    limit: usize,
    history_lines: &[String],
) -> TuiMessageSpec {
    let caption = format!("session={session_id} limit={limit}");
    let history_section = TuiSectionSpec::Narrative {
        title: Some("sliding window".to_owned()),
        lines: history_lines.to_vec(),
    };

    TuiMessageSpec {
        role: "history".to_owned(),
        caption: Some(caption),
        sections: vec![history_section],
        footer_lines: Vec::new(),
    }
}

fn render_cli_chat_assistant_lines_with_width(assistant_text: &str, width: usize) -> Vec<String> {
    if let Some(screen_spec) = build_cli_chat_approval_screen_spec(assistant_text) {
        return render_tui_screen_spec(&screen_spec, width, false);
    }
    let message_spec = build_cli_chat_assistant_message_spec(assistant_text);
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_assistant_message_spec(assistant_text: &str) -> TuiMessageSpec {
    let sections = parse_cli_chat_markdown_sections(assistant_text);

    TuiMessageSpec {
        role: "loongclaw".to_owned(),
        caption: Some("reply".to_owned()),
        sections,
        footer_lines: Vec::new(),
    }
}

fn build_cli_chat_approval_screen_spec(assistant_text: &str) -> Option<TuiScreenSpec> {
    let parsed = parse_approval_prompt_view(assistant_text)?;
    let mut intro_lines = Vec::new();
    if let Some(preface) = parsed
        .preface
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        intro_lines.extend(preface.lines().map(|line| line.to_owned()));
    }

    let title = parsed.title();

    let mut sections = Vec::new();
    if let Some(reason) = parsed.reason.as_deref() {
        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Warning,
            title: Some(parsed.pause_reason_title()),
            lines: vec![reason.to_owned()],
        });
    }

    let mut kv_items = Vec::new();
    if let Some(tool_name) = parsed.tool_name.as_deref() {
        kv_items.push(TuiKeyValueSpec::Plain {
            key: parsed.tool_label(),
            value: tool_name.to_owned(),
        });
    }
    if let Some(request_id) = parsed.request_id.as_deref() {
        kv_items.push(TuiKeyValueSpec::Plain {
            key: parsed.request_id_label(),
            value: request_id.to_owned(),
        });
    }
    if !kv_items.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some(parsed.request_section_title()),
            items: kv_items,
        });
    }

    let choices = parsed
        .actions
        .iter()
        .map(|action| TuiChoiceSpec {
            key: action.numeric_alias.clone(),
            label: action.label.clone(),
            detail_lines: action.detail_lines.clone(),
            recommended: action.recommended,
        })
        .collect::<Vec<_>>();

    let footer_lines = if parsed.actions.is_empty() {
        Vec::new()
    } else if parsed.locale.is_cjk() {
        vec![
            format!("也可以直接回复：{}", parsed.action_commands_text()),
            format!("数字别名：{}", parsed.action_numeric_aliases_text()),
        ]
    } else {
        vec![
            format!("You can also reply with: {}", parsed.action_commands_text()),
            format!("Numeric aliases: {}", parsed.action_numeric_aliases_text()),
        ]
    };

    Some(TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(parsed.subtitle()),
        title,
        progress_line: None,
        intro_lines,
        sections,
        choices,
        footer_lines,
    })
}

fn build_cli_chat_live_surface_observer(render_width: usize) -> ConversationTurnObserverHandle {
    let render_sink: CliChatLiveSurfaceSink = Arc::new(|lines| {
        print_rendered_cli_chat_lines(&lines);
    });
    let observer = CliChatLiveSurfaceObserver::new(render_width, render_sink);
    Arc::new(observer)
}

impl CliChatLiveSurfaceObserver {
    fn new(render_width: usize, render_sink: CliChatLiveSurfaceSink) -> Self {
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
        | ConversationTurnPhase::Failed => true,
        ConversationTurnPhase::ContextReady | ConversationTurnPhase::Completed => false,
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

fn render_cli_chat_live_surface_lines_with_width(
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

fn parse_cli_chat_markdown_sections(text: &str) -> Vec<TuiSectionSpec> {
    let mut sections = Vec::new();
    let mut pending_title = None;
    let mut narrative_lines = Vec::new();
    let mut callout_lines = Vec::new();
    let mut code_title = None;
    let mut code_language = None;
    let mut code_lines = Vec::new();
    let mut inside_code_block = false;

    for raw_line in text.lines() {
        let trimmed_end = raw_line.trim_end();

        if inside_code_block {
            if is_markdown_fence_close(trimmed_end) {
                push_preformatted_section(
                    &mut sections,
                    &mut code_title,
                    &mut code_language,
                    &mut code_lines,
                );
                inside_code_block = false;
                continue;
            }

            code_lines.push(trimmed_end.to_owned());
            continue;
        }

        if let Some(language) = parse_markdown_fence_language(trimmed_end) {
            push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
            push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
            code_title = pending_title.take();
            code_language = language;
            inside_code_block = true;
            continue;
        }

        if let Some(heading_text) = parse_markdown_heading(trimmed_end) {
            push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
            push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
            push_standalone_title_section(&mut sections, &mut pending_title);
            pending_title = Some(heading_text.to_owned());
            continue;
        }

        if let Some(callout_line) = parse_markdown_quote_line(trimmed_end) {
            push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
            callout_lines.push(callout_line);
            continue;
        }

        if !callout_lines.is_empty() {
            push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
        }

        let normalized_line = normalize_markdown_display_line(trimmed_end);
        let is_blank_line = normalized_line.trim().is_empty();

        if is_blank_line && narrative_lines.is_empty() {
            continue;
        }

        narrative_lines.push(normalized_line);
    }

    if inside_code_block {
        push_preformatted_section(
            &mut sections,
            &mut code_title,
            &mut code_language,
            &mut code_lines,
        );
    }

    push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
    push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
    push_standalone_title_section(&mut sections, &mut pending_title);

    if sections.is_empty() {
        sections.push(TuiSectionSpec::Narrative {
            title: None,
            lines: vec!["(empty reply)".to_owned()],
        });
    }

    sections
}

fn push_narrative_section(
    sections: &mut Vec<TuiSectionSpec>,
    pending_title: &mut Option<String>,
    narrative_lines: &mut Vec<String>,
) {
    trim_blank_line_edges(narrative_lines);
    if narrative_lines.is_empty() {
        return;
    }

    let title = pending_title.take();
    let lines = std::mem::take(narrative_lines);
    sections.push(TuiSectionSpec::Narrative { title, lines });
}

fn push_standalone_title_section(
    sections: &mut Vec<TuiSectionSpec>,
    pending_title: &mut Option<String>,
) {
    let Some(title) = pending_title.take() else {
        return;
    };

    sections.push(TuiSectionSpec::Narrative {
        title: Some(title),
        lines: Vec::new(),
    });
}

fn push_callout_section(
    sections: &mut Vec<TuiSectionSpec>,
    pending_title: &mut Option<String>,
    callout_lines: &mut Vec<String>,
) {
    trim_blank_line_edges(callout_lines);
    if callout_lines.is_empty() {
        return;
    }

    let lines = std::mem::take(callout_lines);
    let title = pending_title
        .take()
        .or_else(|| Some("quoted context".to_owned()));

    sections.push(TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title,
        lines,
    });
}

fn push_preformatted_section(
    sections: &mut Vec<TuiSectionSpec>,
    code_title: &mut Option<String>,
    code_language: &mut Option<String>,
    code_lines: &mut Vec<String>,
) {
    let title = code_title.take();
    let language = code_language.take();
    let lines = std::mem::take(code_lines);
    sections.push(TuiSectionSpec::Preformatted {
        title,
        language,
        lines,
    });
}

fn trim_blank_line_edges(lines: &mut Vec<String>) {
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

fn is_markdown_fence_close(line: &str) -> bool {
    line.trim() == "```"
}

fn parse_markdown_fence_language(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim();
    let raw_language = trimmed.strip_prefix("```")?;
    let language = raw_language.trim();

    if language.is_empty() {
        return Some(None);
    }

    Some(Some(language.to_owned()))
}

fn parse_markdown_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let marker_count = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();

    if marker_count == 0 || marker_count > 6 {
        return None;
    }

    let heading_text = trimmed.get(marker_count..)?;
    let separator = heading_text.chars().next()?;
    if separator != ' ' && separator != '\t' {
        return None;
    }

    let heading_text = heading_text.trim_start_matches([' ', '\t']);
    let normalized_text = trim_markdown_heading_closing_sequence(heading_text).trim();

    if normalized_text.is_empty() {
        return None;
    }

    Some(normalized_text)
}

fn trim_markdown_heading_closing_sequence(text: &str) -> &str {
    let trimmed_end = text.trim_end_matches([' ', '\t']);
    let trailing_hash_count = trimmed_end
        .chars()
        .rev()
        .take_while(|character| *character == '#')
        .count();

    if trailing_hash_count == 0 {
        return trimmed_end;
    }

    let content_end = trimmed_end.len().saturating_sub(trailing_hash_count);
    let content = trimmed_end.get(..content_end).unwrap_or(trimmed_end);
    let ends_with_heading_space = content
        .chars()
        .last()
        .is_some_and(|character| character == ' ' || character == '\t');

    if !ends_with_heading_space {
        return trimmed_end;
    }

    content.trim_end_matches([' ', '\t'])
}

fn parse_markdown_quote_line(line: &str) -> Option<String> {
    let trimmed_start = line.trim_start();
    let quote_body = trimmed_start.strip_prefix('>')?;
    let normalized_text = quote_body.trim_start();
    Some(normalized_text.to_owned())
}

fn normalize_markdown_display_line(line: &str) -> String {
    let trimmed_end = line.trim_end();
    let leading_space_count = trimmed_end
        .chars()
        .take_while(|character| character.is_ascii_whitespace())
        .count();
    let indent = trimmed_end.get(..leading_space_count).unwrap_or("");
    let trimmed_start = trimmed_end.get(leading_space_count..).unwrap_or("");

    if let Some(rest) = trimmed_start.strip_prefix("* ") {
        return format!("{indent}- {rest}");
    }

    if let Some(rest) = trimmed_start.strip_prefix("+ ") {
        return format!("{indent}- {rest}");
    }

    trimmed_end.to_owned()
}

async fn run_cli_turn(
    runtime: &CliTurnRuntime,
    input: &str,
    event_sink: Option<&dyn AcpTurnEventSink>,
    live_surface_enabled: bool,
) -> CliResult<String> {
    let turn_config = reload_cli_turn_config(&runtime.config, runtime.resolved_path.as_path())?;
    let acp_options = if runtime.explicit_acp_request {
        AcpConversationTurnOptions::explicit()
    } else {
        AcpConversationTurnOptions::automatic()
    }
    .with_event_sink(event_sink)
    .with_additional_bootstrap_mcp_servers(&runtime.effective_bootstrap_mcp_servers)
    .with_working_directory(runtime.effective_working_directory.as_deref());
    let live_surface_observer = if live_surface_enabled {
        let render_width = detect_cli_chat_render_width();
        Some(build_cli_chat_live_surface_observer(render_width))
    } else {
        None
    };
    runtime
        .turn_coordinator
        .handle_turn_with_address_and_acp_options_and_observer(
            &turn_config,
            &runtime.session_address,
            input,
            ProviderErrorMode::InlineMessage,
            &acp_options,
            crate::conversation::ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
            live_surface_observer,
        )
        .await
}

fn reload_cli_turn_config(
    config: &LoongClawConfig,
    resolved_path: &Path,
) -> CliResult<LoongClawConfig> {
    config.reload_provider_runtime_state_from_path(resolved_path)
}

fn is_exit_command(config: &LoongClawConfig, input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    config
        .cli
        .exit_commands
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .any(|value| !value.is_empty() && value == lower)
}

#[allow(clippy::print_stdout)] // CLI output
fn print_help() {
    let render_width = detect_cli_chat_render_width();
    let rendered_lines = render_cli_chat_help_lines_with_width(render_width);
    print_rendered_cli_chat_lines(&rendered_lines);
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_history(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let history_lines = load_history_lines(session_id, limit, binding, memory_config).await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_history_lines_with_width(
            session_id,
            limit,
            &history_lines,
            render_width,
        );
        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "history",
            "history unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_window_history_lines(turns: &[memory::WindowTurn]) -> Vec<String> {
    if turns.is_empty() {
        return vec!["(no history yet)".to_owned()];
    }

    turns
        .iter()
        .map(|turn| {
            format!(
                "[{}] {}: {}",
                turn.ts.unwrap_or_default(),
                turn.role,
                turn.content
            )
        })
        .collect()
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_prompt_context_history_lines(entries: &[memory::MemoryContextEntry]) -> Vec<String> {
    if entries.is_empty() {
        return vec!["(no history yet)".to_owned()];
    }

    let mut lines = Vec::new();
    for entry in entries {
        match entry.kind {
            memory::MemoryContextKind::Profile => {
                lines.push("[profile]".to_owned());
                lines.push(entry.content.clone());
            }
            memory::MemoryContextKind::Summary => {
                lines.push("[summary]".to_owned());
                lines.push(entry.content.clone());
            }
            memory::MemoryContextKind::RetrievedMemory => {
                lines.push("[retrieved_memory]".to_owned());
                lines.push(entry.content.clone());
            }
            memory::MemoryContextKind::Turn => {
                lines.push(format!("{}: {}", entry.role, entry.content));
            }
        }
    }
    lines
}

#[cfg(feature = "memory-sqlite")]
async fn load_history_lines(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<Vec<String>> {
    if let Some(ctx) = binding.kernel_context() {
        let request = memory::build_window_request(session_id, limit);
        let caps = BTreeSet::from([Capability::MemoryRead]);
        let outcome = ctx
            .kernel
            .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
            .await
            .map_err(|error| format!("load history via kernel failed: {error}"))?;
        if outcome.status != "ok" {
            return Err(format!(
                "load history via kernel returned non-ok status: {}",
                outcome.status
            ));
        }
        let turns = memory::decode_window_turns(&outcome.payload);
        return Ok(format_window_history_lines(&turns));
    }

    let entries = memory::load_prompt_context(session_id, memory_config)
        .map_err(|error| format!("load history failed: {error}"))?;
    Ok(format_prompt_context_history_lines(&entries))
}

fn parse_safe_lane_summary_limit(input: &str, default_window: usize) -> CliResult<Option<usize>> {
    parse_summary_limit(
        input,
        default_window,
        &["/safe_lane_summary", "/safe-lane-summary"],
    )
}

fn parse_fast_lane_summary_limit(input: &str, default_window: usize) -> CliResult<Option<usize>> {
    parse_summary_limit(
        input,
        default_window,
        &["/fast_lane_summary", "/fast-lane-summary"],
    )
}

fn parse_summary_limit(
    input: &str,
    default_window: usize,
    aliases: &[&str],
) -> CliResult<Option<usize>> {
    let Some(primary_alias) = aliases.first().copied() else {
        return Ok(None);
    };

    let mut tokens = input.split_whitespace();
    let Some(command) = tokens.next() else {
        return Ok(None);
    };
    if !aliases.contains(&command) {
        return Ok(None);
    }

    let usage = format!("usage: {primary_alias} [limit]");
    let default_limit = default_window.saturating_mul(4).max(64);
    let limit = match tokens.next() {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|error| format!("invalid {primary_alias} limit `{raw}`: {error}; {usage}"))?,
        None => default_limit,
    };
    if limit == 0 {
        return Err(format!("invalid {primary_alias} limit `0`; {usage}"));
    }
    if tokens.next().is_some() {
        return Err(usage);
    }
    Ok(Some(limit))
}

fn parse_turn_checkpoint_summary_limit(
    input: &str,
    default_window: usize,
) -> CliResult<Option<usize>> {
    parse_summary_limit(
        input,
        default_window,
        &["/turn_checkpoint_summary", "/turn-checkpoint-summary"],
    )
}

fn is_turn_checkpoint_repair_command(input: &str) -> CliResult<bool> {
    let mut tokens = input.split_whitespace();
    let Some(command) = tokens.next() else {
        return Ok(false);
    };
    if command != "/turn_checkpoint_repair" && command != "/turn-checkpoint-repair" {
        return Ok(false);
    }
    if tokens.next().is_some() {
        return Err("usage: /turn_checkpoint_repair".to_owned());
    }
    Ok(true)
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_fast_lane_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let summary =
            load_fast_lane_tool_batch_event_summary(session_id, limit, binding, memory_config)
                .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines =
            render_fast_lane_summary_lines_with_width(session_id, limit, &summary, render_width);

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "fast-lane",
            "fast-lane summary unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_safe_lane_summary(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let summary =
            load_safe_lane_event_summary(session_id, limit, binding, memory_config).await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_safe_lane_summary_lines_with_width(
            session_id,
            limit,
            conversation_config,
            &summary,
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, conversation_config, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "safe-lane",
            "safe-lane summary unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_turn_checkpoint_summary(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongClawConfig,
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] _memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let diagnostics = turn_coordinator
            .load_turn_checkpoint_diagnostics_with_limit(config, session_id, limit, binding)
            .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_turn_checkpoint_summary_lines_with_width(
            session_id,
            limit,
            &diagnostics,
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (turn_coordinator, config, session_id, limit, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "checkpoint",
            "turn checkpoint summary unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_turn_checkpoint_repair(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongClawConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let outcome = turn_coordinator
            .repair_turn_checkpoint_tail(config, session_id, binding)
            .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines =
            render_turn_checkpoint_repair_lines_with_width(session_id, &outcome, render_width);

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (turn_coordinator, config, session_id, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "repair",
            "turn checkpoint repair unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[cfg(not(feature = "memory-sqlite"))]
fn render_cli_chat_feature_unavailable_lines_with_width(
    role: &str,
    detail: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_feature_unavailable_message_spec(role, detail);
    render_tui_message_spec(&message_spec, width)
}

#[cfg(not(feature = "memory-sqlite"))]
fn build_cli_chat_feature_unavailable_message_spec(role: &str, detail: &str) -> TuiMessageSpec {
    let sections = vec![TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Warning,
        title: Some("feature unavailable".to_owned()),
        lines: vec![detail.to_owned()],
    }];

    TuiMessageSpec {
        role: role.to_owned(),
        caption: Some("unavailable".to_owned()),
        sections,
        footer_lines: Vec::new(),
    }
}

fn tui_plain_item(key: &str, value: String) -> TuiKeyValueSpec {
    let key = key.to_owned();

    TuiKeyValueSpec::Plain { key, value }
}

fn tui_csv_item(key: &str, values: Vec<String>) -> TuiKeyValueSpec {
    let key = key.to_owned();

    TuiKeyValueSpec::Csv { key, values }
}

fn csv_values_or_dash(values: Vec<String>) -> Vec<String> {
    if values.is_empty() {
        return vec!["-".to_owned()];
    }

    values
}

fn collect_rollup_values(counts: &std::collections::BTreeMap<String, u32>) -> Vec<String> {
    counts
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect()
}

fn bool_yes_no_value(value: bool) -> String {
    if value {
        return "yes".to_owned();
    }

    "no".to_owned()
}

fn recovery_callout_tone(recovery_needed: bool) -> TuiCalloutTone {
    if recovery_needed {
        return TuiCalloutTone::Warning;
    }

    TuiCalloutTone::Success
}

fn safe_lane_health_tone(severity: &str) -> TuiCalloutTone {
    if severity == "critical" || severity == "warn" {
        return TuiCalloutTone::Warning;
    }

    if severity == "ok" {
        return TuiCalloutTone::Success;
    }

    TuiCalloutTone::Info
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_turn_checkpoint_health_error_lines_with_width(
    session_id: &str,
    error: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_health_error_message_spec(session_id, error);
    render_tui_message_spec(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_health_error_message_spec(
    session_id: &str,
    error: &str,
) -> TuiMessageSpec {
    let caption = format!("session={session_id}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("durability status".to_owned()),
            items: vec![
                tui_plain_item("state", "unavailable".to_owned()),
                tui_plain_item("session", session_id.to_owned()),
            ],
        },
        TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Warning,
            title: Some("durability unavailable".to_owned()),
            lines: vec![format!("error: {error}")],
        },
    ];

    TuiMessageSpec {
        role: "checkpoint".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: Vec::new(),
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_turn_checkpoint_startup_health_lines_with_width(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Option<Vec<String>> {
    let message_spec = build_turn_checkpoint_startup_health_message_spec(session_id, diagnostics)?;
    let rendered_lines = render_tui_message_spec(&message_spec, width);

    Some(rendered_lines)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_startup_health_message_spec(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
) -> Option<TuiMessageSpec> {
    let summary = diagnostics.summary();
    if !summary.checkpoint_durable {
        return None;
    }

    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let failure_step = format_turn_checkpoint_failure_step(summary.latest_failure_step);
    let recovery_needed = bool_yes_no_value(summary.requires_recovery);
    let reply_durable = bool_yes_no_value(summary.reply_durable);
    let checkpoint_durable = bool_yes_no_value(summary.checkpoint_durable);
    let recovery_tone = recovery_callout_tone(summary.requires_recovery);
    let caption = format!("session={session_id}");
    let recovery_reason = recovery_labels.reason.to_owned();

    let mut sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("durability status".to_owned()),
            items: vec![
                tui_plain_item("state", render_labels.session_state.to_owned()),
                tui_plain_item("durability", durability_labels.durability.to_owned()),
                tui_plain_item("reply durable", reply_durable),
                tui_plain_item("checkpoint durable", checkpoint_durable),
            ],
        },
        TuiSectionSpec::Callout {
            tone: recovery_tone,
            title: Some("recovery".to_owned()),
            lines: vec![
                format!("recovery needed: {recovery_needed}"),
                format!("action: {}", recovery_labels.action),
                format!("source: {}", recovery_labels.source),
                format!("reason: {recovery_reason}"),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("latest turn".to_owned()),
            items: vec![
                tui_plain_item("stage", render_labels.stage.to_owned()),
                tui_plain_item("after turn", render_labels.after_turn.to_owned()),
                tui_plain_item("compaction", render_labels.compaction.to_owned()),
                tui_plain_item("lane", render_labels.lane.to_owned()),
                tui_plain_item("result kind", render_labels.result_kind.to_owned()),
                tui_plain_item(
                    "persistence mode",
                    render_labels.persistence_mode.to_owned(),
                ),
                tui_plain_item("identity", render_labels.identity.to_owned()),
                tui_plain_item("failure step", failure_step.to_owned()),
            ],
        },
    ];

    if render_labels.safe_lane_route_decision != "-"
        || render_labels.safe_lane_route_reason != "-"
        || render_labels.safe_lane_route_source != "-"
    {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("safe-lane route".to_owned()),
            items: vec![
                tui_plain_item(
                    "decision",
                    render_labels.safe_lane_route_decision.to_owned(),
                ),
                tui_plain_item("reason", render_labels.safe_lane_route_reason.to_owned()),
                tui_plain_item("source", render_labels.safe_lane_route_source.to_owned()),
            ],
        });
    }

    if let Some(probe) = diagnostics.runtime_probe() {
        let probe_lines = vec![
            format!("action: {}", probe.action().as_str()),
            format!("source: {}", probe.source().as_str()),
            format!("reason: {}", probe.reason().as_str()),
        ];

        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("runtime probe".to_owned()),
            lines: probe_lines,
        });
    }

    Some(TuiMessageSpec {
        role: "checkpoint".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: Vec::new(),
    })
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_fast_lane_summary_lines_with_width(
    session_id: &str,
    limit: usize,
    summary: &FastLaneToolBatchEventSummary,
    width: usize,
) -> Vec<String> {
    let message_spec = build_fast_lane_summary_message_spec(session_id, limit, summary);
    render_tui_message_spec(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_fast_lane_summary_message_spec(
    session_id: &str,
    limit: usize,
    summary: &FastLaneToolBatchEventSummary,
) -> TuiMessageSpec {
    let parallel_safe_ratio = format_ratio(
        summary.total_parallel_safe_intents_seen,
        summary.total_intents_seen,
    );
    let serial_only_ratio = format_ratio(
        summary.total_serial_only_intents_seen,
        summary.total_intents_seen,
    );
    let configured_max_in_flight_avg = format_average(
        summary.parallel_execution_max_in_flight_sum,
        summary.parallel_execution_max_in_flight_samples,
    );
    let observed_peak_in_flight_avg = format_average(
        summary.observed_peak_in_flight_sum,
        summary.observed_peak_in_flight_samples,
    );
    let observed_wall_time_ms_avg = format_average(
        summary.observed_wall_time_ms_sum,
        summary.observed_wall_time_ms_samples,
    );
    let scheduling_class_values = collect_rollup_values(&summary.scheduling_class_counts);
    let execution_mode_values = collect_rollup_values(&summary.execution_mode_counts);
    let rollup_scheduling_classes = csv_values_or_dash(scheduling_class_values);
    let rollup_execution_modes = csv_values_or_dash(execution_mode_values);
    let latest_segment_lines = build_fast_lane_segment_lines(&summary.latest_segments);
    let caption = format!("session={session_id} limit={limit}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("events".to_owned()),
            items: vec![
                tui_plain_item("batch events", summary.batch_events.to_string()),
                tui_plain_item(
                    "schema version",
                    format_fast_lane_summary_optional(summary.latest_schema_version),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("batch mix".to_owned()),
            items: vec![
                tui_plain_item(
                    "parallel enabled",
                    summary.parallel_execution_enabled_batches.to_string(),
                ),
                tui_plain_item("parallel only", summary.parallel_only_batches.to_string()),
                tui_plain_item("mixed", summary.mixed_execution_batches.to_string()),
                tui_plain_item(
                    "sequential only",
                    summary.sequential_only_batches.to_string(),
                ),
                tui_plain_item(
                    "without segments",
                    summary.batches_without_segments.to_string(),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("intent mix".to_owned()),
            items: vec![
                tui_plain_item("total intents", summary.total_intents_seen.to_string()),
                tui_plain_item(
                    "parallel-safe intents",
                    summary.total_parallel_safe_intents_seen.to_string(),
                ),
                tui_plain_item(
                    "serial-only intents",
                    summary.total_serial_only_intents_seen.to_string(),
                ),
                tui_plain_item("parallel-safe ratio", parallel_safe_ratio),
                tui_plain_item("serial-only ratio", serial_only_ratio),
                tui_plain_item(
                    "parallel segments",
                    summary.total_parallel_segments_seen.to_string(),
                ),
                tui_plain_item(
                    "sequential segments",
                    summary.total_sequential_segments_seen.to_string(),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("execution".to_owned()),
            items: vec![
                tui_plain_item("configured max in flight avg", configured_max_in_flight_avg),
                tui_plain_item(
                    "configured max in flight max",
                    format_fast_lane_summary_optional(summary.parallel_execution_max_in_flight_max),
                ),
                tui_plain_item(
                    "configured max in flight samples",
                    summary.parallel_execution_max_in_flight_samples.to_string(),
                ),
                tui_plain_item("observed peak avg", observed_peak_in_flight_avg),
                tui_plain_item(
                    "observed peak max",
                    format_fast_lane_summary_optional(summary.observed_peak_in_flight_max),
                ),
                tui_plain_item(
                    "observed peak samples",
                    summary.observed_peak_in_flight_samples.to_string(),
                ),
                tui_plain_item("wall time avg", observed_wall_time_ms_avg),
                tui_plain_item(
                    "wall time max",
                    format_fast_lane_summary_optional(summary.observed_wall_time_ms_max),
                ),
                tui_plain_item(
                    "wall time samples",
                    summary.observed_wall_time_ms_samples.to_string(),
                ),
                tui_plain_item(
                    "degraded parallel segments",
                    summary.degraded_parallel_segments.to_string(),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("latest batch".to_owned()),
            items: vec![
                tui_plain_item(
                    "total intents",
                    format_fast_lane_summary_optional(summary.latest_total_intents),
                ),
                tui_plain_item(
                    "parallel enabled",
                    format_fast_lane_summary_optional(summary.latest_parallel_execution_enabled),
                ),
                tui_plain_item(
                    "max in flight",
                    format_fast_lane_summary_optional(
                        summary.latest_parallel_execution_max_in_flight,
                    ),
                ),
                tui_plain_item(
                    "observed peak",
                    format_fast_lane_summary_optional(summary.latest_observed_peak_in_flight),
                ),
                tui_plain_item(
                    "wall time ms",
                    format_fast_lane_summary_optional(summary.latest_observed_wall_time_ms),
                ),
                tui_plain_item(
                    "parallel-safe intents",
                    format_fast_lane_summary_optional(summary.latest_parallel_safe_intents),
                ),
                tui_plain_item(
                    "serial-only intents",
                    format_fast_lane_summary_optional(summary.latest_serial_only_intents),
                ),
                tui_plain_item(
                    "parallel segments",
                    format_fast_lane_summary_optional(summary.latest_parallel_segments),
                ),
                tui_plain_item(
                    "sequential segments",
                    format_fast_lane_summary_optional(summary.latest_sequential_segments),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("rollups".to_owned()),
            items: vec![
                tui_csv_item("scheduling classes", rollup_scheduling_classes),
                tui_csv_item("execution modes", rollup_execution_modes),
            ],
        },
        TuiSectionSpec::Narrative {
            title: Some("latest segments".to_owned()),
            lines: latest_segment_lines,
        },
    ];

    TuiMessageSpec {
        role: "fast-lane".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: Vec::new(),
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_fast_lane_segment_lines(segments: &[FastLaneToolBatchSegmentSnapshot]) -> Vec<String> {
    if segments.is_empty() {
        return vec!["- no segment snapshot recorded".to_owned()];
    }

    let mut lines = Vec::new();

    for segment in segments {
        let peak_in_flight = format_fast_lane_summary_optional(segment.observed_peak_in_flight);
        let wall_time_ms = format_fast_lane_summary_optional(segment.observed_wall_time_ms);
        let line = format!(
            "- segment {}: class={} mode={} intents={} peak={} wall_ms={}",
            segment.segment_index,
            segment.scheduling_class,
            segment.execution_mode,
            segment.intent_count,
            peak_in_flight,
            wall_time_ms,
        );

        lines.push(line);
    }

    lines
}

#[cfg(test)]
async fn load_fast_lane_summary_output(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<String> {
    let summary =
        load_fast_lane_tool_batch_event_summary(session_id, limit, binding, memory_config).await?;

    Ok(format_fast_lane_summary(session_id, limit, &summary))
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_safe_lane_summary_lines_with_width(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
    width: usize,
) -> Vec<String> {
    let message_spec =
        build_safe_lane_summary_message_spec(session_id, limit, conversation_config, summary);
    render_tui_message_spec(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_safe_lane_summary_message_spec(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
) -> TuiMessageSpec {
    let final_status = match summary.final_status {
        Some(SafeLaneFinalStatus::Succeeded) => "succeeded",
        Some(SafeLaneFinalStatus::Failed) => "failed",
        None => "unknown",
    };
    let final_failure_code = summary.final_failure_code.as_deref().unwrap_or("-");
    let final_route_decision = summary.final_route_decision.as_deref().unwrap_or("-");
    let final_route_reason = summary.final_route_reason.as_deref().unwrap_or("-");
    let metrics = summary.latest_metrics.as_ref();
    let rounds_started = metrics
        .map(|value| value.rounds_started as f64)
        .unwrap_or(summary.round_started_events as f64);
    let replan_rate = if rounds_started > 0.0 {
        summary.replan_triggered_events as f64 / rounds_started
    } else {
        0.0
    };
    let verify_failure_rate = if rounds_started > 0.0 {
        summary.verify_failed_events as f64 / rounds_started
    } else {
        0.0
    };
    let governor_trend_failure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_failure_ewma_milli);
    let governor_trend_backpressure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_backpressure_ewma_milli);
    let latest_tool_truncation_ratio = format_milli_ratio(
        summary
            .latest_tool_output
            .as_ref()
            .map(|snapshot| snapshot.truncation_ratio_milli),
    );
    let aggregate_tool_truncation_ratio_milli = summary
        .tool_output_aggregate_truncation_ratio_milli
        .or_else(|| {
            if summary.tool_output_result_lines_total == 0 {
                return None;
            }

            let truncated_lines = summary.tool_output_truncated_result_lines_total;
            let total_lines = summary.tool_output_result_lines_total;
            let ratio_milli = truncated_lines
                .saturating_mul(1000)
                .saturating_div(total_lines)
                .min(u32::MAX as u64) as u32;

            Some(ratio_milli)
        });
    let aggregate_tool_truncation_ratio =
        aggregate_tool_truncation_ratio_milli.map(|milli| (milli as f64) / 1000.0);
    let aggregate_tool_truncation_ratio_text = aggregate_tool_truncation_ratio
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "-".to_owned());
    let health_signal = derive_safe_lane_health_signal(
        conversation_config,
        summary,
        replan_rate,
        verify_failure_rate,
        aggregate_tool_truncation_ratio,
    );
    let health_flags = if health_signal.flags.is_empty() {
        "none".to_owned()
    } else {
        health_signal.flags.join(", ")
    };
    let latest_health_event_severity = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| snapshot.severity.as_str())
        .unwrap_or("-");
    let latest_health_event_flags = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| {
            if snapshot.flags.is_empty() {
                return "none".to_owned();
            }

            snapshot.flags.join(", ")
        })
        .unwrap_or_else(|| "-".to_owned());
    let route_decision_values = collect_rollup_values(&summary.route_decision_counts);
    let route_reason_values = collect_rollup_values(&summary.route_reason_counts);
    let failure_code_values = collect_rollup_values(&summary.failure_code_counts);
    let rollup_route_decisions = csv_values_or_dash(route_decision_values);
    let rollup_route_reasons = csv_values_or_dash(route_reason_values);
    let rollup_failure_codes = csv_values_or_dash(failure_code_values);
    let health_tone = safe_lane_health_tone(health_signal.severity);
    let latest_metrics_section = match metrics {
        Some(metrics) => TuiSectionSpec::KeyValues {
            title: Some("latest metrics".to_owned()),
            items: vec![
                tui_plain_item("rounds started", metrics.rounds_started.to_string()),
                tui_plain_item("rounds succeeded", metrics.rounds_succeeded.to_string()),
                tui_plain_item("rounds failed", metrics.rounds_failed.to_string()),
                tui_plain_item("verify failures", metrics.verify_failures.to_string()),
                tui_plain_item("replans triggered", metrics.replans_triggered.to_string()),
                tui_plain_item("attempts used", metrics.total_attempts_used.to_string()),
            ],
        },
        None => TuiSectionSpec::KeyValues {
            title: Some("latest metrics".to_owned()),
            items: vec![tui_plain_item("status", "unavailable".to_owned())],
        },
    };
    let caption = format!("session={session_id} limit={limit}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("terminal status".to_owned()),
            items: vec![
                tui_plain_item("status", final_status.to_owned()),
                tui_plain_item("failure code", final_failure_code.to_owned()),
                tui_plain_item("route decision", final_route_decision.to_owned()),
                tui_plain_item("route reason", final_route_reason.to_owned()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("events".to_owned()),
            items: vec![
                tui_plain_item("lane selected", summary.lane_selected_events.to_string()),
                tui_plain_item("round started", summary.round_started_events.to_string()),
                tui_plain_item(
                    "round succeeded",
                    summary.round_completed_succeeded_events.to_string(),
                ),
                tui_plain_item(
                    "round failed",
                    summary.round_completed_failed_events.to_string(),
                ),
                tui_plain_item("verify failed", summary.verify_failed_events.to_string()),
                tui_plain_item(
                    "verify policy adjusted",
                    summary.verify_policy_adjusted_events.to_string(),
                ),
                tui_plain_item(
                    "replan triggered",
                    summary.replan_triggered_events.to_string(),
                ),
                tui_plain_item("final status", summary.final_status_events.to_string()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("rates".to_owned()),
            items: vec![
                tui_plain_item("replan per round", format!("{replan_rate:.3}")),
                tui_plain_item("verify fail per round", format!("{verify_failure_rate:.3}")),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("governor".to_owned()),
            items: vec![
                tui_plain_item(
                    "engaged events",
                    summary.session_governor_engaged_events.to_string(),
                ),
                tui_plain_item(
                    "force no replan",
                    summary.session_governor_force_no_replan_events.to_string(),
                ),
                tui_plain_item(
                    "failed threshold triggers",
                    summary
                        .session_governor_failed_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "backpressure threshold triggers",
                    summary
                        .session_governor_backpressure_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "trend threshold triggers",
                    summary
                        .session_governor_trend_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "recovery threshold triggers",
                    summary
                        .session_governor_recovery_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "metric snapshots",
                    summary.session_governor_metrics_snapshots_seen.to_string(),
                ),
                tui_plain_item(
                    "trend samples",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_trend_samples,
                    ),
                ),
                tui_plain_item(
                    "trend min samples",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_trend_min_samples,
                    ),
                ),
                tui_plain_item("trend failure ewma", governor_trend_failure_ewma),
                tui_plain_item("trend backpressure ewma", governor_trend_backpressure_ewma),
                tui_plain_item(
                    "recovery success streak",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_recovery_success_streak,
                    ),
                ),
                tui_plain_item(
                    "recovery streak threshold",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_recovery_success_streak_threshold,
                    ),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("tool output".to_owned()),
            items: vec![
                tui_plain_item("snapshots", summary.tool_output_snapshots_seen.to_string()),
                tui_plain_item(
                    "truncated events",
                    summary.tool_output_truncated_events.to_string(),
                ),
                tui_plain_item(
                    "result lines total",
                    summary.tool_output_result_lines_total.to_string(),
                ),
                tui_plain_item(
                    "truncated result lines",
                    summary.tool_output_truncated_result_lines_total.to_string(),
                ),
                tui_plain_item("latest truncation ratio", latest_tool_truncation_ratio),
                tui_plain_item(
                    "aggregate truncation ratio",
                    aggregate_tool_truncation_ratio_text,
                ),
                tui_plain_item(
                    "aggregate truncation ratio milli",
                    format_fast_lane_summary_optional(aggregate_tool_truncation_ratio_milli),
                ),
                tui_plain_item(
                    "truncation verify failed",
                    summary
                        .tool_output_truncation_verify_failed_events
                        .to_string(),
                ),
                tui_plain_item(
                    "truncation replan",
                    summary.tool_output_truncation_replan_events.to_string(),
                ),
                tui_plain_item(
                    "truncation final failure",
                    summary
                        .tool_output_truncation_final_failure_events
                        .to_string(),
                ),
            ],
        },
        TuiSectionSpec::Callout {
            tone: health_tone,
            title: Some("health".to_owned()),
            lines: vec![
                format!("severity: {}", health_signal.severity),
                format!("flags: {health_flags}"),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("health events".to_owned()),
            items: vec![
                tui_plain_item(
                    "snapshots",
                    summary.health_signal_snapshots_seen.to_string(),
                ),
                tui_plain_item("warn events", summary.health_signal_warn_events.to_string()),
                tui_plain_item(
                    "critical events",
                    summary.health_signal_critical_events.to_string(),
                ),
                tui_plain_item("latest severity", latest_health_event_severity.to_owned()),
                tui_plain_item("latest flags", latest_health_event_flags),
            ],
        },
        latest_metrics_section,
        TuiSectionSpec::KeyValues {
            title: Some("rollups".to_owned()),
            items: vec![
                tui_csv_item("route decisions", rollup_route_decisions),
                tui_csv_item("route reasons", rollup_route_reasons),
                tui_csv_item("failure codes", rollup_failure_codes),
            ],
        },
    ];

    TuiMessageSpec {
        role: "safe-lane".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: Vec::new(),
    }
}

#[cfg(test)]
async fn load_safe_lane_summary_output(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<String> {
    let summary = load_safe_lane_event_summary(session_id, limit, binding, memory_config).await?;

    Ok(format_safe_lane_summary(
        session_id,
        limit,
        conversation_config,
        &summary,
    ))
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_turn_checkpoint_summary_lines_with_width(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_summary_message_spec(session_id, limit, diagnostics);
    render_tui_message_spec(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_summary_message_spec(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
) -> TuiMessageSpec {
    let summary = diagnostics.summary();
    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let failure_step = format_turn_checkpoint_failure_step(summary.latest_failure_step);
    let failure_error = summary.latest_failure_error.as_deref().unwrap_or("-");
    let reply_durable = bool_yes_no_value(summary.reply_durable);
    let checkpoint_durable = bool_yes_no_value(summary.checkpoint_durable);
    let recovery_needed = bool_yes_no_value(summary.requires_recovery);
    let recovery_tone = recovery_callout_tone(summary.requires_recovery);
    let stage_rollup_values = collect_rollup_values(&summary.stage_counts);
    let stage_rollups = csv_values_or_dash(stage_rollup_values);
    let caption = format!("session={session_id} limit={limit}");
    let mut sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("summary".to_owned()),
            items: vec![
                tui_plain_item("checkpoints", summary.checkpoint_events.to_string()),
                tui_plain_item("state", render_labels.session_state.to_owned()),
                tui_plain_item("durability", durability_labels.durability.to_owned()),
                tui_plain_item("reply durable", reply_durable),
                tui_plain_item("checkpoint durable", checkpoint_durable),
                tui_plain_item("requires recovery", recovery_needed),
            ],
        },
        TuiSectionSpec::Callout {
            tone: recovery_tone,
            title: Some("recovery".to_owned()),
            lines: vec![
                format!("action: {}", recovery_labels.action),
                format!("source: {}", recovery_labels.source),
                format!("reason: {}", recovery_labels.reason),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("latest checkpoint".to_owned()),
            items: vec![
                tui_plain_item("stage", render_labels.stage.to_owned()),
                tui_plain_item("after turn", render_labels.after_turn.to_owned()),
                tui_plain_item("compaction", render_labels.compaction.to_owned()),
                tui_plain_item("lane", render_labels.lane.to_owned()),
                tui_plain_item("result kind", render_labels.result_kind.to_owned()),
                tui_plain_item(
                    "persistence mode",
                    render_labels.persistence_mode.to_owned(),
                ),
                tui_plain_item("identity", render_labels.identity.to_owned()),
                tui_plain_item("failure step", failure_step.to_owned()),
                tui_plain_item("failure error", failure_error.to_owned()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("events".to_owned()),
            items: vec![
                tui_plain_item("post persist", summary.post_persist_events.to_string()),
                tui_plain_item("finalized", summary.finalized_events.to_string()),
                tui_plain_item(
                    "finalization failed",
                    summary.finalization_failed_events.to_string(),
                ),
                tui_plain_item(
                    "schema version",
                    format_fast_lane_summary_optional(summary.latest_schema_version),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("rollups".to_owned()),
            items: vec![tui_csv_item("stages", stage_rollups)],
        },
    ];

    if render_labels.safe_lane_route_decision != "-"
        || render_labels.safe_lane_route_reason != "-"
        || render_labels.safe_lane_route_source != "-"
    {
        sections.insert(
            3,
            TuiSectionSpec::KeyValues {
                title: Some("safe-lane route".to_owned()),
                items: vec![
                    tui_plain_item(
                        "decision",
                        render_labels.safe_lane_route_decision.to_owned(),
                    ),
                    tui_plain_item("reason", render_labels.safe_lane_route_reason.to_owned()),
                    tui_plain_item("source", render_labels.safe_lane_route_source.to_owned()),
                ],
            },
        );
    }

    if let Some(probe) = diagnostics.runtime_probe() {
        let probe_lines = vec![
            format!("action: {}", probe.action().as_str()),
            format!("source: {}", probe.source().as_str()),
            format!("reason: {}", probe.reason().as_str()),
        ];

        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("runtime probe".to_owned()),
            lines: probe_lines,
        });
    }

    TuiMessageSpec {
        role: "checkpoint".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: Vec::new(),
    }
}

#[cfg(test)]
async fn load_turn_checkpoint_summary_output(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongClawConfig,
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<String> {
    let diagnostics = turn_coordinator
        .load_turn_checkpoint_diagnostics_with_limit(config, session_id, limit, binding)
        .await?;

    Ok(format_turn_checkpoint_summary_output(
        session_id,
        limit,
        &diagnostics,
    ))
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_turn_checkpoint_repair_lines_with_width(
    session_id: &str,
    outcome: &TurnCheckpointTailRepairOutcome,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_repair_message_spec(session_id, outcome);
    render_tui_message_spec(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_repair_message_spec(
    session_id: &str,
    outcome: &TurnCheckpointTailRepairOutcome,
) -> TuiMessageSpec {
    let after_turn = outcome.after_turn_status().unwrap_or("-");
    let compaction = outcome.compaction_status().unwrap_or("-");
    let source = outcome.source().map(|value| value.as_str()).unwrap_or("-");
    let status = outcome.status();
    let (callout_tone, callout_lines) = match status {
        TurnCheckpointTailRepairStatus::Repaired => (
            TuiCalloutTone::Success,
            vec!["Repair completed and durable checkpoint state was updated.".to_owned()],
        ),
        TurnCheckpointTailRepairStatus::ManualRequired => (
            TuiCalloutTone::Warning,
            vec!["Manual inspection is still required before replaying the session.".to_owned()],
        ),
        TurnCheckpointTailRepairStatus::NotNeeded => (
            TuiCalloutTone::Success,
            vec!["No repair action was required for the latest durable checkpoint.".to_owned()],
        ),
        TurnCheckpointTailRepairStatus::NoCheckpoint => (
            TuiCalloutTone::Info,
            vec!["No durable checkpoint was available to repair.".to_owned()],
        ),
    };
    let caption = format!("session={session_id}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("repair status".to_owned()),
            items: vec![
                tui_plain_item("status", status.as_str().to_owned()),
                tui_plain_item("action", outcome.action().as_str().to_owned()),
                tui_plain_item("source", source.to_owned()),
                tui_plain_item("reason", outcome.reason().as_str().to_owned()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("checkpoint state".to_owned()),
            items: vec![
                tui_plain_item("session state", outcome.session_state().as_str().to_owned()),
                tui_plain_item("checkpoints", outcome.checkpoint_events().to_string()),
                tui_plain_item("after turn", after_turn.to_owned()),
                tui_plain_item("compaction", compaction.to_owned()),
            ],
        },
        TuiSectionSpec::Callout {
            tone: callout_tone,
            title: Some("repair result".to_owned()),
            lines: callout_lines,
        },
    ];

    TuiMessageSpec {
        role: "repair".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: Vec::new(),
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_fast_lane_summary_optional<T>(value: Option<T>) -> String
where
    T: std::fmt::Display,
{
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

#[cfg(test)]
fn format_fast_lane_segments(segments: &[FastLaneToolBatchSegmentSnapshot]) -> String {
    if segments.is_empty() {
        return "-".to_owned();
    }

    segments
        .iter()
        .map(|segment| {
            let observed_suffix = match (
                segment.observed_peak_in_flight,
                segment.observed_wall_time_ms,
            ) {
                (None, None) => String::new(),
                (observed_peak_in_flight, observed_wall_time_ms) => format!(
                    "[peak={} wall_ms={}]",
                    format_fast_lane_summary_optional(observed_peak_in_flight),
                    format_fast_lane_summary_optional(observed_wall_time_ms)
                ),
            };
            format!(
                "{}:{}/{}/{}{}",
                segment.segment_index,
                segment.scheduling_class,
                segment.execution_mode,
                segment.intent_count,
                observed_suffix,
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
fn format_fast_lane_summary(
    session_id: &str,
    limit: usize,
    summary: &FastLaneToolBatchEventSummary,
) -> String {
    let parallel_safe_ratio = format_ratio(
        summary.total_parallel_safe_intents_seen,
        summary.total_intents_seen,
    );
    let serial_only_ratio = format_ratio(
        summary.total_serial_only_intents_seen,
        summary.total_intents_seen,
    );
    let configured_max_in_flight_avg = format_average(
        summary.parallel_execution_max_in_flight_sum,
        summary.parallel_execution_max_in_flight_samples,
    );
    let observed_peak_in_flight_avg = format_average(
        summary.observed_peak_in_flight_sum,
        summary.observed_peak_in_flight_samples,
    );
    let observed_wall_time_ms_avg = format_average(
        summary.observed_wall_time_ms_sum,
        summary.observed_wall_time_ms_samples,
    );
    let scheduling_class_rollup = format_rollup_counts(&summary.scheduling_class_counts);
    let execution_mode_rollup = format_rollup_counts(&summary.execution_mode_counts);

    [
        format!("fast_lane_summary session={session_id} limit={limit}"),
        format!(
            "events batch_events={} schema_version={}",
            summary.batch_events,
            format_fast_lane_summary_optional(summary.latest_schema_version)
        ),
        format!(
            "aggregate_batches parallel_enabled={} parallel_only={} mixed={} sequential_only={} without_segments={}",
            summary.parallel_execution_enabled_batches,
            summary.parallel_only_batches,
            summary.mixed_execution_batches,
            summary.sequential_only_batches,
            summary.batches_without_segments,
        ),
        format!(
            "aggregate_intents total={} parallel_safe={} serial_only={} parallel_safe_ratio={} serial_only_ratio={}",
            summary.total_intents_seen,
            summary.total_parallel_safe_intents_seen,
            summary.total_serial_only_intents_seen,
            parallel_safe_ratio,
            serial_only_ratio,
        ),
        format!(
            "aggregate_segments parallel={} sequential={}",
            summary.total_parallel_segments_seen,
            summary.total_sequential_segments_seen,
        ),
        format!(
            "aggregate_execution configured_max_in_flight_avg={} configured_max_in_flight_max={} configured_max_in_flight_samples={} observed_peak_in_flight_avg={} observed_peak_in_flight_max={} observed_peak_in_flight_samples={} degraded_parallel_segments={}",
            configured_max_in_flight_avg,
            format_fast_lane_summary_optional(summary.parallel_execution_max_in_flight_max),
            summary.parallel_execution_max_in_flight_samples,
            observed_peak_in_flight_avg,
            format_fast_lane_summary_optional(summary.observed_peak_in_flight_max),
            summary.observed_peak_in_flight_samples,
            summary.degraded_parallel_segments,
        ),
        format!(
            "aggregate_latency observed_wall_time_ms_avg={} observed_wall_time_ms_max={} observed_wall_time_ms_samples={}",
            observed_wall_time_ms_avg,
            format_fast_lane_summary_optional(summary.observed_wall_time_ms_max),
            summary.observed_wall_time_ms_samples,
        ),
        format!("rollup scheduling_classes={scheduling_class_rollup}"),
        format!("rollup execution_modes={execution_mode_rollup}"),
        format!(
            "latest_batch total_intents={} parallel_enabled={} max_in_flight={} observed_peak_in_flight={} observed_wall_time_ms={} parallel_safe_intents={} serial_only_intents={} parallel_segments={} sequential_segments={}",
            format_fast_lane_summary_optional(summary.latest_total_intents),
            format_fast_lane_summary_optional(summary.latest_parallel_execution_enabled),
            format_fast_lane_summary_optional(summary.latest_parallel_execution_max_in_flight),
            format_fast_lane_summary_optional(summary.latest_observed_peak_in_flight),
            format_fast_lane_summary_optional(summary.latest_observed_wall_time_ms),
            format_fast_lane_summary_optional(summary.latest_parallel_safe_intents),
            format_fast_lane_summary_optional(summary.latest_serial_only_intents),
            format_fast_lane_summary_optional(summary.latest_parallel_segments),
            format_fast_lane_summary_optional(summary.latest_sequential_segments),
        ),
        format!(
            "latest_segments={}",
            format_fast_lane_segments(&summary.latest_segments)
        ),
    ]
    .join("\n")
}

#[cfg(test)]
fn format_safe_lane_summary(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
) -> String {
    let final_status = match summary.final_status {
        Some(SafeLaneFinalStatus::Succeeded) => "succeeded",
        Some(SafeLaneFinalStatus::Failed) => "failed",
        None => "unknown",
    };
    let final_failure_code = summary.final_failure_code.as_deref().unwrap_or("-");
    let final_route = summary.final_route_decision.as_deref().unwrap_or("-");
    let final_route_reason = summary.final_route_reason.as_deref().unwrap_or("-");
    let metrics = summary.latest_metrics.as_ref();
    let rounds_started = metrics
        .map(|value| value.rounds_started as f64)
        .unwrap_or(summary.round_started_events as f64);
    let replan_rate = if rounds_started > 0.0 {
        summary.replan_triggered_events as f64 / rounds_started
    } else {
        0.0
    };
    let verify_failure_rate = if rounds_started > 0.0 {
        summary.verify_failed_events as f64 / rounds_started
    } else {
        0.0
    };
    let route_rollup = format_rollup_counts(&summary.route_decision_counts);
    let route_reason_rollup = format_rollup_counts(&summary.route_reason_counts);
    let failure_rollup = format_rollup_counts(&summary.failure_code_counts);
    let governor_trend_failure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_failure_ewma_milli);
    let governor_trend_backpressure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_backpressure_ewma_milli);
    let latest_tool_truncation_ratio = format_milli_ratio(
        summary
            .latest_tool_output
            .as_ref()
            .map(|snapshot| snapshot.truncation_ratio_milli),
    );
    let aggregate_tool_truncation_ratio_milli = summary
        .tool_output_aggregate_truncation_ratio_milli
        .or_else(|| {
            if summary.tool_output_result_lines_total == 0 {
                None
            } else {
                Some(
                    summary
                        .tool_output_truncated_result_lines_total
                        .saturating_mul(1000)
                        .saturating_div(summary.tool_output_result_lines_total)
                        .min(u32::MAX as u64) as u32,
                )
            }
        });
    let aggregate_tool_truncation_ratio =
        aggregate_tool_truncation_ratio_milli.map(|milli| (milli as f64) / 1000.0);
    let aggregate_tool_truncation_ratio_text = aggregate_tool_truncation_ratio
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "-".to_owned());
    let health_signal = derive_safe_lane_health_signal(
        conversation_config,
        summary,
        replan_rate,
        verify_failure_rate,
        aggregate_tool_truncation_ratio,
    );
    let health_flags = if health_signal.flags.is_empty() {
        "-".to_owned()
    } else {
        health_signal.flags.join(",")
    };
    let health_payload = serde_json::json!({
        "severity": health_signal.severity,
        "flags": health_signal.flags,
    })
    .to_string();
    let latest_health_event_severity = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| snapshot.severity.as_str())
        .unwrap_or("-");
    let latest_health_event_flags = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| {
            if snapshot.flags.is_empty() {
                "-".to_owned()
            } else {
                snapshot.flags.join(",")
            }
        })
        .unwrap_or_else(|| "-".to_owned());

    let metrics_line = if let Some(metrics) = metrics {
        format!(
            "latest_metrics rounds_started={} rounds_succeeded={} rounds_failed={} verify_failures={} replans_triggered={} total_attempts_used={}",
            metrics.rounds_started,
            metrics.rounds_succeeded,
            metrics.rounds_failed,
            metrics.verify_failures,
            metrics.replans_triggered,
            metrics.total_attempts_used
        )
    } else {
        "latest_metrics unavailable".to_owned()
    };

    [
        format!("safe_lane_summary session={session_id} limit={limit}"),
        format!(
            "events lane_selected={} round_started={} round_completed_succeeded={} round_completed_failed={} verify_failed={} verify_policy_adjusted={} replan_triggered={} final_status={} governor_engaged={} governor_force_no_replan={}",
            summary.lane_selected_events,
            summary.round_started_events,
            summary.round_completed_succeeded_events,
            summary.round_completed_failed_events,
            summary.verify_failed_events,
            summary.verify_policy_adjusted_events,
            summary.replan_triggered_events,
            summary.final_status_events,
            summary.session_governor_engaged_events,
            summary.session_governor_force_no_replan_events
        ),
        format!(
            "terminal status={} failure_code={} route_decision={} route_reason={}",
            final_status, final_failure_code, final_route, final_route_reason
        ),
        format!(
            "governor trigger_failed_threshold={} trigger_backpressure_threshold={} trigger_trend_threshold={} trigger_recovery_threshold={}",
            summary.session_governor_failed_threshold_triggered_events,
            summary.session_governor_backpressure_threshold_triggered_events,
            summary.session_governor_trend_threshold_triggered_events,
            summary.session_governor_recovery_threshold_triggered_events
        ),
        format!(
            "governor_latest snapshots={} trend_samples={} trend_min_samples={} trend_failure_ewma={} trend_backpressure_ewma={} recovery_success_streak={} recovery_streak_threshold={}",
            summary.session_governor_metrics_snapshots_seen,
            summary
                .session_governor_latest_trend_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_trend_min_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            governor_trend_failure_ewma,
            governor_trend_backpressure_ewma,
            summary
                .session_governor_latest_recovery_success_streak
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_recovery_success_streak_threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
        ),
        format!(
            "rates replan_per_round={:.3} verify_fail_per_round={:.3}",
            replan_rate, verify_failure_rate
        ),
        format!(
            "tool_output snapshots={} truncated_events={} result_lines_total={} truncated_result_lines_total={} latest_truncation_ratio={} aggregate_truncation_ratio={} aggregate_truncation_ratio_milli={} truncation_verify_failed_events={} truncation_replan_events={} truncation_final_failure_events={}",
            summary.tool_output_snapshots_seen,
            summary.tool_output_truncated_events,
            summary.tool_output_result_lines_total,
            summary.tool_output_truncated_result_lines_total,
            latest_tool_truncation_ratio,
            aggregate_tool_truncation_ratio_text
            ,
            aggregate_tool_truncation_ratio_milli
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary.tool_output_truncation_verify_failed_events,
            summary.tool_output_truncation_replan_events,
            summary.tool_output_truncation_final_failure_events
        ),
        format!(
            "health severity={} flags={health_flags}",
            health_signal.severity
        ),
        format!("health_payload {health_payload}"),
        format!(
            "health_events snapshots={} warn={} critical={} latest_severity={} latest_flags={}",
            summary.health_signal_snapshots_seen,
            summary.health_signal_warn_events,
            summary.health_signal_critical_events,
            latest_health_event_severity,
            latest_health_event_flags
        ),
        metrics_line,
        format!("rollup route_decisions={route_rollup}"),
        format!("rollup route_reasons={route_reason_rollup}"),
        format!("rollup failure_codes={failure_rollup}"),
    ]
    .join("\n")
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_stage(stage: Option<TurnCheckpointStage>) -> &'static str {
    match stage {
        Some(TurnCheckpointStage::PostPersist) => "post_persist",
        Some(TurnCheckpointStage::Finalized) => "finalized",
        Some(TurnCheckpointStage::FinalizationFailed) => "finalization_failed",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_progress(status: Option<TurnCheckpointProgressStatus>) -> &'static str {
    match status {
        Some(TurnCheckpointProgressStatus::Pending) => "pending",
        Some(TurnCheckpointProgressStatus::Skipped) => "skipped",
        Some(TurnCheckpointProgressStatus::Completed) => "completed",
        Some(TurnCheckpointProgressStatus::Failed) => "failed",
        Some(TurnCheckpointProgressStatus::FailedOpen) => "failed_open",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_failure_step(step: Option<TurnCheckpointFailureStep>) -> &'static str {
    match step {
        Some(TurnCheckpointFailureStep::AfterTurn) => "after_turn",
        Some(TurnCheckpointFailureStep::Compaction) => "compaction",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_identity_presence(identity_present: Option<bool>) -> &'static str {
    match identity_present {
        Some(true) => "present",
        Some(false) => "missing",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_session_state(state: TurnCheckpointSessionState) -> &'static str {
    match state {
        TurnCheckpointSessionState::NotDurable => "not_durable",
        TurnCheckpointSessionState::PendingFinalization => "pending_finalization",
        TurnCheckpointSessionState::Finalized => "finalized",
        TurnCheckpointSessionState::FinalizationFailed => "finalization_failed",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_recovery_action(action: TurnCheckpointRecoveryAction) -> &'static str {
    action.as_str()
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_recovery_reason(
    reason: Option<TurnCheckpointTailRepairReason>,
) -> &'static str {
    reason
        .map(TurnCheckpointTailRepairReason::as_str)
        .unwrap_or("-")
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TurnCheckpointRecoveryRenderLabels {
    action: &'static str,
    source: &'static str,
    reason: &'static str,
}

#[cfg(any(test, feature = "memory-sqlite"))]
impl TurnCheckpointRecoveryRenderLabels {
    fn from_assessment(assessment: TurnCheckpointRecoveryAssessment) -> Self {
        Self {
            action: format_turn_checkpoint_recovery_action(assessment.action()),
            source: assessment.source().as_str(),
            reason: format_turn_checkpoint_recovery_reason(assessment.reason()),
        }
    }

    #[cfg(test)]
    fn from_outcome(outcome: &TurnCheckpointTailRepairOutcome) -> Self {
        Self {
            action: outcome.action().as_str(),
            source: outcome.source().map(|value| value.as_str()).unwrap_or("-"),
            reason: outcome.reason().as_str(),
        }
    }

    #[cfg(test)]
    fn from_probe(probe: &TurnCheckpointTailRepairRuntimeProbe) -> Self {
        Self {
            action: probe.action().as_str(),
            source: probe.source().as_str(),
            reason: probe.reason().as_str(),
        }
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TurnCheckpointSummaryRenderLabels<'a> {
    session_state: &'static str,
    stage: &'static str,
    after_turn: &'static str,
    compaction: &'static str,
    lane: &'a str,
    result_kind: &'a str,
    persistence_mode: &'a str,
    safe_lane_route_decision: &'static str,
    safe_lane_route_reason: &'static str,
    safe_lane_route_source: &'static str,
    identity: &'static str,
}

#[cfg(any(test, feature = "memory-sqlite"))]
impl<'a> TurnCheckpointSummaryRenderLabels<'a> {
    fn from_summary(summary: &'a TurnCheckpointEventSummary) -> Self {
        let (safe_lane_route_decision, safe_lane_route_reason, safe_lane_route_source) =
            summary.latest_safe_lane_route_labels_or_default();
        Self {
            session_state: format_turn_checkpoint_session_state(summary.session_state),
            stage: format_turn_checkpoint_stage(summary.latest_stage),
            after_turn: format_turn_checkpoint_progress(summary.latest_after_turn),
            compaction: format_turn_checkpoint_progress(summary.latest_compaction),
            lane: summary.latest_lane.as_deref().unwrap_or("-"),
            result_kind: summary.latest_result_kind.as_deref().unwrap_or("-"),
            persistence_mode: summary.latest_persistence_mode.as_deref().unwrap_or("-"),
            safe_lane_route_decision,
            safe_lane_route_reason,
            safe_lane_route_source,
            identity: format_turn_checkpoint_identity_presence(summary.latest_identity_present),
        }
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TurnCheckpointDurabilityRenderLabels {
    checkpoint_durable: u8,
    reply_durable: u8,
    durability: &'static str,
}

#[cfg(any(test, feature = "memory-sqlite"))]
impl TurnCheckpointDurabilityRenderLabels {
    fn from_summary(summary: &TurnCheckpointEventSummary) -> Self {
        let checkpoint_durable = u8::from(summary.checkpoint_durable);
        let reply_durable = u8::from(summary.reply_durable);
        let durability = if checkpoint_durable == 0 {
            "not_durable"
        } else if reply_durable == 1 {
            "reply"
        } else {
            "checkpoint_only"
        };
        Self {
            checkpoint_durable,
            reply_durable,
            durability,
        }
    }
}

#[cfg(test)]
fn format_turn_checkpoint_summary(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
) -> String {
    let summary = diagnostics.summary();
    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let failure_step = format_turn_checkpoint_failure_step(summary.latest_failure_step);
    let requires_recovery = if summary.requires_recovery { 1 } else { 0 };
    let failure_error = summary.latest_failure_error.as_deref().unwrap_or("-");

    let mut lines = vec![format!(
        "turn_checkpoint_summary session={session_id} limit={limit} checkpoints={} state={} durable={} checkpoint_durable={} durability={} requires_recovery={requires_recovery} recovery_action={} recovery_source={} recovery_reason={} stage={} after_turn={} compaction={} lane={} result_kind={} persistence_mode={} safe_lane_route_decision={} safe_lane_route_reason={} safe_lane_route_source={} identity={} failure_step={failure_step} failure_error={failure_error}",
        summary.checkpoint_events,
        render_labels.session_state,
        durability_labels.reply_durable,
        durability_labels.checkpoint_durable,
        durability_labels.durability,
        recovery_labels.action,
        recovery_labels.source,
        recovery_labels.reason,
        render_labels.stage,
        render_labels.after_turn,
        render_labels.compaction,
        render_labels.lane,
        render_labels.result_kind,
        render_labels.persistence_mode,
        render_labels.safe_lane_route_decision,
        render_labels.safe_lane_route_reason,
        render_labels.safe_lane_route_source,
        render_labels.identity,
    )];
    lines.push(format!(
        "events post_persist={} finalized={} finalization_failed={}",
        summary.post_persist_events, summary.finalized_events, summary.finalization_failed_events
    ));
    if !summary.stage_counts.is_empty() {
        let stage_rollup = summary
            .stage_counts
            .iter()
            .map(|(stage_name, count)| format!("{stage_name}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!("rollup stages={stage_rollup}"));
    }
    lines.join("\n")
}

#[cfg(test)]
fn format_turn_checkpoint_summary_output(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
) -> String {
    let mut rendered = format_turn_checkpoint_summary(session_id, limit, diagnostics);
    if let Some(probe) = diagnostics.runtime_probe() {
        rendered.push('\n');
        rendered.push_str(&format_turn_checkpoint_runtime_probe(session_id, probe));
    }
    rendered
}

#[cfg(test)]
fn format_turn_checkpoint_startup_health(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
) -> Option<String> {
    let summary = diagnostics.summary();
    if !summary.checkpoint_durable {
        return None;
    }

    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let recovery_needed = if summary.requires_recovery { 1 } else { 0 };

    Some(format!(
        "turn_checkpoint_health session={session_id} state={} reply_durable={} checkpoint_durable={} durability={} recovery_needed={recovery_needed} action={} source={} reason={} stage={} after_turn={} compaction={} lane={} result_kind={} persistence_mode={} safe_lane_route_decision={} safe_lane_route_reason={} safe_lane_route_source={} identity={}",
        render_labels.session_state,
        durability_labels.reply_durable,
        durability_labels.checkpoint_durable,
        durability_labels.durability,
        recovery_labels.action,
        recovery_labels.source,
        recovery_labels.reason,
        render_labels.stage,
        render_labels.after_turn,
        render_labels.compaction,
        render_labels.lane,
        render_labels.result_kind,
        render_labels.persistence_mode,
        render_labels.safe_lane_route_decision,
        render_labels.safe_lane_route_reason,
        render_labels.safe_lane_route_source,
        render_labels.identity,
    ))
}

#[cfg(test)]
fn format_turn_checkpoint_repair(
    session_id: &str,
    outcome: &TurnCheckpointTailRepairOutcome,
) -> String {
    let after_turn = outcome.after_turn_status().unwrap_or("-");
    let compaction = outcome.compaction_status().unwrap_or("-");
    let render_labels = TurnCheckpointRecoveryRenderLabels::from_outcome(outcome);
    format!(
        "turn_checkpoint_repair session={session_id} status={} action={} source={} reason={} state={} checkpoints={} after_turn={after_turn} compaction={compaction}",
        outcome.status().as_str(),
        render_labels.action,
        render_labels.source,
        render_labels.reason,
        outcome.session_state().as_str(),
        outcome.checkpoint_events(),
    )
}

#[cfg(test)]
fn format_turn_checkpoint_runtime_probe(
    session_id: &str,
    probe: &TurnCheckpointTailRepairRuntimeProbe,
) -> String {
    let render_labels = TurnCheckpointRecoveryRenderLabels::from_probe(probe);
    format!(
        "turn_checkpoint_probe session={session_id} action={} source={} reason={}",
        render_labels.action, render_labels.source, render_labels.reason,
    )
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn derive_safe_lane_health_signal(
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
    replan_rate: f64,
    verify_failure_rate: f64,
    aggregate_truncation_ratio: Option<f64>,
) -> SafeLaneHealthSignal {
    let mut flags = Vec::new();
    let mut has_critical = false;
    let truncation_warn_threshold =
        conversation_config.safe_lane_health_truncation_warn_threshold();
    let truncation_critical_threshold =
        conversation_config.safe_lane_health_truncation_critical_threshold();
    let verify_failure_warn_threshold =
        conversation_config.safe_lane_health_verify_failure_warn_threshold();
    let replan_warn_threshold = conversation_config.safe_lane_health_replan_warn_threshold();

    if let Some(ratio) = aggregate_truncation_ratio {
        if ratio >= truncation_critical_threshold {
            flags.push(format!("truncation_severe({ratio:.3})"));
            has_critical = true;
        } else if ratio >= truncation_warn_threshold {
            flags.push(format!("truncation_pressure({ratio:.3})"));
        }
    }
    if verify_failure_rate >= verify_failure_warn_threshold {
        flags.push(format!("verify_failure_pressure({verify_failure_rate:.3})"));
    }
    if replan_rate >= replan_warn_threshold {
        flags.push(format!("replan_pressure({replan_rate:.3})"));
    }
    let terminal_instability = summary.has_terminal_instability_final_failure();
    if terminal_instability {
        flags.push("terminal_instability".to_owned());
        has_critical = true;
    }

    SafeLaneHealthSignal {
        severity: if has_critical {
            "critical"
        } else if flags.is_empty() {
            "ok"
        } else {
            "warn"
        },
        flags,
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SafeLaneHealthSignal {
    severity: &'static str,
    flags: Vec<String>,
}

#[cfg(test)]
fn format_rollup_counts(counts: &std::collections::BTreeMap<String, u32>) -> String {
    if counts.is_empty() {
        return "-".to_owned();
    }
    counts
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_milli_ratio(value: Option<u32>) -> String {
    value
        .map(|raw| format!("{:.3}", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_ratio(numerator: u64, denominator: u64) -> String {
    if denominator == 0 {
        return "-".to_owned();
    }
    format!("{:.3}", numerator as f64 / denominator as f64)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_average(sum: u64, samples: u32) -> String {
    if samples == 0 {
        return "-".to_owned();
    }
    format!("{:.3}", sum as f64 / f64::from(samples))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ConversationRuntimeBinding;
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::sync::Arc;
    #[cfg(feature = "memory-sqlite")]
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Mutex,
    };

    #[cfg(feature = "memory-sqlite")]
    use async_trait::async_trait;
    #[cfg(feature = "memory-sqlite")]
    use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind, MemoryPlaneError};
    #[cfg(feature = "memory-sqlite")]
    use loongclaw_kernel::{
        CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, MemoryCoreOutcome,
        MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
    };
    #[cfg(feature = "memory-sqlite")]
    use serde_json::{Value, json};
    #[test]
    fn cli_chat_options_detect_explicit_acp_requests() {
        assert!(
            CliChatOptions {
                acp_requested: true,
                ..CliChatOptions::default()
            }
            .requests_explicit_acp()
        );

        assert!(
            CliChatOptions {
                acp_bootstrap_mcp_servers: vec!["filesystem".to_owned()],
                ..CliChatOptions::default()
            }
            .requests_explicit_acp()
        );

        assert!(
            CliChatOptions {
                acp_working_directory: Some(PathBuf::from("/workspace/project")),
                ..CliChatOptions::default()
            }
            .requests_explicit_acp()
        );
    }

    #[test]
    fn cli_chat_options_keep_automatic_routing_without_explicit_acp_inputs() {
        assert!(!CliChatOptions::default().requests_explicit_acp());
    }

    #[test]
    fn build_onboard_command_defaults_to_current_executable() {
        let expected_executable = std::env::current_exe().expect("current executable");
        let command =
            build_onboard_command(None, Path::new("/tmp/loongclaw.toml")).expect("onboard command");

        assert_eq!(command.get_program(), expected_executable.as_os_str());
        assert_eq!(
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["onboard".to_owned()]
        );
    }

    #[test]
    fn build_onboard_command_forwards_explicit_config_path_to_output() {
        let command = build_onboard_command_for_executable(
            PathBuf::from("/tmp/loongclaw"),
            Some("custom.toml"),
            Path::new("/tmp/custom.toml"),
        );

        assert_eq!(command.get_program(), OsStr::new("/tmp/loongclaw"));
        assert_eq!(
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "onboard".to_owned(),
                "--output".to_owned(),
                "/tmp/custom.toml".to_owned()
            ]
        );
    }

    #[test]
    fn onboard_command_hint_preserves_explicit_config_path() {
        let hint = format_onboard_command_hint(Some("custom.toml"), Path::new("/tmp/custom.toml"));

        assert_eq!(hint, "loongclaw onboard --output /tmp/custom.toml");
    }

    #[cfg(feature = "memory-sqlite")]
    fn unique_chat_sqlite_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-chat-binding-{label}-{}.sqlite3",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    fn cleanup_chat_test_memory(sqlite_path: &Path) {
        let _ = std::fs::remove_file(sqlite_path);
        let _ = std::fs::remove_file(format!("{}-wal", sqlite_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", sqlite_path.display()));
    }

    #[cfg(feature = "memory-sqlite")]
    fn init_chat_test_memory(label: &str) -> (LoongClawConfig, MemoryRuntimeConfig, PathBuf) {
        let sqlite_path = unique_chat_sqlite_path(label);
        cleanup_chat_test_memory(&sqlite_path);

        let mut config = LoongClawConfig::default();
        config.memory.sqlite_path = sqlite_path.display().to_string();
        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        crate::memory::ensure_memory_db_ready(
            Some(config.memory.resolved_sqlite_path()),
            &memory_config,
        )
        .expect("initialize sqlite memory");

        (config, memory_config, sqlite_path)
    }

    #[cfg(feature = "memory-sqlite")]
    struct SharedTestMemoryAdapter {
        invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
        status: String,
        window_turns: Value,
    }

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl CoreMemoryAdapter for SharedTestMemoryAdapter {
        fn name(&self) -> &str {
            "chat-binding-memory-shared"
        }

        async fn execute_core_memory(
            &self,
            request: MemoryCoreRequest,
        ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
            let payload = if request.operation == crate::memory::MEMORY_OP_WINDOW {
                json!({
                    "turns": self.window_turns.clone()
                })
            } else {
                json!({})
            };
            self.invocations
                .lock()
                .expect("invocations lock")
                .push(request);
            Ok(MemoryCoreOutcome {
                status: self.status.clone(),
                payload,
            })
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn build_kernel_context_with_window_turns(
        window_turns: Value,
    ) -> (crate::KernelContext, Arc<Mutex<Vec<MemoryCoreRequest>>>) {
        build_kernel_context_with_window_outcome("ok", window_turns)
    }

    #[cfg(feature = "memory-sqlite")]
    fn build_kernel_context_with_window_outcome(
        status: &str,
        window_turns: Value,
    ) -> (crate::KernelContext, Arc<Mutex<Vec<MemoryCoreRequest>>>) {
        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

        let pack = VerticalPackManifest {
            pack_id: "chat-test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::MemoryRead, Capability::MemoryWrite]),
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");

        let invocations = Arc::new(Mutex::new(Vec::new()));
        kernel.register_core_memory_adapter(SharedTestMemoryAdapter {
            invocations: invocations.clone(),
            status: status.to_owned(),
            window_turns,
        });
        kernel
            .set_default_core_memory_adapter("chat-binding-memory-shared")
            .expect("set default memory adapter");

        let token = kernel
            .issue_token("chat-test-pack", "chat-test-agent", 3600)
            .expect("issue token");

        let ctx = crate::KernelContext {
            kernel: Arc::new(kernel),
            token,
        };
        (ctx, invocations)
    }

    #[cfg(feature = "memory-sqlite")]
    fn append_assistant_payloads(
        session_id: &str,
        payloads: &[String],
        memory_config: &MemoryRuntimeConfig,
    ) {
        for payload in payloads {
            crate::memory::append_turn_direct(session_id, "assistant", payload, memory_config)
                .expect("persist assistant payload");
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn assistant_window_turns(payloads: &[String]) -> Value {
        json!(
            payloads
                .iter()
                .enumerate()
                .map(|(index, payload)| json!({
                    "role": "assistant",
                    "content": payload,
                    "ts": index as i64 + 1
                }))
                .collect::<Vec<_>>()
        )
    }

    #[cfg(feature = "memory-sqlite")]
    fn safe_lane_event_payloads() -> Vec<String> {
        vec![
            json!({
                "type": "conversation_event",
                "event": "plan_round_started",
                "payload": {
                    "round": 0
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "verify_failed",
                "payload": {
                    "failure_code": "safe_lane_plan_verify_failed"
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "failed",
                    "failure_code": "safe_lane_plan_verify_failed",
                    "route_decision": "terminal"
                }
            })
            .to_string(),
        ]
    }

    #[cfg(feature = "memory-sqlite")]
    fn turn_checkpoint_event_payloads() -> Vec<String> {
        vec![
            json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "post_persist",
                    "checkpoint": {
                        "lane": {
                            "lane": "safe",
                            "result_kind": "tool_call"
                        },
                        "finalization": {
                            "persistence_mode": "success"
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "pending",
                        "compaction": "pending"
                    },
                    "failure": null
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "finalized",
                    "checkpoint": {
                        "lane": {
                            "lane": "safe",
                            "result_kind": "tool_call"
                        },
                        "finalization": {
                            "persistence_mode": "success"
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "completed",
                        "compaction": "skipped"
                    },
                    "failure": null
                }
            })
            .to_string(),
        ]
    }

    #[cfg(feature = "memory-sqlite")]
    fn fast_lane_tool_batch_event_payloads() -> Vec<String> {
        vec![
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 2,
                    "total_intents": 5,
                    "parallel_execution_enabled": true,
                    "parallel_execution_max_in_flight": 2,
                    "observed_peak_in_flight": 2,
                    "observed_wall_time_ms": 34,
                    "parallel_safe_intents": 4,
                    "serial_only_intents": 1,
                    "parallel_segments": 2,
                    "sequential_segments": 1,
                    "segments": [
                        {
                            "segment_index": 0,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 2,
                            "observed_peak_in_flight": 2,
                            "observed_wall_time_ms": 14
                        },
                        {
                            "segment_index": 1,
                            "scheduling_class": "serial_only",
                            "execution_mode": "sequential",
                            "intent_count": 1,
                            "observed_peak_in_flight": 1,
                            "observed_wall_time_ms": 8
                        },
                        {
                            "segment_index": 2,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 2,
                            "observed_peak_in_flight": 2,
                            "observed_wall_time_ms": 12
                        }
                    ]
                }
            })
            .to_string(),
        ]
    }

    #[cfg(feature = "memory-sqlite")]
    fn legacy_fast_lane_tool_batch_event_payloads() -> Vec<String> {
        vec![
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 1,
                    "total_intents": 3,
                    "parallel_execution_enabled": true,
                    "parallel_execution_max_in_flight": 2,
                    "parallel_safe_intents": 2,
                    "serial_only_intents": 1,
                    "parallel_segments": 1,
                    "sequential_segments": 1,
                    "segments": [
                        {
                            "segment_index": 0,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 2
                        },
                        {
                            "segment_index": 1,
                            "scheduling_class": "serial_only",
                            "execution_mode": "sequential",
                            "intent_count": 1
                        }
                    ]
                }
            })
            .to_string(),
        ]
    }

    #[tokio::test]
    async fn run_cli_ask_rejects_empty_message() {
        let error = run_cli_ask(None, None, "   ", &CliChatOptions::default())
            .await
            .expect_err("empty one-shot message should fail");

        assert!(error.contains("ask message must not be empty"));
    }

    #[test]
    fn concurrent_cli_host_requires_explicit_session_id() {
        let shutdown = ConcurrentCliShutdown::new();
        let error = run_concurrent_cli_host(&ConcurrentCliHostOptions {
            resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
            config: LoongClawConfig::default(),
            session_id: "   ".to_owned(),
            shutdown,
            initialize_runtime_environment: false,
        })
        .expect_err("concurrent host should reject an implicit session id");

        assert!(
            error.contains("explicit session"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    #[cfg(feature = "memory-sqlite")]
    async fn concurrent_cli_host_exits_when_shutdown_is_requested() {
        let (mut config, _memory_config, sqlite_path) = init_chat_test_memory("concurrent-host");
        config.audit.mode = crate::config::AuditMode::InMemory;
        let options = CliChatOptions::default();
        let runtime = initialize_cli_turn_runtime_with_loaded_config(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("cli-supervisor"),
            &options,
            "cli-chat-concurrent-test",
            CliSessionRequirement::RequireExplicit,
            false,
        )
        .expect("concurrent host runtime");
        let shutdown = ConcurrentCliShutdown::new();
        shutdown.request_shutdown();

        run_concurrent_cli_host_loop(&runtime, &options, &shutdown)
            .await
            .expect("concurrent host should stop cleanly when shutdown is requested");

        cleanup_chat_test_memory(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn print_history_accepts_explicit_runtime_binding() {
        let (config, memory_config, sqlite_path) = init_chat_test_memory("diagnostics");

        let session_id = "chat-binding-history-direct";
        crate::memory::append_turn_direct(session_id, "user", "hello", &memory_config)
            .expect("persist user turn");
        crate::memory::append_turn_direct(session_id, "assistant", "world", &memory_config)
            .expect("persist assistant turn");

        let direct_lines = load_history_lines(
            session_id,
            config.memory.sliding_window,
            ConversationRuntimeBinding::direct(),
            &memory_config,
        )
        .await
        .expect("load history lines with explicit direct binding");
        assert_eq!(
            direct_lines,
            vec!["user: hello".to_owned(), "assistant: world".to_owned()]
        );

        let (kernel_ctx, invocations) = build_kernel_context_with_window_turns(json!([
            {
                "role": "user",
                "content": "kernel hello",
                "ts": 7
            },
            {
                "role": "assistant",
                "content": "kernel world",
                "ts": 8
            }
        ]));
        let kernel_lines = load_history_lines(
            "chat-binding-history-kernel",
            16,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            &memory_config,
        )
        .await
        .expect("load history lines with explicit kernel binding");
        assert_eq!(
            kernel_lines,
            vec![
                "[7] user: kernel hello".to_owned(),
                "[8] assistant: kernel world".to_owned()
            ]
        );

        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
        assert_eq!(
            captured[0].payload["session_id"],
            "chat-binding-history-kernel"
        );
        assert_eq!(captured[0].payload["limit"], json!(16));

        cleanup_chat_test_memory(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn print_history_rejects_non_ok_kernel_memory_outcome() {
        let (_config, memory_config, sqlite_path) = init_chat_test_memory("diagnostics-non-ok");

        let (kernel_ctx, invocations) = build_kernel_context_with_window_outcome(
            "error",
            json!([
                {
                    "role": "user",
                    "content": "kernel hello",
                    "ts": 7
                }
            ]),
        );
        let error = load_history_lines(
            "chat-binding-history-kernel-non-ok",
            16,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            &memory_config,
        )
        .await
        .expect_err("non-ok kernel memory outcome should fail closed");
        assert!(error.contains("non-ok status"), "unexpected error: {error}");
        assert!(error.contains("error"), "unexpected error: {error}");

        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
        assert_eq!(
            captured[0].payload["session_id"],
            "chat-binding-history-kernel-non-ok"
        );
        assert_eq!(captured[0].payload["limit"], json!(16));

        cleanup_chat_test_memory(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn safe_lane_summary_output_accepts_explicit_runtime_binding() {
        let (config, memory_config, sqlite_path) = init_chat_test_memory("safe-lane-output");

        let direct_payloads = safe_lane_event_payloads();
        append_assistant_payloads(
            "chat-binding-safe-lane-direct",
            &direct_payloads,
            &memory_config,
        );
        let direct_output = load_safe_lane_summary_output(
            "chat-binding-safe-lane-direct",
            64,
            &config.conversation,
            ConversationRuntimeBinding::direct(),
            &memory_config,
        )
        .await
        .expect("load safe lane summary via direct binding");
        assert!(
            direct_output
                .contains("safe_lane_summary session=chat-binding-safe-lane-direct limit=64")
        );
        assert!(direct_output.contains("round_started=1"));
        assert!(direct_output.contains("verify_failed=1"));
        assert!(direct_output.contains("failure_code=safe_lane_plan_verify_failed"));

        let kernel_payloads = safe_lane_event_payloads();
        let (kernel_ctx, invocations) =
            build_kernel_context_with_window_turns(assistant_window_turns(&kernel_payloads));
        let kernel_output = load_safe_lane_summary_output(
            "chat-binding-safe-lane-kernel",
            80,
            &config.conversation,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            &memory_config,
        )
        .await
        .expect("load safe lane summary via kernel binding");
        assert!(
            kernel_output
                .contains("safe_lane_summary session=chat-binding-safe-lane-kernel limit=80")
        );
        assert!(kernel_output.contains("round_started=1"));
        assert!(kernel_output.contains("verify_failed=1"));
        assert!(kernel_output.contains("failure_code=safe_lane_plan_verify_failed"));

        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
        assert_eq!(
            captured[0].payload["session_id"],
            "chat-binding-safe-lane-kernel"
        );
        assert_eq!(captured[0].payload["limit"], json!(80));
        assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

        cleanup_chat_test_memory(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fast_lane_summary_output_accepts_explicit_runtime_binding() {
        let (_config, memory_config, sqlite_path) = init_chat_test_memory("fast-lane-output");

        let direct_payloads = fast_lane_tool_batch_event_payloads();
        append_assistant_payloads(
            "chat-binding-fast-lane-direct",
            &direct_payloads,
            &memory_config,
        );
        let direct_output = load_fast_lane_summary_output(
            "chat-binding-fast-lane-direct",
            72,
            ConversationRuntimeBinding::direct(),
            &memory_config,
        )
        .await
        .expect("load fast lane summary via direct binding");
        assert!(
            direct_output
                .contains("fast_lane_summary session=chat-binding-fast-lane-direct limit=72")
        );
        assert!(direct_output.contains("batch_events=1"));
        assert!(direct_output.contains("total_intents=5"));
        assert!(direct_output.contains("parallel_safe_intents=4"));
        assert!(direct_output.contains(
            "aggregate_batches parallel_enabled=1 parallel_only=0 mixed=1 sequential_only=0 without_segments=0"
        ));
        assert!(direct_output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=2 configured_max_in_flight_samples=1 observed_peak_in_flight_avg=2.000 observed_peak_in_flight_max=2 observed_peak_in_flight_samples=1 degraded_parallel_segments=0"
        ));
        assert!(direct_output.contains(
            "aggregate_latency observed_wall_time_ms_avg=34.000 observed_wall_time_ms_max=34 observed_wall_time_ms_samples=1"
        ));
        assert!(direct_output.contains(
            "latest_batch total_intents=5 parallel_enabled=true max_in_flight=2 observed_peak_in_flight=2 observed_wall_time_ms=34 parallel_safe_intents=4 serial_only_intents=1 parallel_segments=2 sequential_segments=1"
        ));
        assert!(direct_output.contains(
            "latest_segments=0:parallel_safe/parallel/2[peak=2 wall_ms=14],1:serial_only/sequential/1[peak=1 wall_ms=8],2:parallel_safe/parallel/2[peak=2 wall_ms=12]"
        ));

        let kernel_payloads = fast_lane_tool_batch_event_payloads();
        let (kernel_ctx, invocations) =
            build_kernel_context_with_window_turns(assistant_window_turns(&kernel_payloads));
        let kernel_output = load_fast_lane_summary_output(
            "chat-binding-fast-lane-kernel",
            88,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            &memory_config,
        )
        .await
        .expect("load fast lane summary via kernel binding");
        assert!(
            kernel_output
                .contains("fast_lane_summary session=chat-binding-fast-lane-kernel limit=88")
        );
        assert!(kernel_output.contains("batch_events=1"));
        assert!(kernel_output.contains("total_intents=5"));
        assert!(kernel_output.contains("parallel_safe_intents=4"));
        assert!(kernel_output.contains(
            "aggregate_batches parallel_enabled=1 parallel_only=0 mixed=1 sequential_only=0 without_segments=0"
        ));
        assert!(kernel_output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=2 configured_max_in_flight_samples=1 observed_peak_in_flight_avg=2.000 observed_peak_in_flight_max=2 observed_peak_in_flight_samples=1 degraded_parallel_segments=0"
        ));
        assert!(kernel_output.contains(
            "aggregate_latency observed_wall_time_ms_avg=34.000 observed_wall_time_ms_max=34 observed_wall_time_ms_samples=1"
        ));
        assert!(kernel_output.contains(
            "latest_batch total_intents=5 parallel_enabled=true max_in_flight=2 observed_peak_in_flight=2 observed_wall_time_ms=34 parallel_safe_intents=4 serial_only_intents=1 parallel_segments=2 sequential_segments=1"
        ));
        assert!(kernel_output.contains(
            "latest_segments=0:parallel_safe/parallel/2[peak=2 wall_ms=14],1:serial_only/sequential/1[peak=1 wall_ms=8],2:parallel_safe/parallel/2[peak=2 wall_ms=12]"
        ));

        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
        assert_eq!(
            captured[0].payload["session_id"],
            "chat-binding-fast-lane-kernel"
        );
        assert_eq!(captured[0].payload["limit"], json!(88));
        assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

        cleanup_chat_test_memory(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fast_lane_summary_output_accepts_legacy_schema_v1_events() {
        let (_config, memory_config, sqlite_path) = init_chat_test_memory("fast-lane-legacy");

        let payloads = legacy_fast_lane_tool_batch_event_payloads();
        append_assistant_payloads("chat-binding-fast-lane-legacy", &payloads, &memory_config);

        let output = load_fast_lane_summary_output(
            "chat-binding-fast-lane-legacy",
            32,
            ConversationRuntimeBinding::direct(),
            &memory_config,
        )
        .await
        .expect("load fast lane summary for legacy schema");

        assert!(output.contains("schema_version=1"));
        assert!(output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=2 configured_max_in_flight_samples=1 observed_peak_in_flight_avg=- observed_peak_in_flight_max=- observed_peak_in_flight_samples=0 degraded_parallel_segments=0"
        ));
        assert!(output.contains(
            "aggregate_latency observed_wall_time_ms_avg=- observed_wall_time_ms_max=- observed_wall_time_ms_samples=0"
        ));
        assert!(output.contains(
            "latest_batch total_intents=3 parallel_enabled=true max_in_flight=2 observed_peak_in_flight=- observed_wall_time_ms=- parallel_safe_intents=2 serial_only_intents=1 parallel_segments=1 sequential_segments=1"
        ));
        assert!(
            output
                .contains("latest_segments=0:parallel_safe/parallel/2,1:serial_only/sequential/1")
        );

        cleanup_chat_test_memory(&sqlite_path);
    }

    #[test]
    fn format_fast_lane_summary_includes_window_aggregates() {
        let summary = FastLaneToolBatchEventSummary {
            batch_events: 4,
            latest_schema_version: Some(2),
            latest_total_intents: Some(0),
            latest_parallel_execution_enabled: Some(false),
            latest_parallel_execution_max_in_flight: None,
            latest_observed_peak_in_flight: Some(1),
            latest_observed_wall_time_ms: Some(11),
            latest_parallel_safe_intents: Some(0),
            latest_serial_only_intents: Some(0),
            latest_parallel_segments: Some(0),
            latest_sequential_segments: Some(0),
            latest_segments: Vec::new(),
            parallel_execution_enabled_batches: 2,
            parallel_only_batches: 1,
            mixed_execution_batches: 1,
            sequential_only_batches: 1,
            batches_without_segments: 1,
            total_intents_seen: 8,
            total_parallel_safe_intents_seen: 5,
            total_serial_only_intents_seen: 3,
            total_parallel_segments_seen: 3,
            total_sequential_segments_seen: 3,
            parallel_execution_max_in_flight_samples: 3,
            parallel_execution_max_in_flight_sum: 6,
            parallel_execution_max_in_flight_max: Some(3),
            observed_peak_in_flight_samples: 3,
            observed_peak_in_flight_sum: 5,
            observed_peak_in_flight_max: Some(3),
            observed_wall_time_ms_samples: 3,
            observed_wall_time_ms_sum: 72,
            observed_wall_time_ms_max: Some(33),
            degraded_parallel_segments: 1,
            scheduling_class_counts: BTreeMap::from([
                ("parallel_safe".to_owned(), 3),
                ("serial_only".to_owned(), 3),
            ]),
            execution_mode_counts: BTreeMap::from([
                ("parallel".to_owned(), 3),
                ("sequential".to_owned(), 3),
            ]),
        };

        let output = format_fast_lane_summary("session-fast-lane", 64, &summary);

        assert!(output.contains("fast_lane_summary session=session-fast-lane limit=64"));
        assert!(output.contains(
            "aggregate_batches parallel_enabled=2 parallel_only=1 mixed=1 sequential_only=1 without_segments=1"
        ));
        assert!(output.contains(
            "aggregate_intents total=8 parallel_safe=5 serial_only=3 parallel_safe_ratio=0.625 serial_only_ratio=0.375"
        ));
        assert!(output.contains("aggregate_segments parallel=3 sequential=3"));
        assert!(output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=3 configured_max_in_flight_samples=3 observed_peak_in_flight_avg=1.667 observed_peak_in_flight_max=3 observed_peak_in_flight_samples=3 degraded_parallel_segments=1"
        ));
        assert!(output.contains(
            "aggregate_latency observed_wall_time_ms_avg=24.000 observed_wall_time_ms_max=33 observed_wall_time_ms_samples=3"
        ));
        assert!(output.contains(
            "latest_batch total_intents=0 parallel_enabled=false max_in_flight=- observed_peak_in_flight=1 observed_wall_time_ms=11 parallel_safe_intents=0 serial_only_intents=0 parallel_segments=0 sequential_segments=0"
        ));
        assert!(output.contains("rollup scheduling_classes=parallel_safe:3,serial_only:3"));
        assert!(output.contains("rollup execution_modes=parallel:3,sequential:3"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn turn_checkpoint_summary_output_accepts_explicit_runtime_binding() {
        let (config, memory_config, sqlite_path) = init_chat_test_memory("turn-checkpoint-output");

        let direct_payloads = turn_checkpoint_event_payloads();
        append_assistant_payloads(
            "chat-binding-turn-checkpoint-direct",
            &direct_payloads,
            &memory_config,
        );
        let coordinator = ConversationTurnCoordinator::new();
        let direct_output = load_turn_checkpoint_summary_output(
            &coordinator,
            &config,
            "chat-binding-turn-checkpoint-direct",
            96,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("load turn checkpoint summary via direct binding");
        assert!(direct_output.contains("turn_checkpoint_summary session=chat-binding-turn-checkpoint-direct limit=96 checkpoints=2"));
        assert!(direct_output.contains("state=finalized"));
        assert!(direct_output.contains("after_turn=completed"));
        assert!(direct_output.contains("compaction=skipped"));

        let kernel_payloads = turn_checkpoint_event_payloads();
        let (kernel_ctx, invocations) =
            build_kernel_context_with_window_turns(assistant_window_turns(&kernel_payloads));
        let kernel_output = load_turn_checkpoint_summary_output(
            &coordinator,
            &config,
            "chat-binding-turn-checkpoint-kernel",
            112,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("load turn checkpoint summary via kernel binding");
        assert!(kernel_output.contains("turn_checkpoint_summary session=chat-binding-turn-checkpoint-kernel limit=112 checkpoints=2"));
        assert!(kernel_output.contains("state=finalized"));
        assert!(kernel_output.contains("after_turn=completed"));
        assert!(kernel_output.contains("compaction=skipped"));

        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
        assert_eq!(
            captured[0].payload["session_id"],
            "chat-binding-turn-checkpoint-kernel"
        );
        assert_eq!(captured[0].payload["limit"], json!(112));
        assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

        cleanup_chat_test_memory(&sqlite_path);
    }

    #[test]
    fn render_cli_chat_startup_lines_prioritize_first_turn_guidance() {
        let lines = render_cli_chat_startup_lines_with_width(
            &CliChatStartupSummary {
                config_path: "/tmp/loongclaw.toml".to_owned(),
                memory_label: "/tmp/loongclaw.db".to_owned(),
                session_id: "default".to_owned(),
                context_engine_id: "threaded".to_owned(),
                context_engine_source: "config".to_owned(),
                acp_enabled: true,
                dispatch_enabled: true,
                conversation_routing: "automatic".to_owned(),
                allowed_channels: vec!["cli".to_owned()],
                acp_backend_id: "builtin".to_owned(),
                acp_backend_source: "default".to_owned(),
                explicit_acp_request: false,
                event_stream_enabled: false,
                bootstrap_mcp_servers: Vec::new(),
                working_directory: None,
            },
            80,
        );

        assert!(
            lines
                .first()
                .is_some_and(|line| line.starts_with("LOONGCLAW")),
            "chat startup should now use the shared compact brand header: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line == "start here: Summarize this repository and suggest the best next step."
            }),
            "chat startup should render the first prompt through the structured action group: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- type your request, or use /help for commands"),
            "chat startup should keep the usage hint, but under the assistant-first opening block: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "session details"),
            "chat startup should keep session/config facts in a structured key-value section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "runtime details"),
            "chat startup should still preserve runtime context in a compact secondary section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "- session: default"),
            "chat startup should continue to show session identity after the handoff block: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_startup_lines_surface_explicit_acp_overrides() {
        let lines = render_cli_chat_startup_lines_with_width(
            &CliChatStartupSummary {
                config_path: "/tmp/loongclaw.toml".to_owned(),
                memory_label: "/tmp/loongclaw.db".to_owned(),
                session_id: "thread-42".to_owned(),
                context_engine_id: "threaded".to_owned(),
                context_engine_source: "env".to_owned(),
                acp_enabled: true,
                dispatch_enabled: true,
                conversation_routing: "manual".to_owned(),
                allowed_channels: vec!["cli".to_owned(), "telegram".to_owned()],
                acp_backend_id: "jsonrpc".to_owned(),
                acp_backend_source: "config".to_owned(),
                explicit_acp_request: true,
                event_stream_enabled: true,
                bootstrap_mcp_servers: vec!["filesystem".to_owned()],
                working_directory: Some("/workspace/project".to_owned()),
            },
            80,
        );

        assert!(
            lines.iter().any(|line| { line == "note: acp overrides" }),
            "chat startup should group ACP overrides under a dedicated callout heading: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- bootstrap MCP servers: filesystem"),
            "chat startup should still surface the bootstrap MCP override details: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- working directory: /workspace/project"),
            "chat startup should still surface the working directory override: {lines:#?}"
        );
    }

    #[test]
    fn should_run_missing_config_onboard_uses_default_yes_and_respects_decline() {
        assert!(should_run_missing_config_onboard(1, "\n"));
        assert!(should_run_missing_config_onboard(1, "yes\n"));
        assert!(!should_run_missing_config_onboard(1, "n\n"));
        assert!(!should_run_missing_config_onboard(0, ""));
    }

    #[test]
    fn render_cli_chat_missing_config_lines_wrap_setup_prompt_in_surface() {
        let command = "loongclaw onboard --output /tmp/loongclaw.toml";
        let lines = render_cli_chat_missing_config_lines_with_width(command, 80);

        assert!(
            lines
                .first()
                .is_some_and(|line| line.starts_with("LOONGCLAW")),
            "missing-config setup prompt should keep the shared compact header: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "setup required"),
            "missing-config setup prompt should promote the title into the shared screen surface: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "setup command: loongclaw onboard --output /tmp/loongclaw.toml"),
            "missing-config setup prompt should surface the setup command block: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "y) run setup wizard (recommended)"),
            "missing-config setup prompt should show the default acceptance choice explicitly: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "Press Enter to accept y."),
            "missing-config setup prompt should explain the default-enter behavior: {lines:#?}"
        );
    }

    #[test]
    fn render_turn_checkpoint_startup_health_lines_surface_recovery_and_probe() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
            latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
            latest_lane: Some("safe".to_owned()),
            latest_result_kind: Some("tool_error".to_owned()),
            latest_persistence_mode: Some("success".to_owned()),
            latest_safe_lane_terminal_route: Some(
                crate::conversation::SafeLaneTerminalRouteSnapshot {
                    decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
                    reason: crate::conversation::SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
                    source: crate::conversation::SafeLaneFailureRouteSource::BackpressureGuard,
                },
            ),
            latest_identity_present: Some(true),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };
        let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );
        let diagnostics = test_turn_checkpoint_diagnostics(summary, Some(probe));
        let lines = render_turn_checkpoint_startup_health_lines_with_width(
            "session-health",
            &diagnostics,
            80,
        )
        .expect("startup health surface");

        assert_eq!(lines[0], "checkpoint: session=session-health");
        assert!(
            lines.iter().any(|line| line == "durability status"),
            "startup health should group durability facts under a shared key-value section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "attention: recovery"),
            "startup health should surface pending recovery as a warning callout: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- action: inspect_manually"),
            "startup health should preserve the concrete recovery action in the callout: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "note: runtime probe"),
            "startup health should surface runtime probe context as a secondary structured callout: {lines:#?}"
        );
    }

    #[test]
    fn render_fast_lane_summary_lines_surface_aggregates_and_segments() {
        let mut summary = FastLaneToolBatchEventSummary {
            batch_events: 2,
            total_intents_seen: 4,
            total_parallel_safe_intents_seen: 3,
            total_serial_only_intents_seen: 1,
            total_parallel_segments_seen: 2,
            total_sequential_segments_seen: 1,
            parallel_execution_max_in_flight_samples: 1,
            parallel_execution_max_in_flight_sum: 4,
            observed_peak_in_flight_samples: 1,
            observed_peak_in_flight_sum: 3,
            observed_wall_time_ms_samples: 1,
            observed_wall_time_ms_sum: 120,
            latest_schema_version: Some(3),
            latest_total_intents: Some(2),
            latest_parallel_execution_enabled: Some(true),
            latest_parallel_execution_max_in_flight: Some(4),
            latest_observed_peak_in_flight: Some(3),
            latest_observed_wall_time_ms: Some(120),
            latest_parallel_safe_intents: Some(2),
            latest_serial_only_intents: Some(0),
            latest_parallel_segments: Some(1),
            latest_sequential_segments: Some(0),
            latest_segments: vec![FastLaneToolBatchSegmentSnapshot {
                segment_index: 0,
                scheduling_class: "parallel_safe".to_owned(),
                execution_mode: "parallel".to_owned(),
                intent_count: 2,
                observed_peak_in_flight: Some(3),
                observed_wall_time_ms: Some(120),
            }],
            ..FastLaneToolBatchEventSummary::default()
        };
        summary
            .scheduling_class_counts
            .insert("parallel_safe".to_owned(), 2);
        summary
            .execution_mode_counts
            .insert("parallel".to_owned(), 2);

        let lines = render_fast_lane_summary_lines_with_width("session-fast", 64, &summary, 80);

        assert_eq!(lines[0], "fast-lane: session=session-fast limit=64");
        assert!(
            lines.iter().any(|line| line == "intent mix"),
            "fast-lane summary should promote aggregate intent counters into a titled section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "latest segments"),
            "fast-lane summary should keep the latest segment narrative visible: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line == "- segment 0: class=parallel_safe mode=parallel intents=2 peak=3 wall_ms=120"
            }),
            "fast-lane summary should render latest segment details as readable surface lines: {lines:#?}"
        );
    }

    #[test]
    fn render_safe_lane_summary_lines_surface_health_and_rollups() {
        let config = ConversationConfig::default();
        let mut summary = SafeLaneEventSummary {
            lane_selected_events: 1,
            round_started_events: 2,
            round_completed_succeeded_events: 1,
            round_completed_failed_events: 1,
            verify_failed_events: 1,
            replan_triggered_events: 1,
            final_status_events: 1,
            final_status: Some(SafeLaneFinalStatus::Failed),
            final_failure_code: Some("safe_lane_plan_verify_failed".to_owned()),
            final_route_decision: Some("terminal".to_owned()),
            final_route_reason: Some("session_governor_no_replan".to_owned()),
            latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
                rounds_started: 2,
                rounds_succeeded: 1,
                rounds_failed: 1,
                verify_failures: 1,
                replans_triggered: 1,
                total_attempts_used: 3,
            }),
            tool_output_snapshots_seen: 2,
            tool_output_truncated_events: 1,
            tool_output_result_lines_total: 3,
            tool_output_truncated_result_lines_total: 1,
            tool_output_aggregate_truncation_ratio_milli: Some(333),
            ..SafeLaneEventSummary::default()
        };
        summary
            .route_decision_counts
            .insert("terminal".to_owned(), 1);
        summary
            .route_reason_counts
            .insert("session_governor_no_replan".to_owned(), 1);
        summary
            .failure_code_counts
            .insert("safe_lane_plan_verify_failed".to_owned(), 1);

        let lines =
            render_safe_lane_summary_lines_with_width("session-safe", 32, &config, &summary, 80);

        assert_eq!(lines[0], "safe-lane: session=session-safe limit=32");
        assert!(
            lines.iter().any(|line| line == "attention: health"),
            "safe-lane summary should surface warning health as a structured callout: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "- severity: critical"),
            "safe-lane health callout should preserve the derived severity: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "rollups"),
            "safe-lane summary should keep the route and failure rollups in a dedicated section: {lines:#?}"
        );
    }

    #[test]
    fn render_turn_checkpoint_summary_lines_surface_runtime_probe() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 2,
            post_persist_events: 1,
            finalized_events: 1,
            latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Completed),
            latest_compaction: Some(TurnCheckpointProgressStatus::Failed),
            latest_lane: Some("fast".to_owned()),
            latest_result_kind: Some("final_text".to_owned()),
            latest_persistence_mode: Some("success".to_owned()),
            latest_identity_present: Some(true),
            session_state: TurnCheckpointSessionState::FinalizationFailed,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };
        let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );
        let diagnostics = test_turn_checkpoint_diagnostics(summary, Some(probe));
        let lines = render_turn_checkpoint_summary_lines_with_width(
            "session-summary",
            64,
            &diagnostics,
            80,
        );

        assert_eq!(lines[0], "checkpoint: session=session-summary limit=64");
        assert!(
            lines.iter().any(|line| line == "summary"),
            "turn checkpoint summary should group the latest durability state in a titled section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "note: runtime probe"),
            "turn checkpoint summary should append runtime probe context as a structured callout: {lines:#?}"
        );
    }

    #[test]
    fn render_turn_checkpoint_repair_lines_surface_manual_result() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };
        let outcome = crate::conversation::TurnCheckpointTailRepairOutcome::from_summary(
            crate::conversation::TurnCheckpointTailRepairStatus::ManualRequired,
            TurnCheckpointRecoveryAction::InspectManually,
            Some(crate::conversation::TurnCheckpointTailRepairSource::Summary),
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointIdentityMissing,
            &summary,
        );
        let lines = render_turn_checkpoint_repair_lines_with_width("session-repair", &outcome, 80);

        assert_eq!(lines[0], "repair: session=session-repair");
        assert!(
            lines.iter().any(|line| line == "repair status"),
            "turn checkpoint repair should group repair facts in a structured key-value section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "attention: repair result"),
            "manual repair outcomes should surface a warning callout: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_help_lines_promotes_commands_to_surface() {
        let lines = render_cli_chat_help_lines_with_width(72);

        assert_eq!(lines[0], "chat: commands");
        assert!(
            lines.iter().any(|line| line == "slash commands"),
            "help output should keep a dedicated slash-command section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- /history: print the current session sliding window"),
            "help output should render slash commands as readable key-value rows: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "note: usage notes"),
            "help output should preserve operator guidance as a callout: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_history_lines_wrap_history_in_surface() {
        let history_lines = vec![
            "user: summarize the current repo".to_owned(),
            "assistant: start with the daemon crate".to_owned(),
        ];
        let lines = render_cli_chat_history_lines_with_width("session-7", 24, &history_lines, 72);

        assert_eq!(lines[0], "history: session=session-7 limit=24");
        assert!(
            lines.iter().any(|line| line == "sliding window"),
            "history output should keep a dedicated window section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "user: summarize the current repo"),
            "history output should still surface the original transcript entries: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_assistant_lines_promotes_markdown_to_structured_sections() {
        let assistant_text = "\
## Plan

- inspect the active config
* compare runtime state
> reuse current provider settings when safe

```rust
let value = input.trim();
println!(\"{value}\");
```";
        let lines = render_cli_chat_assistant_lines_with_width(assistant_text, 72);

        assert_eq!(lines[0], "loongclaw: reply");
        assert!(
            lines.iter().any(|line| line == "Plan"),
            "markdown headings should become section titles: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- inspect the active config"),
            "markdown list items should remain visible in the narrative block: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "- compare runtime state"),
            "markdown star bullets should normalize into wrapped display bullets: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "note: quoted context"),
            "markdown blockquotes should render as structured callouts: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "code [rust]"),
            "markdown fences should render as preformatted sections: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "    let value = input.trim();"),
            "preformatted sections should keep code indentation intact: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_assistant_lines_preserve_heading_before_quotes_and_at_eof() {
        let assistant_text = "\
## Risks
> keep credentials in env vars

## Next";
        let lines = render_cli_chat_assistant_lines_with_width(assistant_text, 72);

        assert!(
            lines.iter().any(|line| line == "note: Risks"),
            "headings should stay attached to quoted sections instead of falling back to a generic title: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- keep credentials in env vars"),
            "quoted content should stay visible after preserving the heading: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "Next"),
            "a trailing heading should still render even when it has no body lines yet: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_assistant_lines_promotes_tool_approval_to_choice_screen() {
        let assistant_text = "\
我准备调用 provider.switch 来切换后续会话的 provider。
[tool_approval_required]
tool: provider.switch
request_id: apr_provider_switch
rule_id: session_tool_consent_auto_blocked
reason: `provider.switch` is not eligible for auto mode and needs operator confirmation
allowed_decisions: yes / auto / full / esc";
        let lines = render_cli_chat_assistant_lines_with_width(assistant_text, 72);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("准备调用 provider.switch")),
            "approval replies should render as a dedicated screen title: {lines:#?}"
        );
        let first_choice_visible = lines.iter().any(|line| line.trim_start().starts_with("1)"));
        let second_choice_visible = lines.iter().any(|line| line.trim_start().starts_with("2)"));

        assert!(
            first_choice_visible && second_choice_visible,
            "approval choice screen should expose numbered choices in order: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("yes / auto / full / esc")),
            "approval choice screen should keep the raw keyword controls visible: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_live_surface_lines_show_pipeline_status_and_preview() {
        let snapshot = CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(1),
            lane: None,
            tool_call_count: 0,
            message_count: Some(4),
            estimated_tokens: Some(128),
            draft_preview: Some("Inspecting the repo layout...".to_owned()),
            tool_activity_lines: Vec::new(),
        };
        let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, 72);

        assert_eq!(lines[0], "loongclaw: live");
        assert!(
            lines.iter().any(|line| line == "note: querying model"),
            "live surface should explain the active phase through a callout: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "turn pipeline"),
            "live surface should keep the pipeline checklist visible: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line.starts_with("[WARN] call model")
                    && line.contains("provider round 1 in progress")
            }),
            "live surface should keep the model step actively highlighted: {lines:#?}"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("streaming provider round 1")),
            "live surface should avoid claiming streaming when the snapshot does not encode that capability: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "draft preview"),
            "live surface should surface partial text as a dedicated preview block: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "Inspecting the repo layout..."),
            "live surface should preserve the partial preview text: {lines:#?}"
        );
    }

    #[test]
    fn cli_chat_live_surface_observer_emits_phase_and_stream_preview_batches() {
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
        let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

        observer.on_phase(ConversationTurnPhaseEvent::preparing());
        observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
            1,
            3,
            Some(96),
        ));
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "text_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: Some("Draft response".to_owned()),
                tool_call: None,
            },
            index: None,
        });

        let batches = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned");
        assert!(
            batches.len() >= 3,
            "observer should emit both phase updates and the first preview update: {batches:#?}"
        );

        let preview_batch = batches
            .iter()
            .find(|lines| lines.iter().any(|line| line == "draft preview"))
            .expect("preview batch");
        assert!(
            preview_batch.iter().any(|line| line == "Draft response"),
            "preview batch should include the streamed text: {preview_batch:#?}"
        );
    }

    #[test]
    fn cli_chat_live_surface_observer_renders_tool_lifecycle_updates() {
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
        let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

        observer.on_phase(ConversationTurnPhaseEvent::running_tools(
            1,
            ExecutionLane::Fast,
            1,
        ));
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "tool_call_start".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: Some("file.read".to_owned()),
                    args: None,
                    id: Some("call-tool-1".to_owned()),
                }),
            },
            index: Some(0),
        });
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "tool_call_input_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: None,
                    args: Some("{\"path\":\"README.md\"}".to_owned()),
                    id: None,
                }),
            },
            index: Some(0),
        });
        observer.on_tool(ConversationTurnToolEvent::completed(
            "call-tool-1",
            "file.read",
            Some("ok".to_owned()),
        ));

        let batches = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned");
        let running_batch = batches
            .iter()
            .find(|lines| lines.iter().any(|line| line == "tool activity"))
            .expect("running tool batch");
        let completed_batch = batches
            .iter()
            .rev()
            .find(|lines| {
                lines
                    .iter()
                    .any(|line| line == "[completed] file.read (id=call-tool-1) - ok")
            })
            .expect("completed tool batch");

        assert!(
            running_batch
                .iter()
                .any(|line| line == "[running] file.read (id=call-tool-1)"),
            "tool batch should surface the running tool state: {running_batch:#?}"
        );

        assert!(
            completed_batch
                .iter()
                .any(|line| line == "[completed] file.read (id=call-tool-1) - ok"),
            "tool batch should surface the completed tool state: {completed_batch:#?}"
        );
        assert!(
            completed_batch
                .iter()
                .any(|line| line == "args: {\"path\":\"README.md\"}"),
            "tool batch should preserve streamed tool args: {completed_batch:#?}"
        );
    }

    #[test]
    fn parse_markdown_heading_follows_commonmark_atx_rules() {
        assert_eq!(parse_markdown_heading("## Plan"), Some("Plan"));
        assert_eq!(parse_markdown_heading("### Plan ###"), Some("Plan"));
        assert_eq!(parse_markdown_heading("## C#"), Some("C#"));
        assert_eq!(parse_markdown_heading("#NoSpace"), None);
        assert_eq!(parse_markdown_heading("#!/bin/bash"), None);
        assert_eq!(parse_markdown_heading("####### too many"), None);
    }

    #[test]
    fn cli_chat_live_surface_observer_resets_request_scoped_buffers_between_rounds() {
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
        let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

        observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
            1,
            3,
            Some(96),
        ));
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "text_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: Some("Draft response".to_owned()),
                tool_call: None,
            },
            index: None,
        });
        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "tool_call_input_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: None,
                    args: Some("{\"query\":\"rust\"}".to_owned()),
                    id: None,
                }),
            },
            index: Some(0),
        });
        observer.on_phase(ConversationTurnPhaseEvent::requesting_followup_provider(
            2,
            ExecutionLane::Fast,
            1,
            5,
            Some(128),
        ));

        let batches = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned");
        let last_batch = batches.last().expect("follow-up request batch");

        assert!(
            !last_batch.iter().any(|line| line == "draft preview"),
            "follow-up provider requests should reset the previous draft preview: {last_batch:#?}"
        );
        assert!(
            !last_batch.iter().any(|line| line == "tool activity"),
            "follow-up provider requests should not reuse prior tool activity lines: {last_batch:#?}"
        );
        assert!(
            !last_batch.iter().any(|line| line == "Draft response"),
            "follow-up provider requests should not carry the previous request preview text: {last_batch:#?}"
        );
    }

    #[test]
    fn cli_chat_live_surface_observer_waits_for_tools_phase_before_rendering_tool_activity() {
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
        let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

        observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
            1,
            3,
            Some(96),
        ));

        let batch_count_before_tool_delta = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned")
            .len();

        observer.on_streaming_token(crate::acp::StreamingTokenEvent {
            event_type: "tool_call_start".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: Some("search".to_owned()),
                    args: None,
                    id: Some("call_123".to_owned()),
                }),
            },
            index: Some(0),
        });

        let batch_count_after_tool_delta = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned")
            .len();
        assert_eq!(
            batch_count_after_tool_delta, batch_count_before_tool_delta,
            "tool-call deltas should wait for the tools phase before re-rendering"
        );

        observer.on_phase(ConversationTurnPhaseEvent::running_tools(
            1,
            ExecutionLane::Fast,
            1,
        ));

        let batches = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned");
        let last_batch = batches.last().expect("running-tools batch");

        assert!(
            last_batch.iter().any(|line| line == "tool activity"),
            "the tools phase should render the accumulated tool activity: {last_batch:#?}"
        );
        assert!(
            last_batch
                .iter()
                .any(|line| line == "[running] search (id=call_123)"),
            "the tools phase should surface the streamed tool metadata: {last_batch:#?}"
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn reload_cli_turn_config_refreshes_provider_state_without_mutating_cli_settings() {
        let path = std::env::temp_dir().join(format!(
            "loongclaw-chat-provider-reload-{}.toml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let path_string = path.display().to_string();

        let mut in_memory = LoongClawConfig::default();
        in_memory.cli.exit_commands = vec!["/bye".to_owned()];
        let mut openai =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai.model = "gpt-5".to_owned();
        in_memory.set_active_provider_profile(
            "openai-gpt-5",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai,
            },
        );

        let mut on_disk = in_memory.clone();
        on_disk.cli.exit_commands = vec!["/different".to_owned()];
        let mut deepseek =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
        deepseek.model = "deepseek-chat".to_owned();
        on_disk.providers.insert(
            "deepseek-chat".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: deepseek.clone(),
            },
        );
        on_disk.provider = deepseek;
        on_disk.active_provider = Some("deepseek-chat".to_owned());
        crate::config::write(Some(&path_string), &on_disk, true).expect("write config fixture");

        let reloaded = reload_cli_turn_config(&in_memory, path.as_path()).expect("reload");
        assert_eq!(reloaded.active_provider_id(), Some("deepseek-chat"));
        assert_eq!(reloaded.provider.model, "deepseek-chat");
        assert_eq!(reloaded.cli.exit_commands, vec!["/bye".to_owned()]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn parse_summary_limit_accepts_aliases_and_preserves_usage_text() {
        assert_eq!(
            parse_summary_limit(
                "/fast_lane_summary",
                20,
                &["/fast_lane_summary", "/fast-lane-summary"],
            )
            .expect("parse"),
            Some(80)
        );
        assert_eq!(
            parse_summary_limit(
                "/fast-lane-summary 144",
                20,
                &["/fast_lane_summary", "/fast-lane-summary"],
            )
            .expect("parse"),
            Some(144)
        );
        assert_eq!(
            parse_summary_limit(
                "/other_summary",
                20,
                &["/fast_lane_summary", "/fast-lane-summary"],
            )
            .expect("parse"),
            None
        );

        let error = parse_summary_limit(
            "/fast_lane_summary 0",
            20,
            &["/fast_lane_summary", "/fast-lane-summary"],
        )
        .expect_err("zero limit should be rejected");
        assert_eq!(
            error,
            "invalid /fast_lane_summary limit `0`; usage: /fast_lane_summary [limit]"
        );

        let error = parse_summary_limit(
            "/fast_lane_summary nope",
            20,
            &["/fast_lane_summary", "/fast-lane-summary"],
        )
        .expect_err("non-number limit should be rejected");
        assert!(error.contains("invalid /fast_lane_summary limit `nope`"));
        assert!(error.contains("usage: /fast_lane_summary [limit]"));

        let error = parse_summary_limit(
            "/fast-lane-summary 12 extra",
            20,
            &["/fast_lane_summary", "/fast-lane-summary"],
        )
        .expect_err("extra args should be rejected");
        assert_eq!(error, "usage: /fast_lane_summary [limit]");
    }

    #[test]
    fn parse_safe_lane_summary_limit_accepts_default_and_explicit_limit() {
        assert_eq!(
            parse_safe_lane_summary_limit("/safe_lane_summary", 20).expect("parse"),
            Some(80)
        );
        assert_eq!(
            parse_safe_lane_summary_limit("/safe-lane-summary 120", 20).expect("parse"),
            Some(120)
        );
    }

    #[test]
    fn parse_safe_lane_summary_limit_rejects_invalid_input() {
        let error = parse_safe_lane_summary_limit("/safe_lane_summary 0", 20)
            .expect_err("zero limit should be rejected");
        assert!(error.contains("usage"));

        let error = parse_safe_lane_summary_limit("/safe_lane_summary abc", 20)
            .expect_err("non-number limit should be rejected");
        assert!(error.contains("invalid"));
    }

    #[test]
    fn parse_fast_lane_summary_limit_accepts_default_and_explicit_limit() {
        assert_eq!(
            parse_fast_lane_summary_limit("/fast_lane_summary", 20).expect("parse"),
            Some(80)
        );
        assert_eq!(
            parse_fast_lane_summary_limit("/fast-lane-summary 144", 20).expect("parse"),
            Some(144)
        );
    }

    #[test]
    fn parse_fast_lane_summary_limit_rejects_invalid_input() {
        let error = parse_fast_lane_summary_limit("/fast_lane_summary 0", 20)
            .expect_err("zero limit should be rejected");
        assert!(error.contains("usage"));

        let error = parse_fast_lane_summary_limit("/fast_lane_summary nope", 20)
            .expect_err("non-number limit should be rejected");
        assert!(error.contains("invalid"));
    }

    #[test]
    fn parse_turn_checkpoint_summary_limit_accepts_default_and_explicit_limit() {
        assert_eq!(
            parse_turn_checkpoint_summary_limit("/turn_checkpoint_summary", 20).expect("parse"),
            Some(80)
        );
        assert_eq!(
            parse_turn_checkpoint_summary_limit("/turn-checkpoint-summary 96", 20).expect("parse"),
            Some(96)
        );
    }

    #[test]
    fn parse_turn_checkpoint_summary_limit_rejects_invalid_input() {
        let error = parse_turn_checkpoint_summary_limit("/turn_checkpoint_summary 0", 20)
            .expect_err("zero limit should be rejected");
        assert!(error.contains("usage"));

        let error = parse_turn_checkpoint_summary_limit("/turn_checkpoint_summary nope", 20)
            .expect_err("non-number limit should be rejected");
        assert!(error.contains("invalid"));
    }

    #[test]
    fn is_turn_checkpoint_repair_command_accepts_aliases_and_rejects_extra_args() {
        assert!(is_turn_checkpoint_repair_command("/turn_checkpoint_repair").expect("parse"));
        assert!(is_turn_checkpoint_repair_command("/turn-checkpoint-repair").expect("parse"));
        assert!(!is_turn_checkpoint_repair_command("/turn_checkpoint_summary").expect("parse"));

        let error = is_turn_checkpoint_repair_command("/turn_checkpoint_repair now")
            .expect_err("extra args should be rejected");
        assert!(error.contains("usage"));
    }

    fn test_turn_checkpoint_diagnostics(
        summary: TurnCheckpointEventSummary,
        runtime_probe: Option<TurnCheckpointTailRepairRuntimeProbe>,
    ) -> crate::conversation::TurnCheckpointDiagnostics {
        let recovery =
            crate::conversation::TurnCheckpointRecoveryAssessment::from_summary(&summary);
        crate::conversation::TurnCheckpointDiagnostics::new(summary, recovery, runtime_probe)
    }

    #[test]
    fn format_turn_checkpoint_summary_reports_recovery_state_and_failure() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 2,
            post_persist_events: 1,
            finalization_failed_events: 1,
            latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Completed),
            latest_compaction: Some(TurnCheckpointProgressStatus::Failed),
            latest_failure_step: Some(TurnCheckpointFailureStep::Compaction),
            latest_failure_error: Some("context compaction failed".to_owned()),
            latest_lane: Some("safe".to_owned()),
            latest_result_kind: Some("tool_call".to_owned()),
            latest_persistence_mode: Some("error".to_owned()),
            latest_safe_lane_terminal_route: Some(
                crate::conversation::SafeLaneTerminalRouteSnapshot {
                    decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
                    reason:
                        crate::conversation::SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                    source: crate::conversation::SafeLaneFailureRouteSource::SessionGovernor,
                },
            ),
            latest_identity_present: Some(false),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::FinalizationFailed,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let formatted = format_turn_checkpoint_summary("session-checkpoint", 128, &diagnostics);

        assert!(formatted.contains("turn_checkpoint_summary session=session-checkpoint limit=128"));
        assert!(formatted.contains("state=finalization_failed"));
        assert!(formatted.contains("durable=1"));
        assert!(formatted.contains("requires_recovery=1"));
        assert!(formatted.contains("stage=finalization_failed"));
        assert!(formatted.contains("after_turn=completed"));
        assert!(formatted.contains("compaction=failed"));
        assert!(formatted.contains("lane=safe"));
        assert!(formatted.contains("result_kind=tool_call"));
        assert!(formatted.contains("persistence_mode=error"));
        assert!(formatted.contains("safe_lane_route_decision=terminal"));
        assert!(formatted.contains("safe_lane_route_reason=session_governor_no_replan"));
        assert!(formatted.contains("safe_lane_route_source=session_governor"));
        assert!(formatted.contains("identity=missing"));
        assert!(formatted.contains("failure_step=compaction"));
        assert!(formatted.contains("failure_error=context compaction failed"));
        assert!(formatted.contains("recovery_action=inspect_manually"));
        assert!(formatted.contains("recovery_source=summary"));
        assert!(formatted.contains("recovery_reason=checkpoint_identity_missing"));
    }

    #[test]
    fn format_turn_checkpoint_summary_marks_checkpoint_only_durability_for_return_error_sessions() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            finalized_events: 1,
            latest_stage: Some(TurnCheckpointStage::Finalized),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
            latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
            latest_lane: None,
            latest_result_kind: None,
            latest_persistence_mode: None,
            latest_identity_present: Some(false),
            latest_runs_after_turn: Some(false),
            latest_attempts_context_compaction: Some(false),
            session_state: TurnCheckpointSessionState::Finalized,
            checkpoint_durable: true,
            requires_recovery: false,
            reply_durable: false,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let formatted = format_turn_checkpoint_summary("session-checkpoint", 64, &diagnostics);

        assert!(formatted.contains("durable=0"));
        assert!(formatted.contains("checkpoint_durable=1"));
        assert!(formatted.contains("durability=checkpoint_only"));
        assert!(formatted.contains("state=finalized"));
    }

    #[test]
    fn format_turn_checkpoint_summary_uses_typed_checkpoint_durability() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::Finalized),
            session_state: TurnCheckpointSessionState::Finalized,
            checkpoint_durable: false,
            reply_durable: false,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let formatted = format_turn_checkpoint_summary("session-checkpoint", 32, &diagnostics);

        assert!(formatted.contains("state=finalized"));
        assert!(formatted.contains("checkpoint_durable=0"));
        assert!(formatted.contains("durability=not_durable"));
    }

    #[test]
    fn format_turn_checkpoint_startup_health_reports_recovery_action() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            post_persist_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
            latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
            latest_lane: Some("safe".to_owned()),
            latest_result_kind: Some("tool_error".to_owned()),
            latest_persistence_mode: Some("success".to_owned()),
            latest_safe_lane_terminal_route: Some(
                crate::conversation::SafeLaneTerminalRouteSnapshot {
                    decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
                    reason: crate::conversation::SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
                    source: crate::conversation::SafeLaneFailureRouteSource::BackpressureGuard,
                },
            ),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let formatted =
            format_turn_checkpoint_startup_health("session-health", &diagnostics).expect("health");

        assert!(formatted.contains("turn_checkpoint_health session=session-health"));
        assert!(formatted.contains("state=pending_finalization"));
        assert!(formatted.contains("recovery_needed=1"));
        assert!(formatted.contains("action=run_after_turn_and_compaction"));
        assert!(formatted.contains("source=summary"));
        assert!(formatted.contains("reason=-"));
        assert!(formatted.contains("lane=safe"));
        assert!(formatted.contains("result_kind=tool_error"));
        assert!(formatted.contains("safe_lane_route_decision=terminal"));
        assert!(formatted.contains("safe_lane_route_reason=backpressure_attempts_exhausted"));
        assert!(formatted.contains("safe_lane_route_source=backpressure_guard"));
        assert!(formatted.contains("identity=present"));
    }

    #[test]
    fn format_turn_checkpoint_startup_health_reports_route_aware_manual_reason() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            post_persist_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
            latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
            latest_lane: Some("safe".to_owned()),
            latest_result_kind: Some("tool_error".to_owned()),
            latest_persistence_mode: Some("success".to_owned()),
            latest_safe_lane_terminal_route: Some(
                crate::conversation::SafeLaneTerminalRouteSnapshot {
                    decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
                    reason:
                        crate::conversation::SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                    source: crate::conversation::SafeLaneFailureRouteSource::SessionGovernor,
                },
            ),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(false),
            latest_attempts_context_compaction: Some(false),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let formatted =
            format_turn_checkpoint_startup_health("session-health", &diagnostics).expect("health");

        assert!(formatted.contains("turn_checkpoint_health session=session-health"));
        assert!(formatted.contains("action=inspect_manually"));
        assert!(formatted.contains("source=summary"));
        assert!(
            formatted
                .contains("reason=safe_lane_session_governor_terminal_requires_manual_inspection")
        );
        assert!(formatted.contains("safe_lane_route_reason=session_governor_no_replan"));
        assert!(formatted.contains("safe_lane_route_source=session_governor"));
    }

    #[test]
    fn format_turn_checkpoint_startup_health_marks_checkpoint_only_durability() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            finalized_events: 1,
            latest_stage: Some(TurnCheckpointStage::Finalized),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
            latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
            latest_identity_present: Some(false),
            latest_runs_after_turn: Some(false),
            latest_attempts_context_compaction: Some(false),
            session_state: TurnCheckpointSessionState::Finalized,
            checkpoint_durable: true,
            requires_recovery: false,
            reply_durable: false,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let formatted =
            format_turn_checkpoint_startup_health("session-health", &diagnostics).expect("health");

        assert!(formatted.contains("reply_durable=0"));
        assert!(formatted.contains("checkpoint_durable=1"));
        assert!(formatted.contains("durability=checkpoint_only"));
    }

    #[test]
    fn format_turn_checkpoint_startup_health_uses_typed_checkpoint_durability_gate() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::Finalized),
            session_state: TurnCheckpointSessionState::Finalized,
            checkpoint_durable: false,
            reply_durable: false,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);

        assert!(format_turn_checkpoint_startup_health("session-health", &diagnostics).is_none());
    }

    #[test]
    fn format_turn_checkpoint_startup_health_skips_non_durable_sessions() {
        let diagnostics =
            test_turn_checkpoint_diagnostics(TurnCheckpointEventSummary::default(), None);
        assert!(format_turn_checkpoint_startup_health("session-empty", &diagnostics).is_none());
    }

    #[test]
    fn format_turn_checkpoint_runtime_probe_reports_runtime_only_manual_reason() {
        let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );

        let formatted = format_turn_checkpoint_runtime_probe("session-probe", &probe);

        assert!(formatted.contains("turn_checkpoint_probe session=session-probe"));
        assert!(formatted.contains("action=inspect_manually"));
        assert!(formatted.contains("source=runtime"));
        assert!(formatted.contains("reason=checkpoint_preparation_fingerprint_mismatch"));
    }

    #[test]
    fn format_turn_checkpoint_summary_output_appends_runtime_probe_line() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            post_persist_events: 1,
            latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Completed),
            latest_compaction: Some(TurnCheckpointProgressStatus::Failed),
            latest_lane: Some("fast".to_owned()),
            latest_result_kind: Some("final_text".to_owned()),
            latest_persistence_mode: Some("success".to_owned()),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::FinalizationFailed,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };
        let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );

        let diagnostics = test_turn_checkpoint_diagnostics(summary, Some(probe));
        let formatted = format_turn_checkpoint_summary_output("session-summary", 64, &diagnostics);

        assert!(formatted.contains("turn_checkpoint_summary session=session-summary limit=64"));
        assert!(formatted.contains("turn_checkpoint_probe session=session-summary"));
        assert!(formatted.contains("source=runtime"));
        assert!(formatted.contains("reason=checkpoint_preparation_fingerprint_mismatch"));
    }

    #[test]
    fn format_turn_checkpoint_repair_reports_summary_source() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };
        let outcome = crate::conversation::TurnCheckpointTailRepairOutcome::from_summary(
            crate::conversation::TurnCheckpointTailRepairStatus::ManualRequired,
            TurnCheckpointRecoveryAction::InspectManually,
            Some(crate::conversation::TurnCheckpointTailRepairSource::Summary),
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointIdentityMissing,
            &summary,
        );

        let formatted = format_turn_checkpoint_repair("session-repair", &outcome);

        assert!(formatted.contains("turn_checkpoint_repair session=session-repair"));
        assert!(formatted.contains("status=manual_required"));
        assert!(formatted.contains("source=summary"));
        assert!(formatted.contains("reason=checkpoint_identity_missing"));
    }

    #[test]
    fn format_turn_checkpoint_summary_output_omits_runtime_probe_line_without_probe() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            post_persist_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
            latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
            latest_lane: Some("fast".to_owned()),
            latest_result_kind: Some("final_text".to_owned()),
            latest_persistence_mode: Some("success".to_owned()),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let formatted = format_turn_checkpoint_summary_output("session-summary", 64, &diagnostics);

        assert!(formatted.contains("turn_checkpoint_summary session=session-summary limit=64"));
        assert!(!formatted.contains("turn_checkpoint_probe"));
        assert!(!formatted.ends_with('\n'));
    }

    #[test]
    fn format_safe_lane_summary_includes_rollups_and_rates() {
        let config = ConversationConfig::default();
        let mut summary = SafeLaneEventSummary {
            lane_selected_events: 1,
            round_started_events: 2,
            round_completed_succeeded_events: 1,
            round_completed_failed_events: 1,
            verify_failed_events: 1,
            replan_triggered_events: 1,
            final_status_events: 1,
            session_governor_engaged_events: 1,
            session_governor_force_no_replan_events: 1,
            session_governor_failed_threshold_triggered_events: 1,
            session_governor_backpressure_threshold_triggered_events: 0,
            session_governor_trend_threshold_triggered_events: 1,
            session_governor_recovery_threshold_triggered_events: 0,
            session_governor_metrics_snapshots_seen: 2,
            session_governor_latest_trend_samples: Some(5),
            session_governor_latest_trend_min_samples: Some(4),
            session_governor_latest_trend_failure_ewma_milli: Some(250),
            session_governor_latest_trend_backpressure_ewma_milli: Some(63),
            session_governor_latest_recovery_success_streak: Some(4),
            session_governor_latest_recovery_success_streak_threshold: Some(3),
            final_status: Some(SafeLaneFinalStatus::Failed),
            final_failure_code: Some("safe_lane_plan_verify_failed".to_owned()),
            final_route_decision: Some("terminal".to_owned()),
            final_route_reason: Some("session_governor_no_replan".to_owned()),
            latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
                rounds_started: 2,
                rounds_succeeded: 1,
                rounds_failed: 1,
                verify_failures: 1,
                replans_triggered: 1,
                total_attempts_used: 3,
            }),
            latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
                output_lines: 2,
                result_lines: 2,
                truncated_result_lines: 1,
                any_truncated: true,
                truncation_ratio_milli: 500,
            }),
            tool_output_snapshots_seen: 2,
            tool_output_truncated_events: 1,
            tool_output_result_lines_total: 3,
            tool_output_truncated_result_lines_total: 1,
            tool_output_aggregate_truncation_ratio_milli: Some(333),
            tool_output_truncation_verify_failed_events: 1,
            tool_output_truncation_replan_events: 1,
            tool_output_truncation_final_failure_events: 1,
            latest_health_signal: Some(crate::conversation::SafeLaneHealthSignalSnapshot {
                severity: "critical".to_owned(),
                flags: vec!["terminal_instability".to_owned()],
            }),
            health_signal_snapshots_seen: 2,
            health_signal_warn_events: 1,
            health_signal_critical_events: 1,
            ..SafeLaneEventSummary::default()
        };
        summary
            .route_decision_counts
            .insert("terminal".to_owned(), 1);
        summary
            .route_reason_counts
            .insert("session_governor_no_replan".to_owned(), 1);
        summary
            .failure_code_counts
            .insert("safe_lane_plan_verify_failed".to_owned(), 1);
        let formatted = format_safe_lane_summary("session-a", 128, &config, &summary);

        assert!(formatted.contains("safe_lane_summary session=session-a limit=128"));
        assert!(formatted.contains("status=failed"));
        assert!(formatted.contains("route_decision=terminal"));
        assert!(formatted.contains("route_reason=session_governor_no_replan"));
        assert!(formatted.contains("replan_per_round=0.500"));
        assert!(formatted.contains("governor_engaged=1"));
        assert!(formatted.contains("governor_force_no_replan=1"));
        assert!(formatted.contains("trigger_failed_threshold=1"));
        assert!(formatted.contains("trigger_trend_threshold=1"));
        assert!(formatted.contains("governor_latest snapshots=2"));
        assert!(formatted.contains("trend_failure_ewma=0.250"));
        assert!(formatted.contains(
            "tool_output snapshots=2 truncated_events=1 result_lines_total=3 truncated_result_lines_total=1"
        ));
        assert!(formatted.contains("latest_truncation_ratio=0.500"));
        assert!(formatted.contains("aggregate_truncation_ratio=0.333"));
        assert!(formatted.contains("aggregate_truncation_ratio_milli=333"));
        assert!(formatted.contains("truncation_verify_failed_events=1"));
        assert!(formatted.contains("truncation_replan_events=1"));
        assert!(formatted.contains("truncation_final_failure_events=1"));
        assert!(formatted.contains("health severity=critical"));
        assert!(formatted.contains("health_payload {\"flags\":"));
        assert!(formatted.contains("\"severity\":\"critical\""));
        assert!(formatted.contains(
            "health_events snapshots=2 warn=1 critical=1 latest_severity=critical latest_flags=terminal_instability"
        ));
        assert!(formatted.contains("truncation_pressure(0.333)"));
        assert!(formatted.contains("verify_failure_pressure(0.500)"));
        assert!(formatted.contains("replan_pressure(0.500)"));
        assert!(formatted.contains("terminal_instability"));
        assert!(formatted.contains("rollup route_decisions=terminal:1"));
        assert!(formatted.contains("rollup route_reasons=session_governor_no_replan:1"));
        assert!(formatted.contains("rollup failure_codes=safe_lane_plan_verify_failed:1"));
    }

    #[test]
    fn format_safe_lane_summary_health_is_ok_when_no_risk_signals() {
        let config = ConversationConfig::default();
        let summary = SafeLaneEventSummary {
            lane_selected_events: 1,
            round_started_events: 3,
            final_status_events: 1,
            final_status: Some(SafeLaneFinalStatus::Succeeded),
            latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
                rounds_started: 3,
                rounds_succeeded: 3,
                rounds_failed: 0,
                verify_failures: 0,
                replans_triggered: 0,
                total_attempts_used: 3,
            }),
            tool_output_snapshots_seen: 1,
            tool_output_result_lines_total: 2,
            tool_output_truncated_result_lines_total: 0,
            latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
                output_lines: 2,
                result_lines: 2,
                truncated_result_lines: 0,
                any_truncated: false,
                truncation_ratio_milli: 0,
            }),
            ..SafeLaneEventSummary::default()
        };
        let formatted = format_safe_lane_summary("session-ok", 64, &config, &summary);
        assert!(formatted.contains("health severity=ok flags=-"));
        assert!(formatted.contains("health_payload {\"flags\":[],\"severity\":\"ok\"}"));
        assert!(formatted.contains(
            "health_events snapshots=0 warn=0 critical=0 latest_severity=- latest_flags=-"
        ));
    }

    #[test]
    fn format_safe_lane_summary_respects_configurable_health_thresholds() {
        let config = ConversationConfig {
            safe_lane_health_truncation_warn_threshold: 0.20,
            safe_lane_health_truncation_critical_threshold: 0.50,
            safe_lane_health_verify_failure_warn_threshold: 0.70,
            safe_lane_health_replan_warn_threshold: 0.70,
            ..ConversationConfig::default()
        };
        let summary = SafeLaneEventSummary {
            round_started_events: 4,
            verify_failed_events: 1,
            replan_triggered_events: 1,
            tool_output_snapshots_seen: 1,
            tool_output_result_lines_total: 4,
            tool_output_truncated_result_lines_total: 1,
            tool_output_aggregate_truncation_ratio_milli: Some(250),
            latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
                output_lines: 4,
                result_lines: 4,
                truncated_result_lines: 1,
                any_truncated: true,
                truncation_ratio_milli: 250,
            }),
            ..SafeLaneEventSummary::default()
        };

        let formatted = format_safe_lane_summary("session-threshold", 32, &config, &summary);
        assert!(formatted.contains("health severity=warn"));
        assert!(formatted.contains("truncation_pressure(0.250)"));
        assert!(!formatted.contains("verify_failure_pressure"));
        assert!(!formatted.contains("replan_pressure"));
    }

    #[test]
    fn format_safe_lane_summary_does_not_mark_unknown_failure_code_substrings_as_instability() {
        let config = ConversationConfig::default();
        let summary = SafeLaneEventSummary {
            final_status: Some(SafeLaneFinalStatus::Failed),
            final_failure_code: Some("unknown_session_governor_hint".to_owned()),
            ..SafeLaneEventSummary::default()
        };

        let formatted = format_safe_lane_summary("session-unknown-code", 16, &config, &summary);
        assert!(formatted.contains("health severity=ok"));
        assert!(!formatted.contains("terminal_instability"));
    }
}
