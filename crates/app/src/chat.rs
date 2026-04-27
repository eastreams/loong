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

mod boot;
mod checkpoint;
mod checkpoint_labels;
#[cfg(test)]
mod checkpoint_text;
mod cli_input;
mod cli_render;
mod commands;
mod control_plane;
mod deck;
mod fast;
mod history;
#[cfg(all(test, feature = "memory-sqlite"))]
#[allow(clippy::expect_used)]
mod latest_session_selector_tests;
mod live_runtime;
mod onboard;
mod ops;
mod render;
mod report;
mod safe;
mod safe_text;
mod session;
mod startup_state;
mod startup_view;
mod status_view;

use self::boot::*;
use self::checkpoint::*;
use self::checkpoint_labels::*;
#[cfg(test)]
use self::checkpoint_text::*;
use self::cli_input::ConcurrentCliInputReader;
use self::cli_render::*;
use self::commands::*;
use self::fast::*;
use self::live_runtime::*;
#[cfg(test)]
use self::ops::CliChatStartupSummary;
#[cfg(test)]
use self::ops::ManualCompactionResult;
#[cfg(test)]
use self::ops::ManualCompactionStatus;
#[cfg(test)]
use self::ops::build_cli_chat_startup_summary;
use self::ops::is_cli_chat_status_command;
use self::ops::is_manual_compaction_command;
use self::ops::is_turn_checkpoint_repair_command;
#[cfg(test)]
use self::ops::load_history_lines;
#[cfg(test)]
use self::ops::load_manual_compaction_result;
#[cfg(test)]
use self::ops::manual_compaction_status_from_report;
use self::ops::parse_exact_chat_command;
use self::ops::parse_fast_lane_summary_limit;
use self::ops::parse_safe_lane_summary_limit;
#[cfg(test)]
use self::ops::parse_summary_limit;
use self::ops::parse_turn_checkpoint_summary_limit;
use self::ops::print_cli_chat_startup;
use self::ops::print_cli_chat_status;
use self::ops::print_fast_lane_summary;
use self::ops::print_help;
use self::ops::print_history;
use self::ops::print_manual_compaction;
use self::ops::print_safe_lane_summary;
use self::ops::print_turn_checkpoint_repair;
use self::ops::print_turn_checkpoint_summary;
#[cfg(test)]
use self::ops::render_cli_chat_help_lines_with_width;
#[cfg(test)]
use self::ops::render_cli_chat_history_lines_with_width;
use self::ops::render_cli_chat_missing_config_decline_lines_with_width;
use self::ops::render_cli_chat_missing_config_lines_with_width;
#[cfg(test)]
use self::ops::render_cli_chat_startup_lines_with_width;
#[cfg(test)]
use self::ops::render_cli_chat_status_lines_with_width;
#[cfg(test)]
use self::ops::render_manual_compaction_lines_with_width;
use self::ops::should_run_missing_config_onboard;
use self::render::*;
use self::safe::*;
use self::safe_text::*;
#[cfg(test)]
use crate::conversation::DefaultConversationRuntime;

pub(crate) use self::boot::{
    initialize_cli_turn_runtime, initialize_cli_turn_runtime_with_loaded_config,
    initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx,
};

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
    if session::interactive_terminal_surface_supported() {
        return session::run_cli_chat_surface(config_path, session_hint, options).await;
    }

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
    if session::interactive_terminal_surface_supported() {
        return session::run_concurrent_cli_host_surface(options);
    }

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

#[cfg(test)]
#[cfg(any(test, feature = "memory-sqlite"))]
fn render_turn_checkpoint_startup_health_lines_with_width(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Option<Vec<String>> {
    ops::render_turn_checkpoint_startup_health_lines_with_width(session_id, diagnostics, width)
}

#[cfg(test)]
mod tests;
