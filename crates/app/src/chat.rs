#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::Capability;
use tokio::io::{self as tokio_io, AsyncBufReadExt, BufReader};
use tokio::sync::Notify;

use crate::CliResult;
use crate::acp::{
    AcpConversationTurnOptions, AcpTurnEventSink, JsonlAcpTurnEventSink,
    resolve_acp_backend_selection,
};
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context_with_config};

use super::config::{self, ConversationConfig, LoongClawConfig};
use super::conversation::{
    ConversationRuntimeBinding, ConversationSessionAddress, ConversationTurnCoordinator,
    ProviderErrorMode, resolve_context_engine_selection,
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
    TurnCheckpointTailRepairReason, TurnCheckpointTailRepairRuntimeProbe,
};
#[cfg(feature = "memory-sqlite")]
use super::conversation::{load_fast_lane_tool_batch_event_summary, load_safe_lane_event_summary};
#[cfg(any(test, feature = "memory-sqlite"))]
use super::memory;
#[cfg(feature = "memory-sqlite")]
use super::memory::runtime_config::MemoryRuntimeConfig;

pub const DEFAULT_FIRST_PROMPT: &str = "Summarize this repository and suggest the best next step.";
const TEST_ONBOARD_EXECUTABLE_ENV: &str = "LOONGCLAW_TEST_ONBOARD_EXECUTABLE";

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
    pub shutdown: Arc<Notify>,
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
        println!("Welcome to LoongClaw!");
        println!();
        println!("No configuration found. Would you like to run the setup wizard now? [Y/n]");

        let mut input = String::new();
        let read = io::stdin()
            .read_line(&mut input)
            .map_err(|e| format!("read stdin failed: {e}"))?;
        let input = input.trim().to_ascii_lowercase();

        if read > 0 && matches!(input.as_str(), "y" | "yes") {
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
            println!("You can run '{onboard_hint}' later to get started.");
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
                println!("loongclaw> {assistant_text}");
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
        options,
        acp_event_printer
            .as_ref()
            .map(|printer| printer as &dyn AcpTurnEventSink),
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
        true,
    )?;
    print_cli_chat_startup(&runtime, &chat_options)?;

    let host_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize concurrent CLI host runtime: {error}"))?;

    host_runtime.block_on(async {
        print_turn_checkpoint_startup_health(&runtime).await;
        run_concurrent_cli_host_loop(&runtime, &chat_options, options.shutdown.as_ref()).await
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
            if let Some(health) =
                format_turn_checkpoint_startup_health(&runtime.session_id, &diagnostics)
            {
                println!("{health}");
                if let Some(probe) = diagnostics.runtime_probe() {
                    println!(
                        "{}",
                        format_turn_checkpoint_runtime_probe(&runtime.session_id, probe)
                    );
                }
            }
        }
        Err(error) => {
            println!(
                "turn_checkpoint_health session={} state=unavailable error={error}",
                runtime.session_id
            );
        }
    }
}

#[allow(clippy::print_stdout)] // CLI output
async fn run_concurrent_cli_host_loop(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
    shutdown: &Notify,
) -> CliResult<()> {
    let mut stdin_lines = BufReader::new(tokio_io::stdin()).lines();

    loop {
        print!("you> ");
        io::stdout()
            .flush()
            .map_err(|error| format!("flush stdout failed: {error}"))?;

        let next_line = tokio::select! {
            _ = shutdown.notified() => None,
            line = stdin_lines.next_line() => Some(
                line.map_err(|error| format!("read stdin failed: {error}"))?
            ),
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
                println!("loongclaw> {assistant_text}");
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
        run_cli_turn(runtime, input, options, event_sink).await?,
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
    let mut lines = vec![
        "loongclaw chat ready".to_owned(),
        "start here".to_owned(),
        format!("- first prompt: {DEFAULT_FIRST_PROMPT}"),
        "- type your request, or use /help for commands".to_owned(),
        "session details".to_owned(),
        format!("- session: {}", summary.session_id),
        format!("- config: {}", summary.config_path),
        format!("- memory: {}", summary.memory_label),
        "runtime details".to_owned(),
    ];

    let allowed_channels = if summary.allowed_channels.is_empty() {
        "-".to_owned()
    } else {
        summary.allowed_channels.join(",")
    };
    lines.push(format!(
        "- context engine: {} ({})",
        summary.context_engine_id, summary.context_engine_source
    ));
    lines.push(format!(
        "- acp: enabled={} dispatch_enabled={} routing={} backend={} ({}) allowed_channels={allowed_channels}",
        summary.acp_enabled,
        summary.dispatch_enabled,
        summary.conversation_routing,
        summary.acp_backend_id,
        summary.acp_backend_source,
    ));

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
        lines.push(format!(
            "- acp overrides: explicit={} event_stream={} bootstrap_mcp_servers={bootstrap_label} cwd={cwd_label}",
            summary.explicit_acp_request, summary.event_stream_enabled,
        ));
    }

    lines
}

async fn run_cli_turn(
    runtime: &CliTurnRuntime,
    input: &str,
    _options: &CliChatOptions,
    event_sink: Option<&dyn AcpTurnEventSink>,
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
    runtime
        .turn_coordinator
        .handle_turn_with_address_and_acp_options(
            &turn_config,
            &runtime.session_address,
            input,
            ProviderErrorMode::InlineMessage,
            &acp_options,
            crate::conversation::ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
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
    println!("/help    show this help");
    println!("/history print current session sliding window");
    println!("/fast_lane_summary [limit]  summarize fast-lane batch execution events");
    println!("/safe_lane_summary [limit]  summarize safe-lane runtime events");
    println!("/turn_checkpoint_summary [limit]  summarize durable turn finalization state");
    println!("/turn_checkpoint_repair  repair durable turn finalization tail when safe");
    println!("/exit    quit chat");
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
        for line in load_history_lines(session_id, limit, binding, memory_config).await? {
            println!("{line}");
        }
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        println!("history unavailable: memory-sqlite feature disabled");
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
        println!(
            "{}",
            load_fast_lane_summary_output(session_id, limit, binding, memory_config).await?
        );
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        println!("fast-lane summary unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
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
        println!(
            "{}",
            load_safe_lane_summary_output(
                session_id,
                limit,
                conversation_config,
                binding,
                memory_config,
            )
            .await?
        );
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, conversation_config, binding);
        println!("safe-lane summary unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
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
        println!(
            "{}",
            load_turn_checkpoint_summary_output(
                turn_coordinator,
                config,
                session_id,
                limit,
                binding,
            )
            .await?
        );
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (turn_coordinator, config, session_id, limit, binding);
        println!("turn checkpoint summary unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
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

#[allow(clippy::print_stdout)] // CLI output
async fn print_turn_checkpoint_repair(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongClawConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        println!(
            "{}",
            load_turn_checkpoint_repair_output(turn_coordinator, config, session_id, binding)
                .await?
        );
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (turn_coordinator, config, session_id, binding);
        println!("turn checkpoint repair unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
async fn load_turn_checkpoint_repair_output(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongClawConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<String> {
    let outcome = turn_coordinator
        .repair_turn_checkpoint_tail(config, session_id, binding)
        .await?;
    Ok(format_turn_checkpoint_repair(session_id, &outcome))
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

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
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

    fn from_outcome(outcome: &TurnCheckpointTailRepairOutcome) -> Self {
        Self {
            action: outcome.action().as_str(),
            source: outcome.source().map(|value| value.as_str()).unwrap_or("-"),
            reason: outcome.reason().as_str(),
        }
    }

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

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
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
    use tokio::sync::Notify;

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
        let shutdown = Arc::new(Notify::new());
        let error = run_concurrent_cli_host(&ConcurrentCliHostOptions {
            resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
            config: LoongClawConfig::default(),
            session_id: "   ".to_owned(),
            shutdown,
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
        let shutdown = Arc::new(Notify::new());
        shutdown.notify_one();

        run_concurrent_cli_host_loop(&runtime, &options, shutdown.as_ref())
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
        let lines = render_cli_chat_startup_lines(&CliChatStartupSummary {
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
        });

        assert_eq!(lines[0], "loongclaw chat ready");
        assert!(
            lines.iter().any(|line| line == "start here"),
            "chat startup should lead with a dedicated first-action heading: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line == "- first prompt: Summarize this repository and suggest the best next step."
            }),
            "chat startup should suggest a concrete first prompt: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "- type your request, or use /help for commands"),
            "chat startup should keep the usage hint, but under the assistant-first opening block: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "session details"),
            "chat startup should tuck session/config facts into a secondary section: {lines:#?}"
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
        let lines = render_cli_chat_startup_lines(&CliChatStartupSummary {
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
        });

        assert!(
            lines.iter().any(|line| {
                line
                    == "- acp overrides: explicit=true event_stream=true bootstrap_mcp_servers=filesystem cwd=/workspace/project"
            }),
            "chat startup should surface ACP override knobs only when they matter: {lines:#?}"
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
