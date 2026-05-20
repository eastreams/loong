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
    io::{IsTerminal, Write},
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
    QQBOT_SEND_CLI_SPEC, SIGNAL_SEND_CLI_SPEC, SLACK_SEND_CLI_SPEC, SYNOLOGY_CHAT_SEND_CLI_SPEC,
    TEAMS_SEND_CLI_SPEC, TELEGRAM_SEND_CLI_SPEC, TELEGRAM_SERVE_CLI_SPEC, TWITCH_SEND_CLI_SPEC,
    WEBHOOK_SEND_CLI_SPEC, WECOM_SEND_CLI_SPEC, WECOM_SERVE_CLI_SPEC, WEIXIN_SEND_CLI_SPEC,
    WEIXIN_SERVE_CLI_SPEC, WHATSAPP_PERSONAL_SEND_CLI_SPEC, WHATSAPP_PERSONAL_SERVE_CLI_SPEC,
    WHATSAPP_SEND_CLI_SPEC,
};
pub use self::channel_send_target_kind::{
    default_twitch_send_target_kind, parse_twitch_send_target_kind,
};
pub use self::channels_cli::{ChannelsCommands, run_grouped_channels_cli};
pub use self::cli_json::build_runtime_snapshot_cli_json_payload;
pub use self::delegate_child_cli::run_detached_delegate_child_cli;
pub use self::env_compat::make_env_compatible;
pub use self::managed_plugin_bridge_runtime::{
    default_onebot_send_target_kind, default_qqbot_send_target_kind,
    default_weixin_send_target_kind, parse_onebot_send_target_kind, parse_qqbot_send_target_kind,
    parse_weixin_send_target_kind, run_onebot_send_cli_impl, run_onebot_serve_cli_impl,
    run_qqbot_send_cli_impl, run_weixin_send_cli_impl, run_weixin_serve_cli_impl,
    run_whatsapp_personal_send_cli_impl, run_whatsapp_personal_serve_cli_impl,
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
pub use loong_bench::{
    run_programmatic_pressure_baseline_lint_cli, run_programmatic_pressure_benchmark_cli,
    run_wasm_cache_benchmark_cli,
};
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
pub use memory_context_benchmark::run_memory_context_benchmark_cli;
pub use runtime_cli::{RuntimeCommands, run_runtime_cli};
pub use runtime_snapshot_compaction_hygiene::RuntimeSnapshotCompactionHygieneState;
pub use runtime_trajectory_cli::{format_runtime_trajectory_summary, run_runtime_trajectory_cli};
pub use whatsapp_personal_cli::run_whatsapp_personal_command;
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

mod access_terms;
pub mod acp_cli;
pub mod audit_cli;
mod channel_access_policy_render;
mod channel_bridge_render;
mod channel_cli_specs;
mod channel_resolution;
#[cfg(test)]
mod channel_send_cli_tests;
mod channel_send_target_kind;
mod channel_serve_cli;
pub mod channels_cli;
mod cli_handoff;
mod cli_json;
mod command_kind;
pub mod completions_cli;
mod configured_account_keys;
mod control_plane_server;
mod copilot_onboarding;
pub mod debug_cli;
mod delegate_child_cli;
pub mod doctor_cli;
mod doctor_presentation;
pub mod doctor_security_cli;
mod env_compat;
pub mod feishu_cli;
mod feishu_onboarding;
pub mod feishu_support;
mod first_run_action_presentation;
pub mod gateway;
pub mod import_cli;
mod lib_default_entry;
mod lib_runtime_snapshot_support;
mod lib_spec_io;
mod managed_plugin_bridge_runtime;
mod mcp_cli;
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
mod memory_context_benchmark;
pub mod migrate_cli;
pub mod migration;
pub mod next_actions;
mod observability;
pub mod onboard_cli;
mod onboard_finalize;
mod onboard_import;
mod onboard_preflight;
mod onboard_preflight_presentation;
pub mod onboard_presentation;
mod onboard_types;
mod onboard_web_search;
mod onboarding_model_policy;
mod operator_inventory_cli;
pub mod operator_prompt;
pub mod personalize_cli;
mod personalize_presentation;
mod plugin_bridge_account_summary;
pub mod plugins_cli;
mod provider_credential_policy;
mod provider_credentials_guidance;
mod provider_model_probe_policy;
pub mod provider_presentation;
mod provider_route_diagnostics;
mod query_search_guidance;
mod query_search_surface;
mod runtime_access;
pub mod runtime_capability_cli;
pub mod runtime_cli;
pub mod runtime_experiment_cli;
pub mod runtime_restore_cli;
mod runtime_snapshot_compaction_assessment;
mod runtime_snapshot_compaction_hygiene;
mod runtime_snapshot_compaction_presentation;
mod runtime_snapshot_compaction_sequence;
mod runtime_snapshot_render;
mod runtime_snapshot_types;
pub mod runtime_trajectory_cli;
pub mod session_cli;
mod session_prompt_frame_cli;
mod session_runtime_truth_cli;
pub mod sessions_cli;
mod setup_boundary;
pub mod skills_cli;
mod skills_policy_probe;
pub mod source_presentation;
mod status_access;
pub mod status_cli;
pub mod supervisor;
mod task_execution;
pub mod tasks_cli;
mod tlon_cli;
mod tool_calling_readiness;
pub mod trajectory_cli;
mod turn_cli;
pub mod update_cli;
pub mod weixin_cli;
mod weixin_onboarding;
mod whatsapp_personal_cli;
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
use first_run_action_presentation::{
    build_first_run_action_sections, first_run_group_for_setup_action_kind,
};
pub use gateway::read_models::{ChannelsCliJsonPayload, ChannelsCliJsonSchema};
pub(crate) use lib_runtime_snapshot_support::{
    RUNTIME_TOOL_ACCESS_SEPARATION_NOTE, RuntimeToolAccessSummary,
    collect_runtime_snapshot_cli_state_from_loaded_config,
    collect_runtime_snapshot_runtime_plugins_state, persist_json_artifact,
};
pub use lib_runtime_snapshot_support::{
    RuntimeSnapshotArtifactDocument, RuntimeSnapshotArtifactLineage,
    RuntimeSnapshotArtifactMetadata, RuntimeSnapshotArtifactSchema, RuntimeSnapshotCliState,
    RuntimeSnapshotInventoryStatus, RuntimeSnapshotRestoreManagedSkillSpec,
    RuntimeSnapshotRestoreManagedSkillsSpec, RuntimeSnapshotRestoreProviderSpec,
    RuntimeSnapshotRestoreSpec, RuntimeSnapshotRuntimePluginState,
    RuntimeSnapshotRuntimePluginsState, RuntimeSnapshotSkillsState,
    build_runtime_snapshot_artifact_json_payload, collect_runtime_snapshot_cli_state,
    run_runtime_snapshot_cli,
};
pub use loong_spec::programmatic::{
    acquire_programmatic_circuit_slot, record_programmatic_circuit_outcome,
};
pub use observability::{debug_variant_name, init_tracing, summarize_error};
use personalize_presentation::{PERSONALIZE_COMMAND_ABOUT, PERSONALIZE_COMMAND_LONG_ABOUT};
use runtime_snapshot_compaction_hygiene::collect_runtime_snapshot_compaction_hygiene_state;
pub use runtime_snapshot_render::render_runtime_snapshot_text;
pub(crate) use runtime_snapshot_render::{
    runtime_snapshot_acp_json, runtime_snapshot_context_engine_json,
    runtime_snapshot_memory_system_json, runtime_snapshot_provider_json,
    runtime_snapshot_runtime_plugins_json, runtime_snapshot_skills_json,
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
pub use turn_cli::{TurnCommands, run_ask_cli, run_chat_cli, run_turn_run_cli};
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
    FEISHU_SERVE_CLI_SPEC, LINE_SERVE_CLI_SPEC, QQBOT_SERVE_CLI_SPEC, WEBHOOK_SERVE_CLI_SPEC,
    WHATSAPP_SERVE_CLI_SPEC,
};

pub const PUBLIC_GITHUB_REPO: &str = "loong-ai/loong";
pub const CLI_COMMAND_NAME: &str = mvp::config::CLI_COMMAND_NAME;
pub(crate) use lib_default_entry::resolved_default_entry_config_path;
pub use lib_default_entry::{
    redacted_command_name, render_welcome_banner, resolve_default_entry_command,
    resolve_default_entry_post_onboard_command, run_welcome_cli,
};
#[cfg(test)]
use lib_default_entry::{resolve_welcome_config_path, should_resolve_default_entry_to_chat};
pub use lib_spec_io::{
    read_spec_file, read_spec_file_with_bridge_support_resolution,
    read_spec_file_with_bridge_support_selection, write_json_file,
};

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
        "Show the configured welcome banner and quick commands.\n\nquick commands:\n- {command_name}\n- {command_name} ask --config <path> --message \"...\"\n- {command_name} personalize --config <path>\n- {command_name} doctor --config <path>\n- {command_name} --help\n\nRunning `{command_name}` with no subcommand opens the main TUI. If config is missing, first-run setup stays inside that shell; if your config lives elsewhere, set LOONG_CONFIG_PATH first."
    )
}

fn render_import_long_about(command_name: &str) -> String {
    format!(
        "Power-user import flow for previewing or applying detected migration sources explicitly.\n\nUse this when you want exact CLI control over which source and domains are reused. If you want the guided path, use `{command_name} onboard` instead. When the same source kind resolves to multiple detected configs, rerun with `--source-path <path>` to choose one exact source."
    )
}

fn render_migrate_long_about(command_name: &str) -> String {
    format!(
        "Power-user config import flow for discovering, previewing, or applying external workspace state explicitly.\n\nUse this when you want exact CLI control over import mode selection and output handling for older workspace roots. If you want the guided path, use `{command_name} onboard` instead.\n\nMode quick reference:\n- discover, plan_many, recommend_primary, merge_profiles, map_skills: require `--input`\n- plan: requires `--input`; `--output` is optional preview target\n- apply: requires `--input` and `--output`\n- apply_selected: requires `--input` and `--output`; use `--source-id` to pin one discovered source, and `--apply-skills-plan` to bridge installable local skills into the managed runtime\n- rollback_last_apply: requires `--output`"
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
    pub stop_requested: bool,
    pub stop_duplicates_requested: bool,
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
    about = "Loong assistant and runtime CLI",
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
        long_about = "Show the configured welcome banner and quick commands.\n\nquick commands:\n- loong\n- loong ask --config <path> --message \"...\"\n- loong personalize --config <path>\n- loong doctor --config <path>\n- loong --help\n\nRunning `loong` with no subcommand opens the main TUI. If config is missing, first-run setup stays inside that shell; if your config lives elsewhere, set LOONG_CONFIG_PATH first."
    )]
    /// Show a welcome banner for an already configured install
    Welcome,
    #[command(hide = true)]
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
    #[command(
        about = "Run agent turns through the unified runtime entry surface",
        long_about = "Run agent turns through the unified runtime entry surface.\n\nUse this namespace for direct runtime E2E checks, scripted provider/tool-call debugging, and other workflows that should exercise the same agent turn path as normal CLI conversations."
    )]
    /// Run agent turns through the unified runtime entry surface
    Turn {
        #[command(subcommand)]
        command: TurnCommands,
    },
    #[command(hide = true)]
    /// Invoke one connector operation through kernel policy gate
    InvokeConnector {
        #[arg(long)]
        operation: String,
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    #[command(hide = true)]
    /// Demonstrate audit lifecycle with fixed clock and token revocation
    AuditDemo,
    #[command(hide = true)]
    /// Generate a runnable JSON spec template for quick vertical customization
    InitSpec {
        #[arg(long, default_value = "loong.spec.json")]
        output: String,
        #[arg(long, value_enum, default_value_t = InitSpecPreset::Default)]
        preset: InitSpecPreset,
    },
    #[command(hide = true)]
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
    #[command(hide = true)]
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
    #[command(hide = true)]
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
    #[command(hide = true)]
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
    #[command(hide = true)]
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
    #[command(hide = true)]
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
        about = PERSONALIZE_COMMAND_ABOUT,
        long_about = PERSONALIZE_COMMAND_LONG_ABOUT
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
        long_about = "Power-user config import flow for discovering, previewing, or applying external workspace state explicitly.\n\nUse this when you want exact CLI control over import mode selection and output handling for older workspace roots. If you want the guided path, use `loong onboard` instead.\n\nMode quick reference:\n- discover, plan_many, recommend_primary, merge_profiles, map_skills: require `--input`\n- plan: requires `--input`; `--output` is optional preview target\n- apply: requires `--input` and `--output`\n- apply_selected: requires `--input` and `--output`; use `--source-id` to pin one discovered source, and `--apply-skills-plan` to bridge installable local skills into the managed runtime\n- rollback_last_apply: requires `--output`"
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
        /// Bridge installable local skills into the managed runtime during apply_selected
        #[arg(
            long = "apply-skills-plan",
            alias = "apply-external-skills-plan",
            default_value_t = false
        )]
        apply_skills_plan: bool,
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
    /// Build one developer-facing debug bundle over runtime, provider, ACP, session, and audit signals
    Debug {
        /// Path to the Loong config file, or omit to use normal config discovery
        #[arg(long, global = true)]
        config: Option<String>,
        /// Emit machine-readable JSON instead of the operator text view
        #[arg(long, global = true, default_value_t = false)]
        json: bool,
        /// Current session selector used when a subcommand does not provide `--session-id`
        #[arg(long, global = true, default_value = "default")]
        session: String,
        #[command(subcommand)]
        command: debug_cli::DebugCommands,
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
    /// Manage installed skills through an operator-facing CLI surface
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
    /// Inspect channels or run canonical grouped channel operations
    Channels {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        resolve: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
        #[command(subcommand)]
        command: Option<channels_cli::ChannelsCommands>,
    },
    /// Inspect runtime, ACP, MCP, snapshot, and trajectory operator surfaces
    Runtime {
        #[command(subcommand)]
        command: runtime_cli::RuntimeCommands,
    },
    #[command(
        about = "Run one non-interactive assistant turn",
        long_about = "Run one non-interactive one-shot assistant turn.\n\nUse this when you want a fast answer without entering the interactive `loong chat` REPL. The command reuses the normal CLI conversation runtime, session memory, provider selection, and ACP options."
    )]
    Ask {
        /// Path to the Loong config file, or omit to use normal config discovery
        #[arg(long)]
        config: Option<String>,
        /// Session id or selector such as `latest`; defaults to the normal CLI session
        #[arg(long)]
        session: Option<String>,
        /// User message to send through the real one-shot turn runtime
        #[arg(long)]
        message: String,
        /// Enable ACP bridge behavior for this turn
        #[arg(long, default_value_t = false)]
        acp: bool,
        /// Stream ACP turn events while the assistant turn runs
        #[arg(long, default_value_t = false)]
        acp_event_stream: bool,
        /// Bootstrap an MCP server before the ACP turn starts; repeat to add more servers
        #[arg(long = "acp-bootstrap-mcp-server")]
        acp_bootstrap_mcp_server: Vec<String>,
        /// Working directory used for ACP and bootstrapped MCP server context
        #[arg(long = "acp-cwd")]
        acp_cwd: Option<String>,
    },
    /// Start interactive CLI chat channel with sliding-window memory
    Chat {
        /// Path to the Loong config file, or omit to use normal config discovery
        #[arg(long)]
        config: Option<String>,
        /// Session id or selector such as `latest`; defaults to the normal CLI session
        #[arg(long)]
        session: Option<String>,
        /// Enable ACP bridge behavior for this chat session
        #[arg(long, default_value_t = false)]
        acp: bool,
        /// Stream ACP turn events while chat turns run
        #[arg(long, default_value_t = false)]
        acp_event_stream: bool,
        /// Bootstrap an MCP server before the ACP session starts; repeat to add more servers
        #[arg(long = "acp-bootstrap-mcp-server")]
        acp_bootstrap_mcp_server: Vec<String>,
        /// Working directory used for ACP and bootstrapped MCP server context
        #[arg(long = "acp-cwd")]
        acp_cwd: Option<String>,
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
    /// Run the Weixin bridge onboarding namespace
    Weixin {
        #[command(subcommand)]
        command: weixin_cli::WeixinCommand,
    },
    /// Operate the personal WhatsApp QR bridge namespace
    #[command(name = "whatsapp-personal")]
    WhatsappPersonal {
        #[command(subcommand)]
        command: whatsapp_personal_cli::WhatsappPersonalCommand,
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

    let normalized_channel_id = mvp::channel::normalize_channel_catalog_id(channel_token)
        .ok_or_else(|| {
            let supported_channels = supported_multi_channel_serve_channel_ids().join(", ");
            format!(
                "unrecognized multi-channel service channel `{channel_token}` (available runtime-backed channels: {supported_channels})"
            )
        })?;
    let supported_channel_ids = supported_multi_channel_serve_channel_ids();
    let runtime_channel_id = normalized_channel_id;
    let runtime_is_supported = supported_channel_ids.contains(&runtime_channel_id);
    if !runtime_is_supported {
        let supported_channels = supported_channel_ids.join(", ");
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
        channel_id: runtime_channel_id.to_owned(),
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
#[path = "lib_multi_channel_serve_tests.rs"]
mod multi_channel_serve_tests;

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

pub const RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION: u32 = 3;
pub const RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 3;

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

pub async fn with_graceful_shutdown<F>(serve_future: F) -> CliResult<()>
where
    F: std::future::Future<Output = CliResult<()>>,
{
    tokio::select! {
        result = serve_future => result,
        result = wait_for_shutdown_reason() => result.map(|_| ()),
    }
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
    if args.stop_requested {
        let channel_id = spec.family.channel_id;
        let stop_result = match channel_id {
            "telegram" => {
                request_runtime_backed_channel_serve_stop(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Telegram,
                    args.account,
                    |config, account| {
                        let resolved = config.telegram.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "feishu" => {
                request_runtime_backed_channel_serve_stop(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Feishu,
                    args.account,
                    |config, account| {
                        let resolved = config.feishu.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "line" => {
                request_runtime_backed_channel_serve_stop(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Line,
                    args.account,
                    |config, account| {
                        let resolved = config.line.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "matrix" => {
                request_runtime_backed_channel_serve_stop(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Matrix,
                    args.account,
                    |config, account| {
                        let resolved = config.matrix.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "wecom" => {
                request_runtime_backed_channel_serve_stop(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Wecom,
                    args.account,
                    |config, account| {
                        let resolved = config.wecom.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "webhook" => {
                request_runtime_backed_channel_serve_stop(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Webhook,
                    args.account,
                    |config, account| {
                        let resolved = config.webhook.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "whatsapp" => {
                request_runtime_backed_channel_serve_stop(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::WhatsApp,
                    args.account,
                    |config, account| {
                        let resolved = config.whatsapp.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            _ => Err(format!(
                "{} does not support --stop on this serve surface",
                spec.family.serve.command
            )),
        };
        return stop_result;
    }
    if args.stop_duplicates_requested {
        let channel_id = spec.family.channel_id;
        let stop_result = match channel_id {
            "telegram" => {
                request_runtime_backed_channel_serve_duplicate_cleanup(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Telegram,
                    args.account,
                    |config, account| {
                        let resolved = config.telegram.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "feishu" => {
                request_runtime_backed_channel_serve_duplicate_cleanup(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Feishu,
                    args.account,
                    |config, account| {
                        let resolved = config.feishu.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "line" => {
                request_runtime_backed_channel_serve_duplicate_cleanup(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Line,
                    args.account,
                    |config, account| {
                        let resolved = config.line.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "matrix" => {
                request_runtime_backed_channel_serve_duplicate_cleanup(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Matrix,
                    args.account,
                    |config, account| {
                        let resolved = config.matrix.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "wecom" => {
                request_runtime_backed_channel_serve_duplicate_cleanup(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Wecom,
                    args.account,
                    |config, account| {
                        let resolved = config.wecom.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "webhook" => {
                request_runtime_backed_channel_serve_duplicate_cleanup(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::Webhook,
                    args.account,
                    |config, account| {
                        let resolved = config.webhook.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            "whatsapp" => {
                request_runtime_backed_channel_serve_duplicate_cleanup(
                    args.config_path,
                    channel_id,
                    mvp::channel::ChannelPlatform::WhatsApp,
                    args.account,
                    |config, account| {
                        let resolved = config.whatsapp.resolve_account(account)?;
                        Ok((
                            resolved.configured_account_id,
                            resolved.account.id,
                            resolved.account.label,
                        ))
                    },
                )
                .await
            }
            _ => Err(format!(
                "{} does not support --stop-duplicates on this serve surface",
                spec.family.serve.command
            )),
        };
        return stop_result;
    }
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
        let target = require_channel_send_target("channels send discord", args.target)?;
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
        let target = require_channel_send_target("channels send email", args.target)?;
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
        let target = require_channel_send_target("channels send mattermost", args.target)?;
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
        let target = require_channel_send_target("channels send nextcloud-talk", args.target)?;
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
        let target = require_channel_send_target("channels send irc", args.target)?;
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
        let target = require_channel_send_target("channels send imessage", args.target)?;
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
        let target = require_channel_send_target("channels send twitch", args.target)?;
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
        if args.stop_requested {
            return request_runtime_backed_channel_serve_stop(
                args.config_path,
                "telegram",
                mvp::channel::ChannelPlatform::Telegram,
                args.account,
                |config, account| {
                    let resolved = config.telegram.resolve_account(account)?;
                    Ok((
                        resolved.configured_account_id,
                        resolved.account.id,
                        resolved.account.label,
                    ))
                },
            )
            .await;
        }
        if args.stop_duplicates_requested {
            return request_runtime_backed_channel_serve_duplicate_cleanup(
                args.config_path,
                "telegram",
                mvp::channel::ChannelPlatform::Telegram,
                args.account,
                |config, account| {
                    let resolved = config.telegram.resolve_account(account)?;
                    Ok((
                        resolved.configured_account_id,
                        resolved.account.id,
                        resolved.account.label,
                    ))
                },
            )
            .await;
        }
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
        if args.stop_requested {
            return request_runtime_backed_channel_serve_stop(
                args.config_path,
                "matrix",
                mvp::channel::ChannelPlatform::Matrix,
                args.account,
                |config, account| {
                    let resolved = config.matrix.resolve_account(account)?;
                    Ok((
                        resolved.configured_account_id,
                        resolved.account.id,
                        resolved.account.label,
                    ))
                },
            )
            .await;
        }
        if args.stop_duplicates_requested {
            return request_runtime_backed_channel_serve_duplicate_cleanup(
                args.config_path,
                "matrix",
                mvp::channel::ChannelPlatform::Matrix,
                args.account,
                |config, account| {
                    let resolved = config.matrix.resolve_account(account)?;
                    Ok((
                        resolved.configured_account_id,
                        resolved.account.id,
                        resolved.account.label,
                    ))
                },
            )
            .await;
        }
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
        if args.stop_requested {
            return request_runtime_backed_channel_serve_stop(
                args.config_path,
                "wecom",
                mvp::channel::ChannelPlatform::Wecom,
                args.account,
                |config, account| {
                    let resolved = config.wecom.resolve_account(account)?;
                    Ok((
                        resolved.configured_account_id,
                        resolved.account.id,
                        resolved.account.label,
                    ))
                },
            )
            .await;
        }
        if args.stop_duplicates_requested {
            return request_runtime_backed_channel_serve_duplicate_cleanup(
                args.config_path,
                "wecom",
                mvp::channel::ChannelPlatform::Wecom,
                args.account,
                |config, account| {
                    let resolved = config.wecom.resolve_account(account)?;
                    Ok((
                        resolved.configured_account_id,
                        resolved.account.id,
                        resolved.account.label,
                    ))
                },
            )
            .await;
        }
        with_graceful_shutdown(mvp::channel::run_wecom_channel(
            args.config_path,
            args.account,
        ))
        .await
    })
}

async fn request_runtime_backed_channel_serve_stop<F>(
    config_path: Option<&str>,
    channel_id: &str,
    platform: mvp::channel::ChannelPlatform,
    account_id: Option<&str>,
    resolve_account: F,
) -> CliResult<()>
where
    F: FnOnce(&mvp::config::LoongConfig, Option<&str>) -> CliResult<(String, String, String)>,
{
    let (_resolved_path, config) = mvp::config::load(config_path)?;
    let (configured_account_id, runtime_account_id, runtime_account_label) =
        resolve_account(&config, account_id)?;
    let outcome = mvp::channel::request_channel_operation_stop(
        platform,
        mvp::channel::CHANNEL_OPERATION_SERVE_ID,
        Some(runtime_account_id.as_str()),
    )?;

    let outcome_label = match outcome {
        mvp::channel::ChannelOperationStopRequestOutcome::Requested => "requested",
        mvp::channel::ChannelOperationStopRequestOutcome::AlreadyRequested => "already_requested",
        mvp::channel::ChannelOperationStopRequestOutcome::AlreadyStopped => "already_stopped",
    };
    #[allow(clippy::print_stdout)]
    {
        println!(
            "{} serve stop {} (configured_account={}, account={})",
            channel_id, outcome_label, configured_account_id, runtime_account_label
        );
    }

    Ok(())
}

async fn request_runtime_backed_channel_serve_duplicate_cleanup<F>(
    config_path: Option<&str>,
    channel_id: &str,
    platform: mvp::channel::ChannelPlatform,
    account_id: Option<&str>,
    resolve_account: F,
) -> CliResult<()>
where
    F: FnOnce(&mvp::config::LoongConfig, Option<&str>) -> CliResult<(String, String, String)>,
{
    let (_resolved_path, config) = mvp::config::load(config_path)?;
    let (configured_account_id, runtime_account_id, runtime_account_label) =
        resolve_account(&config, account_id)?;
    let result = mvp::channel::request_channel_operation_duplicate_cleanup(
        platform,
        mvp::channel::CHANNEL_OPERATION_SERVE_ID,
        Some(runtime_account_id.as_str()),
    )?;

    let outcome_label = match result.outcome {
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::Requested => "requested",
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::AlreadyRequested => {
            "already_requested"
        }
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::NoDuplicates => "no_duplicates",
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::AlreadyStopped => "already_stopped",
    };
    let preferred_owner_pid = result
        .preferred_owner_pid
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let cleanup_owner_pids = if result.targeted_owner_pids.is_empty() {
        "-".to_owned()
    } else {
        result
            .targeted_owner_pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    #[allow(clippy::print_stdout)]
    {
        println!(
            "{} serve duplicate cleanup {} (configured_account={}, account={}, preferred_owner_pid={}, cleanup_owner_pids={})",
            channel_id,
            outcome_label,
            configured_account_id,
            runtime_account_label,
            preferred_owner_pid,
            cleanup_owner_pids,
        );
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::build_cli_command;

    #[test]
    fn build_cli_command_personalize_subcommand_uses_guidance_copy() {
        let command = build_cli_command("loong");
        let personalize = command
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == "personalize")
            .expect("personalize subcommand");

        let about = personalize
            .get_about()
            .map(ToString::to_string)
            .expect("personalize about");
        let long_about = personalize
            .get_long_about()
            .map(ToString::to_string)
            .expect("personalize long_about");

        assert!(
            about.contains("Teach Loong your working style"),
            "personalize about should match the operator-facing guidance copy: {about}"
        );
        assert!(
            long_about.contains("Teach Loong your working style"),
            "personalize help should lead with the same guidance copy: {long_about}"
        );
        assert!(
            !long_about.contains("working preferences"),
            "personalize help should not fall back to the older field-oriented wording: {long_about}"
        );
    }
}
