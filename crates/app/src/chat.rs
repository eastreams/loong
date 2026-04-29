use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Mutex as StdMutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::Notify;

use crate::CliResult;
use crate::acp::{
    AcpConversationTurnOptions, AcpTurnEventSink, AcpTurnProvenance, JsonlAcpTurnEventSink,
};

mod cli_input;
mod cli_render;
#[cfg(all(test, feature = "memory-sqlite"))]
#[allow(clippy::expect_used)]
mod latest_session_selector_tests;
mod live_runtime;
mod operator_surfaces;
mod render_support;

use self::cli_input::ConcurrentCliInputReader;
use self::cli_render::*;
use self::live_runtime::*;
#[cfg(test)]
use self::operator_surfaces::CliChatStartupSummary;
#[cfg(test)]
use self::operator_surfaces::ManualCompactionResult;
#[cfg(test)]
use self::operator_surfaces::ManualCompactionStatus;
#[cfg(test)]
use self::operator_surfaces::build_cli_chat_startup_summary;
use self::operator_surfaces::is_cli_chat_status_command;
use self::operator_surfaces::is_manual_compaction_command;
use self::operator_surfaces::is_turn_checkpoint_repair_command;
#[cfg(test)]
use self::operator_surfaces::load_history_lines;
#[cfg(test)]
use self::operator_surfaces::load_manual_compaction_result;
#[cfg(test)]
use self::operator_surfaces::manual_compaction_status_from_report;
use self::operator_surfaces::parse_exact_chat_command;
use self::operator_surfaces::parse_fast_lane_summary_limit;
use self::operator_surfaces::parse_safe_lane_summary_limit;
#[cfg(test)]
use self::operator_surfaces::parse_summary_limit;
use self::operator_surfaces::parse_turn_checkpoint_summary_limit;
use self::operator_surfaces::print_cli_chat_startup;
use self::operator_surfaces::print_cli_chat_status;
use self::operator_surfaces::print_help;
use self::operator_surfaces::print_history;
use self::operator_surfaces::print_manual_compaction;
#[cfg(test)]
use self::operator_surfaces::render_cli_chat_help_lines_with_width;
#[cfg(test)]
use self::operator_surfaces::render_cli_chat_history_lines_with_width;
use self::operator_surfaces::render_cli_chat_missing_config_decline_lines_with_width;
use self::operator_surfaces::render_cli_chat_missing_config_lines_with_width;
#[cfg(test)]
use self::operator_surfaces::render_cli_chat_startup_lines_with_width;
#[cfg(test)]
use self::operator_surfaces::render_cli_chat_status_lines_with_width;
#[cfg(test)]
use self::operator_surfaces::render_manual_compaction_lines_with_width;
use self::operator_surfaces::should_run_missing_config_onboard;
use self::render_support::*;
#[cfg(test)]
use crate::conversation::DefaultConversationRuntime;

use super::config::{self, ConversationConfig, LoongConfig};
#[cfg(test)]
use super::conversation::ContextCompactionReport;
#[cfg(test)]
use super::conversation::TurnCheckpointTailRepairRuntimeProbe;
use super::conversation::{
    ConversationIngressContext, ConversationRuntimeBinding, ConversationSessionAddress,
    ConversationTurnCoordinator, ConversationTurnObserver, ConversationTurnObserverHandle,
    ConversationTurnPhase, ConversationTurnPhaseEvent, ConversationTurnRuntimeEvent,
    ConversationTurnToolEvent, ConversationTurnToolState, ExecutionLane, ProviderErrorMode,
    parse_approval_prompt_view,
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
#[cfg(feature = "memory-sqlite")]
use super::session::LATEST_SESSION_SELECTOR;
#[cfg(feature = "memory-sqlite")]
use super::session::latest_resumable_root_session_id;
#[cfg(feature = "memory-sqlite")]
use super::session::store::{self, SessionStoreConfig};
use super::tui_surface::{
    TuiCalloutTone, TuiChecklistItemSpec, TuiChecklistStatus, TuiChoiceSpec, TuiHeaderStyle,
    TuiKeyValueSpec, TuiMessageSpec, TuiScreenSpec, TuiSectionSpec, render_tui_message_body_spec,
    render_tui_screen_spec,
};
#[cfg(test)]
use crate::tools::runtime_events::{ToolCommandMetrics, ToolFileChangePreview, ToolOutputDelta};
use crate::tools::runtime_events::{ToolFileChangeKind, ToolRuntimeEvent, ToolRuntimeStream};

pub const DEFAULT_FIRST_PROMPT: &str = "Summarize this repository and suggest the best next step.";
const TEST_ONBOARD_EXECUTABLE_ENV: &str = "LOONG_TEST_ONBOARD_EXECUTABLE";
const CLI_CHAT_HELP_COMMAND: &str = "/help";
const CLI_CHAT_COMPACT_COMMAND: &str = "/compact";
const CLI_CHAT_STATUS_COMMAND: &str = "/status";
const CLI_CHAT_HISTORY_COMMAND: &str = "/history";
const CLI_CHAT_MISSION_COMMAND: &str = "/mission";
const CLI_CHAT_REVIEW_COMMAND: &str = "/review";
const CLI_CHAT_WORKERS_COMMAND: &str = "/workers";
const CLI_CHAT_SESSIONS_COMMAND: &str = "/sessions";
const CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND: &str = "/turn_checkpoint_repair";
const CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND_ALIAS: &str = "/turn-checkpoint-repair";
const CLI_CHAT_COMPOSER_PROMPT: &str = "╰─ you · compose › ";

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
    pub config: LoongConfig,
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
    let mut command = format!("{} onboard", config::active_cli_command_name());
    if config_path.is_some() {
        command.push_str(" --output ");
        command.push_str(&resolved_config_path.display().to_string());
    }
    command
}

pub(crate) struct CliTurnRuntime {
    pub(crate) resolved_path: PathBuf,
    pub(crate) config: LoongConfig,
    pub(crate) session_id: String,
    pub(crate) session_address: ConversationSessionAddress,
    pub(crate) turn_coordinator: ConversationTurnCoordinator,
    pub(crate) runtime_kernel: crate::runtime_bridge::RuntimeKernelOwner,
    pub(crate) explicit_acp_request: bool,
    pub(crate) effective_bootstrap_mcp_servers: Vec<String>,
    pub(crate) effective_working_directory: Option<PathBuf>,
    pub(crate) memory_label: String,
    #[cfg(feature = "memory-sqlite")]
    pub(crate) memory_config: SessionStoreConfig,
}

impl CliTurnRuntime {
    pub(crate) fn conversation_binding(&self) -> ConversationRuntimeBinding<'_> {
        self.runtime_kernel.conversation_binding()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliSessionRequirement {
    /// Interactive entrypoints may fall back to the implicit default session
    /// (and, with sqlite memory enabled, resolve the `latest` selector).
    AllowImplicitDefault,
    /// Embedded or multiplexed hosts must provide a session id explicitly so
    /// they never attach to the wrong transcript by accident.
    RequireExplicit,
}

enum CliChatLoopControl {
    Continue,
    Exit,
    AssistantText(String),
}

#[allow(clippy::print_stdout)] // CLI REPL output
pub async fn run_cli_chat(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    options: &CliChatOptions,
) -> CliResult<()> {
    ensure_cli_channel_enabled_for_entrypoint(config_path)?;
    // The old fullscreen operator-cockpit chat surface is deprecated.
    // Keep the canonical `chat` entrypoint on the newer shell-first path
    // until a replacement fullscreen surface lands.
    run_cli_chat_repl(config_path, session_hint, options).await
}

#[allow(clippy::print_stdout)] // CLI REPL output
async fn run_cli_chat_repl(
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
        print!("{CLI_CHAT_COMPOSER_PROMPT}");
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
    ensure_cli_channel_enabled_for_entrypoint(config_path)?;

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
    reject_disabled_cli_channel(&options.config)?;
    // The fullscreen operator-cockpit surface is deprecated for production
    // chat entrypoints and concurrent hosts alike. Keep the shared shell-first
    // runtime path as the only live production host until a replacement lands.
    run_concurrent_cli_host_repl(options)
}

fn run_concurrent_cli_host_repl(options: &ConcurrentCliHostOptions) -> CliResult<()> {
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

pub(crate) fn reject_disabled_cli_channel(config: &LoongConfig) -> CliResult<()> {
    if config.cli.enabled {
        return Ok(());
    }

    Err("CLI channel is disabled by config.cli.enabled=false".to_owned())
}

fn ensure_cli_channel_enabled_for_entrypoint(config_path: Option<&str>) -> CliResult<()> {
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
        return Ok(());
    }

    let (_resolved_path, config) = config::load(config_path)?;
    reject_disabled_cli_channel(&config)
}

/// Assemble a CLI turn runtime starting from a config path on disk.
///
/// This is the highest-level bootstrap used by `chat`/`ask`: it loads the
/// config, permits implicit default-session resolution, exports runtime
/// environment variables, bootstraps a fresh kernel context, and delegates the
/// final session/memory assembly to the lower-level helpers below.
pub(crate) fn initialize_cli_turn_runtime(
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

/// Assemble a CLI turn runtime when the caller already owns a resolved config.
///
/// Compared with `initialize_cli_turn_runtime`, this skips config loading but
/// still normalizes the runtime workspace root, optionally exports runtime
/// environment variables, bootstraps a fresh kernel context, and then delegates
/// the final session/memory assembly to
/// `initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx`.
///
/// Use the `_and_kernel_ctx` variant when the caller must reuse an existing
/// kernel authority—such as channel-triggered turns—rather than minting a new
/// token for the same logical operation.
pub(crate) fn initialize_cli_turn_runtime_with_loaded_config(
    resolved_path: PathBuf,
    config: LoongConfig,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    kernel_scope: &'static str,
    session_requirement: CliSessionRequirement,
    initialize_runtime_environment: bool,
) -> CliResult<CliTurnRuntime> {
    let mut config = config;
    // Interactive chat surfaces should anchor tool-relative filesystem access
    // to the launch directory when possible, rather than forcing every turn to
    // inherit the static configured file root.
    let runtime_workspace_root = std::env::current_dir()
        .ok()
        .unwrap_or_else(|| config.tools.resolved_file_root());
    let runtime_workspace_root =
        dunce::canonicalize(&runtime_workspace_root).unwrap_or(runtime_workspace_root);
    let runtime_workspace_root = runtime_workspace_root.display().to_string();
    config.tools.runtime_workspace_root = Some(runtime_workspace_root);

    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));
    }
    let runtime_kernel =
        crate::runtime_bridge::RuntimeKernelOwner::bootstrap(kernel_scope, &config)?;
    let kernel_ctx = runtime_kernel.cloned_kernel_context();
    initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx(
        resolved_path,
        config,
        session_hint,
        options,
        kernel_ctx,
        session_requirement,
    )
}

/// Final assembly step for CLI/chat turn state once config and kernel authority
/// are already available.
///
/// This helper resolves ACP defaults, prepares memory/sqlite state, derives the
/// effective session id/address, and constructs the `CliTurnRuntime`. It
/// deliberately does not mutate process environment variables or bootstrap a
/// new kernel context; callers use it when those concerns were already handled
/// by an outer runtime surface.
pub(crate) fn initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx(
    resolved_path: PathBuf,
    config: LoongConfig,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    kernel_ctx: crate::KernelContext,
    session_requirement: CliSessionRequirement,
) -> CliResult<CliTurnRuntime> {
    let explicit_acp_request = options.requests_explicit_acp();
    let effective_bootstrap_mcp_servers = config
        .acp
        .dispatch
        .bootstrap_mcp_server_names_with_additions(&options.acp_bootstrap_mcp_servers)?;
    let effective_working_directory = options
        .acp_working_directory
        .clone()
        .or_else(|| config.acp.dispatch.resolved_working_directory());

    #[cfg(feature = "memory-sqlite")]
    let memory_config = SessionStoreConfig::from_memory_config(&config.memory);

    #[cfg(feature = "memory-sqlite")]
    let memory_label = {
        let sqlite_path = config.memory.resolved_sqlite_path();
        let initialized = store::ensure_session_store_ready(Some(sqlite_path), &memory_config)
            .map_err(|error| format!("failed to initialize sqlite memory: {error}"))?;
        initialized.display().to_string()
    };

    #[cfg(not(feature = "memory-sqlite"))]
    let memory_label = "disabled".to_owned();

    #[cfg(feature = "memory-sqlite")]
    let session_id =
        resolve_cli_runtime_session_id(session_hint, session_requirement, &memory_config)?;

    #[cfg(not(feature = "memory-sqlite"))]
    let session_id = resolve_cli_session_id(session_hint, session_requirement)?;

    let session_address = ConversationSessionAddress::from_session_id(session_id.clone());
    let runtime_kernel = crate::runtime_bridge::RuntimeKernelOwner::new(kernel_ctx);

    Ok(CliTurnRuntime {
        resolved_path,
        config,
        session_id,
        session_address,
        turn_coordinator: ConversationTurnCoordinator::new(),
        runtime_kernel,
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

#[cfg(feature = "memory-sqlite")]
fn resolve_cli_runtime_session_id(
    session_hint: Option<&str>,
    session_requirement: CliSessionRequirement,
    memory_config: &SessionStoreConfig,
) -> CliResult<String> {
    let session_id = resolve_cli_session_id(session_hint, session_requirement)?;
    let should_resolve_latest = session_requirement == CliSessionRequirement::AllowImplicitDefault
        && session_id == LATEST_SESSION_SELECTOR;

    if !should_resolve_latest {
        return Ok(session_id);
    }

    let latest_session_id = latest_resumable_root_session_id(memory_config)?;
    let latest_session_id = latest_session_id.ok_or_else(|| {
        "CLI session selector `latest` did not find any resumable root session".to_owned()
    })?;

    Ok(latest_session_id)
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_turn_checkpoint_startup_health(runtime: &CliTurnRuntime) {
    operator_surfaces::print_turn_checkpoint_startup_health(runtime).await;
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

        print!("{CLI_CHAT_COMPOSER_PROMPT}");
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
    options: &CliChatOptions,
    event_sink: Option<&dyn AcpTurnEventSink>,
) -> CliResult<CliChatLoopControl> {
    if input.is_empty() {
        return Ok(CliChatLoopControl::Continue);
    }
    if is_exit_command(&runtime.config, input) {
        return Ok(CliChatLoopControl::Exit);
    }
    match classify_chat_command_match_result(parse_exact_chat_command(
        input,
        &[CLI_CHAT_HELP_COMMAND],
        "usage: /help",
    ))? {
        ChatCommandMatchResult::Matched => {
            print_help();
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    match classify_chat_command_match_result(is_cli_chat_status_command(input))? {
        ChatCommandMatchResult::Matched => {
            print_cli_chat_status(runtime, options).await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    match classify_chat_command_match_result(is_manual_compaction_command(input))? {
        ChatCommandMatchResult::Matched => {
            print_manual_compaction(runtime).await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    match classify_chat_command_match_result(parse_exact_chat_command(
        input,
        &[CLI_CHAT_HISTORY_COMMAND],
        "usage: /history",
    ))? {
        ChatCommandMatchResult::Matched => {
            #[cfg(feature = "memory-sqlite")]
            print_history(
                &runtime.session_id,
                runtime.config.memory.sliding_window,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_history(
                &runtime.session_id,
                runtime.config.memory.sliding_window,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    let fast_lane_limit_result =
        parse_fast_lane_summary_limit(input, runtime.config.memory.sliding_window);
    match fast_lane_limit_result {
        Ok(Some(limit)) => {
            #[cfg(feature = "memory-sqlite")]
            print_fast_lane_summary(
                &runtime.session_id,
                limit,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_fast_lane_summary(&runtime.session_id, limit, runtime.conversation_binding())
                .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(usage_lines) = maybe_render_nonfatal_usage_error(error.as_str()) {
                print_rendered_cli_chat_lines(&usage_lines);
                return Ok(CliChatLoopControl::Continue);
            }

            return Err(error);
        }
    }

    let safe_lane_limit_result =
        parse_safe_lane_summary_limit(input, runtime.config.memory.sliding_window);
    match safe_lane_limit_result {
        Ok(Some(limit)) => {
            #[cfg(feature = "memory-sqlite")]
            print_safe_lane_summary(
                &runtime.session_id,
                limit,
                &runtime.config.conversation,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_safe_lane_summary(
                &runtime.session_id,
                limit,
                &runtime.config.conversation,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(usage_lines) = maybe_render_nonfatal_usage_error(error.as_str()) {
                print_rendered_cli_chat_lines(&usage_lines);
                return Ok(CliChatLoopControl::Continue);
            }

            return Err(error);
        }
    }

    let turn_checkpoint_limit_result =
        parse_turn_checkpoint_summary_limit(input, runtime.config.memory.sliding_window);
    match turn_checkpoint_limit_result {
        Ok(Some(limit)) => {
            #[cfg(feature = "memory-sqlite")]
            print_turn_checkpoint_summary(
                &runtime.turn_coordinator,
                &runtime.config,
                &runtime.session_id,
                limit,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_turn_checkpoint_summary(
                &runtime.turn_coordinator,
                &runtime.config,
                &runtime.session_id,
                limit,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(usage_lines) = maybe_render_nonfatal_usage_error(error.as_str()) {
                print_rendered_cli_chat_lines(&usage_lines);
                return Ok(CliChatLoopControl::Continue);
            }

            return Err(error);
        }
    }
    match classify_chat_command_match_result(is_turn_checkpoint_repair_command(input))? {
        ChatCommandMatchResult::Matched => {
            print_turn_checkpoint_repair(
                &runtime.turn_coordinator,
                &runtime.config,
                &runtime.session_id,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let render_width = detect_cli_chat_render_width();
            let usage_lines = render_cli_chat_command_usage_lines_with_width(&usage, render_width);
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }

    let turn_request = crate::agent_runtime::AgentTurnRequest {
        message: input.to_owned(),
        turn_mode: crate::agent_runtime::AgentTurnMode::Interactive,
        channel_id: runtime.session_address.channel_id.clone(),
        account_id: runtime.session_address.account_id.clone(),
        conversation_id: runtime.session_address.conversation_id.clone(),
        participant_id: runtime.session_address.participant_id.clone(),
        thread_id: runtime.session_address.thread_id.clone(),
        metadata: BTreeMap::new(),
        acp: runtime.explicit_acp_request,
        acp_event_stream: event_sink.is_some(),
        acp_bootstrap_mcp_servers: runtime.effective_bootstrap_mcp_servers.clone(),
        acp_cwd: runtime
            .effective_working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
        live_surface_enabled: true,
    };
    let turn_options = crate::agent_runtime::TurnExecutionOptions {
        event_sink,
        ..Default::default()
    };
    let turn_service = crate::agent_runtime::RuntimeTurnExecutionService::new(runtime);
    let turn_result = turn_service.execute(&turn_request, turn_options).await?;

    Ok(CliChatLoopControl::AssistantText(turn_result.output_text))
}

pub(crate) async fn run_cli_turn(
    runtime: &CliTurnRuntime,
    input: &str,
    event_sink: Option<&dyn AcpTurnEventSink>,
    live_surface_enabled: bool,
) -> CliResult<String> {
    run_cli_turn_with_address(
        runtime,
        &runtime.session_address,
        input,
        event_sink,
        live_surface_enabled,
        None,
        None,
    )
    .await
}

pub(crate) async fn run_cli_turn_with_address(
    runtime: &CliTurnRuntime,
    address: &ConversationSessionAddress,
    input: &str,
    event_sink: Option<&dyn AcpTurnEventSink>,
    live_surface_enabled: bool,
    metadata: Option<&BTreeMap<String, String>>,
    observer_override: Option<ConversationTurnObserverHandle>,
) -> CliResult<String> {
    run_cli_turn_with_address_and_ingress_and_error_mode(
        runtime,
        address,
        input,
        event_sink,
        live_surface_enabled,
        metadata,
        None,
        AcpTurnProvenance::default(),
        ProviderErrorMode::InlineMessage,
        observer_override,
        None,
        None,
    )
    .await
}

pub(crate) async fn run_cli_turn_with_address_and_ingress_and_error_mode(
    runtime: &CliTurnRuntime,
    address: &ConversationSessionAddress,
    input: &str,
    event_sink: Option<&dyn AcpTurnEventSink>,
    live_surface_enabled: bool,
    metadata: Option<&BTreeMap<String, String>>,
    ingress: Option<&ConversationIngressContext>,
    provenance: AcpTurnProvenance<'_>,
    provider_error_mode: ProviderErrorMode,
    observer_override: Option<ConversationTurnObserverHandle>,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
    acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
) -> CliResult<String> {
    run_cli_turn_with_address_and_ingress_and_error_mode_outcome(
        runtime,
        address,
        input,
        event_sink,
        live_surface_enabled,
        metadata,
        ingress,
        provenance,
        provider_error_mode,
        observer_override,
        retry_progress,
        acp_manager,
    )
    .await
    .map(|outcome| outcome.reply)
}

pub(crate) async fn run_cli_turn_with_address_and_ingress_and_error_mode_outcome(
    runtime: &CliTurnRuntime,
    address: &ConversationSessionAddress,
    input: &str,
    event_sink: Option<&dyn AcpTurnEventSink>,
    live_surface_enabled: bool,
    metadata: Option<&BTreeMap<String, String>>,
    ingress: Option<&ConversationIngressContext>,
    provenance: AcpTurnProvenance<'_>,
    provider_error_mode: ProviderErrorMode,
    observer_override: Option<ConversationTurnObserverHandle>,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
    acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
) -> CliResult<crate::conversation::ConversationTurnOutcome> {
    let turn_config = reload_cli_turn_config(&runtime.config, runtime.resolved_path.as_path())?;
    let acp_options = if runtime.explicit_acp_request {
        AcpConversationTurnOptions::explicit()
    } else {
        AcpConversationTurnOptions::automatic()
    }
    .with_event_sink(event_sink)
    .with_additional_bootstrap_mcp_servers(&runtime.effective_bootstrap_mcp_servers)
    .with_working_directory(runtime.effective_working_directory.as_deref())
    .with_metadata(metadata)
    .with_provenance(provenance);
    let live_surface_observer = if let Some(observer) = observer_override {
        Some(observer)
    } else if live_surface_enabled {
        let render_width = detect_cli_chat_render_width();
        Some(build_cli_chat_live_surface_observer(render_width))
    } else {
        None
    };
    let binding = runtime.conversation_binding();
    if let Some(ingress) = ingress {
        runtime
            .turn_coordinator
            .handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
                &turn_config,
                address,
                input,
                provider_error_mode,
                &acp_options,
                binding,
                Some(ingress),
                live_surface_observer,
                retry_progress,
                acp_manager,
            )
            .await
            .map(|reply| crate::conversation::ConversationTurnOutcome { reply, usage: None })
    } else {
        #[cfg(feature = "memory-sqlite")]
        let memory_config =
            crate::session::store::session_store_config_from_memory_config_without_env_overrides(
                &turn_config.memory,
            );
        #[cfg(feature = "memory-sqlite")]
        let hosted_runtime = crate::conversation::HostedConversationRuntime::new_with_memory_config(
            crate::conversation::DefaultConversationRuntime::from_config_or_env(&turn_config)?,
            memory_config,
        );
        #[cfg(not(feature = "memory-sqlite"))]
        let hosted_runtime =
            crate::conversation::DefaultConversationRuntime::from_config_or_env(&turn_config)?;

        runtime
            .turn_coordinator
            .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_outcome(
                &turn_config,
                address,
                input,
                provider_error_mode,
                &hosted_runtime,
                &acp_options,
                binding,
                None,
                live_surface_observer,
                retry_progress,
                acp_manager,
            )
            .await
    }
}

fn reload_cli_turn_config(config: &LoongConfig, resolved_path: &Path) -> CliResult<LoongConfig> {
    if resolved_path.as_os_str().is_empty() {
        return Ok(config.clone());
    }
    config.reload_provider_runtime_state_from_path(resolved_path)
}

fn is_exit_command(config: &LoongConfig, input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    config
        .cli
        .exit_commands
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .any(|value| !value.is_empty() && value == lower)
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_fast_lane_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &SessionStoreConfig,
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
    #[cfg(feature = "memory-sqlite")] memory_config: &SessionStoreConfig,
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
    config: &LoongConfig,
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] _memory_config: &SessionStoreConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let diagnostics = turn_coordinator
            .load_production_turn_checkpoint_diagnostics_with_limit(
                config, session_id, limit, binding,
            )
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
    config: &LoongConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let outcome = turn_coordinator
            .repair_production_turn_checkpoint_tail(config, session_id, binding)
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

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_turn_checkpoint_health_error_lines_with_width(
    session_id: &str,
    error: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_health_error_message_spec(session_id, error);
    render_cli_chat_message_spec_with_width(&message_spec, width)
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
        footer_lines: vec![
            "Durability state is unavailable until the next successful checkpoint sample."
                .to_owned(),
        ],
    }
}

#[cfg(test)]
#[cfg(any(test, feature = "memory-sqlite"))]
fn render_turn_checkpoint_startup_health_lines_with_width(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Option<Vec<String>> {
    operator_surfaces::render_turn_checkpoint_startup_health_lines_with_width(
        session_id,
        diagnostics,
        width,
    )
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn render_fast_lane_summary_lines_with_width(
    session_id: &str,
    limit: usize,
    summary: &FastLaneToolBatchEventSummary,
    width: usize,
) -> Vec<String> {
    let message_spec = build_fast_lane_summary_message_spec(session_id, limit, summary);
    render_cli_chat_message_spec_with_width(&message_spec, width)
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
        footer_lines: vec![
            "Use /fast_lane_summary after tool-heavy turns to inspect concurrency behavior."
                .to_owned(),
        ],
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
    memory_config: &SessionStoreConfig,
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
    render_cli_chat_message_spec_with_width(&message_spec, width)
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
        footer_lines: vec![
            "Use /safe_lane_summary when verify/replan behavior needs inspection.".to_owned(),
        ],
    }
}

#[cfg(test)]
async fn load_safe_lane_summary_output(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &SessionStoreConfig,
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
    render_cli_chat_message_spec_with_width(&message_spec, width)
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
        footer_lines: vec![
            "Use /turn_checkpoint_repair when the latest durable state needs repair.".to_owned(),
        ],
    }
}

#[cfg(test)]
async fn load_turn_checkpoint_summary_output(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongConfig,
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<String> {
    let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
    let runtime_ref = &runtime;
    let diagnostics_future = turn_coordinator
        .load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            config,
            session_id,
            limit,
            runtime_ref,
            binding,
        );
    let diagnostics = diagnostics_future.await?;

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
    render_cli_chat_message_spec_with_width(&message_spec, width)
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
        footer_lines: vec![
            "Re-run /status after repair to confirm the checkpoint state.".to_owned(),
        ],
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
    use crate::test_support::ScopedEnv;
    use serde_json::json;
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
    use loong_contracts::{Capability, ExecutionRoute, HarnessKind, MemoryPlaneError};
    #[cfg(feature = "memory-sqlite")]
    use loong_kernel::{
        CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongKernel, MemoryCoreOutcome,
        MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
    };
    #[cfg(feature = "memory-sqlite")]
    use serde_json::Value;

    #[cfg(feature = "memory-sqlite")]
    fn test_config() -> LoongConfig {
        let mut config = LoongConfig::default();
        config.provider = crate::config::ProviderConfig::default();
        config.audit.mode = crate::config::AuditMode::InMemory;
        config
    }

    #[cfg(feature = "memory-sqlite")]
    fn test_kernel_context_with_memory(
        agent_id: &str,
        memory_config: &SessionStoreConfig,
    ) -> crate::KernelContext {
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let audit = Arc::new(InMemoryAuditSink::default());
        let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

        let pack = VerticalPackManifest {
            pack_id: "test-pack-memory".to_owned(),
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

        kernel
            .register_pack(pack)
            .expect("register memory test pack");

        let adapter = crate::session::store::session_memory_adapter(memory_config);
        kernel.register_core_memory_adapter(adapter);

        kernel
            .set_default_core_memory_adapter("mvp-memory")
            .expect("set memory test adapter");

        let token = kernel
            .issue_token("test-pack-memory", agent_id, 60)
            .expect("issue memory test token");

        crate::KernelContext {
            kernel: Arc::new(kernel),
            token,
        }
    }

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
            build_onboard_command(None, Path::new("/tmp/loong.toml")).expect("onboard command");

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
    fn build_onboard_command_prefers_loong_test_override() {
        let mut env = ScopedEnv::new();
        env.remove(TEST_ONBOARD_EXECUTABLE_ENV);
        env.set(TEST_ONBOARD_EXECUTABLE_ENV, "/tmp/loong-onboard");

        let command =
            build_onboard_command(None, Path::new("/tmp/loong.toml")).expect("onboard command");

        assert_eq!(command.get_program(), OsStr::new("/tmp/loong-onboard"));
    }

    #[test]
    fn build_onboard_command_forwards_explicit_config_path_to_output() {
        let command = build_onboard_command_for_executable(
            PathBuf::from("/tmp/loong"),
            Some("custom.toml"),
            Path::new("/tmp/custom.toml"),
        );

        assert_eq!(command.get_program(), OsStr::new("/tmp/loong"));
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

        assert_eq!(hint, "loong onboard --output /tmp/custom.toml");
    }

    #[cfg(feature = "memory-sqlite")]
    fn unique_chat_sqlite_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "loong-chat-binding-{label}-{}.sqlite3",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    pub(super) fn cleanup_chat_test_memory(sqlite_path: &Path) {
        let _ = std::fs::remove_file(sqlite_path);
        let _ = std::fs::remove_file(format!("{}-wal", sqlite_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", sqlite_path.display()));
    }

    #[cfg(feature = "memory-sqlite")]
    pub(super) fn init_chat_test_memory(label: &str) -> (LoongConfig, SessionStoreConfig, PathBuf) {
        let sqlite_path = unique_chat_sqlite_path(label);
        cleanup_chat_test_memory(&sqlite_path);

        let mut config = LoongConfig::default();
        config.audit.mode = crate::config::AuditMode::InMemory;
        config.memory.sqlite_path = sqlite_path.display().to_string();
        let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
        store::ensure_session_store_ready(
            Some(config.memory.resolved_sqlite_path()),
            &memory_config,
        )
        .expect("initialize sqlite memory");

        (config, memory_config, sqlite_path)
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn init_chat_test_memory_uses_in_memory_audit_mode() {
        let (config, _memory_config, sqlite_path) = init_chat_test_memory("audit-mode");

        assert_eq!(config.audit.mode, crate::config::AuditMode::InMemory);

        cleanup_chat_test_memory(&sqlite_path);
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
        let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

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
        memory_config: &SessionStoreConfig,
    ) {
        for payload in payloads {
            store::append_session_turn_direct(session_id, "assistant", payload, memory_config)
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
            resolved_path: PathBuf::from("/tmp/loong.toml"),
            config: LoongConfig::default(),
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
            PathBuf::from("/tmp/loong.toml"),
            config,
            Some("cli-supervisor"),
            &options,
            "cli-chat-concurrent-test",
            CliSessionRequirement::RequireExplicit,
            false,
        )
        .expect("concurrent host runtime");
        assert!(runtime.conversation_binding().is_kernel_bound());
        assert_eq!(
            runtime.runtime_kernel.kernel_context().agent_id(),
            "cli-chat-concurrent-test"
        );
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
        store::append_session_turn_direct(session_id, "user", "hello", &memory_config)
            .expect("persist user turn");
        store::append_session_turn_direct(session_id, "assistant", "world", &memory_config)
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
                workspace_root: Some("/workspace/project".to_owned()),
                provider_label: "OpenAI / gpt-5.4".to_owned(),
                config_path: "/tmp/loong.toml".to_owned(),
                memory_label: "/tmp/loong.db".to_owned(),
                session_id: "default".to_owned(),
                context_engine_id: "threaded".to_owned(),
                context_engine_source: "config".to_owned(),
                compaction_enabled: true,
                compaction_min_messages: Some(6),
                compaction_trigger_estimated_tokens: Some(120),
                compaction_preserve_recent_turns: 4,
                compaction_fail_open: false,
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
            lines.iter().any(|line| line.contains("v0.1.2-alpha.1")),
            "chat startup should now surface the richer branded header: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line
                == "start here: Summarize this repository and suggest the best next step."),
            "chat startup should render the first answer handoff through the structured action group: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("quick commands")),
            "chat startup should surface the quick-commands section beside the first answer handoff: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("current setup snapshot")),
            "chat startup should now open on a current-setup snapshot instead of a runtime dashboard: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- workspace: /workspace/project")),
            "chat startup should anchor the shell to the current workspace: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- provider: OpenAI / gpt-5.4")),
            "chat startup should surface the current provider in the setup snapshot: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("fast lane")),
            "chat startup should surface the onboarding-style fast lane callout: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("note: how chat works")),
            "chat startup should keep the usage guidance as a structured callout: {lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line == "runtime posture"),
            "chat startup should leave the dedicated runtime-posture block to /status: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_startup_lines_surface_explicit_acp_overrides() {
        let lines = render_cli_chat_startup_lines_with_width(
            &CliChatStartupSummary {
                workspace_root: Some("/workspace/project".to_owned()),
                provider_label: "OpenAI / gpt-5.4".to_owned(),
                config_path: "/tmp/loong.toml".to_owned(),
                memory_label: "/tmp/loong.db".to_owned(),
                session_id: "thread-42".to_owned(),
                context_engine_id: "threaded".to_owned(),
                context_engine_source: "env".to_owned(),
                compaction_enabled: true,
                compaction_min_messages: Some(6),
                compaction_trigger_estimated_tokens: Some(120),
                compaction_preserve_recent_turns: 4,
                compaction_fail_open: false,
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
            lines
                .iter()
                .any(|line| line.contains("note: acp overrides")),
            "chat startup should group ACP overrides under a dedicated callout heading: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- bootstrap MCP servers: filesystem")),
            "chat startup should still surface the bootstrap MCP override details: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- working directory: /workspace/project")),
            "chat startup should still surface the working directory override: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_startup_output_uses_rich_shell_when_requested() {
        let lines = operator_surfaces::render_cli_chat_startup_output_with_width(
            &CliChatStartupSummary {
                workspace_root: Some("/workspace/project".to_owned()),
                provider_label: "OpenAI / gpt-5.4".to_owned(),
                config_path: "/tmp/loong.toml".to_owned(),
                memory_label: "/tmp/loong.db".to_owned(),
                session_id: "default".to_owned(),
                context_engine_id: "threaded".to_owned(),
                context_engine_source: "config".to_owned(),
                compaction_enabled: true,
                compaction_min_messages: Some(6),
                compaction_trigger_estimated_tokens: Some(120),
                compaction_preserve_recent_turns: 4,
                compaction_fail_open: false,
                acp_enabled: false,
                dispatch_enabled: false,
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
            true,
        );

        assert!(
            lines.iter().any(|line| line.contains("chat ready")),
            "rich startup shell should keep the guided startup title visible: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("│") || line.contains("╭") || line.contains("╰")),
            "rich startup shell should render inside shell blocks instead of plain lines: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_status_lines_focus_on_runtime_state_without_start_here() {
        let lines = render_cli_chat_status_lines_with_width(
            &CliChatStartupSummary {
                workspace_root: None,
                provider_label: "OpenAI / gpt-5.4".to_owned(),
                config_path: "/tmp/loong.toml".to_owned(),
                memory_label: "/tmp/loong.db".to_owned(),
                session_id: "default".to_owned(),
                context_engine_id: "threaded".to_owned(),
                context_engine_source: "config".to_owned(),
                compaction_enabled: true,
                compaction_min_messages: Some(6),
                compaction_trigger_estimated_tokens: Some(120),
                compaction_preserve_recent_turns: 4,
                compaction_fail_open: false,
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

        assert_eq!(lines[0], "╭─ chat status · session=default");
        assert!(
            lines.iter().any(|line| line.contains("session anchor")),
            "status output should keep session facts grouped under a section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("runtime posture")),
            "status output should keep runtime facts grouped under a section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("continuity guardrails")),
            "status output should surface compaction maintenance settings: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("note: next moves")),
            "status output should include the operator control callout: {lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line.starts_with("start here:")),
            "status output should not re-render the first-turn guidance block: {lines:#?}"
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
        let command = "loong onboard --output /tmp/loong.toml";
        let lines = render_cli_chat_missing_config_lines_with_width(command, 80);

        assert!(
            lines.first().is_some_and(|line| line.starts_with("LOONG")),
            "missing-config setup prompt should keep the shared compact header: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "setup required"),
            "missing-config setup prompt should promote the title into the shared screen surface: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "setup command: loong onboard --output /tmp/loong.toml"),
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

        assert_eq!(lines[0], "╭─ checkpoint · session=session-health");
        assert!(
            lines.iter().any(|line| line.contains("durability status")),
            "startup health should group durability facts under a shared key-value section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("attention: recovery")),
            "startup health should surface pending recovery as a warning callout: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- action: inspect_manually")),
            "startup health should preserve the concrete recovery action in the callout: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("note: runtime probe")),
            "startup health should surface runtime probe context as a secondary structured callout: {lines:#?}"
        );
    }

    #[test]
    fn render_turn_checkpoint_startup_health_lines_skip_non_durable_sessions() {
        let summary = TurnCheckpointEventSummary {
            session_state: TurnCheckpointSessionState::NotDurable,
            checkpoint_durable: false,
            reply_durable: false,
            ..TurnCheckpointEventSummary::default()
        };
        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);

        let lines = render_turn_checkpoint_startup_health_lines_with_width(
            "session-health",
            &diagnostics,
            80,
        );

        assert!(
            lines.is_none(),
            "startup health should stay quiet until a durable checkpoint exists"
        );
    }

    #[test]
    fn render_turn_checkpoint_startup_health_lines_surface_non_durable_recovery() {
        let summary = TurnCheckpointEventSummary {
            session_state: TurnCheckpointSessionState::NotDurable,
            checkpoint_durable: false,
            reply_durable: false,
            requires_recovery: true,
            ..TurnCheckpointEventSummary::default()
        };
        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);

        let lines = render_turn_checkpoint_startup_health_lines_with_width(
            "session-health",
            &diagnostics,
            80,
        )
        .expect("non-durable recovery should still render");

        assert!(
            lines
                .iter()
                .any(|line| line.contains("recovery needed: yes")),
            "startup health should surface non-durable recovery cases: {lines:#?}"
        );
    }

    #[test]
    fn render_turn_checkpoint_status_health_lines_surface_non_durable_sessions() {
        let summary = TurnCheckpointEventSummary {
            session_state: TurnCheckpointSessionState::NotDurable,
            checkpoint_durable: false,
            reply_durable: false,
            ..TurnCheckpointEventSummary::default()
        };
        let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
        let lines = operator_surfaces::render_turn_checkpoint_status_health_lines_with_width(
            "session-health",
            &diagnostics,
            80,
        );

        assert_eq!(lines[0], "╭─ checkpoint · session=session-health");
        assert!(
            lines.iter().any(|line| line.contains("state: not_durable")),
            "status health should surface non-durable sessions explicitly: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("checkpoint durable: no")),
            "status health should surface checkpoint durability explicitly: {lines:#?}"
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

        assert_eq!(lines[0], "╭─ fast-lane · session=session-fast limit=64");
        assert!(
            lines.iter().any(|line| line.contains("intent mix")),
            "fast-lane summary should promote aggregate intent counters into a titled section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("latest segments")),
            "fast-lane summary should keep the latest segment narrative visible: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line.contains(
                    "- segment 0: class=parallel_safe mode=parallel intents=2 peak=3 wall_ms=120",
                )
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

        assert_eq!(lines[0], "╭─ safe-lane · session=session-safe limit=32");
        assert!(
            lines.iter().any(|line| line.contains("attention: health")),
            "safe-lane summary should surface warning health as a structured callout: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- severity: critical")),
            "safe-lane health callout should preserve the derived severity: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("rollups")),
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

        assert_eq!(lines[0], "╭─ checkpoint · session=session-summary limit=64");
        assert!(
            lines.iter().any(|line| line.contains("summary")),
            "turn checkpoint summary should group the latest durability state in a titled section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("note: runtime probe")),
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

        assert_eq!(lines[0], "╭─ repair · session=session-repair");
        assert!(
            lines.iter().any(|line| line.contains("repair status")),
            "turn checkpoint repair should group repair facts in a structured key-value section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("attention: repair result")),
            "manual repair outcomes should surface a warning callout: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_help_lines_promotes_commands_to_surface() {
        let lines = render_cli_chat_help_lines_with_width(72);

        assert_eq!(lines[0], "╭─ help · operator deck");
        assert!(
            lines.iter().any(|line| line.contains("slash commands")),
            "help output should keep a dedicated slash-command section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("/history: print the current session sliding window")),
            "help output should render slash commands as readable key-value rows: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line.contains("/review: reopen the latest approval/review summary")
            }),
            "help output should surface the review command: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("/mission:") && line.contains("mission control"))
                || lines.iter().any(|line| line.contains("/mission:"))
                    && lines
                        .iter()
                        .any(|line| line.contains("current session scope")),
            "help output should surface the mission-control command: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("/sessions: inspect visible sessions")),
            "help output should surface the session queue command: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line.contains("/workers: inspect visible worker/delegate sessions")
            }),
            "help output should surface the workers command: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line
                .contains("/status: show session, runtime, compaction, and durability status")),
            "help output should surface the status command: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line
                .contains("/compact: write a continuity-safe checkpoint into the active window")),
            "help output should surface the manual compaction command: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("note: surface controls")),
            "help output should surface the command-menu control deck: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("keyboard")),
            "help output should surface keyboard shortcuts as a first-class section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("note: usage notes")),
            "help output should preserve operator guidance as a callout: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_command_usage_lines_wrap_usage_in_warning_card() {
        let lines = render_cli_chat_command_usage_lines_with_width("usage: /history", 72);

        assert_eq!(lines[0], "╭─ chat · command");
        assert!(
            lines.iter().any(|line| line.contains("attention: usage")),
            "usage errors should render inside a warning pane: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("usage: /history")),
            "usage pane should preserve the concrete command usage: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_status_lines_surface_runtime_and_compaction_controls() {
        let summary = CliChatStartupSummary {
            workspace_root: None,
            provider_label: "OpenAI / gpt-5.4".to_owned(),
            config_path: "/tmp/loong.toml".to_owned(),
            memory_label: "window_plus_summary".to_owned(),
            session_id: "session-status".to_owned(),
            context_engine_id: "default".to_owned(),
            context_engine_source: "config".to_owned(),
            compaction_enabled: true,
            compaction_min_messages: Some(6),
            compaction_trigger_estimated_tokens: Some(12_000),
            compaction_preserve_recent_turns: 4,
            compaction_fail_open: true,
            acp_enabled: true,
            dispatch_enabled: true,
            conversation_routing: "auto".to_owned(),
            allowed_channels: vec!["cli".to_owned()],
            acp_backend_id: "builtin".to_owned(),
            acp_backend_source: "config".to_owned(),
            explicit_acp_request: false,
            event_stream_enabled: false,
            bootstrap_mcp_servers: Vec::new(),
            working_directory: None,
        };

        let lines = render_cli_chat_status_lines_with_width(&summary, 80);

        assert_eq!(lines[0], "╭─ chat status · session=session-status");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("continuity guardrails")),
            "status output should expose compaction settings as a dedicated section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("compaction: true")),
            "status output should expose compaction enablement: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("trigger tokens: 12000")),
            "status output should surface the compaction token trigger: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("note: next moves")),
            "status output should append the operator controls callout: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("checkpoint the active session window on demand")),
            "status output should direct operators toward manual compaction: {lines:#?}"
        );
    }

    #[test]
    fn render_manual_compaction_lines_surface_structured_result() {
        let result = ManualCompactionResult {
            status: ManualCompactionStatus::Applied,
            before_turns: 8,
            after_turns: 3,
            estimated_tokens_before: Some(1200),
            estimated_tokens_after: Some(420),
            summary_headline: Some("Compacted 6 earlier turns".to_owned()),
            detail: "Compacted 6 earlier turns. Session-local recall only. It does not replace Runtime Self Context.".to_owned(),
        };

        let lines = render_manual_compaction_lines_with_width("session-compact", &result, 80);

        assert_eq!(lines[0], "╭─ compact · session=session-compact");
        assert!(
            lines.iter().any(|line| line.contains("compaction result")),
            "manual compaction should render a dedicated result section: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("status: applied")),
            "manual compaction should surface the applied status: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("tokens after: 420")),
            "manual compaction should surface token estimates: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Runtime Self Context")),
            "manual compaction should preserve the continuity boundary detail: {lines:#?}"
        );
    }

    #[test]
    fn render_cli_chat_history_lines_wrap_history_in_surface() {
        let history_lines = vec![
            "user: summarize the current repo".to_owned(),
            "assistant: start with the daemon crate".to_owned(),
        ];
        let lines = render_cli_chat_history_lines_with_width("session-7", 24, &history_lines, 72);

        assert_eq!(lines[0], "╭─ history · session=session-7 limit=24");
        assert!(
            lines.iter().any(|line| line.contains("sliding window")),
            "history output should keep a dedicated window section: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("user: summarize the current repo")),
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

        assert_eq!(lines[0], "╭─ loong · reply");
        assert!(
            lines.iter().any(|line| line.contains("Plan")),
            "markdown headings should become section titles: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- inspect the active config")),
            "markdown list items should remain visible in the narrative block: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- compare runtime state")),
            "markdown star bullets should normalize into wrapped display bullets: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("note: quoted context")),
            "markdown blockquotes should render as structured callouts: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("code [rust]")),
            "markdown fences should render as preformatted sections: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("let value = input.trim();")),
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
            lines.iter().any(|line| line.contains("note: Risks")),
            "headings should stay attached to quoted sections instead of falling back to a generic title: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("- keep credentials in env vars")),
            "quoted content should stay visible after preserving the heading: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("Next")),
            "a trailing heading should still render even when it has no body lines yet: {lines:#?}"
        );
    }

    #[test]
    fn parse_cli_chat_markdown_sections_promotes_reasoning_heading_to_callout() {
        let sections = parse_cli_chat_markdown_sections(
            "## Reasoning\nThe provider compared two options before choosing one.",
        );
        assert!(matches!(
            sections.first(),
            Some(TuiSectionSpec::Callout { title, .. }) if title.as_deref() == Some("reasoning")
        ));
    }

    #[test]
    fn parse_cli_chat_markdown_sections_promotes_tool_activity_heading_to_callout() {
        let sections = parse_cli_chat_markdown_sections(
            "## Tool Activity\nread completed with 1 result line.",
        );
        assert!(matches!(
            sections.first(),
            Some(TuiSectionSpec::Callout { title, .. }) if title.as_deref() == Some("tool activity")
        ));
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
            first_token_latency_ms: Some(123),
            draft_preview: Some("Inspecting the repo layout...".to_owned()),
            tools: Vec::new(),
        };
        let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, 72);

        assert_eq!(
            lines[0],
            "╭─ loong · live · round 1 · 4 msgs · ~128 tok · ttft 123ms"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("note: querying model")),
            "live surface should explain the active phase through a callout: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("turn pipeline")),
            "live surface should keep the pipeline checklist visible: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line.contains("[WARN] call model") && line.contains("first token in 123 ms")
            }),
            "live surface should keep the model step actively highlighted: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("first token") && line.contains("123 ms")),
            "live surface should surface first-token latency in status rows: {lines:#?}"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("streaming provider round 1")),
            "live surface should avoid claiming streaming when the snapshot does not encode that capability: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("draft preview")),
            "live surface should surface partial text as a dedicated preview block: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Inspecting the repo layout...")),
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
            elapsed_ms: Some(42),
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
            .find(|lines| lines.iter().any(|line| line.contains("draft preview")))
            .expect("preview batch");
        assert!(
            preview_batch
                .iter()
                .any(|line| line.contains("Draft response")),
            "preview batch should include the streamed text: {preview_batch:#?}"
        );
        assert!(
            preview_batch.iter().any(|line| line.contains("ttft 42ms")),
            "preview batch should include the first-token latency in the title: {preview_batch:#?}"
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
            elapsed_ms: None,
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
            elapsed_ms: None,
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
            .find(|lines| lines.iter().any(|line| line.contains("tool activity")))
            .expect("running tool batch");
        let completed_batch = batches
            .iter()
            .rev()
            .find(|lines| {
                lines
                    .iter()
                    .any(|line| line.contains("[completed] read (id=call-tool-1) - ok"))
            })
            .expect("completed tool batch");

        assert!(
            running_batch
                .iter()
                .any(|line| line.contains("[running] read (id=call-tool-1)")),
            "tool batch should surface the running tool state: {running_batch:#?}"
        );

        assert!(
            completed_batch
                .iter()
                .any(|line| line.contains("[completed] read (id=call-tool-1) - ok")),
            "tool batch should surface the completed tool state: {completed_batch:#?}"
        );
        assert!(
            completed_batch
                .iter()
                .any(|line| line.contains("args: {\"path\":\"README.md\"}")),
            "tool batch should preserve streamed tool args: {completed_batch:#?}"
        );
    }

    #[test]
    fn cli_chat_live_surface_observer_renders_runtime_output_and_file_change_updates() {
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
        observer.on_tool(
            ConversationTurnToolEvent::running("call-tool-2", "shell.exec")
                .with_request_summary(Some("{\"command\":\"printf\"}".to_owned())),
        );
        observer.on_runtime(ConversationTurnRuntimeEvent::new(
            "call-tool-2",
            ToolRuntimeEvent::OutputDelta(ToolOutputDelta {
                stream: ToolRuntimeStream::Stdout,
                chunk: "first line\nsecond line".to_owned(),
                total_bytes: 22,
                total_lines: 2,
                truncated: false,
            }),
        ));
        observer.on_runtime(ConversationTurnRuntimeEvent::new(
            "call-tool-2",
            ToolRuntimeEvent::FileChangePreview(ToolFileChangePreview {
                path: "src/lib.rs".to_owned(),
                kind: ToolFileChangeKind::Edit,
                added_lines: 2,
                removed_lines: 1,
                preview: Some("@@ -1,1 +1,2 @@\n-old\n+new\n+line".to_owned()),
            }),
        ));
        observer.on_runtime(ConversationTurnRuntimeEvent::new(
            "call-tool-2",
            ToolRuntimeEvent::CommandMetrics(ToolCommandMetrics {
                exit_code: Some(0),
                duration_ms: 42,
            }),
        ));
        observer.on_tool(ConversationTurnToolEvent::completed(
            "call-tool-2",
            "shell.exec",
            Some("ok".to_owned()),
        ));

        let batches = captured_batches
            .lock()
            .expect("captured batches lock should not be poisoned");
        let final_batch = batches.last().expect("final runtime batch");

        assert!(
            final_batch
                .iter()
                .any(|line| line.contains("[completed] exec (id=call-tool-2) - ok")),
            "runtime output should surface the visible tool name: {final_batch:#?}"
        );
        assert!(
            final_batch
                .iter()
                .any(|line| line.contains("stdout: 2 lines · 22 bytes")),
            "runtime output should surface stdout counters: {final_batch:#?}"
        );
        assert!(
            final_batch.iter().any(|line| line.contains("first line")),
            "runtime output should retain stdout preview lines: {final_batch:#?}"
        );
        assert!(
            final_batch
                .iter()
                .any(|line| line.contains("file: edit src/lib.rs (+2 / -1)")),
            "runtime output should surface file change summaries: {final_batch:#?}"
        );
        assert!(
            final_batch
                .iter()
                .any(|line| line.contains("metrics: 42ms · exit=0")),
            "runtime output should surface command metrics: {final_batch:#?}"
        );
    }

    #[test]
    fn build_cli_chat_live_surface_snapshot_preserves_structured_tool_state() {
        let mut state = CliChatLiveSurfaceState {
            latest_phase_event: Some(ConversationTurnPhaseEvent::running_tools(
                1,
                ExecutionLane::Fast,
                1,
            )),
            first_token_latency_ms: Some(88),
            ..CliChatLiveSurfaceState::default()
        };

        let tool_state = ensure_cli_chat_live_tool_state(&mut state, "call-structured");
        tool_state.name = Some("shell.exec".to_owned());
        tool_state.request_summary = Some("{\"command\":\"printf\"}".to_owned());
        tool_state.args = "{\"command\":\"printf\"}".to_owned();
        tool_state.stdout = CliChatLiveOutputView {
            text: "hello".to_owned(),
            total_bytes: 5,
            total_lines: 1,
            truncated: false,
        };
        tool_state.duration_ms = Some(12);
        tool_state.exit_code = Some(0);

        let snapshot =
            build_cli_chat_live_surface_snapshot(&state).expect("snapshot should be available");
        let tool = snapshot
            .tools
            .first()
            .expect("snapshot should include one tool");

        assert_eq!(snapshot.tools.len(), 1);
        assert_eq!(tool.tool_call_id, "call-structured");
        assert_eq!(tool.name.as_deref(), Some("exec"));
        assert_eq!(
            tool.request_summary.as_deref(),
            Some("{\"command\":\"printf\"}")
        );
        assert_eq!(tool.args, "{\"command\":\"printf\"}");
        assert_eq!(tool.stdout.text, "hello");
        assert_eq!(tool.duration_ms, Some(12));
        assert_eq!(tool.exit_code, Some(0));
        assert_eq!(snapshot.first_token_latency_ms, Some(88));
    }

    #[test]
    fn build_cli_chat_live_surface_snapshot_keeps_precise_hidden_tool_names() {
        let mut state = CliChatLiveSurfaceState {
            latest_phase_event: Some(ConversationTurnPhaseEvent::running_tools(
                1,
                ExecutionLane::Fast,
                1,
            )),
            ..CliChatLiveSurfaceState::default()
        };

        let tool_state = ensure_cli_chat_live_tool_state(&mut state, "call-hidden");
        tool_state.name = Some("delegate_async".to_owned());

        let snapshot =
            build_cli_chat_live_surface_snapshot(&state).expect("snapshot should be available");
        let tool = snapshot
            .tools
            .first()
            .expect("snapshot should include one tool");

        assert_eq!(tool.name.as_deref(), Some("delegate_async"));
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
            elapsed_ms: Some(55),
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
            elapsed_ms: None,
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
            !last_batch.iter().any(|line| line.contains("draft preview")),
            "follow-up provider requests should reset the previous draft preview: {last_batch:#?}"
        );
        assert!(
            !last_batch.iter().any(|line| line.contains("tool activity")),
            "follow-up provider requests should not reuse prior tool activity lines: {last_batch:#?}"
        );
        assert!(
            !last_batch.iter().any(|line| line.contains("ttft 55ms")),
            "follow-up provider requests should reset prior first-token latency: {last_batch:#?}"
        );
        assert!(
            !last_batch
                .iter()
                .any(|line| line.contains("Draft response")),
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
            elapsed_ms: None,
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
            last_batch.iter().any(|line| line.contains("tool activity")),
            "the tools phase should render the accumulated tool activity: {last_batch:#?}"
        );
        assert!(
            last_batch
                .iter()
                .any(|line| line.contains("[running] search (id=call_123)")),
            "the tools phase should surface the streamed tool metadata: {last_batch:#?}"
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn reload_cli_turn_config_refreshes_provider_state_without_mutating_cli_settings() {
        let path = std::env::temp_dir().join(format!(
            "loong-chat-provider-reload-{}.toml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let path_string = path.display().to_string();

        let mut in_memory = LoongConfig::default();
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

    #[test]
    fn is_cli_chat_status_command_accepts_exact_match_and_rejects_extra_args() {
        assert!(is_cli_chat_status_command("/status").expect("parse"));
        assert!(!is_cli_chat_status_command("/history").expect("parse"));

        let error =
            is_cli_chat_status_command("/status now").expect_err("extra args should be rejected");
        assert_eq!(error, "usage: /status");
    }

    #[test]
    fn is_manual_compaction_command_accepts_exact_match_and_rejects_extra_args() {
        assert!(is_manual_compaction_command("/compact").expect("parse"));
        assert!(!is_manual_compaction_command("/history").expect("parse"));

        let error = is_manual_compaction_command("/compact now")
            .expect_err("extra args should be rejected");
        assert_eq!(error, "usage: /compact");
    }

    #[test]
    fn help_and_history_commands_reject_extra_args() {
        let help_error =
            parse_exact_chat_command("/help now", &[CLI_CHAT_HELP_COMMAND], "usage: /help")
                .expect_err("help should reject extra args");
        assert_eq!(help_error, "usage: /help");

        let history_error = parse_exact_chat_command(
            "/history now",
            &[CLI_CHAT_HISTORY_COMMAND],
            "usage: /history",
        )
        .expect_err("history should reject extra args");
        assert_eq!(history_error, "usage: /history");

        let sessions_error = parse_exact_chat_command(
            "/sessions now",
            &[CLI_CHAT_SESSIONS_COMMAND],
            "usage: /sessions",
        )
        .expect_err("sessions should reject extra args");
        assert_eq!(sessions_error, "usage: /sessions");

        let mission_error = parse_exact_chat_command(
            "/mission now",
            &[CLI_CHAT_MISSION_COMMAND],
            "usage: /mission",
        )
        .expect_err("mission should reject extra args");
        assert_eq!(mission_error, "usage: /mission");

        let review_error =
            parse_exact_chat_command("/review now", &[CLI_CHAT_REVIEW_COMMAND], "usage: /review")
                .expect_err("review should reject extra args");
        assert_eq!(review_error, "usage: /review");

        let workers_error = parse_exact_chat_command(
            "/workers now",
            &[CLI_CHAT_WORKERS_COMMAND],
            "usage: /workers",
        )
        .expect_err("workers should reject extra args");
        assert_eq!(workers_error, "usage: /workers");
    }

    #[test]
    fn classify_chat_command_match_result_treats_usage_as_non_fatal() {
        let usage_result =
            classify_chat_command_match_result(Err("usage: /help".to_owned())).expect("classify");
        assert_eq!(
            usage_result,
            ChatCommandMatchResult::UsageError("usage: /help".to_owned())
        );

        let matched_result =
            classify_chat_command_match_result(Ok(true)).expect("classify matched");
        assert_eq!(matched_result, ChatCommandMatchResult::Matched);

        let not_matched_result =
            classify_chat_command_match_result(Ok(false)).expect("classify non-match");
        assert_eq!(not_matched_result, ChatCommandMatchResult::NotMatched);
    }

    #[test]
    fn maybe_render_nonfatal_usage_error_accepts_embedded_usage_text() {
        let error = "invalid fast lane summary limit `nope`; usage: /fast_lane_summary [limit]";
        let usage_lines =
            maybe_render_nonfatal_usage_error(error).expect("usage should render non-fatally");

        assert!(
            usage_lines
                .iter()
                .any(|line| line.contains("/fast_lane_summary [limit]")),
            "embedded usage text should still render the usage card: {usage_lines:#?}"
        );
    }

    #[test]
    fn manual_compaction_status_from_report_maps_failed_open() {
        let report = ContextCompactionReport {
            status: TurnCheckpointProgressStatus::FailedOpen,
            estimated_tokens_before: Some(420),
            estimated_tokens_after: Some(420),
        };

        let status =
            manual_compaction_status_from_report(&report).expect("failed_open should map cleanly");

        assert_eq!(status, ManualCompactionStatus::FailedOpen);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn manual_compaction_result_applies_and_surfaces_continuity_checkpoint() {
        let mut config = test_config();
        let sqlite_path = unique_chat_sqlite_path("chat-manual-compaction");
        cleanup_chat_test_memory(&sqlite_path);
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config.memory.sliding_window = 32;
        config.conversation.compact_enabled = false;
        config.conversation.compact_preserve_recent_turns = 2;

        let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
        let kernel_ctx = test_kernel_context_with_memory("chat-manual-compaction", &memory_config);
        let session_id = "chat-manual-compaction";

        for (role, content) in [
            ("user", "ask 1"),
            ("assistant", "reply 1"),
            ("user", "ask 2"),
            ("assistant", "reply 2"),
            ("user", "ask 3"),
            ("assistant", "reply 3"),
            ("user", "recent ask"),
            ("assistant", "recent reply"),
        ] {
            store::append_session_turn_direct(session_id, role, content, &memory_config)
                .expect("seed turns should succeed");
        }

        let binding = ConversationRuntimeBinding::kernel(&kernel_ctx);
        let turn_coordinator = ConversationTurnCoordinator::new();
        let result = load_manual_compaction_result(&config, session_id, &turn_coordinator, binding)
            .await
            .expect("manual compaction should succeed");

        assert_eq!(result.status, ManualCompactionStatus::Applied);
        assert_eq!(result.before_turns, 8);
        assert_eq!(result.after_turns, 3);
        assert!(
            result.estimated_tokens_before.is_some(),
            "manual compaction should surface a before-token estimate"
        );
        assert!(
            result.estimated_tokens_after.is_some(),
            "manual compaction should surface an after-token estimate"
        );
        assert!(
            result
                .summary_headline
                .as_deref()
                .is_some_and(|headline| headline.contains("Compacted 6 earlier turns"))
        );
        assert!(
            result.detail.contains("Runtime Self Context"),
            "manual compaction detail should reuse the continuity boundary note"
        );

        let turns = store::window_session_turns(session_id, 32, &memory_config)
            .expect("window load should succeed");
        assert!(
            turns[0]
                .content
                .contains("Does not replace Runtime Self Context"),
            "manual compaction should persist the continuity-aware checkpoint"
        );

        cleanup_chat_test_memory(&sqlite_path);
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
