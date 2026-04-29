#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    private_interfaces
)] // CLI daemon binary

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    pin::Pin,
    process,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use kernel::{
    BootstrapTaskStatus, Capability, ConnectorCommand, FixedClock, InMemoryAuditSink,
    PluginActivationStatus, PluginScanner, PluginSetupReadinessContext, PluginTranslator,
    TaskIntent, ToolCoreOutcome, ToolCoreRequest, evaluate_plugin_setup_requirements,
};
use loong_contracts::SecretRef;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub use loong_app as mvp;
pub use loong_spec::spec_execution::*;
pub use loong_spec::spec_runtime::*;
pub use loong_spec::{CliResult, DEFAULT_AGENT_ID, DEFAULT_PACK_ID, kernel_bootstrap};

pub use self::channel_cli_specs::{
    DINGTALK_SEND_CLI_SPEC, DISCORD_SEND_CLI_SPEC, EMAIL_SEND_CLI_SPEC, FEISHU_SEND_CLI_SPEC,
    GOOGLE_CHAT_SEND_CLI_SPEC, IMESSAGE_SEND_CLI_SPEC, IRC_SEND_CLI_SPEC, LINE_SEND_CLI_SPEC,
    MATRIX_SEND_CLI_SPEC, MATRIX_SERVE_CLI_SPEC, MATTERMOST_SEND_CLI_SPEC,
    NEXTCLOUD_TALK_SEND_CLI_SPEC, NOSTR_SEND_CLI_SPEC, ONEBOT_SEND_CLI_SPEC, ONEBOT_SERVE_CLI_SPEC,
    QQBOT_SEND_CLI_SPEC, QQBOT_SERVE_CLI_SPEC, SIGNAL_SEND_CLI_SPEC, SLACK_SEND_CLI_SPEC,
    SYNOLOGY_CHAT_SEND_CLI_SPEC, TEAMS_SEND_CLI_SPEC, TELEGRAM_SEND_CLI_SPEC,
    TELEGRAM_SERVE_CLI_SPEC, TWITCH_SEND_CLI_SPEC, WEBHOOK_SEND_CLI_SPEC, WECOM_SEND_CLI_SPEC,
    WECOM_SERVE_CLI_SPEC, WEIXIN_SEND_CLI_SPEC, WEIXIN_SERVE_CLI_SPEC, WHATSAPP_SEND_CLI_SPEC,
};
pub use self::channel_send_target_kind::{
    default_twitch_send_target_kind, parse_twitch_send_target_kind,
};
pub use self::cli_json::build_runtime_snapshot_cli_json_payload;
pub use self::delegate_child_cli::run_detached_delegate_child_cli;
pub use self::env_compat::make_env_compatible;
pub use self::managed_plugin_bridge_runtime::{
    default_onebot_send_target_kind, default_qqbot_send_target_kind,
    default_weixin_send_target_kind, parse_onebot_send_target_kind, parse_qqbot_send_target_kind,
    parse_weixin_send_target_kind, run_onebot_send_cli_impl, run_onebot_serve_cli_impl,
    run_qqbot_send_cli_impl, run_qqbot_serve_cli_impl, run_weixin_send_cli_impl,
    run_weixin_serve_cli_impl,
};
pub use self::mcp_cli::{
    build_mcp_server_detail_cli_json_payload, build_mcp_servers_cli_json_payload,
    run_list_mcp_servers_cli, run_show_mcp_server_cli,
};
pub use self::operator_inventory_cli::{
    CHANNELS_CLI_JSON_LEGACY_VIEWS, CHANNELS_CLI_JSON_SCHEMA_VERSION,
    build_channels_cli_json_payload, format_capability_names, format_milli_ratio,
    push_channel_surface_header, render_channel_onboarding_line,
    render_channel_operation_requirement_ids, render_channel_surfaces_shell_text,
    render_channel_surfaces_text, render_channel_target_kind_ids, run_channels_cli,
    run_list_context_engines_cli, run_list_memory_systems_cli, run_safe_lane_summary_cli,
};
pub use self::runtime_plugin_discovery::{
    RuntimePluginDiscoveryActionView, RuntimePluginDiscoveryGuidanceView,
    RuntimePluginShadowingConflictView,
};
pub use loong_bench::{
    run_programmatic_pressure_baseline_lint_cli, run_programmatic_pressure_benchmark_cli,
    run_wasm_cache_benchmark_cli,
};
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
pub use memory_context_benchmark::run_memory_context_benchmark_cli;
pub use runtime_trajectory_cli::{format_runtime_trajectory_summary, run_runtime_trajectory_cli};
#[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
pub fn run_memory_context_benchmark_cli(
    output_path: &str,
    temp_root: Option<&str>,
    history_turns: usize,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    suite_repetitions: usize,
    enforce_gate: bool,
    min_steady_state_speedup_ratio: f64,
) -> CliResult<()> {
    let _ = (
        output_path,
        temp_root,
        history_turns,
        sliding_window,
        summary_max_chars,
        words_per_turn,
        rebuild_iterations,
        hot_iterations,
        warmup_iterations,
        suite_repetitions,
        enforce_gate,
        min_steady_state_speedup_ratio,
    );
    Err("benchmark-memory-context requires the daemon `memory-sqlite` feature".to_owned())
}

pub use {base64, kernel, sha2};

pub mod acp_cli;
pub mod audit_cli;
mod browser_companion_diagnostics;
pub mod browser_preview;
mod channel_access_policy_render;
mod channel_bridge_render;
mod channel_cli_specs;
mod channel_resolution;
#[cfg(test)]
mod channel_send_cli_tests;
mod channel_send_target_kind;
mod channel_serve_cli;
mod cli_handoff;
mod cli_json;
mod command_kind;
pub mod completions_cli;
mod control_plane_server;
mod copilot_onboarding;
mod delegate_child_cli;
pub mod doctor_cli;
pub mod doctor_security_cli;
mod env_compat;
mod external_skills_policy_probe;
pub mod feishu_cli;
mod feishu_onboarding;
pub mod feishu_support;
pub mod gateway;
pub mod import_cli;
mod managed_plugin_bridge_runtime;
mod mcp_cli;
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
mod memory_context_benchmark;
pub mod migrate_cli;
pub mod migration;
mod native_extension_authoring;
pub mod next_actions;
mod observability;
pub mod onboard_cli;
mod onboard_finalize;
mod onboard_preflight;
pub mod onboard_presentation;
mod onboard_types;
mod onboard_web_search;
mod onboarding_model_policy;
mod operator_inventory_cli;
pub mod operator_prompt;
pub mod personalize_cli;
mod plugin_bridge_account_summary;
pub mod plugins_cli;
mod provider_credential_policy;
mod provider_model_probe_policy;
pub mod provider_presentation;
mod provider_route_diagnostics;
pub mod runtime_capability_cli;
pub mod runtime_experiment_cli;
mod runtime_plugin_discovery;
pub mod runtime_restore_cli;
mod runtime_snapshot_render;
mod runtime_snapshot_types;
pub mod runtime_trajectory_cli;
pub mod session_cli;
mod session_prompt_frame_cli;
mod session_runtime_truth_cli;
pub mod sessions_cli;
pub mod skills_cli;
pub mod source_presentation;
pub mod status_cli;
pub mod supervisor;
mod task_execution;
pub mod tasks_cli;
mod tlon_cli;
mod tool_calling_readiness;
pub mod trajectory_cli;
mod turn_cli;
pub mod update_cli;
pub mod work_unit_cli;
pub use self::acp_cli::{
    acp_backend_metadata_json, acp_binding_scope_json, acp_control_plane_json,
    acp_dispatch_decision_json, acp_dispatch_prediction_provenance_json, acp_doctor_json,
    acp_event_summary_json, acp_manager_observability_json, acp_session_activation_provenance_json,
    acp_session_metadata_json, acp_session_mode_label, acp_session_state_label,
    acp_session_status_json, acp_turn_provenance_json, build_acp_dispatch_address,
    format_acp_event_summary, resolve_acp_status_session_key, run_acp_dispatch_cli,
    run_acp_doctor_cli, run_acp_event_summary_cli, run_acp_observability_cli, run_acp_status_cli,
    run_list_acp_backends_cli, run_list_acp_sessions_cli,
};
use channel_access_policy_render::{
    channel_access_policy_by_account, render_channel_access_policy_line,
};
use channel_bridge_render::{
    push_channel_surface_managed_plugin_bridge_discovery,
    push_channel_surface_plugin_bridge_contract,
};
pub(crate) use channel_bridge_render::{
    render_line_safe_optional_text_value, render_line_safe_text_value, render_line_safe_text_values,
};
pub use gateway::read_models::{ChannelsCliJsonPayload, ChannelsCliJsonSchema};
pub use loong_spec::programmatic::{
    acquire_programmatic_circuit_slot, record_programmatic_circuit_outcome,
};
pub use observability::{debug_variant_name, init_tracing, summarize_error};
pub use runtime_snapshot_render::render_runtime_snapshot_text;
pub(crate) use runtime_snapshot_render::{
    runtime_snapshot_acp_json, runtime_snapshot_context_engine_json,
    runtime_snapshot_external_skills_json, runtime_snapshot_memory_system_json,
    runtime_snapshot_provider_json, runtime_snapshot_runtime_plugins_json,
    runtime_snapshot_tool_runtime_json,
};
pub use runtime_snapshot_types::{
    RuntimeSnapshotProviderProfileState, RuntimeSnapshotProviderState,
    RuntimeSnapshotProviderTransportState,
};
pub use session_cli::{
    SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION, SessionSearchArtifactDocument,
    SessionSearchArtifactResult, SessionSearchArtifactSchema, collect_session_search_artifact,
    format_session_search_inspect_text, format_session_search_text, load_session_search_artifact,
    run_session_search_cli, run_session_search_inspect_cli,
};
use task_execution::execute_daemon_task_with_supervisor;
pub use task_execution::{DaemonTaskExecution, run_demo, run_task_cli};
pub use tlon_cli::TLON_SEND_CLI_SPEC;
use tlon_cli::{default_tlon_send_target_kind, parse_tlon_send_target_kind};
pub use turn_cli::{TurnCommands, build_cli_chat_options, run_ask_cli, run_chat_cli};
pub use update_cli::run_update_cli;
#[rustfmt::skip]
use tool_calling_readiness::{RuntimeSnapshotToolCallingState, collect_runtime_snapshot_tool_calling_state};
pub use trajectory_cli::{
    TRAJECTORY_EXPORT_ARTIFACT_JSON_SCHEMA_VERSION, TrajectoryExportArtifactDocument,
    TrajectoryExportArtifactSchema, TrajectoryExportEvent, TrajectoryExportSessionSummary,
    TrajectoryExportTurn, collect_trajectory_export_artifact, format_trajectory_export_text,
    format_trajectory_inspect_text, load_trajectory_export_artifact, run_trajectory_export_cli,
    run_trajectory_inspect_cli,
};
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::missing_panics_doc
)]
#[doc(hidden)]
pub mod test_support;
pub use channel_serve_cli::{
    FEISHU_SERVE_CLI_SPEC, LINE_SERVE_CLI_SPEC, WEBHOOK_SERVE_CLI_SPEC, WHATSAPP_SERVE_CLI_SPEC,
};

pub const PUBLIC_GITHUB_REPO: &str = "loong-ai/loong";
pub const CLI_COMMAND_NAME: &str = mvp::config::CLI_COMMAND_NAME;

pub fn active_cli_command_name() -> &'static str {
    mvp::config::active_cli_command_name()
}

pub(crate) fn render_operator_shell_surface(
    title: &str,
    subtitle: &str,
    intro_lines: Vec<String>,
    body_lines: Vec<String>,
    footer_lines: Vec<String>,
) -> String {
    let width = mvp::presentation::detect_render_width();
    let mut sections = Vec::new();
    if !body_lines.is_empty() {
        sections.push(mvp::tui_surface::TuiSectionSpec::Narrative {
            title: None,
            lines: body_lines,
        });
    }
    let screen = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some(subtitle.to_owned()),
        title: Some(title.to_owned()),
        progress_line: None,
        intro_lines,
        sections,
        choices: Vec::new(),
        footer_lines,
    };
    mvp::tui_surface::render_tui_screen_spec_ratatui(&screen, width, false).join("\n")
}

pub(crate) fn render_operator_shell_surface_from_body(
    title: &str,
    subtitle: &str,
    body: String,
) -> String {
    render_operator_shell_surface(
        title,
        subtitle,
        Vec::new(),
        body.lines().map(str::to_owned).collect(),
        Vec::new(),
    )
}

fn render_welcome_long_about(command_name: &str) -> String {
    format!(
        "Show the configured welcome banner and quick commands.\n\nquick commands:\n- {command_name} ask --config <path> --message \"...\"\n- {command_name} chat --config <path>\n- {command_name} personalize --config <path>\n- {command_name} doctor --config <path>\n- {command_name} --help\n\nReplace <path> with your current config path, or set LOONG_CONFIG_PATH first."
    )
}

fn render_import_long_about(command_name: &str) -> String {
    format!(
        "Power-user import flow for previewing or applying detected migration sources explicitly.\n\nUse this when you want exact CLI control over which source and domains are reused. If you want the guided path, use `{command_name} onboard` instead. When the same source kind resolves to multiple detected configs, rerun with `--source-path <path>` to choose one exact source."
    )
}

fn render_migrate_long_about(command_name: &str) -> String {
    format!(
        "Power-user config import flow for discovering, previewing, or applying external workspace state explicitly.\n\nUse this when you want exact CLI control over import mode selection and output handling for compatibility sources and older workspace roots. If you want the guided path, use `{command_name} onboard` instead.\n\nMode quick reference:\n- discover, plan_many, recommend_primary, merge_profiles, map_external_skills: require `--input`\n- plan: requires `--input`; `--output` is optional preview target\n- apply: requires `--input` and `--output`\n- apply_selected: requires `--input` and `--output`; use `--source-id` to pin one discovered source, and `--apply-external-skills-plan` to bridge installable local external skills into the managed runtime\n- rollback_last_apply: requires `--output`"
    )
}

fn render_ask_long_about(command_name: &str) -> String {
    format!(
        "Run one non-interactive one-shot assistant turn.\n\nUse this when you want a fast answer without entering the interactive `{command_name} chat` REPL. The command reuses the normal CLI conversation runtime, session memory, provider selection, and ACP options."
    )
}

pub fn build_cli_command(command_name: &'static str) -> clap::Command {
    Cli::command()
        .name(command_name)
        .bin_name(command_name)
        .mut_subcommand("welcome", |command| {
            command.long_about(render_welcome_long_about(command_name))
        })
        .mut_subcommand("import", |command| {
            command.long_about(render_import_long_about(command_name))
        })
        .mut_subcommand("migrate", |command| {
            command
                .about("Preview or apply config import modes explicitly")
                .long_about(render_migrate_long_about(command_name))
        })
        .mut_subcommand("ask", |command| {
            command.long_about(render_ask_long_about(command_name))
        })
}

pub fn parse_cli() -> Cli {
    let mut matches = build_cli_command(active_cli_command_name()).get_matches();
    Cli::from_arg_matches_mut(&mut matches).unwrap_or_else(|error| error.exit())
}

pub use control_plane_server::{build_control_plane_router, run_control_plane_serve_cli};

pub fn native_spec_tool_executor(
    request: ToolCoreRequest,
) -> Option<Result<ToolCoreOutcome, String>> {
    if mvp::tools::canonical_tool_name(request.tool_name.as_str()) != "config.import" {
        return None;
    }
    Some(mvp::tools::execute_tool_core_with_config(
        request,
        &mvp::tools::runtime_config::ToolRuntimeConfig::default(),
    ))
}

pub type ChannelCliCommandFuture<'a> = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BridgeSupportProfileArg {
    NativeBalanced,
    OpenclawEcosystemBalanced,
}

impl BridgeSupportProfileArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::NativeBalanced => "native-balanced",
            Self::OpenclawEcosystemBalanced => "openclaw-ecosystem-balanced",
        }
    }
}

#[derive(clap::Args, Debug, Clone, Default)]
pub struct RunSpecBridgeSupportArgs {
    /// Optional JSON file containing a bridge support policy override for this spec run
    #[arg(long, conflicts_with_all = ["bridge_profile", "bridge_support_delta"])]
    pub bridge_support: Option<String>,
    /// Optional bundled bridge support profile override for this spec run
    #[arg(long, value_enum, conflicts_with_all = ["bridge_support", "bridge_support_delta"])]
    pub bridge_profile: Option<BridgeSupportProfileArg>,
    /// Optional delta artifact JSON file derived from a bundled bridge support profile
    #[arg(long, conflicts_with_all = ["bridge_support", "bridge_profile"])]
    pub bridge_support_delta: Option<String>,
    /// Optional sha256 pin for the resolved bridge support policy override
    #[arg(long)]
    pub bridge_support_sha256: Option<String>,
    /// Optional sha256 pin for the bridge support delta artifact override
    #[arg(long)]
    pub bridge_support_delta_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ChannelSendCliArgs<'a> {
    pub config_path: Option<&'a str>,
    pub account: Option<&'a str>,
    pub target: Option<&'a str>,
    pub target_kind: mvp::channel::ChannelOutboundTargetKind,
    pub text: &'a str,
    pub as_card: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ChannelServeCliArgs<'a> {
    pub config_path: Option<&'a str>,
    pub account: Option<&'a str>,
    pub once: bool,
    pub bind_override: Option<&'a str>,
    pub path_override: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ChannelSendCliSpec {
    pub family: mvp::channel::ChannelCatalogCommandFamilyDescriptor,
    pub run: for<'a> fn(ChannelSendCliArgs<'a>) -> ChannelCliCommandFuture<'a>,
}

#[derive(Debug, Clone, Copy)]
pub struct ChannelServeCliSpec {
    pub family: mvp::channel::ChannelCatalogCommandFamilyDescriptor,
    pub run: for<'a> fn(ChannelServeCliArgs<'a>) -> ChannelCliCommandFuture<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiChannelServeChannelAccount {
    pub channel_id: String,
    pub account_id: String,
}

impl std::str::FromStr for MultiChannelServeChannelAccount {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        parse_multi_channel_serve_channel_account(raw)
    }
}

#[derive(Parser, Debug)]
#[command(
    name = CLI_COMMAND_NAME,
    about = "Loong low-level runtime daemon",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum InitSpecPreset {
    #[default]
    Default,
    PluginTrustGuard,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(
        long_about = "Show the configured welcome banner and quick commands.\n\nquick commands:\n- loong ask --config <path> --message \"...\"\n- loong chat --config <path>\n- loong personalize --config <path>\n- loong doctor --config <path>\n- loong --help\n\nReplace <path> with your current config path, or set LOONG_CONFIG_PATH first."
    )]
    /// Show a welcome banner for an already configured install
    Welcome,
    /// Run the original end-to-end bootstrap demo
    Demo,
    #[command(
        long_about = "Download and apply the latest stable GitHub release for the current Loong binary.\n\nThis command intentionally follows the latest stable release channel only. GitHub prereleases are excluded."
    )]
    /// Update this Loong install to the latest stable GitHub release
    Update,
    #[command(hide = true)]
    /// Deprecated compatibility alias for the generic task runner
    RunTask {
        #[arg(long)]
        objective: String,
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    /// Run agent turns through the unified runtime entry surface
    Turn {
        #[command(subcommand)]
        command: TurnCommands,
    },
    /// Invoke one connector operation through kernel policy gate
    InvokeConnector {
        #[arg(long)]
        operation: String,
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    /// Demonstrate audit lifecycle with fixed clock and token revocation
    AuditDemo,
    /// Generate a runnable JSON spec template for quick vertical customization
    InitSpec {
        #[arg(long, default_value = "loong.spec.json")]
        output: String,
        #[arg(long, value_enum, default_value_t = InitSpecPreset::Default)]
        preset: InitSpecPreset,
    },
    /// Run a full workflow from a JSON spec (task/connector/runtime/tool/memory)
    RunSpec {
        #[arg(long)]
        spec: String,
        #[arg(long, default_value_t = false)]
        print_audit: bool,
        #[arg(long, default_value_t = false)]
        render_summary: bool,
        #[command(flatten)]
        bridge_support: RunSpecBridgeSupportArgs,
    },
    /// Run pressure benchmarks for programmatic orchestration and optional regression gate checks
    BenchmarkProgrammaticPressure {
        #[arg(
            long,
            default_value = "examples/benchmarks/programmatic-pressure-matrix.json"
        )]
        matrix: String,
        #[arg(long)]
        baseline: Option<String>,
        #[arg(
            long,
            default_value = "target/benchmarks/programmatic-pressure-report.json"
        )]
        output: String,
        #[arg(long, default_value_t = false)]
        enforce_gate: bool,
        #[arg(long, default_value_t = false)]
        preflight_fail_on_warnings: bool,
    },
    /// Lint pressure baseline coverage without running benchmark scenarios
    BenchmarkProgrammaticPressureLint {
        #[arg(
            long,
            default_value = "examples/benchmarks/programmatic-pressure-matrix.json"
        )]
        matrix: String,
        #[arg(long)]
        baseline: Option<String>,
        #[arg(
            long,
            default_value = "target/benchmarks/programmatic-pressure-baseline-lint-report.json"
        )]
        output: String,
        #[arg(long, default_value_t = false)]
        enforce_gate: bool,
        #[arg(long, default_value_t = false)]
        fail_on_warnings: bool,
    },
    /// Benchmark Wasm compile cache behavior and enforce hot-path speedup gate
    BenchmarkWasmCache {
        #[arg(long, default_value = "examples/plugins-wasm/secure_echo.wasm")]
        wasm: String,
        #[arg(
            long,
            default_value = "target/benchmarks/wasm-cache-benchmark-report.json"
        )]
        output: String,
        #[arg(long, default_value_t = 8)]
        cold_iterations: usize,
        #[arg(long, default_value_t = 24)]
        hot_iterations: usize,
        #[arg(long, default_value_t = 2)]
        warmup_iterations: usize,
        #[arg(long, default_value_t = false)]
        enforce_gate: bool,
        #[arg(long, default_value_t = 1.5)]
        min_speedup_ratio: f64,
    },
    /// Benchmark memory prompt-context hydration across window-only, rebuild, steady-state, and shrink catch-up summary paths
    BenchmarkMemoryContext {
        #[arg(
            long,
            default_value = "target/benchmarks/memory-context-benchmark-report.json"
        )]
        output: String,
        #[arg(long)]
        temp_root: Option<String>,
        #[arg(long, default_value_t = 256)]
        history_turns: usize,
        #[arg(long, default_value_t = 24)]
        sliding_window: usize,
        #[arg(long, default_value_t = 1024)]
        summary_max_chars: usize,
        #[arg(long, default_value_t = 24)]
        words_per_turn: usize,
        #[arg(long, default_value_t = 12)]
        rebuild_iterations: usize,
        #[arg(long, default_value_t = 32)]
        hot_iterations: usize,
        #[arg(long, default_value_t = 4)]
        warmup_iterations: usize,
        #[arg(long, default_value_t = 1)]
        suite_repetitions: usize,
        #[arg(long, default_value_t = false)]
        enforce_gate: bool,
        #[arg(long, default_value_t = 1.2)]
        min_steady_state_speedup_ratio: f64,
    },
    /// Validate config semantics and report structured diagnostics
    ValidateConfig {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
        #[arg(long, value_enum)]
        output: Option<ValidateConfigOutput>,
        #[arg(long, default_value = "en")]
        locale: String,
        #[arg(long, default_value_t = false)]
        fail_on_diagnostics: bool,
    },
    #[command(
        about = "Guided onboarding for fast first-chat setup with preflight diagnostics",
        long_about = "Guided onboarding for fast first-chat setup with preflight diagnostics.\n\nThis is the default path for most users. Loong will detect reusable settings for provider, channels, or workspace guidance, suggest a starting point, and walk through quick review before first chat."
    )]
    Onboard {
        /// Write the resulting config to a custom path instead of the default loong config location
        #[arg(long)]
        output: Option<String>,
        /// Overwrite an existing target config path instead of stopping for manual review
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Use provided flags only and skip interactive prompts except required safety checks
        #[arg(long, default_value_t = false)]
        non_interactive: bool,
        /// Confirm the onboarding risk acknowledgement in non-interactive mode
        #[arg(long, default_value_t = false)]
        accept_risk: bool,
        #[arg(
            long,
            value_name = mvp::config::PROVIDER_SELECTOR_PLACEHOLDER,
            help = mvp::config::PROVIDER_SELECTOR_HUMAN_SUMMARY
        )]
        provider: Option<String>,
        /// Preselect the model to use after the provider choice is resolved
        #[arg(long)]
        model: Option<String>,
        /// Provider credential environment variable name, for example OPENAI_API_KEY
        #[arg(long = "api-key", alias = "api-key-env")]
        api_key_env: Option<String>,
        #[arg(
            long = "web-search-provider",
            value_name = "PROVIDER",
            help = mvp::config::WEB_SEARCH_PROVIDER_VALID_VALUES
        )]
        web_search_provider: Option<String>,
        /// Web search credential environment variable name, for example TAVILY_API_KEY
        #[arg(long = "web-search-api-key", alias = "web-search-api-key-env")]
        web_search_api_key_env: Option<String>,
        /// Select a native prompt personality in non-interactive mode
        #[arg(long)]
        personality: Option<String>,
        /// Select a memory profile in non-interactive mode
        #[arg(long)]
        memory_profile: Option<String>,
        /// Preseed the CLI system prompt instead of editing it interactively
        #[arg(long)]
        system_prompt: Option<String>,
        /// Skip probing the resolved provider model list during onboarding
        #[arg(long, default_value_t = false)]
        skip_model_probe: bool,
    },
    #[command(
        about = "Capture optional operator preferences for future sessions",
        long_about = "Capture optional operator preferences for future sessions.\n\nThis command stores advisory working preferences such as preferred name, response density, initiative level, and standing boundaries. Rerun it any time to update or clear saved preferences. It does not replace runtime identity files, and it does not change the primary setup path. If you do not have a config yet, run `loong onboard` first."
    )]
    Personalize {
        /// Config file path to update (defaults to auto-discovery)
        #[arg(long)]
        config: Option<String>,
    },
    #[command(
        about = "Preview or apply migration sources explicitly",
        long_about = "Power-user import flow for previewing or applying detected migration sources explicitly.\n\nUse this when you want exact CLI control over which source and domains are reused. If you want the guided path, use `loong onboard` instead. When the same source kind resolves to multiple detected configs, rerun with `--source-path <path>` to choose one exact source."
    )]
    Import {
        /// Write the imported config to a custom path instead of the default loong config location
        #[arg(long)]
        output: Option<String>,
        /// Overwrite an existing target config path instead of stopping for manual review
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Print the selected import candidate preview in text mode
        #[arg(long, default_value_t = false)]
        preview: bool,
        /// Apply the selected import candidate to the target config path
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// Emit machine-readable preview JSON for scripting or automation
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Limit selection to one source kind such as recommended, existing, codex, or env
        #[arg(long)]
        from: Option<String>,
        /// Choose one exact detected source path when multiple candidates of the same kind exist
        #[arg(long)]
        source_path: Option<String>,
        #[arg(
            long,
            value_name = mvp::config::PROVIDER_SELECTOR_PLACEHOLDER,
            help = mvp::config::PROVIDER_SELECTOR_HUMAN_SUMMARY
        )]
        provider: Option<String>,
        /// Reuse only the listed domains, for example provider,channels
        #[arg(long, value_delimiter = ',')]
        include: Vec<String>,
        /// Exclude the listed domains from the selected import candidate
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<String>,
    },
    #[command(
        about = "Preview or apply config import modes explicitly",
        long_about = "Power-user config import flow for discovering, previewing, or applying external workspace state explicitly.\n\nUse this when you want exact CLI control over import mode selection and output handling for compatibility sources and older workspace roots. If you want the guided path, use `loong onboard` instead.\n\nMode quick reference:\n- discover, plan_many, recommend_primary, merge_profiles, map_external_skills: require `--input`\n- plan: requires `--input`; `--output` is optional preview target\n- apply: requires `--input` and `--output`\n- apply_selected: requires `--input` and `--output`; use `--source-id` to pin one discovered source, and `--apply-external-skills-plan` to bridge installable local external skills into the managed runtime\n- rollback_last_apply: requires `--output`"
    )]
    Migrate {
        /// Path to the legacy agent workspace or root to inspect
        #[arg(long)]
        input: Option<String>,
        /// Target Loong config path to preview, write, or roll back
        #[arg(long)]
        output: Option<String>,
        /// Hint the legacy claw-family source kind for single-source plan/apply modes
        #[arg(long)]
        source: Option<String>,
        /// Migration mode to run
        #[arg(long, value_enum)]
        mode: migrate_cli::MigrateMode,
        /// Emit machine-readable JSON instead of text output
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Explicit discovered source id to apply for apply_selected mode
        #[arg(long)]
        source_id: Option<String>,
        /// Merge profile-lane content while keeping one prompt owner
        #[arg(long, default_value_t = false)]
        safe_profile_merge: bool,
        /// Explicit primary source id when safe profile merge is enabled
        #[arg(long)]
        primary_source_id: Option<String>,
        /// Bridge installable local external skills into the managed runtime during apply_selected
        #[arg(long, default_value_t = false)]
        apply_external_skills_plan: bool,
        /// Overwrite an existing target config path instead of stopping for manual review
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Run configuration diagnostics and optionally apply safe config/path fixes
    Doctor {
        /// Config file path to validate (defaults to auto-discovery)
        #[arg(long, global = true)]
        config: Option<String>,
        /// Apply safe auto-fixes for detected diagnostics
        #[arg(long, global = true, default_value_t = false)]
        fix: bool,
        /// Emit machine-readable JSON diagnostics
        #[arg(long, global = true, default_value_t = false)]
        json: bool,
        /// Skip provider model probing during diagnostics
        #[arg(long, global = true, default_value_t = false)]
        skip_model_probe: bool,
        #[command(subcommand)]
        command: Option<doctor_cli::DoctorCommands>,
    },
    /// Inspect the retained audit journal through a bounded CLI surface
    Audit {
        #[arg(long, global = true)]
        config: Option<String>,
        #[arg(long, global = true, default_value_t = false)]
        json: bool,
        #[command(subcommand)]
        command: audit_cli::AuditCommands,
    },
    /// Manage installed external skills through an operator-facing CLI surface
    Skills {
        #[arg(long, global = true)]
        config: Option<String>,
        #[arg(long, global = true, default_value_t = false)]
        json: bool,
        #[command(subcommand)]
        command: skills_cli::SkillsCommands,
    },
    /// Manage async background tasks on top of the current session runtime
    Tasks {
        #[arg(long, global = true)]
        config: Option<String>,
        #[arg(long, global = true, default_value_t = false)]
        json: bool,
        #[arg(long, global = true, default_value = "default")]
        session: String,
        #[command(subcommand)]
        command: tasks_cli::TasksCommands,
    },
    #[command(hide = true)]
    DelegateChildRun {
        #[arg(long)]
        config_path: String,
        #[arg(long)]
        payload_file: String,
    },
    #[command(
        about = "Inspect and manage persisted runtime sessions through an operator-facing session shell",
        long_about = "Bounded operator-facing session shell for persisted runtime sessions.\n\nUse this surface to list visible sessions, inspect one session's workflow metadata, review lifecycle events, inspect transcript history, and apply bounded recover, cancel, or archive actions without inventing a second session model."
    )]
    Sessions {
        #[arg(long, global = true)]
        config: Option<String>,
        #[arg(long, global = true, default_value_t = false)]
        json: bool,
        #[arg(long, global = true, default_value = "default")]
        session: String,
        #[command(subcommand)]
        command: sessions_cli::SessionsCommands,
    },
    /// Print one operator-readable runtime summary over gateway, ACP, and durable work-unit health
    #[rustfmt::skip]
    Status { #[arg(long)] config: Option<String>, #[arg(long, default_value_t = false)] json: bool },
    #[command(
        visible_alias = "plugin",
        about = "Author manifest-first plugin packages and inspect shared plugin governance truth",
        long_about = "Manifest-first plugin namespace for bounded authoring bootstrap, inspecting manifest-first package inventory, diagnosing package-author contract issues, evaluating profile-aware preflight, and consuming the deduplicated operator action plan.\n\nThis command does not introduce a second policy engine. It reuses the existing spec `plugin_inventory` and `plugin_preflight` surfaces for shared plugin truth and adds thin author-facing surfaces for external package roots."
    )]
    Plugins {
        #[arg(long, global = true, default_value_t = false)]
        json: bool,
        #[command(subcommand)]
        command: plugins_cli::PluginsCommands,
    },
    /// List compiled channel surfaces, aliases, and readiness status
    Channels {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        resolve: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Fetch and print currently available provider model list
    ListModels {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Print a unified runtime snapshot for experiment reproducibility and lineage capture
    RuntimeSnapshot {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
        #[arg(long)]
        output: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        experiment_id: Option<String>,
        #[arg(long)]
        parent_snapshot_id: Option<String>,
    },
    #[command(
        long_about = "Restore a persisted runtime snapshot artifact into the current config and managed skill state.\n\nDry-run by default; pass --apply to mutate config or managed skills."
    )]
    /// Restore a persisted runtime snapshot artifact into the current config and managed skill state
    RuntimeRestore {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        snapshot: String,
        #[arg(long, default_value_t = false)]
        json: bool,
        #[arg(long, default_value_t = false)]
        apply: bool,
    },
    /// Manage snapshot-linked experiment run records
    RuntimeExperiment {
        #[command(subcommand)]
        command: runtime_experiment_cli::RuntimeExperimentCommands,
    },
    /// Manage run-derived capability candidates, family readiness, promotion plans, and governed apply outputs
    RuntimeCapability {
        #[command(subcommand)]
        command: runtime_capability_cli::RuntimeCapabilityCommands,
    },
    /// Manage durable work units for long-running runtime orchestration
    WorkUnit {
        #[command(subcommand)]
        command: work_unit_cli::WorkUnitCommands,
    },
    /// List available conversation context engines and selected runtime engine
    ListContextEngines {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List available memory systems and selected runtime memory system
    ListMemorySystems {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List configured MCP servers and their runtime-visible inventory state
    ListMcpServers {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show one configured MCP server and its runtime-visible inventory state
    ShowMcpServer {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List available ACP runtime backends and current control-plane selection
    ListAcpBackends {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List persisted ACP session metadata from the local control-plane store
    ListAcpSessions {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Inspect live ACP session status by session key or conversation identity
    AcpStatus {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, conflicts_with_all = ["conversation_id", "route_session_id"])]
        session: Option<String>,
        #[arg(long, conflicts_with_all = ["session", "route_session_id"])]
        conversation_id: Option<String>,
        #[arg(long, conflicts_with_all = ["session", "conversation_id"])]
        route_session_id: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Inspect ACP control-plane observability snapshot from the shared session manager
    AcpObservability {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Print ACP runtime event summary for a conversation session
    AcpEventSummary {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Evaluate ACP conversation dispatch policy for a session or structured channel address
    AcpDispatch {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        conversation_id: Option<String>,
        #[arg(long)]
        account_id: Option<String>,
        #[arg(long)]
        participant_id: Option<String>,
        #[arg(long)]
        thread_id: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Run ACP backend readiness diagnostics for the selected or requested backend
    AcpDoctor {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        backend: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(
        about = "Run the loopback-only internal control-plane skeleton",
        long_about = "Run the internal control-plane skeleton.\n\nBy default this control-plane listener binds 127.0.0.1 only. You may provide `--bind <host:port>` to override the listener address, but non-loopback binds require `--config` plus `control_plane.allow_remote=true` and a configured `control_plane.shared_token`. Baseline endpoints are `/readyz`, `/healthz`, `/control/challenge`, `/control/connect`, `/control/subscribe`, `/control/snapshot`, and `/control/events`. When `--config` is provided, repository-backed `/session/list`, `/session/read`, `/approval/list`, `/pairing/list`, `/pairing/resolve`, `/acp/session/list`, and `/acp/session/read` views become available for the selected session root."
    )]
    ControlPlaneServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    #[command(
        about = "Run one non-interactive assistant turn",
        long_about = "Run one non-interactive one-shot assistant turn.\n\nUse this when you want a fast answer without entering the interactive `loong chat` REPL. The command reuses the normal CLI conversation runtime, session memory, provider selection, and ACP options."
    )]
    Ask {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        message: String,
        #[arg(long, default_value_t = false)]
        acp: bool,
        #[arg(long, default_value_t = false)]
        acp_event_stream: bool,
        #[arg(long = "acp-bootstrap-mcp-server")]
        acp_bootstrap_mcp_server: Vec<String>,
        #[arg(long = "acp-cwd")]
        acp_cwd: Option<String>,
    },
    /// Start interactive CLI chat channel with sliding-window memory
    Chat {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = false)]
        acp: bool,
        #[arg(long, default_value_t = false)]
        acp_event_stream: bool,
        #[arg(long = "acp-bootstrap-mcp-server")]
        acp_bootstrap_mcp_server: Vec<String>,
        #[arg(long = "acp-cwd")]
        acp_cwd: Option<String>,
    },
    /// Print safe-lane runtime event summary for a session
    SafeLaneSummary {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Search transcript turns across visible sessions
    SessionSearch {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = false)]
        include_archived: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Inspect one exported session-search artifact
    SessionSearchInspect {
        #[arg(long)]
        artifact: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Export one session trajectory artifact with transcript turns and session events
    TrajectoryExport {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Inspect one exported trajectory artifact
    TrajectoryInspect {
        #[arg(long)]
        artifact: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Export or inspect runtime trajectory artifacts for replay, evaluation, or research workflows
    RuntimeTrajectory {
        #[command(subcommand)]
        command: runtime_trajectory_cli::RuntimeTrajectoryCommands,
    },
    /// Send one Telegram message
    TelegramSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_telegram_send_target_kind(),
            value_parser = parse_telegram_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run Telegram channel polling/response loop
    TelegramServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        once: bool,
        #[arg(long)]
        account: Option<String>,
    },
    /// Send one Feishu message or card
    FeishuSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        receive_id_type: Option<String>,
        #[arg(long = "target", visible_alias = "receive-id")]
        target: String,
        #[arg(
            long,
            default_value_t = default_feishu_send_target_kind(),
            value_parser = parse_feishu_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "post-json")]
        post_json: Option<String>,
        #[arg(long)]
        image_key: Option<String>,
        #[arg(long)]
        file_key: Option<String>,
        #[arg(long)]
        image_path: Option<String>,
        #[arg(long)]
        file_path: Option<String>,
        #[arg(long)]
        file_type: Option<String>,
        #[arg(long, default_value_t = false)]
        card: bool,
        #[arg(long)]
        uuid: Option<String>,
    },
    /// Run Feishu event callback server and auto-reply via provider
    FeishuServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    /// Send one Matrix room message
    MatrixSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_matrix_send_target_kind(),
            value_parser = parse_matrix_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run Matrix sync reply loop
    MatrixServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        once: bool,
        #[arg(long)]
        account: Option<String>,
    },
    /// Send one WeCom AIBot proactive message
    WecomSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_wecom_send_target_kind(),
            value_parser = parse_wecom_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run WeCom AIBot long-connection reply loop
    WecomServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
    },
    /// Send one managed Weixin bridge message
    WeixinSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_weixin_send_target_kind(),
            value_parser = parse_weixin_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run one managed Weixin bridge reply loop
    WeixinServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        once: bool,
        #[arg(long)]
        account: Option<String>,
    },
    /// Send one managed QQBot bridge message
    QqbotSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_qqbot_send_target_kind(),
            value_parser = parse_qqbot_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run one managed QQBot bridge reply loop
    QqbotServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        once: bool,
        #[arg(long)]
        account: Option<String>,
    },
    /// Send one managed OneBot bridge message
    OnebotSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_onebot_send_target_kind(),
            value_parser = parse_onebot_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run one managed OneBot bridge reply loop
    OnebotServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        once: bool,
        #[arg(long)]
        account: Option<String>,
    },
    /// Run WhatsApp Cloud API webhook server and auto-reply via provider
    WhatsappServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    /// Send one Discord channel message
    DiscordSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_discord_send_target_kind(),
            value_parser = parse_discord_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one DingTalk custom robot webhook message
    DingtalkSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: Option<String>,
        #[arg(
            long,
            default_value_t = default_dingtalk_send_target_kind(),
            value_parser = parse_dingtalk_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Slack channel message
    SlackSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_slack_send_target_kind(),
            value_parser = parse_slack_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one LINE push message
    LineSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_line_send_target_kind(),
            value_parser = parse_line_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run LINE webhook callback server and auto-reply via provider
    LineServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    /// Send one WhatsApp business message
    WhatsappSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_whatsapp_send_target_kind(),
            value_parser = parse_whatsapp_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one SMTP email message
    EmailSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_email_send_target_kind(),
            value_parser = parse_email_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one generic webhook POST message
    WebhookSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: Option<String>,
        #[arg(
            long,
            default_value_t = default_webhook_send_target_kind(),
            value_parser = parse_webhook_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run a generic inbound webhook server and auto-reply via provider
    WebhookServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    /// Send one Google Chat incoming webhook message
    GoogleChatSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: Option<String>,
        #[arg(
            long,
            default_value_t = default_google_chat_send_target_kind(),
            value_parser = parse_google_chat_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Microsoft Teams incoming webhook message
    TeamsSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: Option<String>,
        #[arg(
            long,
            default_value_t = default_teams_send_target_kind(),
            value_parser = parse_teams_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Tlon direct message or group post
    TlonSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_tlon_send_target_kind(),
            value_parser = parse_tlon_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Signal direct message
    SignalSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_signal_send_target_kind(),
            value_parser = parse_signal_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Twitch chat message
    TwitchSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_twitch_send_target_kind(),
            value_parser = parse_twitch_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Mattermost channel post
    MattermostSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_mattermost_send_target_kind(),
            value_parser = parse_mattermost_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Nextcloud Talk bot room message
    NextcloudTalkSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_nextcloud_talk_send_target_kind(),
            value_parser = parse_nextcloud_talk_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one Synology Chat incoming webhook message
    SynologyChatSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: Option<String>,
        #[arg(
            long,
            default_value_t = default_synology_chat_send_target_kind(),
            value_parser = parse_synology_chat_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one IRC message to a channel or nick
    IrcSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_irc_send_target_kind(),
            value_parser = parse_irc_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Send one iMessage chat through BlueBubbles
    ImessageSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: String,
        #[arg(
            long,
            default_value_t = default_imessage_send_target_kind(),
            value_parser = parse_imessage_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Publish one signed Nostr text note
    NostrSend {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "target")]
        target: Option<String>,
        #[arg(
            long,
            default_value_t = default_nostr_send_target_kind(),
            value_parser = parse_nostr_send_target_kind
        )]
        target_kind: mvp::channel::ChannelOutboundTargetKind,
        #[arg(long)]
        text: String,
    },
    /// Run the multi-channel supervisor for coordinated runtime-backed service-channel serving
    MultiChannelServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: String,
        #[arg(long = "channel-account", value_name = "CHANNEL=ACCOUNT")]
        channel_account: Vec<MultiChannelServeChannelAccount>,
    },
    /// Run the gateway lifecycle namespace
    Gateway {
        #[command(subcommand)]
        command: gateway::service::GatewayCommand,
    },
    /// Run the Feishu integration namespace
    Feishu {
        #[command(subcommand)]
        command: feishu_cli::FeishuCommand,
    },
    /// Print a shell completion script to stdout
    Completions {
        /// Target shell (bash, zsh, fish, powershell, elvish)
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ValidateConfigOutput {
    Text,
    Json,
    ProblemJson,
}

fn parse_multi_channel_serve_channel_account(
    raw: &str,
) -> Result<MultiChannelServeChannelAccount, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("multi-channel channel-account entries cannot be empty".to_owned());
    }

    let (raw_channel_id, raw_account_id) = trimmed.split_once('=').ok_or_else(|| {
        format!("multi-channel channel-account `{trimmed}` must use CHANNEL=ACCOUNT syntax")
    })?;

    let channel_token = raw_channel_id.trim();
    if channel_token.is_empty() {
        return Err(format!(
            "multi-channel channel-account `{trimmed}` is missing a channel id"
        ));
    }

    let supported_channel_ids = supported_multi_channel_serve_channel_ids();
    let supported_channels = supported_channel_ids.join(", ");
    let runtime_descriptor = mvp::channel::resolve_channel_runtime_command_descriptor(channel_token)
        .ok_or_else(|| {
            format!(
                "unrecognized multi-channel service channel `{channel_token}` (available runtime-backed channels: {supported_channels})"
            )
        })?;
    let runtime_channel_id = runtime_descriptor.channel_id;
    let runtime_is_supported = supported_channel_ids.contains(&runtime_channel_id);
    if !runtime_is_supported {
        return Err(format!(
            "multi-channel service channel `{channel_token}` resolves to `{runtime_channel_id}` but is not supported in this build (expected one of: {supported_channels})"
        ));
    }

    let account_token = raw_account_id.trim();
    if account_token.is_empty() {
        return Err(format!(
            "multi-channel channel-account `{trimmed}` is missing an account id"
        ));
    }

    Ok(MultiChannelServeChannelAccount {
        channel_id: runtime_descriptor.channel_id.to_owned(),
        account_id: account_token.to_owned(),
    })
}

fn supported_multi_channel_serve_channel_ids() -> Vec<&'static str> {
    let supported_channels = mvp::channel::background_channel_runtime_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.channel_id)
        .collect::<BTreeSet<_>>();
    supported_channels.into_iter().collect()
}

#[cfg(test)]
#[cfg(test)]
#[path = "lib_multi_channel_serve_tests.rs"]
mod multi_channel_serve_tests;

fn resolved_default_entry_config_path() -> PathBuf {
    std::env::var_os("LOONG_CONFIG_PATH")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(mvp::config::default_config_path)
}

fn default_onboard_command() -> Commands {
    Commands::Onboard {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: None,
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    }
}

pub fn resolve_default_entry_command() -> Commands {
    if resolved_default_entry_config_path().is_file() {
        Commands::Welcome
    } else {
        default_onboard_command()
    }
}

pub fn redacted_command_name(command: &Commands) -> &'static str {
    command.command_kind_for_logging()
}

fn resolve_welcome_config_path() -> CliResult<PathBuf> {
    let config_path = resolved_default_entry_config_path();
    if config_path.is_file() {
        Ok(config_path)
    } else {
        Err(format!(
            "Config file not found at {}. Run `{} onboard` to set up Loong.",
            config_path.display(),
            active_cli_command_name(),
        ))
    }
}

fn render_welcome_banner(config_path: &Path, config: &mvp::config::LoongConfig) -> String {
    let config_path_display = config_path.display().to_string();
    let next_actions = next_actions::collect_setup_next_actions(config, &config_path_display);
    let primary_action = next_actions.first().cloned();
    let secondary_actions = next_actions.iter().skip(1).cloned().collect::<Vec<_>>();
    let render_width = mvp::presentation::detect_render_width();
    let mut sections = Vec::new();

    if let Some(primary_action) = primary_action {
        sections.push(mvp::tui_surface::TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: false,
            items: vec![mvp::tui_surface::TuiActionSpec {
                label: primary_action.label,
                command: primary_action.command,
            }],
        });
    }

    if !secondary_actions.is_empty() {
        sections.push(mvp::tui_surface::TuiSectionSpec::ActionGroup {
            title: Some("also available".to_owned()),
            inline_title_when_wide: false,
            items: secondary_actions
                .into_iter()
                .map(|action| mvp::tui_surface::TuiActionSpec {
                    label: action.label,
                    command: action.command,
                })
                .collect(),
        });
    }

    sections.push(mvp::tui_surface::TuiSectionSpec::KeyValues {
        title: Some("saved setup".to_owned()),
        items: vec![
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "config".to_owned(),
                value: config_path_display,
            },
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "provider".to_owned(),
                value: crate::provider_presentation::active_provider_detail_label(config),
            },
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "model".to_owned(),
                value: config.provider.model.clone(),
            },
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "memory profile".to_owned(),
                value: config.memory.profile.as_str().to_owned(),
            },
        ],
    });
    sections.push(mvp::tui_surface::TuiSectionSpec::Callout {
        tone: mvp::tui_surface::TuiCalloutTone::Info,
        title: Some("operator flow".to_owned()),
        lines: vec![
            "Start with a first answer, then continue in chat for follow-up work.".to_owned(),
            "Use doctor when setup or runtime health feels off instead of debugging the config by hand.".to_owned(),
        ],
    });

    let screen = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some("configured install".to_owned()),
        title: Some("welcome back".to_owned()),
        progress_line: None,
        intro_lines: vec!["Loong is configured and ready.".to_owned()],
        sections,
        choices: Vec::new(),
        footer_lines: vec![format!(
            "Use {} --help to browse the full operator surface.",
            CLI_COMMAND_NAME
        )],
    };

    mvp::tui_surface::render_tui_screen_spec_ratatui(&screen, render_width, false).join("\n")
}

pub fn run_welcome_cli() -> CliResult<()> {
    let config_path = resolve_welcome_config_path()?;
    let config_path_string = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_string.as_str()))?;
    let (_resolved_path, config) = load_result;
    println!("{}", render_welcome_banner(config_path.as_path(), &config));
    Ok(())
}

#[cfg(test)]
#[path = "lib_first_run_entry_tests.rs"]
mod first_run_entry_tests;

pub async fn invoke_connector_cli(operation: &str, payload_raw: &str) -> CliResult<()> {
    let payload = cli_json::parse_json_payload(payload_raw, "invoke-connector payload")?;

    let kernel = kernel_bootstrap::KernelBuilder::default().build();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
        .map_err(|error| format!("token issue failed: {error}"))?;

    let dispatch = kernel
        .execute_connector_core(
            DEFAULT_PACK_ID,
            &token,
            None,
            ConnectorCommand {
                connector_name: "webhook".to_owned(),
                operation: operation.to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload,
            },
        )
        .await
        .map_err(|error| format!("connector dispatch failed: {error}"))?;

    let pretty = serde_json::to_string_pretty(&dispatch.outcome)
        .map_err(|error| format!("serialize connector outcome failed: {error}"))?;
    println!("{pretty}");
    Ok(())
}

pub async fn run_audit_demo() -> CliResult<()> {
    let fixed_clock = Arc::new(FixedClock::new(1_700_000_000));
    let audit_sink = Arc::new(InMemoryAuditSink::default());

    let kernel = kernel_bootstrap::KernelBuilder::default()
        .clock(fixed_clock.clone())
        .audit(audit_sink.clone())
        .build();

    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 30)
        .map_err(|error| format!("token issue failed: {error}"))?;

    let _ = execute_daemon_task_with_supervisor(
        &kernel,
        DEFAULT_PACK_ID,
        &token,
        TaskIntent {
            task_id: "task-audit-01".to_owned(),
            objective: "produce audit evidence".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({}),
        },
    )
    .await?;

    fixed_clock.advance_by(5);

    let _ = kernel
        .execute_connector_core(
            DEFAULT_PACK_ID,
            &token,
            None,
            ConnectorCommand {
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"channel": "audit"}),
            },
        )
        .await
        .map_err(|error| format!("connector invoke failed: {error}"))?;

    kernel
        .revoke_token(&token.token_id, Some(DEFAULT_AGENT_ID))
        .map_err(|error| format!("token revoke failed: {error}"))?;

    let pretty = serde_json::to_string_pretty(&audit_sink.snapshot())
        .map_err(|error| format!("serialize audit events failed: {error}"))?;
    println!("{pretty}");
    Ok(())
}

pub fn init_spec_cli(output_path: &str, preset: InitSpecPreset) -> CliResult<()> {
    let spec = match preset {
        InitSpecPreset::Default => RunnerSpec::template(),
        InitSpecPreset::PluginTrustGuard => RunnerSpec::plugin_trust_guard_template(),
    };
    write_json_file(output_path, &spec)?;
    println!("spec template written to {}", output_path);
    Ok(())
}

pub async fn run_spec_cli(
    spec_path: &str,
    print_audit: bool,
    render_summary: bool,
    bridge_support: &RunSpecBridgeSupportArgs,
) -> CliResult<()> {
    validate_run_spec_bridge_support_args(bridge_support)?;
    let resolved = read_spec_file_with_bridge_support_resolution(
        spec_path,
        run_spec_bridge_support_selection(bridge_support).as_ref(),
    )?;
    let report = execute_spec_with_native_tool_executor_and_bridge_support_provenance(
        &resolved.spec,
        print_audit,
        Some(native_spec_tool_executor),
        resolved.bridge_support_source,
        resolved.bridge_support_delta_source,
        resolved.bridge_support_delta_sha256,
    )
    .await;
    if render_summary {
        eprintln!("{}", render_spec_run_summary(&report));
    }
    let pretty = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("serialize spec run report failed: {error}"))?;
    println!("{pretty}");
    Ok(())
}

fn validate_run_spec_bridge_support_args(args: &RunSpecBridgeSupportArgs) -> CliResult<()> {
    let has_policy_source = args.bridge_support.is_some()
        || args.bridge_profile.is_some()
        || args.bridge_support_delta.is_some();
    let has_sha256_pin =
        args.bridge_support_sha256.is_some() || args.bridge_support_delta_sha256.is_some();

    if has_policy_source || !has_sha256_pin {
        return Ok(());
    }

    Err(
        "run-spec bridge support sha256 pins require --bridge-support, --bridge-profile, or --bridge-support-delta"
            .to_owned(),
    )
}

fn render_spec_run_summary(report: &SpecRunReport) -> String {
    let mut lines = vec![format!(
        "run-spec summary pack={} agent={} status={} operation={}",
        report.pack_id,
        report.agent_id,
        spec_run_status_label(report),
        report.operation_kind
    )];

    if let Some(blocked_reason) = report.blocked_reason.as_deref() {
        lines.push(format!(
            "blocked_reason={}",
            sanitize_summary_field(blocked_reason)
        ));
    }

    if report.plugin_trust_summary.scanned_plugins > 0 {
        let trust = &report.plugin_trust_summary;
        lines.push(format!(
            "plugin_trust scanned={} official={} verified_community={} unverified={} high_risk={} high_risk_unverified={} blocked_auto_apply={} review_required={}",
            trust.scanned_plugins,
            trust.official_plugins,
            trust.verified_community_plugins,
            trust.unverified_plugins,
            trust.high_risk_plugins,
            trust.high_risk_unverified_plugins,
            trust.blocked_auto_apply_plugins,
            trust.review_required_plugins.len()
        ));

        for entry in trust.review_required_plugins.iter().take(3) {
            lines.push(render_plugin_trust_review_summary(entry));
        }
        if trust.review_required_plugins.len() > 3 {
            lines.push(format!(
                "plugin_review remaining={}",
                trust.review_required_plugins.len() - 3
            ));
        }
    }

    if let Some(summary) = report.tool_search_summary.as_ref() {
        lines.push(format!(
            "tool_search {}",
            sanitize_summary_field(&summary.headline)
        ));

        if summary.trust_filter_summary.applied {
            lines.push(format!(
                "tool_search_filters query_requested={} structured_requested={} effective={} conflicting={} filtered_out_by_tier={}",
                format_string_list_or_dash(&summary.trust_filter_summary.query_requested_tiers),
                format_string_list_or_dash(&summary.trust_filter_summary.structured_requested_tiers),
                format_string_list_or_dash(&summary.trust_filter_summary.effective_tiers),
                summary.trust_filter_summary.conflicting_requested_tiers,
                format_usize_rollup(&summary.trust_filter_summary.filtered_out_tier_counts)
            ));
        }

        for (index, entry) in summary.top_results.iter().enumerate() {
            lines.push(format!(
                "tool_search_top[{}] provider={} connector={} tool_id={} trust={} bridge={} score={} setup_ready={} loaded={} deferred={}",
                index + 1,
                entry.provider_id,
                entry.connector_name,
                entry.tool_id,
                entry.trust_tier.as_deref().unwrap_or("-"),
                entry.bridge_kind,
                entry.score,
                entry.setup_ready,
                entry.loaded,
                entry.deferred
            ));
        }
    }

    lines.join("\n")
}

fn spec_run_status_label(report: &SpecRunReport) -> &'static str {
    if report.blocked_reason.is_some() || report.operation_kind == "blocked" {
        "blocked"
    } else {
        "ok"
    }
}

fn render_plugin_trust_review_summary(entry: &PluginTrustReviewEntry) -> String {
    format!(
        "plugin_review plugin={} tier={} bridge={} activation={} bootstrap={} source={} provenance={} reason={}",
        entry.plugin_id,
        entry.trust_tier.as_str(),
        entry.bridge_kind.as_str(),
        plugin_activation_status_label(entry.activation_status),
        entry
            .bootstrap_status
            .map(bootstrap_task_status_label)
            .unwrap_or("-"),
        sanitize_summary_field(&entry.source_path),
        sanitize_summary_field(&entry.provenance_summary),
        sanitize_summary_field(&entry.reason)
    )
}

fn plugin_activation_status_label(status: PluginActivationStatus) -> &'static str {
    match status {
        PluginActivationStatus::Ready => "ready",
        PluginActivationStatus::SetupIncomplete => "setup_incomplete",
        PluginActivationStatus::BlockedInvalidManifestContract => {
            "blocked_invalid_manifest_contract"
        }
        PluginActivationStatus::BlockedUnsupportedBridge => "blocked_unsupported_bridge",
        PluginActivationStatus::BlockedUnsupportedAdapterFamily => {
            "blocked_unsupported_adapter_family"
        }
        PluginActivationStatus::BlockedCompatibilityMode => "blocked_compatibility_mode",
        PluginActivationStatus::BlockedIncompatibleHost => "blocked_incompatible_host",
        PluginActivationStatus::BlockedSlotClaimConflict => "blocked_slot_claim_conflict",
        PluginActivationStatus::Unknown => "unknown",
    }
}

fn bootstrap_task_status_label(status: BootstrapTaskStatus) -> &'static str {
    match status {
        BootstrapTaskStatus::Applied => "applied",
        BootstrapTaskStatus::DeferredUnsupportedAutoApply => "deferred_unsupported_auto_apply",
        BootstrapTaskStatus::SkippedNotReady => "skipped_not_ready",
        BootstrapTaskStatus::SkippedByPolicyLimit => "skipped_by_policy_limit",
    }
}

fn format_string_list_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    values.join(",")
}

fn sanitize_summary_field(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run_spec_bridge_support_selection(
    args: &RunSpecBridgeSupportArgs,
) -> Option<BridgeSupportSelectionInput> {
    let selection = BridgeSupportSelectionInput {
        path: args.bridge_support.clone(),
        bundled_profile: args
            .bridge_profile
            .map(BridgeSupportProfileArg::as_str)
            .map(str::to_owned),
        delta_artifact: args.bridge_support_delta.clone(),
        expected_sha256: args.bridge_support_sha256.clone(),
        expected_delta_sha256: args.bridge_support_delta_sha256.clone(),
    };
    (selection.path.is_some()
        || selection.bundled_profile.is_some()
        || selection.delta_artifact.is_some())
    .then_some(selection)
}

#[derive(Debug, Clone, Deserialize)]
struct RunnerSpecFileInput {
    #[serde(flatten)]
    spec: RunnerSpec,
    #[serde(default)]
    bridge_support_selection: Option<BridgeSupportSelectionInput>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeSupportSelectionInput {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub bundled_profile: Option<String>,
    #[serde(default)]
    pub delta_artifact: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub expected_delta_sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRunnerSpecFile {
    pub spec: RunnerSpec,
    pub bridge_support_source: Option<String>,
    pub bridge_support_delta_source: Option<String>,
    pub bridge_support_delta_sha256: Option<String>,
}

pub fn run_validate_config_cli(
    config_path: Option<&str>,
    as_json: bool,
    output: Option<ValidateConfigOutput>,
    locale: &str,
    fail_on_diagnostics: bool,
) -> CliResult<()> {
    let output = resolve_validate_output(as_json, output)?;
    let normalized_locale = mvp::config::normalize_validation_locale(locale);
    let supported_locales = mvp::config::supported_validation_locales();
    let (resolved_path, diagnostics) =
        mvp::config::validate_file_with_locale(config_path, &normalized_locale)?;
    let diagnostics_count = diagnostics.len();
    let diagnostics_summary = summarize_validation_diagnostics(&diagnostics);

    match output {
        ValidateConfigOutput::Text => {
            if diagnostics.is_empty() {
                println!("config={} valid=true", resolved_path.display());
            } else {
                println!(
                    "config={} valid={} diagnostics={} errors={} warnings={}",
                    resolved_path.display(),
                    diagnostics_summary.valid,
                    diagnostics_count,
                    diagnostics_summary.error_count,
                    diagnostics_summary.warning_count,
                );
                for diagnostic in &diagnostics {
                    println!("{}", diagnostic.message);
                }
            }
        }
        ValidateConfigOutput::Json => {
            let payload = json!({
                "diagnostics_schema_version": 1,
                "config": resolved_path.display().to_string(),
                "valid": diagnostics_summary.valid,
                "error_count": diagnostics_summary.error_count,
                "warning_count": diagnostics_summary.warning_count,
                "locale": normalized_locale,
                "supported_locales": supported_locales.clone(),
                "diagnostics": diagnostics,
            });
            let pretty = serde_json::to_string_pretty(&payload)
                .map_err(|error| format!("serialize config validation output failed: {error}"))?;
            println!("{pretty}");
        }
        ValidateConfigOutput::ProblemJson => {
            let payload = if diagnostics.is_empty() {
                json!({
                    "type": "urn:loong:problem:none",
                    "title": "Configuration Valid",
                    "detail": "No configuration diagnostics were reported.",
                    "instance": resolved_path.display().to_string(),
                    "valid": true,
                    "error_count": 0,
                    "warning_count": 0,
                    "locale": normalized_locale,
                    "supported_locales": supported_locales.clone(),
                    "diagnostics_schema_version": 1,
                    "errors": [],
                })
            } else {
                json!({
                    "type": if diagnostics_summary.valid {
                        "urn:loong:problem:config.validation_warning"
                    } else {
                        "urn:loong:problem:config.validation_failed"
                    },
                    "title": if diagnostics_summary.valid {
                        "Configuration Warnings Reported"
                    } else {
                        "Configuration Validation Failed"
                    },
                    "detail": format!("{} configuration diagnostic(s) were reported.", diagnostics_count),
                    "instance": resolved_path.display().to_string(),
                    "valid": diagnostics_summary.valid,
                    "error_count": diagnostics_summary.error_count,
                    "warning_count": diagnostics_summary.warning_count,
                    "locale": normalized_locale,
                    "supported_locales": supported_locales.clone(),
                    "diagnostics_schema_version": 1,
                    "errors": diagnostics,
                })
            };
            let pretty = serde_json::to_string_pretty(&payload).map_err(|error| {
                format!("serialize config validation problem output failed: {error}")
            })?;
            println!("{pretty}");
        }
    }

    if fail_on_diagnostics && diagnostics_count > 0 {
        return Err(format!(
            "config validation failed with {diagnostics_count} diagnostic(s)"
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationDiagnosticSummary {
    pub valid: bool,
    pub error_count: usize,
    pub warning_count: usize,
}

pub fn summarize_validation_diagnostics(
    diagnostics: &[mvp::config::ConfigValidationDiagnostic],
) -> ValidationDiagnosticSummary {
    let error_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .count();
    let warning_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "warn")
        .count();
    ValidationDiagnosticSummary {
        valid: error_count == 0,
        error_count,
        warning_count,
    }
}

pub fn resolve_validate_output(
    as_json: bool,
    output: Option<ValidateConfigOutput>,
) -> CliResult<ValidateConfigOutput> {
    if as_json && output.is_some() {
        return Err(
            "validate-config: `--json` conflicts with `--output`; use one of them".to_owned(),
        );
    }
    if as_json {
        return Ok(ValidateConfigOutput::Json);
    }
    Ok(output.unwrap_or(ValidateConfigOutput::Text))
}

pub async fn run_list_models_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let models = mvp::provider::fetch_available_models(&config).await?;
    if as_json {
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "provider_kind": config.provider.kind,
            "models_endpoint": config.provider.models_endpoint(),
            "models": models,
        });
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize model-list output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "config={} provider_kind={:?} models_endpoint={}",
        resolved_path.display(),
        config.provider.kind,
        config.provider.models_endpoint()
    );
    for model in models {
        println!("{model}");
    }
    Ok(())
}

pub const RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION: u32 = 2;
pub const RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 2;
#[derive(Debug, Clone)]
pub struct RuntimeSnapshotCliState {
    pub config: String,
    pub provider: RuntimeSnapshotProviderState,
    pub context_engine: mvp::conversation::ContextEngineRuntimeSnapshot,
    pub memory_system: mvp::memory::MemorySystemRuntimeSnapshot,
    pub acp: mvp::acp::AcpRuntimeSnapshot,
    pub enabled_channel_ids: Vec<String>,
    pub enabled_runtime_backed_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub enabled_plugin_backed_channel_ids: Vec<String>,
    pub enabled_outbound_only_channel_ids: Vec<String>,
    pub channels: mvp::channel::ChannelInventory,
    pub tool_runtime: mvp::tools::runtime_config::ToolRuntimeConfig,
    pub visible_tool_names: Vec<String>,
    pub discoverable_tool_summary: mvp::tools::DiscoverableToolSurfaceSummary,
    pub capability_snapshot: String,
    pub capability_snapshot_sha256: String,
    pub tool_calling: RuntimeSnapshotToolCallingState,
    pub runtime_plugins: RuntimeSnapshotRuntimePluginsState,
    pub external_skills: RuntimeSnapshotExternalSkillsState,
    pub restore_spec: RuntimeSnapshotRestoreSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSnapshotInventoryStatus {
    Ok,
    Disabled,
    Error,
}

impl RuntimeSnapshotInventoryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Disabled => "disabled",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotExternalSkillsState {
    pub policy: mvp::tools::runtime_config::ExternalSkillsRuntimePolicy,
    pub override_active: bool,
    pub inventory_status: RuntimeSnapshotInventoryStatus,
    pub inventory_error: Option<String>,
    pub inventory: Value,
    pub resolved_skill_count: usize,
    pub shadowed_skill_count: usize,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotRuntimePluginsState {
    pub enabled: bool,
    pub roots_source: String,
    pub roots: Vec<String>,
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub inventory_status: RuntimeSnapshotInventoryStatus,
    pub inventory_error: Option<String>,
    pub readiness_evaluation: String,
    pub scanned_root_count: usize,
    pub scanned_file_count: usize,
    pub discovered_plugin_count: usize,
    pub translated_plugin_count: usize,
    pub ready_plugin_count: usize,
    pub setup_incomplete_plugin_count: usize,
    pub blocked_plugin_count: usize,
    pub shadowed_plugin_ids: Vec<String>,
    pub discovery_guidance:
        Option<crate::runtime_plugin_discovery::RuntimePluginDiscoveryGuidanceView>,
    pub native_extension_authoring_summary:
        Option<crate::native_extension_authoring::NativeExtensionAuthoringSummaryView>,
    pub plugins: Vec<RuntimeSnapshotRuntimePluginState>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotRuntimePluginState {
    pub manifest_api_version: Option<String>,
    pub plugin_version: Option<String>,
    pub dialect: String,
    pub dialect_version: Option<String>,
    pub compatibility_mode: String,
    pub compatibility_shim: Option<String>,
    pub compatibility_shim_support_version: Option<String>,
    pub compatibility_shim_supported_dialects: Vec<String>,
    pub compatibility_shim_supported_bridges: Vec<String>,
    pub compatibility_shim_supported_adapter_families: Vec<String>,
    pub compatibility_shim_supported_source_languages: Vec<String>,
    pub compatibility_shim_mismatch_reasons: Vec<String>,
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub source_path: String,
    pub source_kind: String,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub capabilities: Vec<String>,
    pub bridge_kind: String,
    pub adapter_family: String,
    pub source_language: String,
    pub entrypoint_hint: String,
    pub setup_mode: Option<String>,
    pub setup_surface: Option<String>,
    pub slot_claims: Vec<String>,
    pub conflicting_slot_claims: Vec<String>,
    pub status: String,
    pub reason: String,
    pub bootstrap_hint: Option<String>,
    pub diagnostic_codes: Vec<String>,
    pub missing_required_env_vars: Vec<String>,
    pub missing_required_config_keys: Vec<String>,
    pub extension_contract: Option<String>,
    pub extension_family: Option<String>,
    pub extension_trust_lane: Option<String>,
    pub extension_facets: Vec<String>,
    pub extension_methods: Vec<String>,
    pub extension_events: Vec<String>,
    pub extension_host_actions: Vec<String>,
    pub extension_metadata_issues: Vec<String>,
    pub authoring_guidance:
        Option<crate::native_extension_authoring::NativeExtensionAuthoringGuidanceView>,
}

pub(crate) const RUNTIME_WEB_ACCESS_SEPARATION_NOTE: &str = "web-search provider settings affect only query search mode; ordinary network access stays separately governed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeWebAccessSummary {
    pub ordinary_network_access_enabled: bool,
    pub query_search_enabled: bool,
    pub query_search_default_provider: String,
    pub query_search_credential_ready: bool,
    pub separation_note: &'static str,
}

pub(crate) fn runtime_web_access_summary(
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> RuntimeWebAccessSummary {
    let ordinary_network_access_enabled = runtime.web_fetch.enabled;
    let query_search_enabled = runtime.web_search.enabled;
    let query_search_default_provider = runtime.web_search.default_provider.clone();
    let query_search_credential_ready = web_search_provider_credential_ready(&runtime.web_search);
    let separation_note = RUNTIME_WEB_ACCESS_SEPARATION_NOTE;

    RuntimeWebAccessSummary {
        ordinary_network_access_enabled,
        query_search_enabled,
        query_search_default_provider,
        query_search_credential_ready,
        separation_note,
    }
}

fn web_search_provider_credential_ready(
    policy: &mvp::tools::runtime_config::WebSearchRuntimePolicy,
) -> bool {
    let provider = policy.default_provider.trim();
    match provider {
        mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO => true,
        mvp::config::WEB_SEARCH_PROVIDER_BRAVE => {
            option_has_non_empty_runtime_text(policy.brave_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY => {
            option_has_non_empty_runtime_text(policy.tavily_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_PERPLEXITY => {
            option_has_non_empty_runtime_text(policy.perplexity_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_EXA => {
            option_has_non_empty_runtime_text(policy.exa_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL => {
            option_has_non_empty_runtime_text(policy.firecrawl_api_key.as_deref())
        }
        mvp::config::WEB_SEARCH_PROVIDER_JINA => {
            option_has_non_empty_runtime_text(policy.jina_api_key.as_deref())
        }
        _ => false,
    }
}

fn option_has_non_empty_runtime_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshotArtifactMetadata {
    pub created_at: String,
    pub label: Option<String>,
    pub experiment_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshotArtifactLineage {
    pub snapshot_id: String,
    pub created_at: String,
    pub label: Option<String>,
    pub experiment_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSnapshotRestoreSpec {
    pub provider: RuntimeSnapshotRestoreProviderSpec,
    pub conversation: mvp::config::ConversationConfig,
    pub memory: mvp::config::MemoryConfig,
    pub acp: mvp::config::AcpConfig,
    pub tools: mvp::config::ToolConfig,
    pub external_skills: mvp::config::ExternalSkillsConfig,
    #[serde(default)]
    pub runtime_plugins: mvp::config::RuntimePluginsConfig,
    pub managed_skills: RuntimeSnapshotRestoreManagedSkillsSpec,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSnapshotRestoreProviderSpec {
    pub active_provider: Option<String>,
    pub last_provider: Option<String>,
    pub profiles: BTreeMap<String, mvp::config::ProviderProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeSnapshotRestoreManagedSkillsSpec {
    pub skills: Vec<RuntimeSnapshotRestoreManagedSkillSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshotRestoreManagedSkillSpec {
    pub skill_id: String,
    pub display_name: String,
    pub summary: String,
    pub source_kind: String,
    pub source_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshotArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSnapshotArtifactDocument {
    pub config: String,
    pub schema: RuntimeSnapshotArtifactSchema,
    pub lineage: RuntimeSnapshotArtifactLineage,
    pub provider: Value,
    pub context_engine: Value,
    pub memory_system: Value,
    pub acp: Value,
    pub channels: Value,
    pub tool_runtime: Value,
    pub tools: Value,
    #[serde(default)]
    pub runtime_plugins: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_plugin_inventory: Option<crate::plugins_cli::RuntimePluginInventoryReadModel>,
    pub external_skills: Value,
    pub restore_spec: RuntimeSnapshotRestoreSpec,
}

pub async fn run_runtime_snapshot_cli(
    config_path: Option<&str>,
    as_json: bool,
    output_path: Option<&str>,
    label: Option<&str>,
    experiment_id: Option<&str>,
    parent_snapshot_id: Option<&str>,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = collect_runtime_snapshot_cli_state_from_parts(resolved_path.as_path(), &config)?;
    let runtime_plugin_inventory =
        crate::plugins_cli::runtime_plugin_inventory_read_model(&config).await;
    let metadata =
        runtime_snapshot_artifact_metadata_now(label, experiment_id, parent_snapshot_id)?;
    let artifact_payload = build_runtime_snapshot_artifact_json_payload_with_inventory(
        &snapshot,
        &metadata,
        Some(runtime_plugin_inventory),
    )?;

    if let Some(output_path) = output_path {
        persist_json_artifact(output_path, &artifact_payload, "runtime snapshot artifact")?;
    }

    if as_json {
        let pretty = serde_json::to_string_pretty(&artifact_payload).map_err(|error| {
            format!("serialize runtime snapshot artifact output failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_runtime_snapshot_artifact_text(&snapshot, &artifact_payload)
    );
    Ok(())
}

pub fn collect_runtime_snapshot_cli_state(
    config_path: Option<&str>,
) -> CliResult<RuntimeSnapshotCliState> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    collect_runtime_snapshot_cli_state_from_parts(resolved_path.as_path(), &config)
}

pub(crate) fn collect_runtime_snapshot_cli_state_from_loaded_config(
    loaded_config: &supervisor::LoadedSupervisorConfig,
) -> CliResult<RuntimeSnapshotCliState> {
    let resolved_path = loaded_config.resolved_path.as_path();
    let config = &loaded_config.config;
    collect_runtime_snapshot_cli_state_from_parts(resolved_path, config)
}

fn collect_runtime_snapshot_cli_state_from_parts(
    resolved_path: &Path,
    config: &mvp::config::LoongConfig,
) -> CliResult<RuntimeSnapshotCliState> {
    let config_display = resolved_path.display().to_string();
    let provider = collect_runtime_snapshot_provider_state(config);
    let context_engine = mvp::conversation::collect_context_engine_runtime_snapshot(config)?;
    let memory_system = mvp::memory::collect_memory_system_runtime_snapshot(config)?;
    let acp = mvp::acp::collect_acp_runtime_snapshot(config)?;
    let enabled_channel_ids = config.enabled_channel_ids();
    let enabled_runtime_backed_channel_ids = config.enabled_runtime_backed_channel_ids();
    let enabled_service_channel_ids = config.enabled_service_channel_ids();
    let enabled_plugin_backed_channel_ids = config.enabled_plugin_backed_channel_ids();
    let enabled_outbound_only_channel_ids = config.enabled_outbound_only_channel_ids();
    let channels = mvp::channel::channel_inventory(config);
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        config,
        Some(resolved_path),
    );
    let (external_skills, snapshot_tool_runtime) =
        collect_runtime_snapshot_external_skills_state(&tool_runtime);
    let tool_view = mvp::tools::runtime_tool_view_for_runtime_config(&snapshot_tool_runtime);
    let visible_tools = tool_view
        .tool_names()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let discoverable_tool_summary =
        mvp::tools::runtime_discoverable_tool_surface_summary_with_config(
            &snapshot_tool_runtime,
            Some(&tool_view),
        );
    let capability_snapshot = mvp::tools::capability_snapshot_with_config(&snapshot_tool_runtime);
    let capability_snapshot_sha256 =
        runtime_snapshot_tool_digest(&visible_tools, &capability_snapshot)?;
    let tool_calling = collect_runtime_snapshot_tool_calling_state(config, visible_tools.len());
    let runtime_plugins = collect_runtime_snapshot_runtime_plugins_state(config);
    let restore_spec = build_runtime_snapshot_restore_spec(config, &external_skills);
    Ok(RuntimeSnapshotCliState {
        config: config_display,
        provider,
        context_engine,
        memory_system,
        acp,
        enabled_channel_ids,
        enabled_runtime_backed_channel_ids,
        enabled_service_channel_ids,
        enabled_plugin_backed_channel_ids,
        enabled_outbound_only_channel_ids,
        channels,
        tool_runtime: snapshot_tool_runtime,
        visible_tool_names: visible_tools,
        discoverable_tool_summary,
        capability_snapshot,
        capability_snapshot_sha256,
        tool_calling,
        runtime_plugins,
        external_skills,
        restore_spec,
    })
}

fn collect_runtime_snapshot_provider_state(
    config: &mvp::config::LoongConfig,
) -> RuntimeSnapshotProviderState {
    let active_profile_id = config
        .active_provider_id()
        .unwrap_or(config.provider.kind.profile().id)
        .to_owned();
    let saved_profile_ids = provider_presentation::saved_provider_profile_ids(config);
    let profiles = if config.providers.is_empty() {
        vec![build_runtime_snapshot_provider_profile_state(
            active_profile_id.as_str(),
            &mvp::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: config.provider.clone(),
            },
            true,
        )]
    } else {
        saved_profile_ids
            .iter()
            .filter_map(|profile_id| {
                config.providers.get(profile_id).map(|profile| {
                    build_runtime_snapshot_provider_profile_state(
                        profile_id,
                        profile,
                        profile_id == &active_profile_id,
                    )
                })
            })
            .collect::<Vec<_>>()
    };

    let transport_metrics = mvp::provider::provider_http_client_runtime_metrics_snapshot();
    let transport_runtime = RuntimeSnapshotProviderTransportState {
        http_client_cache_entries: transport_metrics.cache_entry_count,
        http_client_cache_hits: transport_metrics.cache_hit_count,
        http_client_cache_misses: transport_metrics.cache_miss_count,
        built_http_clients: transport_metrics.built_client_count,
    };

    RuntimeSnapshotProviderState {
        active_profile_id,
        active_label: provider_presentation::active_provider_detail_label(config),
        last_provider_id: config.last_provider_id().map(str::to_owned),
        saved_profile_ids,
        transport_runtime,
        profiles,
    }
}

fn build_runtime_snapshot_provider_profile_state(
    profile_id: &str,
    profile: &mvp::config::ProviderProfileConfig,
    is_active: bool,
) -> RuntimeSnapshotProviderProfileState {
    let provider = &profile.provider;
    let descriptor = provider.descriptor_document();
    let mut header_names = provider.headers.keys().cloned().collect::<Vec<_>>();
    header_names.sort();

    RuntimeSnapshotProviderProfileState {
        profile_id: profile_id.to_owned(),
        is_active,
        default_for_kind: profile.default_for_kind,
        descriptor,
        kind: provider.kind,
        model: provider.model.clone(),
        wire_api: provider.wire_api,
        base_url: provider.resolved_base_url(),
        endpoint: provider.endpoint(),
        models_endpoint: provider.models_endpoint(),
        protocol_family: provider.kind.profile().protocol_family.as_str(),
        credential_resolved: runtime_snapshot_provider_credentials_resolved(provider),
        auth_env: provider.resolved_auth_env_name(),
        reasoning_effort: provider
            .reasoning_effort
            .map(|value| value.as_str().to_owned()),
        temperature: provider.temperature,
        max_tokens: provider.max_tokens,
        request_timeout_ms: provider.request_timeout_ms,
        retry_max_attempts: provider.retry_max_attempts,
        header_names,
        preferred_models: provider.preferred_models.clone(),
    }
}

fn runtime_snapshot_provider_credentials_resolved(provider: &mvp::config::ProviderConfig) -> bool {
    provider_credential_policy::provider_has_locally_available_credentials(provider)
}

fn collect_runtime_snapshot_external_skills_state(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> (
    RuntimeSnapshotExternalSkillsState,
    mvp::tools::runtime_config::ToolRuntimeConfig,
) {
    let empty_inventory = json!({
        "skills": [],
        "shadowed_skills": [],
    });

    let (effective_policy, override_active) =
        match runtime_snapshot_effective_external_skills_policy(tool_runtime) {
            Ok(policy_state) => policy_state,
            Err(error) => {
                return (
                    RuntimeSnapshotExternalSkillsState {
                        policy: tool_runtime.external_skills.clone(),
                        override_active: false,
                        inventory_status: RuntimeSnapshotInventoryStatus::Error,
                        inventory_error: Some(error.clone()),
                        inventory: json!({
                            "skills": [],
                            "shadowed_skills": [],
                            "error": error,
                        }),
                        resolved_skill_count: 0,
                        shadowed_skill_count: 0,
                    },
                    tool_runtime.clone(),
                );
            }
        };

    let mut effective_tool_runtime = tool_runtime.clone();
    effective_tool_runtime.external_skills = effective_policy.clone();

    if !effective_policy.enabled {
        return (
            RuntimeSnapshotExternalSkillsState {
                policy: effective_policy,
                override_active,
                inventory_status: RuntimeSnapshotInventoryStatus::Disabled,
                inventory_error: None,
                inventory: empty_inventory,
                resolved_skill_count: 0,
                shadowed_skill_count: 0,
            },
            effective_tool_runtime,
        );
    }

    match mvp::tools::execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.list".to_owned(),
            payload: json!({}),
        },
        &effective_tool_runtime,
    ) {
        Ok(outcome) => (
            RuntimeSnapshotExternalSkillsState {
                policy: effective_policy,
                override_active,
                inventory_status: RuntimeSnapshotInventoryStatus::Ok,
                inventory_error: None,
                resolved_skill_count: json_array_len(outcome.payload.get("skills")),
                shadowed_skill_count: json_array_len(outcome.payload.get("shadowed_skills")),
                inventory: outcome.payload,
            },
            effective_tool_runtime,
        ),
        Err(error) => (
            RuntimeSnapshotExternalSkillsState {
                policy: effective_policy,
                override_active,
                inventory_status: RuntimeSnapshotInventoryStatus::Error,
                inventory_error: Some(error.clone()),
                inventory: json!({
                    "skills": [],
                    "shadowed_skills": [],
                    "error": error,
                }),
                resolved_skill_count: 0,
                shadowed_skill_count: 0,
            },
            effective_tool_runtime,
        ),
    }
}

pub(crate) fn collect_runtime_snapshot_runtime_plugins_state(
    config: &mvp::config::LoongConfig,
) -> RuntimeSnapshotRuntimePluginsState {
    let readiness_evaluation = config
        .runtime_plugins
        .readiness_evaluation_label()
        .to_owned();
    let roots = config
        .runtime_plugins
        .resolved_root_selection()
        .roots
        .into_iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>();
    let roots_source = config
        .runtime_plugins
        .resolved_root_selection()
        .source
        .to_owned();
    let supported_bridges = config
        .runtime_plugins
        .resolved_supported_bridges()
        .unwrap_or_default()
        .into_iter()
        .map(|bridge_kind| bridge_kind.as_str().to_owned())
        .collect::<Vec<_>>();
    let supported_adapter_families = config
        .runtime_plugins
        .normalized_supported_adapter_families();

    if !config.runtime_plugins.enabled {
        return RuntimeSnapshotRuntimePluginsState {
            enabled: false,
            roots_source,
            roots,
            supported_bridges,
            supported_adapter_families,
            inventory_status: RuntimeSnapshotInventoryStatus::Disabled,
            inventory_error: None,
            readiness_evaluation,
            scanned_root_count: 0,
            scanned_file_count: 0,
            discovered_plugin_count: 0,
            translated_plugin_count: 0,
            ready_plugin_count: 0,
            setup_incomplete_plugin_count: 0,
            blocked_plugin_count: 0,
            shadowed_plugin_ids: Vec::new(),
            discovery_guidance: None,
            native_extension_authoring_summary: None,
            plugins: Vec::new(),
        };
    }

    let root_selection = config.runtime_plugins.resolved_root_selection();
    let resolved_roots = root_selection.roots;
    if resolved_roots.is_empty() {
        return RuntimeSnapshotRuntimePluginsState {
            enabled: true,
            roots_source,
            roots,
            supported_bridges,
            supported_adapter_families,
            inventory_status: RuntimeSnapshotInventoryStatus::Error,
            inventory_error: Some(
                "runtime_plugins.enabled=true but no runtime plugin roots were found; create `.loong/extensions/` in the project, `~/.loong/agent/extensions/` globally, or set runtime_plugins.roots explicitly"
                    .to_owned(),
            ),
            readiness_evaluation,
            scanned_root_count: 0,
            scanned_file_count: 0,
            discovered_plugin_count: 0,
            translated_plugin_count: 0,
            ready_plugin_count: 0,
            setup_incomplete_plugin_count: 0,
            blocked_plugin_count: 0,
            shadowed_plugin_ids: Vec::new(),
            discovery_guidance: None,
            native_extension_authoring_summary: None,
            plugins: Vec::new(),
        };
    }

    let scanner = PluginScanner::new();
    let mut combined = kernel::PluginScanReport::default();
    for root in &resolved_roots {
        let report = match scanner.scan_path(root) {
            Ok(report) => report,
            Err(error) => {
                return RuntimeSnapshotRuntimePluginsState {
                    enabled: true,
                    roots_source,
                    roots,
                    supported_bridges,
                    supported_adapter_families,
                    inventory_status: RuntimeSnapshotInventoryStatus::Error,
                    inventory_error: Some(format!(
                        "runtime plugin scan failed for {}: {error}",
                        root.display()
                    )),
                    readiness_evaluation,
                    scanned_root_count: 0,
                    scanned_file_count: 0,
                    discovered_plugin_count: 0,
                    translated_plugin_count: 0,
                    ready_plugin_count: 0,
                    setup_incomplete_plugin_count: 0,
                    blocked_plugin_count: 0,
                    shadowed_plugin_ids: Vec::new(),
                    discovery_guidance: None,
                    native_extension_authoring_summary: None,
                    plugins: Vec::new(),
                };
            }
        };
        merge_plugin_scan_report(&mut combined, report);
    }

    let bridge_matrix = match config.runtime_plugins.resolved_bridge_support_matrix() {
        Ok(matrix) => matrix,
        Err(error) => {
            return RuntimeSnapshotRuntimePluginsState {
                enabled: true,
                roots_source,
                roots,
                supported_bridges,
                supported_adapter_families,
                inventory_status: RuntimeSnapshotInventoryStatus::Error,
                inventory_error: Some(error),
                readiness_evaluation,
                scanned_root_count: resolved_roots.len(),
                scanned_file_count: combined.scanned_files,
                discovered_plugin_count: combined.matched_plugins,
                translated_plugin_count: 0,
                ready_plugin_count: 0,
                setup_incomplete_plugin_count: 0,
                blocked_plugin_count: 0,
                shadowed_plugin_ids: Vec::new(),
                discovery_guidance: None,
                native_extension_authoring_summary: None,
                plugins: Vec::new(),
            };
        }
    };

    let translator = PluginTranslator::new();
    let translation = translator.translate_scan_report(&combined);
    let descriptor_by_key = combined
        .descriptors
        .iter()
        .map(|descriptor| {
            (
                (
                    descriptor.path.clone(),
                    descriptor.manifest.plugin_id.clone(),
                ),
                descriptor,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let readiness_context = runtime_plugin_setup_readiness_context(config);
    let activation = translator.plan_activation(&translation, &bridge_matrix, &readiness_context);
    let inventory_entries = activation.inventory_entries(&translation);
    let inventory_by_key = inventory_entries
        .into_iter()
        .map(|entry| ((entry.source_path.clone(), entry.plugin_id.clone()), entry))
        .collect::<BTreeMap<_, _>>();

    let plugins = translation
        .entries
        .iter()
        .map(|entry| {
            let entry_key = (entry.source_path.clone(), entry.plugin_id.clone());
            let inventory_entry = inventory_by_key.get(&entry_key);
            let descriptor = descriptor_by_key.get(&entry_key).copied();
            let setup_mode = entry
                .setup
                .as_ref()
                .map(|setup| setup.mode.as_str().to_owned());
            let setup_surface = entry.setup.as_ref().and_then(|setup| setup.surface.clone());
            let setup_requirements = evaluate_plugin_setup_requirements(
                entry
                    .setup
                    .as_ref()
                    .map(|setup| setup.required_env_vars.as_slice())
                    .unwrap_or(&[]),
                entry
                    .setup
                    .as_ref()
                    .map(|setup| setup.required_config_keys.as_slice())
                    .unwrap_or(&[]),
                &readiness_context,
            );
            let activation_status = inventory_entry.and_then(|item| item.activation_status);
            let slot_claims = entry
                .slot_claims
                .iter()
                .map(kernel::PluginSlotClaim::canonical_label)
                .collect::<Vec<_>>();
            let extension_declarations =
                runtime_plugin_extension_declarations_from_metadata(&entry.metadata);
            let authoring_guidance = crate::native_extension_authoring::
                build_native_extension_authoring_guidance_from_runtime_profile(
                    entry.package_root.as_str(),
                    entry.plugin_id.as_str(),
                    entry.runtime.bridge_kind.as_str(),
                    entry.runtime.source_language.as_str(),
                    extension_declarations.contract.clone(),
                    extension_declarations.family.clone(),
                    extension_declarations.trust_lane.clone(),
                    extension_declarations.methods.clone(),
                    extension_declarations.events.clone(),
                    extension_declarations.host_actions.clone(),
                    extension_declarations.metadata_issues.clone(),
                );
            let compatibility_shim = inventory_entry
                .and_then(|item| item.compatibility_shim.as_ref())
                .map(|shim| shim.shim_id.clone());
            let compatibility_shim_support =
                inventory_entry.and_then(|item| item.compatibility_shim_support.as_ref());
            let compatibility_shim_support_version =
                compatibility_shim_support.and_then(|support| support.version.clone());
            let compatibility_shim_supported_dialects =
                compatibility_shim_support_dialect_labels(compatibility_shim_support);
            let compatibility_shim_supported_bridges =
                compatibility_shim_support_bridge_labels(compatibility_shim_support);
            let compatibility_shim_supported_adapter_families = compatibility_shim_support
                .map(|support| {
                    support
                        .supported_adapter_families
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let compatibility_shim_supported_source_languages = compatibility_shim_support
                .map(|support| {
                    support
                        .supported_source_languages
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let compatibility_shim_mismatch_reasons = inventory_entry
                .map(|item| item.compatibility_shim_support_mismatch_reasons.clone())
                .unwrap_or_default();
            let bootstrap_hint = inventory_entry.and_then(|item| item.bootstrap_hint.clone());
            let diagnostic_codes = inventory_entry
                .map(|item| runtime_plugin_diagnostic_codes(&item.diagnostic_findings))
                .unwrap_or_else(|| runtime_plugin_diagnostic_codes(&entry.diagnostic_findings));
            let conflicting_slot_claims = if matches!(
                activation_status,
                Some(PluginActivationStatus::BlockedSlotClaimConflict)
            ) {
                slot_claims.clone()
            } else {
                Vec::new()
            };
            let status = activation_status
                .map(runtime_plugin_activation_status)
                .unwrap_or("unknown")
                .to_owned();
            let reason = inventory_entry
                .and_then(|item| item.activation_reason.clone())
                .unwrap_or_else(|| "-".to_owned());
            let missing_required_env_vars = if matches!(
                activation_status,
                Some(PluginActivationStatus::SetupIncomplete)
            ) {
                setup_requirements.missing_required_env_vars
            } else {
                Vec::new()
            };
            let missing_required_config_keys = if matches!(
                activation_status,
                Some(PluginActivationStatus::SetupIncomplete)
            ) {
                setup_requirements.missing_required_config_keys
            } else {
                Vec::new()
            };

            RuntimeSnapshotRuntimePluginState {
                manifest_api_version: entry.manifest_api_version.clone(),
                plugin_version: entry.plugin_version.clone(),
                dialect: entry.dialect.as_str().to_owned(),
                dialect_version: entry.dialect_version.clone(),
                compatibility_mode: entry.compatibility_mode.as_str().to_owned(),
                compatibility_shim,
                compatibility_shim_support_version,
                compatibility_shim_supported_dialects,
                compatibility_shim_supported_bridges,
                compatibility_shim_supported_adapter_families,
                compatibility_shim_supported_source_languages,
                compatibility_shim_mismatch_reasons,
                plugin_id: entry.plugin_id.clone(),
                provider_id: entry.provider_id.clone(),
                connector_name: entry.connector_name.clone(),
                source_path: entry.source_path.clone(),
                source_kind: entry.source_kind.as_str().to_owned(),
                package_root: entry.package_root.clone(),
                package_manifest_path: entry.package_manifest_path.clone(),
                summary: descriptor.and_then(|descriptor| descriptor.manifest.summary.clone()),
                tags: descriptor
                    .map(|descriptor| descriptor.manifest.tags.clone())
                    .unwrap_or_default(),
                capabilities: entry
                    .capabilities
                    .iter()
                    .map(|capability| capability.as_str().to_owned())
                    .collect(),
                bridge_kind: entry.runtime.bridge_kind.as_str().to_owned(),
                adapter_family: entry.runtime.adapter_family.clone(),
                source_language: entry.runtime.source_language.clone(),
                entrypoint_hint: entry.runtime.entrypoint_hint.clone(),
                setup_mode,
                setup_surface,
                slot_claims,
                conflicting_slot_claims,
                status,
                reason,
                bootstrap_hint,
                diagnostic_codes,
                missing_required_env_vars,
                missing_required_config_keys,
                extension_contract: extension_declarations.contract,
                extension_family: extension_declarations.family,
                extension_trust_lane: extension_declarations.trust_lane,
                extension_facets: extension_declarations.facets,
                extension_methods: extension_declarations.methods,
                extension_events: extension_declarations.events,
                extension_host_actions: extension_declarations.host_actions,
                extension_metadata_issues: extension_declarations.metadata_issues,
                authoring_guidance,
            }
        })
        .collect::<Vec<_>>();
    let (plugins, shadowed_plugin_ids, shadowed_by_plugin_id) = if roots_source == "auto_discovered"
    {
        let selection =
            kernel::prefer_first_plugin_ids(plugins, |plugin| plugin.plugin_id.as_str());
        (
            selection.effective,
            selection.shadowed_plugin_ids,
            selection.shadowed_by_plugin_id,
        )
    } else {
        (plugins, Vec::new(), BTreeMap::new())
    };
    let native_extension_authoring_summary =
        crate::native_extension_authoring::summarize_native_extension_authoring_guidance(
            &plugins
                .iter()
                .filter_map(|plugin| plugin.authoring_guidance.clone())
                .collect::<Vec<_>>(),
        );
    let shadowed_conflicts =
        crate::runtime_plugin_discovery::build_runtime_plugin_shadowing_conflicts(
            &plugins,
            &shadowed_by_plugin_id,
            |plugin| plugin.plugin_id.as_str(),
            |plugin| plugin.source_path.as_str(),
        );
    let discovery_guidance =
        crate::runtime_plugin_discovery::build_runtime_plugin_discovery_guidance(
            Some(roots_source.as_str()),
            shadowed_conflicts,
        );

    RuntimeSnapshotRuntimePluginsState {
        enabled: true,
        roots_source,
        roots,
        supported_bridges,
        supported_adapter_families,
        inventory_status: RuntimeSnapshotInventoryStatus::Ok,
        inventory_error: None,
        readiness_evaluation,
        scanned_root_count: resolved_roots.len(),
        scanned_file_count: combined.scanned_files,
        discovered_plugin_count: combined.matched_plugins,
        translated_plugin_count: translation.translated_plugins,
        ready_plugin_count: activation.ready_plugins,
        setup_incomplete_plugin_count: activation.setup_incomplete_plugins,
        blocked_plugin_count: activation.blocked_plugins,
        shadowed_plugin_ids,
        discovery_guidance,
        native_extension_authoring_summary,
        plugins,
    }
}

fn merge_plugin_scan_report(
    combined: &mut kernel::PluginScanReport,
    report: kernel::PluginScanReport,
) {
    let kernel::PluginScanReport {
        scanned_files,
        matched_plugins,
        descriptors,
        diagnostic_findings,
    } = report;

    combined.scanned_files += scanned_files;
    combined.matched_plugins += matched_plugins;
    combined.descriptors.extend(descriptors);
    combined.diagnostic_findings.extend(diagnostic_findings);
}

fn runtime_plugin_extension_declarations_from_metadata(
    metadata: &BTreeMap<String, String>,
) -> kernel::PluginNativeExtensionDeclarations {
    kernel::plugin_native_extension_declarations_from_metadata(metadata)
}

fn runtime_plugin_diagnostic_codes(
    diagnostic_findings: &[kernel::PluginDiagnosticFinding],
) -> Vec<String> {
    diagnostic_findings
        .iter()
        .map(|finding| finding.code.as_str().to_owned())
        .collect()
}

fn compatibility_shim_support_dialect_labels(
    support: Option<&kernel::PluginCompatibilityShimSupport>,
) -> Vec<String> {
    support
        .map(|support| {
            support
                .supported_dialects
                .iter()
                .map(|dialect| dialect.as_str().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn compatibility_shim_support_bridge_labels(
    support: Option<&kernel::PluginCompatibilityShimSupport>,
) -> Vec<String> {
    support
        .map(|support| {
            support
                .supported_bridges
                .iter()
                .map(|bridge| bridge.as_str().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn runtime_plugin_setup_readiness_context(
    config: &mvp::config::LoongConfig,
) -> PluginSetupReadinessContext {
    let verified_env_vars = std::env::vars_os()
        .filter_map(|(key, value)| {
            let value_string = value.to_string_lossy();
            let trimmed_value = value_string.trim();
            if trimmed_value.is_empty() {
                return None;
            }

            Some(key.to_string_lossy().to_string())
        })
        .collect();
    let mut verified_config_keys = BTreeSet::new();
    if let Ok(value) = serde_json::to_value(config) {
        collect_config_paths(&value, None, &mut verified_config_keys);
    }

    PluginSetupReadinessContext {
        verified_env_vars,
        verified_config_keys,
    }
}

fn collect_config_paths(value: &Value, prefix: Option<&str>, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next_prefix = match prefix {
                    Some(prefix) => format!("{prefix}.{key}"),
                    None => key.clone(),
                };

                match child {
                    Value::Null => {}
                    Value::Object(_)
                    | Value::Array(_)
                    | Value::Bool(_)
                    | Value::Number(_)
                    | Value::String(_) => {
                        out.insert(next_prefix.clone());
                        collect_config_paths(child, Some(next_prefix.as_str()), out);
                    }
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_config_paths(child, prefix, out);
            }
        }
        Value::Null => {}
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            if let Some(prefix) = prefix {
                out.insert(prefix.to_owned());
            }
        }
    }
}

fn runtime_snapshot_effective_external_skills_policy(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Result<
    (
        mvp::tools::runtime_config::ExternalSkillsRuntimePolicy,
        bool,
    ),
    String,
> {
    let outcome = mvp::tools::execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.policy".to_owned(),
            payload: json!({
                "action": "get",
            }),
        },
        tool_runtime,
    )
    .map_err(|error| format!("resolve effective external skills policy failed: {error}"))?;

    let policy = runtime_snapshot_external_skills_policy_from_payload(&outcome.payload)?;
    let override_active = outcome
        .payload
        .get("override_active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((policy, override_active))
}

fn runtime_snapshot_external_skills_policy_from_payload(
    payload: &Value,
) -> Result<mvp::tools::runtime_config::ExternalSkillsRuntimePolicy, String> {
    let policy = payload
        .get("policy")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "runtime snapshot external skills policy payload missing `policy`".to_owned()
        })?;

    Ok(mvp::tools::runtime_config::ExternalSkillsRuntimePolicy {
        enabled: policy
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "runtime snapshot external skills policy missing `enabled`".to_owned()
            })?,
        require_download_approval: policy
            .get("require_download_approval")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "runtime snapshot external skills policy missing `require_download_approval`"
                    .to_owned()
            })?,
        allowed_domains: json_string_array_to_set(
            policy.get("allowed_domains"),
            "runtime snapshot external skills policy.allowed_domains",
        )?,
        blocked_domains: json_string_array_to_set(
            policy.get("blocked_domains"),
            "runtime snapshot external skills policy.blocked_domains",
        )?,
        install_root: policy
            .get("install_root")
            .and_then(Value::as_str)
            .map(Path::new)
            .map(Path::to_path_buf),
        auto_expose_installed: policy
            .get("auto_expose_installed")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "runtime snapshot external skills policy missing `auto_expose_installed`".to_owned()
            })?,
    })
}

fn runtime_snapshot_tool_digest(
    visible_tool_names: &[String],
    capability_snapshot: &str,
) -> CliResult<String> {
    let serialized = serde_json::to_vec(&json!({
        "visible_tool_names": visible_tool_names,
        "capability_snapshot": capability_snapshot,
    }))
    .map_err(|error| format!("serialize runtime snapshot tool digest input failed: {error}"))?;
    Ok(hex::encode(Sha256::digest(serialized)))
}

fn json_array_len(value: Option<&Value>) -> usize {
    value.and_then(Value::as_array).map_or(0, Vec::len)
}

fn runtime_plugin_activation_status(status: PluginActivationStatus) -> &'static str {
    status.as_str()
}

fn json_string_array_to_set(
    value: Option<&Value>,
    context: &str,
) -> Result<BTreeSet<String>, String> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context} must be an array"))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{context} must contain only strings"))
        })
        .collect()
}

fn build_runtime_snapshot_restore_spec(
    config: &mvp::config::LoongConfig,
    external_skills: &RuntimeSnapshotExternalSkillsState,
) -> RuntimeSnapshotRestoreSpec {
    let mut warnings = Vec::new();
    let mut profiles = runtime_snapshot_restore_provider_profiles(config);
    for (profile_id, profile) in &mut profiles {
        normalize_runtime_snapshot_restore_provider_profile(profile_id, profile, &mut warnings);
    }

    RuntimeSnapshotRestoreSpec {
        provider: RuntimeSnapshotRestoreProviderSpec {
            active_provider: config.active_provider_id().map(str::to_owned),
            last_provider: config.last_provider_id().map(str::to_owned),
            profiles,
        },
        conversation: config.conversation.clone(),
        memory: config.memory.clone(),
        acp: config.acp.clone(),
        tools: config.tools.clone(),
        external_skills: config.external_skills.clone(),
        runtime_plugins: config.runtime_plugins.clone(),
        managed_skills: build_runtime_snapshot_restore_managed_skills_spec(
            external_skills,
            &mut warnings,
        ),
        warnings,
    }
}

fn runtime_snapshot_restore_provider_profiles(
    config: &mvp::config::LoongConfig,
) -> BTreeMap<String, mvp::config::ProviderProfileConfig> {
    if !config.providers.is_empty() {
        return config.providers.clone();
    }

    let profile_id = config
        .active_provider_id()
        .unwrap_or(config.provider.kind.profile().id)
        .to_owned();
    BTreeMap::from([(
        profile_id,
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: config.provider.clone(),
        },
    )])
}

fn normalize_runtime_snapshot_restore_provider_profile(
    profile_id: &str,
    profile: &mut mvp::config::ProviderProfileConfig,
    warnings: &mut Vec<String>,
) {
    runtime_snapshot_migrate_provider_env_reference(
        &mut profile.provider.api_key,
        &mut profile.provider.api_key_env,
    );
    runtime_snapshot_migrate_provider_env_reference(
        &mut profile.provider.oauth_access_token,
        &mut profile.provider.oauth_access_token_env,
    );

    if runtime_snapshot_redact_provider_secret_field(
        profile.provider.api_key.as_mut(),
        profile_id,
        "api_key",
        warnings,
    ) {
        profile.provider.api_key = None;
    }
    if runtime_snapshot_redact_provider_secret_field(
        profile.provider.oauth_access_token.as_mut(),
        profile_id,
        "oauth_access_token",
        warnings,
    ) {
        profile.provider.oauth_access_token = None;
    }

    let header_keys_to_remove = profile
        .provider
        .headers
        .iter()
        .filter(|(header_name, header_value)| {
            !runtime_snapshot_provider_header_is_safe_to_persist(
                profile.provider.kind,
                header_name,
                header_value,
            )
        })
        .map(|(header_name, _)| header_name.clone())
        .collect::<Vec<_>>();
    for header_name in header_keys_to_remove {
        profile.provider.headers.remove(&header_name);
        warnings.push(format!(
            "restore spec redacted inline provider header `{header_name}` for profile `{profile_id}`"
        ));
    }
}

fn runtime_snapshot_redact_provider_secret_field(
    raw: Option<&mut SecretRef>,
    profile_id: &str,
    field_name: &str,
    warnings: &mut Vec<String>,
) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    if raw.inline_literal_value().is_none() {
        return false;
    }
    warnings.push(format!(
        "restore spec redacted inline provider credential `{field_name}` for profile `{profile_id}`"
    ));
    true
}

fn runtime_snapshot_provider_header_is_safe_to_persist(
    provider_kind: mvp::config::ProviderKind,
    header_name: &str,
    header_value: &str,
) -> bool {
    if header_value.trim().is_empty() || runtime_snapshot_is_env_reference_literal(header_value) {
        return true;
    }

    let normalized = header_name.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "accept"
            | "accept-charset"
            | "accept-encoding"
            | "accept-language"
            | "anthropic-version"
            | "cache-control"
            | "content-language"
            | "content-type"
            | "pragma"
            | "user-agent"
            | "anthropic-beta"
            | "openai-beta"
    ) || provider_kind
        .default_headers()
        .iter()
        .any(|(default_name, _)| default_name.eq_ignore_ascii_case(&normalized))
}

fn runtime_snapshot_migrate_provider_env_reference(
    inline_secret: &mut Option<SecretRef>,
    env_name: &mut Option<String>,
) {
    let explicit_env_name = inline_secret
        .as_ref()
        .and_then(SecretRef::explicit_env_name);
    if let Some(explicit_env_name) = explicit_env_name {
        *inline_secret = Some(SecretRef::Env {
            env: explicit_env_name,
        });
        *env_name = None;
        return;
    }

    if inline_secret.as_ref().is_some_and(SecretRef::is_configured) {
        *env_name = None;
        return;
    }

    let configured_env_name = env_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(configured_env_name) = configured_env_name {
        *inline_secret = Some(SecretRef::Env {
            env: configured_env_name,
        });
    }
    *env_name = None;
}

fn runtime_snapshot_is_env_reference_literal(raw: &str) -> bool {
    runtime_snapshot_parse_env_reference(raw).is_some()
}

fn runtime_snapshot_parse_env_reference(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(inner) = trimmed
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
    {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    if let Some(inner) = trimmed.strip_prefix('$') {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    if let Some(inner) = trimmed.strip_prefix("env:") {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    if let Some(inner) = trimmed
        .strip_prefix('%')
        .and_then(|value| value.strip_suffix('%'))
    {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    None
}

fn runtime_snapshot_is_valid_env_name(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn build_runtime_snapshot_restore_managed_skills_spec(
    external_skills: &RuntimeSnapshotExternalSkillsState,
    warnings: &mut Vec<String>,
) -> RuntimeSnapshotRestoreManagedSkillsSpec {
    match external_skills.inventory_status {
        RuntimeSnapshotInventoryStatus::Disabled => {
            warnings.push(
                "restore spec could not enumerate managed external skills because runtime inventory is disabled"
                    .to_owned(),
            );
            return RuntimeSnapshotRestoreManagedSkillsSpec::default();
        }
        RuntimeSnapshotInventoryStatus::Error => {
            warnings.push(
                "restore spec could not enumerate managed external skills because runtime inventory collection failed"
                    .to_owned(),
            );
            return RuntimeSnapshotRestoreManagedSkillsSpec::default();
        }
        RuntimeSnapshotInventoryStatus::Ok => {}
    }

    let Some(skills) = external_skills
        .inventory
        .get("skills")
        .and_then(Value::as_array)
    else {
        return RuntimeSnapshotRestoreManagedSkillsSpec::default();
    };

    let mut managed_skills = skills
        .iter()
        .filter(|skill| skill.get("scope").and_then(Value::as_str) == Some("managed"))
        .filter_map(|skill| {
            let skill_id = skill.get("skill_id").and_then(Value::as_str)?;
            let display_name = skill
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let summary = skill
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let source_kind = skill.get("source_kind").and_then(Value::as_str)?;
            let source_path = skill.get("source_path").and_then(Value::as_str)?;
            let sha256 = skill.get("sha256").and_then(Value::as_str)?;
            Some(RuntimeSnapshotRestoreManagedSkillSpec {
                skill_id: skill_id.to_owned(),
                display_name: display_name.to_owned(),
                summary: summary.to_owned(),
                source_kind: source_kind.to_owned(),
                source_path: source_path.to_owned(),
                sha256: sha256.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    managed_skills.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    RuntimeSnapshotRestoreManagedSkillsSpec {
        skills: managed_skills,
    }
}

#[cfg(test)]
#[path = "lib_runtime_snapshot_restore_spec_tests.rs"]
mod runtime_snapshot_restore_spec_tests;

fn runtime_snapshot_artifact_metadata_now(
    label: Option<&str>,
    experiment_id: Option<&str>,
    parent_snapshot_id: Option<&str>,
) -> CliResult<RuntimeSnapshotArtifactMetadata> {
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("format runtime snapshot artifact timestamp failed: {error}"))?;
    Ok(RuntimeSnapshotArtifactMetadata {
        created_at,
        label: runtime_snapshot_optional_arg(label),
        experiment_id: runtime_snapshot_optional_arg(experiment_id),
        parent_snapshot_id: runtime_snapshot_optional_arg(parent_snapshot_id),
    })
}

fn runtime_snapshot_optional_arg(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn persist_json_artifact(
    output_path: &str,
    payload: &Value,
    artifact_label: &str,
) -> CliResult<()> {
    let output_path = PathBuf::from(output_path);
    let parent_path = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&parent_path).map_err(|error| {
        format!(
            "create {artifact_label} directory {} failed: {error}",
            parent_path.display()
        )
    })?;
    let encoded = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("serialize {artifact_label} failed: {error}"))?;
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    let process_id = process::id();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("build {artifact_label} temp path failed: {error}"))?
        .as_nanos();
    let temp_file_name = format!(".{file_name}.{process_id}.{timestamp}.tmp");
    let temp_path = parent_path.join(temp_file_name);

    let open_result = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path);
    let mut temp_file = open_result.map_err(|error| {
        format!(
            "create {artifact_label} temp file {} failed: {error}",
            temp_path.display()
        )
    })?;
    temp_file.write_all(encoded.as_bytes()).map_err(|error| {
        format!(
            "write {artifact_label} temp file {} failed: {error}",
            temp_path.display()
        )
    })?;
    temp_file.sync_all().map_err(|error| {
        format!(
            "sync {artifact_label} temp file {} failed: {error}",
            temp_path.display()
        )
    })?;
    drop(temp_file);

    let rename_result = fs::rename(&temp_path, &output_path);
    if let Err(error) = rename_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "replace {artifact_label} {} failed: {error}",
            output_path.display()
        ));
    }
    Ok(())
}

pub fn build_runtime_snapshot_artifact_json_payload(
    snapshot: &RuntimeSnapshotCliState,
    metadata: &RuntimeSnapshotArtifactMetadata,
) -> CliResult<Value> {
    build_runtime_snapshot_artifact_json_payload_with_inventory(snapshot, metadata, None)
}

pub fn build_runtime_snapshot_artifact_json_payload_with_inventory(
    snapshot: &RuntimeSnapshotCliState,
    metadata: &RuntimeSnapshotArtifactMetadata,
    runtime_plugin_inventory: Option<crate::plugins_cli::RuntimePluginInventoryReadModel>,
) -> CliResult<Value> {
    let base_payload = cli_json::build_runtime_snapshot_cli_json_payload(snapshot)?;
    let lineage = runtime_snapshot_artifact_lineage(snapshot, metadata)?;
    let document = RuntimeSnapshotArtifactDocument {
        config: snapshot.config.clone(),
        schema: RuntimeSnapshotArtifactSchema {
            version: RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: "runtime_snapshot".to_owned(),
            purpose: "experiment_reproducibility".to_owned(),
        },
        lineage,
        provider: base_payload.get("provider").cloned().unwrap_or(Value::Null),
        context_engine: base_payload
            .get("context_engine")
            .cloned()
            .unwrap_or(Value::Null),
        memory_system: base_payload
            .get("memory_system")
            .cloned()
            .unwrap_or(Value::Null),
        acp: base_payload.get("acp").cloned().unwrap_or(Value::Null),
        channels: base_payload.get("channels").cloned().unwrap_or(Value::Null),
        tool_runtime: base_payload
            .get("tool_runtime")
            .cloned()
            .unwrap_or(Value::Null),
        tools: base_payload.get("tools").cloned().unwrap_or(Value::Null),
        runtime_plugins: base_payload
            .get("runtime_plugins")
            .cloned()
            .unwrap_or(Value::Null),
        runtime_plugin_inventory,
        external_skills: base_payload
            .get("external_skills")
            .cloned()
            .unwrap_or(Value::Null),
        restore_spec: snapshot.restore_spec.clone(),
    };
    serde_json::to_value(document)
        .map_err(|error| format!("serialize runtime snapshot artifact payload failed: {error}"))
}

fn runtime_snapshot_artifact_lineage(
    snapshot: &RuntimeSnapshotCliState,
    metadata: &RuntimeSnapshotArtifactMetadata,
) -> CliResult<RuntimeSnapshotArtifactLineage> {
    let serialized = serde_json::to_vec(&json!({
        "config": snapshot.config,
        "created_at": metadata.created_at,
        "label": metadata.label,
        "experiment_id": metadata.experiment_id,
        "parent_snapshot_id": metadata.parent_snapshot_id,
        "capability_snapshot_sha256": snapshot.capability_snapshot_sha256,
        "active_provider": snapshot.provider.active_profile_id,
    }))
    .map_err(|error| format!("serialize runtime snapshot lineage input failed: {error}"))?;
    Ok(RuntimeSnapshotArtifactLineage {
        snapshot_id: hex::encode(Sha256::digest(serialized)),
        created_at: metadata.created_at.clone(),
        label: metadata.label.clone(),
        experiment_id: metadata.experiment_id.clone(),
        parent_snapshot_id: metadata.parent_snapshot_id.clone(),
    })
}

fn render_runtime_snapshot_artifact_text(
    snapshot: &RuntimeSnapshotCliState,
    artifact_payload: &Value,
) -> String {
    let lineage = artifact_payload
        .get("lineage")
        .cloned()
        .unwrap_or(Value::Null);
    let schema_version = artifact_payload
        .get("schema")
        .and_then(|schema| schema.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION));

    [
        format!("schema.version={schema_version}"),
        format!("snapshot_id={}", json_string_field(&lineage, "snapshot_id")),
        format!("created_at={}", json_string_field(&lineage, "created_at")),
        format!("label={}", json_string_field(&lineage, "label")),
        format!(
            "experiment_id={}",
            json_string_field(&lineage, "experiment_id")
        ),
        format!(
            "parent_snapshot_id={}",
            json_string_field(&lineage, "parent_snapshot_id")
        ),
        format!("restore_warnings={}", snapshot.restore_spec.warnings.len()),
        render_runtime_snapshot_text(snapshot),
    ]
    .join("\n")
}

pub async fn with_graceful_shutdown<F>(serve_future: F) -> CliResult<()>
where
    F: std::future::Future<Output = CliResult<()>>,
{
    tokio::select! {
        result = serve_future => result,
        result = wait_for_shutdown_reason() => result.map(|_| ()),
    }
}

#[cfg(unix)]
pub async fn wait_for_shutdown_reason() -> CliResult<String> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|error| format!("failed to register SIGTERM handler: {error}"))?;

    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.map_err(|error| format!("failed to register Ctrl-C handler: {error}"))?;
            eprintln!("\nReceived Ctrl-C, shutting down gracefully...");
            Ok("ctrl-c received".to_owned())
        }
        _ = sigterm.recv() => {
            eprintln!("\nReceived SIGTERM, shutting down gracefully...");
            Ok("sigterm received".to_owned())
        }
    }
}

#[cfg(not(unix))]
pub async fn wait_for_shutdown_reason() -> CliResult<String> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("failed to register Ctrl-C handler: {error}"))?;
    eprintln!("\nReceived Ctrl-C, shutting down gracefully...");
    Ok("ctrl-c received".to_owned())
}

pub async fn wait_for_shutdown_signal() -> CliResult<()> {
    wait_for_shutdown_reason().await.map(|_| ())
}

pub async fn run_channel_send_cli(
    spec: ChannelSendCliSpec,
    args: ChannelSendCliArgs<'_>,
) -> CliResult<()> {
    let _ = spec.family;
    (spec.run)(args).await
}

pub async fn run_channel_serve_cli(
    spec: ChannelServeCliSpec,
    args: ChannelServeCliArgs<'_>,
) -> CliResult<()> {
    let _ = spec.family;
    (spec.run)(args).await
}

fn require_channel_send_target<'a>(command: &str, target: Option<&'a str>) -> CliResult<&'a str> {
    let target = target.map(str::trim).filter(|value| !value.is_empty());
    let Some(target) = target else {
        return Err(format!("{command} requires --target"));
    };

    Ok(target)
}

pub fn run_telegram_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = args.target.unwrap_or_default();
        mvp::channel::run_telegram_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_feishu_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let target = args.target.unwrap_or_default();
        mvp::channel::run_feishu_send(
            args.config_path,
            args.account,
            &mvp::channel::FeishuChannelSendRequest {
                receive_id: target.to_owned(),
                receive_id_type: Some(args.target_kind.as_str().to_owned()),
                text: Some(args.text.to_owned()),
                post_json: None,
                image_key: None,
                file_key: None,
                image_path: None,
                file_path: None,
                file_type: None,
                card: args.as_card,
                uuid: None,
            },
        )
        .await
    })
}

pub fn run_matrix_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = args.target.unwrap_or_default();
        mvp::channel::run_matrix_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_wecom_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = args.target.unwrap_or_default();
        mvp::channel::run_wecom_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_discord_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("discord-send", args.target)?;
        mvp::channel::run_discord_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_dingtalk_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        mvp::channel::run_dingtalk_send(
            args.config_path,
            args.account,
            args.target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_slack_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = args.target.unwrap_or_default();
        mvp::channel::run_slack_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_line_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = args.target.unwrap_or_default();
        mvp::channel::run_line_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_whatsapp_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = args.target.unwrap_or_default();
        mvp::channel::run_whatsapp_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_email_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("email-send", args.target)?;
        mvp::channel::run_email_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_webhook_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        mvp::channel::run_webhook_send(
            args.config_path,
            args.account,
            args.target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_google_chat_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        mvp::channel::run_google_chat_send(
            args.config_path,
            args.account,
            args.target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_teams_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        mvp::channel::run_teams_send(
            args.config_path,
            args.account,
            args.target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_mattermost_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("mattermost-send", args.target)?;
        mvp::channel::run_mattermost_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_nextcloud_talk_send_cli_impl(
    args: ChannelSendCliArgs<'_>,
) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("nextcloud-talk-send", args.target)?;
        mvp::channel::run_nextcloud_talk_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_synology_chat_send_cli_impl(
    args: ChannelSendCliArgs<'_>,
) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        mvp::channel::run_synology_chat_send(
            args.config_path,
            args.account,
            args.target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_irc_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("irc-send", args.target)?;
        mvp::channel::run_irc_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_imessage_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("imessage-send", args.target)?;
        mvp::channel::run_imessage_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_nostr_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        mvp::channel::run_nostr_send(
            args.config_path,
            args.account,
            args.target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_signal_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = args.target.unwrap_or_default();
        mvp::channel::run_signal_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_twitch_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("twitch-send", args.target)?;
        mvp::channel::run_twitch_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_telegram_serve_cli_impl(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = (args.bind_override, args.path_override);
        with_graceful_shutdown(mvp::channel::run_telegram_channel(
            args.config_path,
            args.once,
            args.account,
        ))
        .await
    })
}

pub fn default_channel_send_target_kind(
    spec: ChannelSendCliSpec,
) -> mvp::channel::ChannelOutboundTargetKind {
    spec.family.default_send_target_kind
}

pub fn parse_channel_send_target_kind(
    spec: ChannelSendCliSpec,
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    let target_kind = raw.parse::<mvp::channel::ChannelOutboundTargetKind>()?;
    let channel_id = spec.family.channel_id;
    let operation = spec.family.send;
    if !operation.supports_target_kind(target_kind) {
        let supported = operation
            .supported_target_kinds
            .iter()
            .map(|kind| format!("`{}`", kind.as_str()))
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(format!(
            "{channel_id} --target-kind does not support `{}`; use {}",
            target_kind.as_str(),
            supported
        ));
    }
    Ok(target_kind)
}

pub fn default_telegram_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(TELEGRAM_SEND_CLI_SPEC)
}

pub fn parse_telegram_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(TELEGRAM_SEND_CLI_SPEC, raw)
}

pub fn default_matrix_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(MATRIX_SEND_CLI_SPEC)
}

pub fn parse_matrix_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(MATRIX_SEND_CLI_SPEC, raw)
}

pub fn default_wecom_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(WECOM_SEND_CLI_SPEC)
}

pub fn parse_wecom_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(WECOM_SEND_CLI_SPEC, raw)
}

pub fn default_feishu_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(FEISHU_SEND_CLI_SPEC)
}

pub fn parse_feishu_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(FEISHU_SEND_CLI_SPEC, raw)
}

pub fn default_discord_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(DISCORD_SEND_CLI_SPEC)
}

pub fn parse_discord_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(DISCORD_SEND_CLI_SPEC, raw)
}

pub fn default_dingtalk_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(DINGTALK_SEND_CLI_SPEC)
}

pub fn parse_dingtalk_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(DINGTALK_SEND_CLI_SPEC, raw)
}

pub fn default_slack_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(SLACK_SEND_CLI_SPEC)
}

pub fn parse_slack_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(SLACK_SEND_CLI_SPEC, raw)
}

pub fn default_line_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(LINE_SEND_CLI_SPEC)
}

pub fn parse_line_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(LINE_SEND_CLI_SPEC, raw)
}

pub fn default_whatsapp_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(WHATSAPP_SEND_CLI_SPEC)
}

pub fn parse_whatsapp_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(WHATSAPP_SEND_CLI_SPEC, raw)
}

pub fn default_email_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(EMAIL_SEND_CLI_SPEC)
}

pub fn parse_email_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(EMAIL_SEND_CLI_SPEC, raw)
}

pub fn default_webhook_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(WEBHOOK_SEND_CLI_SPEC)
}

pub fn parse_webhook_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(WEBHOOK_SEND_CLI_SPEC, raw)
}

pub fn default_google_chat_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(GOOGLE_CHAT_SEND_CLI_SPEC)
}

pub fn parse_google_chat_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(GOOGLE_CHAT_SEND_CLI_SPEC, raw)
}

pub fn default_teams_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(TEAMS_SEND_CLI_SPEC)
}

pub fn parse_teams_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(TEAMS_SEND_CLI_SPEC, raw)
}

pub fn default_signal_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(SIGNAL_SEND_CLI_SPEC)
}

pub fn parse_signal_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(SIGNAL_SEND_CLI_SPEC, raw)
}

pub fn default_mattermost_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(MATTERMOST_SEND_CLI_SPEC)
}

pub fn parse_mattermost_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(MATTERMOST_SEND_CLI_SPEC, raw)
}

pub fn default_nextcloud_talk_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(NEXTCLOUD_TALK_SEND_CLI_SPEC)
}

pub fn parse_nextcloud_talk_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(NEXTCLOUD_TALK_SEND_CLI_SPEC, raw)
}

pub fn default_synology_chat_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(SYNOLOGY_CHAT_SEND_CLI_SPEC)
}

pub fn parse_synology_chat_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(SYNOLOGY_CHAT_SEND_CLI_SPEC, raw)
}

pub fn default_irc_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(IRC_SEND_CLI_SPEC)
}

pub fn parse_irc_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(IRC_SEND_CLI_SPEC, raw)
}

pub fn default_imessage_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(IMESSAGE_SEND_CLI_SPEC)
}

pub fn parse_imessage_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(IMESSAGE_SEND_CLI_SPEC, raw)
}

pub fn default_nostr_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(NOSTR_SEND_CLI_SPEC)
}

pub fn parse_nostr_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(NOSTR_SEND_CLI_SPEC, raw)
}

pub fn run_matrix_serve_cli_impl(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = (args.bind_override, args.path_override);
        with_graceful_shutdown(mvp::channel::run_matrix_channel(
            args.config_path,
            args.once,
            args.account,
        ))
        .await
    })
}

pub fn run_wecom_serve_cli_impl(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        // WeCom AIBot uses a long connection only. `args.once`,
        // `args.bind_override`, and `args.path_override` are intentionally
        // discarded because single-run mode and HTTP bind/path overrides do not
        // apply to this transport.
        let _ = (args.once, args.bind_override, args.path_override);
        with_graceful_shutdown(mvp::channel::run_wecom_channel(
            args.config_path,
            args.account,
        ))
        .await
    })
}

pub async fn run_multi_channel_serve_cli(
    config_path: Option<&str>,
    session: &str,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
) -> CliResult<()> {
    gateway::service::run_multi_channel_serve_gateway_compat_cli(
        config_path,
        session,
        channel_accounts,
    )
    .await
}

pub(crate) fn render_string_list<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let rendered = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        "-".to_owned()
    } else {
        rendered.join(",")
    }
}

fn json_string_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("-")
}

pub fn context_engine_metadata_json(
    metadata: &mvp::conversation::ContextEngineMetadata,
    source: Option<&str>,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("id".to_owned(), json!(metadata.id));
    payload.insert("api_version".to_owned(), json!(metadata.api_version));
    payload.insert(
        "capabilities".to_owned(),
        json!(metadata.capability_names()),
    );
    if let Some(source) = source {
        payload.insert("source".to_owned(), json!(source));
    }
    Value::Object(payload)
}

pub fn memory_system_metadata_json(
    metadata: &mvp::memory::MemorySystemMetadata,
    source: Option<&str>,
) -> Value {
    let supported_stage_families = metadata
        .supported_stage_families
        .iter()
        .copied()
        .map(mvp::memory::MemoryStageFamily::as_str)
        .collect::<Vec<_>>();
    let supported_pre_assembly_stage_families = metadata
        .supported_pre_assembly_stage_families
        .iter()
        .copied()
        .map(mvp::memory::MemoryStageFamily::as_str)
        .collect::<Vec<_>>();
    let supported_recall_modes = metadata
        .supported_recall_modes
        .iter()
        .copied()
        .map(mvp::memory::MemoryRecallMode::as_str)
        .collect::<Vec<_>>();
    let mut payload = serde_json::Map::new();
    payload.insert("id".to_owned(), json!(metadata.id));
    payload.insert("api_version".to_owned(), json!(metadata.api_version));
    payload.insert(
        "capabilities".to_owned(),
        json!(metadata.capability_names()),
    );
    payload.insert(
        "runtime_fallback_kind".to_owned(),
        json!(metadata.runtime_fallback_kind.as_str()),
    );
    payload.insert(
        "supported_stage_families".to_owned(),
        json!(supported_stage_families),
    );
    payload.insert(
        "supported_pre_assembly_stage_families".to_owned(),
        json!(supported_pre_assembly_stage_families),
    );
    payload.insert(
        "supported_recall_modes".to_owned(),
        json!(supported_recall_modes),
    );
    payload.insert("summary".to_owned(), json!(metadata.summary));
    if let Some(source) = source {
        payload.insert("source".to_owned(), json!(source));
    }
    Value::Object(payload)
}

fn format_memory_stage_family_names(families: &[mvp::memory::MemoryStageFamily]) -> String {
    let names = families
        .iter()
        .copied()
        .map(mvp::memory::MemoryStageFamily::as_str)
        .collect::<Vec<_>>();
    render_string_list(names)
}

fn format_memory_recall_mode_names(recall_modes: &[mvp::memory::MemoryRecallMode]) -> String {
    let names = recall_modes
        .iter()
        .copied()
        .map(mvp::memory::MemoryRecallMode::as_str)
        .collect::<Vec<_>>();
    render_string_list(names)
}

fn format_memory_core_operation_names(operations: &[mvp::memory::MemoryCoreOperation]) -> String {
    let names = operations
        .iter()
        .copied()
        .map(mvp::memory::MemoryCoreOperation::as_str)
        .collect::<Vec<_>>();
    render_string_list(names)
}

pub fn memory_system_policy_json(policy: &mvp::memory::MemorySystemPolicySnapshot) -> Value {
    json!({
        "backend": policy.backend.as_str(),
        "profile": policy.profile.as_str(),
        "mode": policy.mode.as_str(),
        "ingest_mode": policy.ingest_mode.as_str(),
        "fail_open": policy.fail_open,
        "strict_mode_requested": policy.strict_mode_requested,
        "strict_mode_active": policy.strict_mode_active,
        "effective_fail_open": policy.effective_fail_open,
    })
}

pub fn build_memory_systems_cli_json_payload(
    config_path: &str,
    snapshot: &mvp::memory::MemorySystemRuntimeSnapshot,
) -> Value {
    json!({
        "config": config_path,
        "selected": memory_system_metadata_json(
            &snapshot.selected_metadata,
            Some(snapshot.selected.source.as_str())
        ),
        "available": snapshot
            .available
            .iter()
            .map(|metadata| memory_system_metadata_json(metadata, None))
            .collect::<Vec<_>>(),
        "core_operations": snapshot
            .core_operations
            .iter()
            .copied()
            .map(mvp::memory::MemoryCoreOperation::as_str)
            .collect::<Vec<_>>(),
        "policy": memory_system_policy_json(&snapshot.policy),
    })
}

pub fn render_memory_system_snapshot_text(
    config_path: &str,
    snapshot: &mvp::memory::MemorySystemRuntimeSnapshot,
) -> String {
    let selected_capabilities = snapshot.selected_metadata.capability_names();
    let selected_stage_families =
        format_memory_stage_family_names(&snapshot.selected_metadata.supported_stage_families);
    let selected_pre_assembly_stages = format_memory_stage_family_names(
        &snapshot
            .selected_metadata
            .supported_pre_assembly_stage_families,
    );
    let selected_recall_modes =
        format_memory_recall_mode_names(&snapshot.selected_metadata.supported_recall_modes);
    let core_operations = format_memory_core_operation_names(&snapshot.core_operations);
    let mut lines = vec![
        format!("config={config_path}"),
        format!(
            "selected={} source={} api_version={} capabilities={} runtime_fallback_kind={} stages={} pre_assembly_stages={} recall_modes={} core_operations={} summary={}",
            snapshot.selected_metadata.id,
            snapshot.selected.source.as_str(),
            snapshot.selected_metadata.api_version,
            format_capability_names(&selected_capabilities),
            snapshot.selected_metadata.runtime_fallback_kind.as_str(),
            selected_stage_families,
            selected_pre_assembly_stages,
            selected_recall_modes,
            core_operations,
            snapshot.selected_metadata.summary
        ),
        format!(
            "policy=backend:{} profile:{} mode:{} ingest_mode:{} fail_open:{} strict_mode_requested:{} strict_mode_active:{} effective_fail_open:{}",
            snapshot.policy.backend.as_str(),
            snapshot.policy.profile.as_str(),
            snapshot.policy.mode.as_str(),
            snapshot.policy.ingest_mode.as_str(),
            snapshot.policy.fail_open,
            snapshot.policy.strict_mode_requested,
            snapshot.policy.strict_mode_active,
            snapshot.policy.effective_fail_open,
        ),
        "available:".to_owned(),
    ];

    for metadata in &snapshot.available {
        let capabilities = metadata.capability_names();
        let stage_families = format_memory_stage_family_names(&metadata.supported_stage_families);
        let pre_assembly_stages =
            format_memory_stage_family_names(&metadata.supported_pre_assembly_stage_families);
        let recall_modes = format_memory_recall_mode_names(&metadata.supported_recall_modes);
        lines.push(format!(
            "- {} api_version={} capabilities={} runtime_fallback_kind={} stages={} pre_assembly_stages={} recall_modes={} summary={}",
            metadata.id,
            metadata.api_version,
            format_capability_names(&capabilities),
            metadata.runtime_fallback_kind.as_str(),
            stage_families,
            pre_assembly_stages,
            recall_modes,
            metadata.summary
        ));
    }

    lines.join("\n")
}

pub fn format_u32_rollup(values: &BTreeMap<String, u32>) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn format_usize_rollup(values: &BTreeMap<String, usize>) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn read_spec_file(path: &str) -> CliResult<RunnerSpec> {
    read_spec_file_with_bridge_support_resolution(path, None).map(|resolved| resolved.spec)
}

pub fn read_spec_file_with_bridge_support_selection(
    path: &str,
    bridge_support_selection_override: Option<&BridgeSupportSelectionInput>,
) -> CliResult<RunnerSpec> {
    read_spec_file_with_bridge_support_resolution(path, bridge_support_selection_override)
        .map(|resolved| resolved.spec)
}

pub fn read_spec_file_with_bridge_support_resolution(
    path: &str,
    bridge_support_selection_override: Option<&BridgeSupportSelectionInput>,
) -> CliResult<ResolvedRunnerSpecFile> {
    let mut input = read_spec_file_input(path)?;
    let spec_has_bridge_support_config =
        input.spec.bridge_support.is_some() || input.bridge_support_selection.is_some();

    if let Some(selection) = bridge_support_selection_override {
        if spec_has_bridge_support_config {
            return Err(format!(
                "spec file {path} accepts either file-local bridge support configuration or CLI bridge support selection overrides, not both"
            ));
        }
        let override_selection = resolve_process_relative_bridge_support_selection(selection)?;
        input.bridge_support_selection = Some(override_selection);
    }

    resolve_spec_file_input(path, input)
}

fn resolve_process_relative_bridge_support_selection(
    selection: &BridgeSupportSelectionInput,
) -> CliResult<BridgeSupportSelectionInput> {
    let path = selection
        .path
        .as_deref()
        .map(resolve_process_relative_path)
        .transpose()?;
    let delta_artifact = selection
        .delta_artifact
        .as_deref()
        .map(resolve_process_relative_path)
        .transpose()?;

    Ok(BridgeSupportSelectionInput {
        path,
        bundled_profile: selection.bundled_profile.clone(),
        delta_artifact,
        expected_sha256: selection.expected_sha256.clone(),
        expected_delta_sha256: selection.expected_delta_sha256.clone(),
    })
}

fn read_spec_file_input(path: &str) -> CliResult<RunnerSpecFileInput> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read spec file {path}: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("failed to parse spec file {path}: {error}"))
}

fn resolve_spec_file_input(
    path: &str,
    mut input: RunnerSpecFileInput,
) -> CliResult<ResolvedRunnerSpecFile> {
    if let Some(selection) = input.bridge_support_selection.take() {
        if input.spec.bridge_support.is_some() {
            return Err(format!(
                "spec file {path} accepts either inline `bridge_support` or `bridge_support_selection`, not both"
            ));
        }

        let policy_path = selection
            .path
            .as_deref()
            .map(|value| resolve_spec_relative_path(path, value));
        let delta_artifact_path = selection
            .delta_artifact
            .as_deref()
            .map(|value| resolve_spec_relative_path(path, value));
        let resolved = resolve_bridge_support_selection(
            policy_path.as_deref(),
            selection.bundled_profile.as_deref(),
            delta_artifact_path.as_deref(),
            selection.expected_sha256.as_deref(),
            selection.expected_delta_sha256.as_deref(),
        )
        .map_err(|error| {
            format!("failed to resolve bridge support selection in {path}: {error}")
        })?;
        let bridge_support_source = resolved
            .as_ref()
            .map(|selection| selection.policy.source.clone());
        let bridge_support_delta_source = resolved
            .as_ref()
            .and_then(|selection| selection.delta_source.clone());
        let bridge_support_delta_sha256 = resolved.as_ref().and_then(|selection| {
            selection
                .delta_artifact
                .as_ref()
                .map(|artifact| artifact.sha256.clone())
        });
        input.spec.bridge_support = resolved.map(|selection| selection.policy.profile);
        return Ok(ResolvedRunnerSpecFile {
            spec: input.spec,
            bridge_support_source,
            bridge_support_delta_source,
            bridge_support_delta_sha256,
        });
    }

    let bridge_support_source = input
        .spec
        .bridge_support
        .as_ref()
        .map(|_| format!("inline:{path}"));

    Ok(ResolvedRunnerSpecFile {
        spec: input.spec,
        bridge_support_source,
        bridge_support_delta_source: None,
        bridge_support_delta_sha256: None,
    })
}

fn resolve_process_relative_path(value: &str) -> CliResult<String> {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return Ok(value.to_owned());
    }

    let current_dir = std::env::current_dir()
        .map_err(|error| format!("resolve current directory failed: {error}"))?;
    let resolved = current_dir.join(candidate);

    Ok(resolved.display().to_string())
}

fn resolve_spec_relative_path(spec_path: &str, value: &str) -> String {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return value.to_owned();
    }

    Path::new(spec_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(candidate)
        .display()
        .to_string()
}

pub fn write_json_file<T: Serialize>(path: &str, value: &T) -> CliResult<()> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize JSON value for output file failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create output directory failed: {error}"))?;
    }
    fs::write(path, serialized)
        .map_err(|error| format!("write JSON output file failed: {error}"))?;
    Ok(())
}
