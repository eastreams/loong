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
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use kernel::{
    BootstrapTaskStatus, Capability, ConnectorCommand, FixedClock, InMemoryAuditSink,
    PluginActivationStatus, TaskIntent, ToolCoreOutcome, ToolCoreRequest,
};
use loongclaw_contracts::SecretRef;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub use loongclaw_app as mvp;
pub use loongclaw_spec::spec_execution::*;
pub use loongclaw_spec::spec_runtime::*;
pub use loongclaw_spec::{CliResult, DEFAULT_AGENT_ID, DEFAULT_PACK_ID, kernel_bootstrap};

pub use self::channel_send_target_kind::{
    default_twitch_send_target_kind, parse_twitch_send_target_kind,
};
pub use loongclaw_bench::{
    run_programmatic_pressure_baseline_lint_cli, run_programmatic_pressure_benchmark_cli,
    run_wasm_cache_benchmark_cli,
};
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
pub use memory_context_benchmark::run_memory_context_benchmark_cli;
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

pub use base64;
pub use kernel;
pub use sha2;

pub mod audit_cli;
mod browser_companion_diagnostics;
pub mod browser_preview;
#[cfg(test)]
mod channel_send_cli_tests;
mod channel_send_target_kind;
mod cli_handoff;
pub mod completions_cli;
pub mod doctor_cli;
pub mod doctor_security_cli;
mod external_skills_policy_probe;
pub mod feishu_cli;
pub mod feishu_support;
pub mod gateway;
pub mod import_cli;
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
mod memory_context_benchmark;
pub mod migrate_cli;
pub mod migration;
pub mod next_actions;
pub mod onboard_cli;
mod onboard_finalize;
mod onboard_preflight;
pub mod onboard_presentation;
mod onboard_types;
mod onboard_web_search;
mod onboarding_model_policy;
pub mod plugins_cli;
mod provider_credential_policy;
mod provider_model_probe_policy;
pub mod provider_presentation;
mod provider_route_diagnostics;
pub mod runtime_capability_cli;
pub mod runtime_experiment_cli;
pub mod runtime_restore_cli;
pub mod skills_cli;
pub mod source_presentation;
pub mod supervisor;
pub mod tasks_cli;
mod tlon_cli;

pub use gateway::read_models::{ChannelsCliJsonPayload, ChannelsCliJsonSchema};
pub use loongclaw_spec::programmatic::{
    acquire_programmatic_circuit_slot, record_programmatic_circuit_outcome,
};
pub use tlon_cli::TLON_SEND_CLI_SPEC;
use tlon_cli::{default_tlon_send_target_kind, parse_tlon_send_target_kind};

#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::missing_panics_doc
)]
#[doc(hidden)]
pub mod test_support;

pub const PUBLIC_GITHUB_REPO: &str = "loongclaw-ai/loongclaw";
pub const CLI_COMMAND_NAME: &str = mvp::config::CLI_COMMAND_NAME;
pub const LEGACY_CLI_COMMAND_NAME: &str = mvp::config::LEGACY_CLI_COMMAND_NAME;

pub fn active_cli_command_name() -> &'static str {
    mvp::config::active_cli_command_name()
}

fn render_welcome_long_about(command_name: &str) -> String {
    format!(
        "Show the configured welcome banner and quick commands.\n\nquick commands:\n- {command_name} ask --config <path> --message \"...\"\n- {command_name} chat --config <path>\n- {command_name} doctor --config <path>\n- {command_name} --help\n\nReplace <path> with your current config path, or set LOONGCLAW_CONFIG_PATH first."
    )
}

fn render_import_long_about(command_name: &str) -> String {
    format!(
        "Power-user import flow for previewing or applying detected migration sources explicitly.\n\nUse this when you want exact CLI control over which source and domains are reused. If you want the guided path, use `{command_name} onboard` instead. When the same source kind resolves to multiple detected configs, rerun with `--source-path <path>` to choose one exact source."
    )
}

fn render_migrate_long_about(command_name: &str) -> String {
    format!(
        "Power-user migration flow for discovering, previewing, or applying legacy claw nativeization explicitly.\n\nUse this when you want exact CLI control over migration mode selection and output handling for older claw-family workspaces. If you want the guided path, use `{command_name} onboard` instead.\n\nMode quick reference:\n- discover, plan_many, recommend_primary, merge_profiles, map_external_skills: require `--input`\n- plan: requires `--input`; `--output` is optional preview target\n- apply: requires `--input` and `--output`\n- apply_selected: requires `--input` and `--output`; use `--source-id` to pin one discovered source, and `--apply-external-skills-plan` to bridge installable local external skills into the managed runtime\n- rollback_last_apply: requires `--output`"
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
            command.long_about(render_migrate_long_about(command_name))
        })
        .mut_subcommand("ask", |command| {
            command.long_about(render_ask_long_about(command_name))
        })
}

pub fn parse_cli() -> Cli {
    let mut matches = build_cli_command(active_cli_command_name()).get_matches();
    Cli::from_arg_matches_mut(&mut matches).unwrap_or_else(|error| error.exit())
}

pub fn native_spec_tool_executor(
    request: ToolCoreRequest,
) -> Option<Result<ToolCoreOutcome, String>> {
    if mvp::tools::canonical_tool_name(request.tool_name.as_str()) != "claw.migrate" {
        return None;
    }
    Some(mvp::tools::execute_tool_core(request))
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
    pub family: mvp::channel::ChannelCommandFamilyDescriptor,
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
    about = "LoongClaw low-level runtime daemon",
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
        long_about = "Show the configured welcome banner and quick commands.\n\nquick commands:\n- loong ask --config <path> --message \"...\"\n- loong chat --config <path>\n- loong doctor --config <path>\n- loong --help\n\nReplace <path> with your current config path, or set LOONGCLAW_CONFIG_PATH first."
    )]
    /// Show a welcome banner for an already configured install
    Welcome,
    /// Run the original end-to-end bootstrap demo
    Demo,
    /// Execute one task through the kernel+harness path
    RunTask {
        #[arg(long)]
        objective: String,
        #[arg(long, default_value = "{}")]
        payload: String,
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
        #[arg(long, default_value = "loongclaw.spec.json")]
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
        long_about = "Guided onboarding for fast first-chat setup with preflight diagnostics.\n\nThis is the default path for most users. LoongClaw will detect reusable settings for provider, channels, or workspace guidance, suggest a starting point, and walk through quick review before first chat."
    )]
    Onboard {
        /// Write the resulting config to a custom path instead of the default loongclaw config location
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
        about = "Preview or apply migration sources explicitly",
        long_about = "Power-user import flow for previewing or applying detected migration sources explicitly.\n\nUse this when you want exact CLI control over which source and domains are reused. If you want the guided path, use `loong onboard` instead. When the same source kind resolves to multiple detected configs, rerun with `--source-path <path>` to choose one exact source."
    )]
    Import {
        /// Write the imported config to a custom path instead of the default loongclaw config location
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
        about = "Preview or apply legacy claw migration explicitly",
        long_about = "Power-user migration flow for discovering, previewing, or applying legacy claw nativeization explicitly.\n\nUse this when you want exact CLI control over migration mode selection and output handling for older claw-family workspaces. If you want the guided path, use `loong onboard` instead.\n\nMode quick reference:\n- discover, plan_many, recommend_primary, merge_profiles, map_external_skills: require `--input`\n- plan: requires `--input`; `--output` is optional preview target\n- apply: requires `--input` and `--output`\n- apply_selected: requires `--input` and `--output`; use `--source-id` to pin one discovered source, and `--apply-external-skills-plan` to bridge installable local external skills into the managed runtime\n- rollback_last_apply: requires `--output`"
    )]
    Migrate {
        /// Path to the legacy claw workspace or root to inspect
        #[arg(long)]
        input: Option<String>,
        /// Target LoongClaw config path to preview, write, or roll back
        #[arg(long)]
        output: Option<String>,
        /// Hint the legacy source kind for single-source plan/apply modes
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
mod multi_channel_serve_tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn supported_multi_channel_serve_channel_ids_follow_background_runtime_registry() {
        let expected_ids = mvp::channel::background_channel_runtime_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.channel_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let actual_ids = supported_multi_channel_serve_channel_ids();

        assert_eq!(actual_ids, expected_ids);
    }

    #[test]
    fn parse_multi_channel_serve_channel_account_rejects_compiled_out_matrix_runtime() {
        let supported_channel_ids = supported_multi_channel_serve_channel_ids();
        let matrix_is_supported = supported_channel_ids.contains(&"matrix");
        if matrix_is_supported {
            return;
        }

        let error = parse_multi_channel_serve_channel_account("matrix=bridge-sync")
            .expect_err("compiled-out matrix runtime should be rejected");

        assert!(
            error.contains(
                "multi-channel service channel `matrix` resolves to `matrix` but is not supported in this build"
            )
        );
    }

    #[test]
    fn parse_multi_channel_serve_channel_account_rejects_unknown_runtime_channel() {
        let error = parse_multi_channel_serve_channel_account("unknown=bridge-sync")
            .expect_err("unknown runtime channel should be rejected");

        assert!(error.contains("unrecognized multi-channel service channel `unknown`"));
    }
}

fn resolved_default_entry_config_path() -> PathBuf {
    std::env::var_os("LOONGCLAW_CONFIG_PATH")
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

fn resolve_welcome_config_path() -> CliResult<PathBuf> {
    let config_path = resolved_default_entry_config_path();
    if config_path.is_file() {
        Ok(config_path)
    } else {
        Err(format!(
            "Config file not found at {}. Run `{} onboard` to set up LoongClaw.",
            config_path.display(),
            active_cli_command_name(),
        ))
    }
}

fn render_welcome_banner(config_path: &Path) -> String {
    let config_path = config_path.to_string_lossy();
    let ask_command = crate::cli_handoff::format_ask_with_config(
        &config_path,
        next_actions::DEFAULT_FIRST_ASK_MESSAGE,
    );
    let chat_command = crate::cli_handoff::format_subcommand_with_config("chat", &config_path);
    let doctor_command = crate::cli_handoff::format_subcommand_with_config("doctor", &config_path);

    format!(
        "LoongClaw is configured and ready.\nVersion: {}\nConfig: {}\n\nQuick commands:\n- First answer: {}\n- Chat: {}\n- Doctor: {}\n- Help: {} --help",
        env!("CARGO_PKG_VERSION"),
        config_path,
        ask_command,
        chat_command,
        doctor_command,
        active_cli_command_name(),
    )
}

pub fn run_welcome_cli() -> CliResult<()> {
    let config_path = resolve_welcome_config_path()?;
    println!("{}", render_welcome_banner(config_path.as_path()));
    Ok(())
}

pub async fn run_demo() -> CliResult<()> {
    let kernel = kernel_bootstrap::KernelBuilder::default().build();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 300)
        .map_err(|error| format!("token issue failed: {error}"))?;

    let task = TaskIntent {
        task_id: "task-bootstrap-01".to_owned(),
        objective: "summarize flaky test clusters".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::MemoryRead]),
        payload: json!({"repo": PUBLIC_GITHUB_REPO}),
    };

    let task_dispatch = kernel
        .execute_task(DEFAULT_PACK_ID, &token, task)
        .await
        .map_err(|error| format!("task dispatch failed: {error}"))?;

    println!(
        "task dispatched via {:?}: {}",
        task_dispatch.adapter_route.harness_kind, task_dispatch.outcome.output
    );

    let connector_dispatch = kernel
        .execute_connector_core(
            DEFAULT_PACK_ID,
            &token,
            None,
            ConnectorCommand {
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"channel": "ops-alerts", "message": "task complete"}),
            },
        )
        .await
        .map_err(|error| format!("connector dispatch failed: {error}"))?;

    println!("connector dispatch: {}", connector_dispatch.outcome.payload);
    Ok(())
}

#[cfg(test)]
mod first_run_entry_tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static UNIQUE_TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let pid = process::id();
        let counter = UNIQUE_TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}-{counter}"))
    }

    fn isolated_home(prefix: &str) -> (mvp::test_support::ScopedEnv, PathBuf) {
        let mut env = mvp::test_support::ScopedEnv::new();
        let home = unique_temp_dir(prefix);
        fs::create_dir_all(&home).expect("create isolated home");
        env.set("HOME", &home);
        env.remove("LOONGCLAW_CONFIG_PATH");
        (env, home)
    }

    #[test]
    fn resolve_default_entry_command_routes_to_onboard_when_config_is_missing() {
        let (_env, _home) = isolated_home("loongclaw-default-entry-missing");

        assert!(
            matches!(resolve_default_entry_command(), Commands::Onboard { .. }),
            "missing config should route to onboard"
        );
    }

    #[test]
    fn resolve_default_entry_command_routes_to_welcome_when_default_config_exists() {
        let (_env, _home) = isolated_home("loongclaw-default-entry-present");
        let config_path = mvp::config::default_config_path();
        mvp::config::write(
            Some(config_path.to_str().expect("utf8 config path")),
            &mvp::config::LoongClawConfig::default(),
            true,
        )
        .expect("write default config");

        assert!(
            matches!(resolve_default_entry_command(), Commands::Welcome),
            "present config should route to welcome"
        );
    }

    #[test]
    fn resolve_default_entry_command_honors_loongclaw_config_path_override() {
        let mut env = mvp::test_support::ScopedEnv::new();
        let config_path = unique_temp_dir("loongclaw-default-entry-env").join("custom-config.toml");
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).expect("create config parent");
        }
        mvp::config::write(
            Some(config_path.to_str().expect("utf8 config path")),
            &mvp::config::LoongClawConfig::default(),
            true,
        )
        .expect("write explicit config");
        env.set("LOONGCLAW_CONFIG_PATH", &config_path);

        assert!(
            matches!(resolve_default_entry_command(), Commands::Welcome),
            "env override config should route to welcome"
        );
    }

    #[test]
    fn resolve_default_entry_command_routes_to_onboard_when_config_path_is_a_directory() {
        let mut env = mvp::test_support::ScopedEnv::new();
        let config_dir = unique_temp_dir("loongclaw-default-entry-dir");
        fs::create_dir_all(&config_dir).expect("create config directory");
        env.set("LOONGCLAW_CONFIG_PATH", &config_dir);

        assert!(
            matches!(resolve_default_entry_command(), Commands::Onboard { .. }),
            "directory config path should still route to onboard"
        );
    }

    #[test]
    fn run_welcome_cli_rejects_missing_config_file() {
        let mut env = mvp::test_support::ScopedEnv::new();
        let config_path = unique_temp_dir("loongclaw-welcome-missing").join("missing-config.toml");
        env.set("LOONGCLAW_CONFIG_PATH", &config_path);

        let error = run_welcome_cli().expect_err("missing config should fail welcome");

        assert!(
            error.contains("Config file not found"),
            "welcome should explain the missing config file: {error}"
        );
        assert!(
            error.contains("loong onboard"),
            "welcome should point users back to onboarding: {error}"
        );
    }

    #[test]
    fn run_welcome_cli_rejects_directory_config_path() {
        let mut env = mvp::test_support::ScopedEnv::new();
        let config_dir = unique_temp_dir("loongclaw-welcome-dir");
        fs::create_dir_all(&config_dir).expect("create config directory");
        env.set("LOONGCLAW_CONFIG_PATH", &config_dir);

        let error = run_welcome_cli().expect_err("directory config path should fail welcome");

        assert!(
            error.contains("Config file not found"),
            "welcome should reject directory config paths as missing config files: {error}"
        );
    }

    #[test]
    fn render_welcome_banner_includes_version_and_next_commands() {
        let rendered = render_welcome_banner(Path::new("/tmp/loongclaw's config.toml"));

        assert!(
            rendered.contains(env!("CARGO_PKG_VERSION")),
            "welcome banner should include the current version: {rendered}"
        );
        assert!(
            rendered.contains("loong ask --config '/tmp/loongclaw'\"'\"'s config.toml'"),
            "welcome banner should include a quoted ask command: {rendered}"
        );
        assert!(
            rendered.contains("loong chat --config '/tmp/loongclaw'\"'\"'s config.toml'"),
            "welcome banner should include a quoted chat command: {rendered}"
        );
        assert!(
            rendered.contains("loong doctor --config '/tmp/loongclaw'\"'\"'s config.toml'"),
            "welcome banner should include a quoted doctor command: {rendered}"
        );
        assert!(
            rendered.contains("loong --help"),
            "welcome banner should point users to root help: {rendered}"
        );
    }
}

pub async fn run_task_cli(objective: &str, payload_raw: &str) -> CliResult<()> {
    let payload = parse_json_payload(payload_raw, "run-task payload")?;

    let kernel = kernel_bootstrap::KernelBuilder::default().build();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
        .map_err(|error| format!("token issue failed: {error}"))?;

    let dispatch = kernel
        .execute_task(
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-cli-01".to_owned(),
                objective: objective.to_owned(),
                required_capabilities: BTreeSet::from([
                    Capability::InvokeTool,
                    Capability::MemoryRead,
                ]),
                payload,
            },
        )
        .await
        .map_err(|error| format!("task dispatch failed: {error}"))?;

    let pretty = serde_json::to_string_pretty(&dispatch.outcome)
        .map_err(|error| format!("serialize task outcome failed: {error}"))?;
    println!("{pretty}");
    Ok(())
}

pub async fn invoke_connector_cli(operation: &str, payload_raw: &str) -> CliResult<()> {
    let payload = parse_json_payload(payload_raw, "invoke-connector payload")?;

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

    let _ = kernel
        .execute_task(
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-audit-01".to_owned(),
                objective: "produce audit evidence".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({}),
            },
        )
        .await
        .map_err(|error| format!("task dispatch failed: {error}"))?;

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
                    "type": "urn:loongclaw:problem:none",
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
                        "urn:loongclaw:problem:config.validation_warning"
                    } else {
                        "urn:loongclaw:problem:config.validation_failed"
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

pub const RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotCliState {
    pub config: String,
    pub provider: RuntimeSnapshotProviderState,
    pub context_engine: mvp::conversation::ContextEngineRuntimeSnapshot,
    pub memory_system: mvp::memory::MemorySystemRuntimeSnapshot,
    pub acp: mvp::acp::AcpRuntimeSnapshot,
    pub enabled_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub channels: mvp::channel::ChannelInventory,
    pub tool_runtime: mvp::tools::runtime_config::ToolRuntimeConfig,
    pub visible_tool_names: Vec<String>,
    pub capability_snapshot: String,
    pub capability_snapshot_sha256: String,
    pub external_skills: RuntimeSnapshotExternalSkillsState,
    pub restore_spec: RuntimeSnapshotRestoreSpec,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotProviderState {
    pub active_profile_id: String,
    pub active_label: String,
    pub last_provider_id: Option<String>,
    pub saved_profile_ids: Vec<String>,
    pub profiles: Vec<RuntimeSnapshotProviderProfileState>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotProviderProfileState {
    pub profile_id: String,
    pub is_active: bool,
    pub default_for_kind: bool,
    pub kind: mvp::config::ProviderKind,
    pub model: String,
    pub wire_api: mvp::config::ProviderWireApi,
    pub base_url: String,
    pub endpoint: String,
    pub models_endpoint: String,
    pub protocol_family: &'static str,
    pub credential_resolved: bool,
    pub auth_env: Option<String>,
    pub reasoning_effort: Option<String>,
    pub temperature: f64,
    pub max_tokens: Option<u32>,
    pub request_timeout_ms: u64,
    pub retry_max_attempts: usize,
    pub header_names: Vec<String>,
    pub preferred_models: Vec<String>,
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
    pub external_skills: Value,
    pub restore_spec: RuntimeSnapshotRestoreSpec,
}

pub fn run_runtime_snapshot_cli(
    config_path: Option<&str>,
    as_json: bool,
    output_path: Option<&str>,
    label: Option<&str>,
    experiment_id: Option<&str>,
    parent_snapshot_id: Option<&str>,
) -> CliResult<()> {
    let snapshot = collect_runtime_snapshot_cli_state(config_path)?;
    let metadata =
        runtime_snapshot_artifact_metadata_now(label, experiment_id, parent_snapshot_id)?;
    let artifact_payload = build_runtime_snapshot_artifact_json_payload(&snapshot, &metadata)?;

    if let Some(output_path) = output_path {
        persist_runtime_snapshot_artifact(output_path, &artifact_payload)?;
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
    config: &mvp::config::LoongClawConfig,
) -> CliResult<RuntimeSnapshotCliState> {
    let config_display = resolved_path.display().to_string();
    let provider = collect_runtime_snapshot_provider_state(config);
    let context_engine = mvp::conversation::collect_context_engine_runtime_snapshot(config)?;
    let memory_system = mvp::memory::collect_memory_system_runtime_snapshot(config)?;
    let acp = mvp::acp::collect_acp_runtime_snapshot(config)?;
    let enabled_channel_ids = config.enabled_channel_ids();
    let enabled_service_channel_ids = config.enabled_service_channel_ids();
    let channels = mvp::channel::channel_inventory(config);
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        Some(resolved_path),
    );
    let (external_skills, snapshot_tool_runtime) =
        collect_runtime_snapshot_external_skills_state(&tool_runtime);
    let tool_view = mvp::tools::runtime_tool_view_for_runtime_config(&snapshot_tool_runtime);
    let visible_tool_names = tool_view
        .tool_names()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let capability_snapshot = mvp::tools::capability_snapshot_with_config(&snapshot_tool_runtime);
    let capability_snapshot_sha256 =
        runtime_snapshot_tool_digest(&visible_tool_names, &capability_snapshot)?;
    let restore_spec = build_runtime_snapshot_restore_spec(config, &external_skills);

    Ok(RuntimeSnapshotCliState {
        config: config_display,
        provider,
        context_engine,
        memory_system,
        acp,
        enabled_channel_ids,
        enabled_service_channel_ids,
        channels,
        tool_runtime: snapshot_tool_runtime,
        visible_tool_names,
        capability_snapshot,
        capability_snapshot_sha256,
        external_skills,
        restore_spec,
    })
}

fn collect_runtime_snapshot_provider_state(
    config: &mvp::config::LoongClawConfig,
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

    RuntimeSnapshotProviderState {
        active_profile_id,
        active_label: provider_presentation::active_provider_detail_label(config),
        last_provider_id: config.last_provider_id().map(str::to_owned),
        saved_profile_ids,
        profiles,
    }
}

fn build_runtime_snapshot_provider_profile_state(
    profile_id: &str,
    profile: &mvp::config::ProviderProfileConfig,
    is_active: bool,
) -> RuntimeSnapshotProviderProfileState {
    let provider = &profile.provider;
    let mut header_names = provider.headers.keys().cloned().collect::<Vec<_>>();
    header_names.sort();

    RuntimeSnapshotProviderProfileState {
        profile_id: profile_id.to_owned(),
        is_active,
        default_for_kind: profile.default_for_kind,
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
    Ok(format!("{:x}", Sha256::digest(serialized)))
}

fn json_array_len(value: Option<&Value>) -> usize {
    value.and_then(Value::as_array).map_or(0, Vec::len)
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
    config: &mvp::config::LoongClawConfig,
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
        managed_skills: build_runtime_snapshot_restore_managed_skills_spec(
            external_skills,
            &mut warnings,
        ),
        warnings,
    }
}

fn runtime_snapshot_restore_provider_profiles(
    config: &mvp::config::LoongClawConfig,
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
mod runtime_snapshot_restore_spec_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_snapshot_restore_managed_skills_keeps_entries_without_display_metadata() {
        let mut warnings = Vec::new();
        let spec = build_runtime_snapshot_restore_managed_skills_spec(
            &RuntimeSnapshotExternalSkillsState {
                policy: mvp::tools::runtime_config::ExternalSkillsRuntimePolicy::default(),
                override_active: false,
                inventory_status: RuntimeSnapshotInventoryStatus::Ok,
                inventory_error: None,
                inventory: json!({
                    "skills": [{
                        "scope": "managed",
                        "skill_id": "demo-skill",
                        "source_kind": "directory",
                        "source_path": "/tmp/demo-skill",
                        "sha256": "deadbeef"
                    }]
                }),
                resolved_skill_count: 1,
                shadowed_skill_count: 0,
            },
            &mut warnings,
        );

        assert!(warnings.is_empty());
        assert_eq!(spec.skills.len(), 1);
        assert_eq!(spec.skills[0].skill_id, "demo-skill");
        assert!(spec.skills[0].display_name.is_empty());
        assert!(spec.skills[0].summary.is_empty());
    }

    #[test]
    fn runtime_snapshot_provider_header_safety_uses_explicit_safe_names_only() {
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Anthropic,
            "anthropic-version",
            "2023-06-01",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "anthropic-version",
            "2023-06-01",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Anthropic,
            "anthropic-beta",
            "prompt-caching-2024-07-31",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Openai,
            "openai-beta",
            "assistants=v2",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "x-goog-api-key",
            "${GOOGLE_API_KEY}",
        ));
        assert!(!runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "x-secret-beta",
            "literal-secret",
        ));
        assert!(!runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "x-secret-version",
            "literal-secret",
        ));
    }

    #[test]
    fn runtime_snapshot_restore_normalization_moves_provider_env_name_fields_into_secret_refs() {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key_env: Some("OPENAI_API_KEY".to_owned()),
                oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_restore_normalization_canonicalizes_matching_explicit_env_reference() {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key: Some(SecretRef::Inline("${INLINE_OPENAI_API_KEY}".to_owned())),
                api_key_env: Some(" INLINE_OPENAI_API_KEY ".to_owned()),
                oauth_access_token: Some(SecretRef::Inline(
                    "$INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
                )),
                oauth_access_token_env: Some("INLINE_OPENAI_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_restore_normalization_prefers_explicit_env_reference_over_legacy_env_field()
    {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key: Some(SecretRef::Inline("${INLINE_OPENAI_API_KEY}".to_owned())),
                api_key_env: Some("CONFIGURED_OPENAI_API_KEY".to_owned()),
                oauth_access_token: Some(SecretRef::Inline(
                    "$INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
                )),
                oauth_access_token_env: Some("CONFIGURED_OPENAI_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_restore_normalization_treats_blank_inline_secret_as_absent() {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key: Some(SecretRef::Inline("   ".to_owned())),
                api_key_env: Some("OPENAI_API_KEY".to_owned()),
                oauth_access_token: Some(SecretRef::Inline("   ".to_owned())),
                oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_tool_runtime_json_reports_browser_execution_tiers() {
        let mut runtime = mvp::tools::runtime_config::ToolRuntimeConfig::default();
        runtime.browser_companion.enabled = true;
        runtime.browser_companion.ready = true;
        runtime.browser_companion.command = Some("browser-companion".to_owned());

        let json = runtime_snapshot_tool_runtime_json(&runtime);

        assert_eq!(json["browser"]["execution_tier"], json!("restricted"));
        assert_eq!(
            json["browser_companion"]["execution_tier"],
            json!("balanced")
        );
    }
}

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

fn persist_runtime_snapshot_artifact(output_path: &str, payload: &Value) -> CliResult<()> {
    let output_path = PathBuf::from(output_path);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime snapshot artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("serialize runtime snapshot artifact failed: {error}"))?;
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "write runtime snapshot artifact {} failed: {error}",
            output_path.display()
        )
    })?;
    Ok(())
}

pub fn build_runtime_snapshot_artifact_json_payload(
    snapshot: &RuntimeSnapshotCliState,
    metadata: &RuntimeSnapshotArtifactMetadata,
) -> CliResult<Value> {
    let base_payload = build_runtime_snapshot_cli_json_payload(snapshot)?;
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
        snapshot_id: format!("{:x}", Sha256::digest(serialized)),
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
pub fn run_channels_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let inventory = mvp::channel::channel_inventory(&config);
    let resolved_path_display = resolved_path.display().to_string();

    if as_json {
        let payload = build_channels_cli_json_payload(&resolved_path_display, &inventory);
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize channel status output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_channel_surfaces_text(&resolved_path_display, &inventory)
    );
    Ok(())
}

pub const CHANNELS_CLI_JSON_SCHEMA_VERSION: u32 = 1;
pub const CHANNELS_CLI_JSON_LEGACY_VIEWS: &[&str] = &["channels", "catalog_only_channels"];

pub fn build_channels_cli_json_payload(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> ChannelsCliJsonPayload {
    gateway::read_models::build_channel_inventory_read_model(config_path, inventory)
}

pub fn render_channel_surfaces_text(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> String {
    let mut lines = vec![format!("config={config_path}")];
    let mut catalog_only_surfaces = Vec::new();

    for surface in &inventory.channel_surfaces {
        if surface.catalog.implementation_status
            == mvp::channel::ChannelCatalogImplementationStatus::Stub
        {
            catalog_only_surfaces.push(surface);
            continue;
        }

        push_channel_surface_header(&mut lines, surface);
        lines.push(render_channel_onboarding_line(&surface.catalog.onboarding));
        for snapshot in &surface.configured_accounts {
            let api_base_url = snapshot.api_base_url.as_deref().unwrap_or("-");
            lines.push(format!(
                "  account configured_account={} configured_account_label={} default_account={} default_source={} compiled={} enabled={} api_base_url={}",
                snapshot.configured_account_id,
                snapshot.configured_account_label,
                snapshot.is_default_account,
                snapshot.default_account_source.as_str(),
                snapshot.compiled,
                snapshot.enabled,
                api_base_url
            ));
            for note in &snapshot.notes {
                lines.push(format!("    note: {note}"));
            }
            for operation in &snapshot.operations {
                let catalog_operation = surface.catalog.operation(operation.id);
                let requirement_ids = catalog_operation
                    .map(|catalog_operation| {
                        render_channel_operation_requirement_ids(catalog_operation.requirements)
                    })
                    .unwrap_or_else(|| "-".to_owned());
                lines.push(format!(
                    "    op {} ({}) {}: {} target_kinds={} requirements={}",
                    operation.id,
                    operation.command,
                    operation.health.as_str(),
                    operation.detail,
                    render_channel_target_kind_ids(
                        catalog_operation
                            .map(|catalog_operation| catalog_operation.supported_target_kinds)
                            .unwrap_or(&[])
                    ),
                    requirement_ids,
                ));
                if let Some(runtime) = &operation.runtime {
                    lines.push(format!(
                        "      runtime account={} account_id={} running={} stale={} busy={} active_runs={} instance_count={} running_instances={} stale_instances={} last_run_activity_at={} last_heartbeat_at={} pid={}",
                        runtime
                            .account_label
                            .as_deref()
                            .unwrap_or("-"),
                        runtime
                            .account_id
                            .as_deref()
                            .unwrap_or("-"),
                        runtime.running,
                        runtime.stale,
                        runtime.busy,
                        runtime.active_runs,
                        runtime.instance_count,
                        runtime.running_instances,
                        runtime.stale_instances,
                        runtime
                            .last_run_activity_at
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_owned()),
                        runtime
                            .last_heartbeat_at
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_owned()),
                        runtime
                            .pid
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_owned())
                    ));
                }
                for issue in &operation.issues {
                    lines.push(format!("      issue: {issue}"));
                }
            }
        }
    }

    if !catalog_only_surfaces.is_empty() {
        lines.push("catalog-only channels:".to_owned());
        for surface in catalog_only_surfaces {
            push_channel_surface_header(&mut lines, surface);
            lines.push(render_channel_onboarding_line(&surface.catalog.onboarding));
            for operation in &surface.catalog.operations {
                lines.push(format!(
                    "  catalog op {} ({}) availability={} tracks_runtime={} target_kinds={} requirements={}",
                    operation.id,
                    operation.command,
                    operation.availability.as_str(),
                    operation.tracks_runtime,
                    render_channel_target_kind_ids(operation.supported_target_kinds),
                    render_channel_operation_requirement_ids(operation.requirements)
                ));
            }
        }
    }
    lines.join("\n")
}

pub fn render_channel_onboarding_line(
    onboarding: &mvp::channel::ChannelOnboardingDescriptor,
) -> String {
    format!(
        "  onboarding strategy={} status_command=\"{}\" repair_command={} setup_hint=\"{}\"",
        onboarding.strategy.as_str(),
        onboarding.status_command,
        onboarding
            .repair_command
            .map(|command| format!("\"{command}\""))
            .unwrap_or_else(|| "-".to_owned()),
        onboarding.setup_hint
    )
}

pub fn render_channel_operation_requirement_ids(
    requirements: &[mvp::channel::ChannelCatalogOperationRequirement],
) -> String {
    if requirements.is_empty() {
        return "-".to_owned();
    }
    requirements
        .iter()
        .map(|requirement| requirement.id)
        .collect::<Vec<_>>()
        .join(",")
}

pub fn render_channel_target_kind_ids(
    target_kinds: &[mvp::channel::ChannelCatalogTargetKind],
) -> String {
    if target_kinds.is_empty() {
        return "-".to_owned();
    }
    target_kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn push_channel_surface_header(
    lines: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
) {
    let aliases = if surface.catalog.aliases.is_empty() {
        "-".to_owned()
    } else {
        surface.catalog.aliases.join(",")
    };
    let capabilities = if surface.catalog.capabilities.is_empty() {
        "-".to_owned()
    } else {
        surface
            .catalog
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    let target_kinds = render_channel_target_kind_ids(&surface.catalog.supported_target_kinds);
    lines.push(format!(
        "{} [{}] implementation_status={} selection_order={} selection_label=\"{}\" capabilities={} aliases={} transport={} target_kinds={} configured_accounts={} default_configured_account={}",
        surface.catalog.label,
        surface.catalog.id,
        surface.catalog.implementation_status.as_str(),
        surface.catalog.selection_order,
        surface.catalog.selection_label,
        capabilities,
        aliases,
        surface.catalog.transport,
        target_kinds,
        surface.configured_accounts.len(),
        surface
            .default_configured_account_id
            .as_deref()
            .unwrap_or("-")
    ));
    lines.push(format!("  blurb: {}", surface.catalog.blurb));
}

pub fn run_list_context_engines_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::conversation::collect_context_engine_runtime_snapshot(&config)?;

    if as_json {
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "selected": context_engine_metadata_json(
                &snapshot.selected_metadata,
                Some(snapshot.selected.source.as_str())
            ),
            "available": snapshot
                .available
                .iter()
                .map(|metadata| context_engine_metadata_json(metadata, None))
                .collect::<Vec<_>>(),
            "compaction": {
                "enabled": snapshot.compaction.enabled,
                "min_messages": snapshot.compaction.min_messages,
                "trigger_estimated_tokens": snapshot.compaction.trigger_estimated_tokens,
                "fail_open": snapshot.compaction.fail_open,
            },
        });
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize context-engine output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "selected={} source={} api_version={} capabilities={}",
        snapshot.selected_metadata.id,
        snapshot.selected.source.as_str(),
        snapshot.selected_metadata.api_version,
        format_capability_names(&snapshot.selected_metadata.capability_names())
    );
    println!(
        "compaction=enabled:{} min_messages:{} trigger_estimated_tokens:{} fail_open:{}",
        snapshot.compaction.enabled,
        snapshot
            .compaction
            .min_messages
            .map_or_else(|| "(none)".to_owned(), |value| value.to_string()),
        snapshot
            .compaction
            .trigger_estimated_tokens
            .map_or_else(|| "(none)".to_owned(), |value| value.to_string()),
        snapshot.compaction.fail_open
    );
    println!("available:");
    for metadata in snapshot.available {
        println!(
            "- {} api_version={} capabilities={}",
            metadata.id,
            metadata.api_version,
            format_capability_names(&metadata.capability_names())
        );
    }
    Ok(())
}

pub fn run_list_memory_systems_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::memory::collect_memory_system_runtime_snapshot(&config)?;

    if as_json {
        let payload =
            build_memory_systems_cli_json_payload(&resolved_path.display().to_string(), &snapshot);
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize memory-system output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_memory_system_snapshot_text(&resolved_path.display().to_string(), &snapshot)
    );
    Ok(())
}

pub fn run_list_acp_backends_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::acp::collect_acp_runtime_snapshot(&config)?;

    if as_json {
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "enabled": snapshot.control_plane.enabled,
            "selected": acp_backend_metadata_json(
                &snapshot.selected_metadata,
                Some(snapshot.selected.source.as_str())
            ),
            "available": snapshot
                .available
                .iter()
                .map(|metadata| acp_backend_metadata_json(metadata, None))
                .collect::<Vec<_>>(),
            "control_plane": acp_control_plane_json(&snapshot.control_plane),
        });
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP backend output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "enabled={} selected={} source={} api_version={} capabilities={}",
        snapshot.control_plane.enabled,
        snapshot.selected_metadata.id,
        snapshot.selected.source.as_str(),
        snapshot.selected_metadata.api_version,
        format_capability_names(&snapshot.selected_metadata.capability_names())
    );
    println!(
        "control_plane=dispatch_enabled:{} conversation_routing:{} allowed_channels:{} allowed_account_ids:{} bootstrap_mcp_servers:{} working_directory:{} thread_routing:{} default_agent:{} allowed_agents:{} max_concurrent_sessions:{} session_idle_ttl_ms:{} startup_timeout_ms:{} turn_timeout_ms:{} queue_owner_ttl_ms:{} bindings_enabled:{} emit_runtime_events:{} allow_mcp_server_injection:{}",
        snapshot.control_plane.dispatch_enabled,
        snapshot.control_plane.conversation_routing.as_str(),
        snapshot.control_plane.allowed_channels.join(","),
        snapshot.control_plane.allowed_account_ids.join(","),
        snapshot.control_plane.bootstrap_mcp_servers.join(","),
        snapshot
            .control_plane
            .working_directory
            .as_deref()
            .unwrap_or(""),
        snapshot.control_plane.thread_routing.as_str(),
        snapshot.control_plane.default_agent,
        snapshot.control_plane.allowed_agents.join(","),
        snapshot.control_plane.max_concurrent_sessions,
        snapshot.control_plane.session_idle_ttl_ms,
        snapshot.control_plane.startup_timeout_ms,
        snapshot.control_plane.turn_timeout_ms,
        snapshot.control_plane.queue_owner_ttl_ms,
        snapshot.control_plane.bindings_enabled,
        snapshot.control_plane.emit_runtime_events,
        snapshot.control_plane.allow_mcp_server_injection
    );
    println!("available:");
    for metadata in snapshot.available {
        println!(
            "- {} api_version={} capabilities={} summary={}",
            metadata.id,
            metadata.api_version,
            format_capability_names(&metadata.capability_names()),
            metadata.summary
        );
    }
    Ok(())
}

pub fn run_list_acp_sessions_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    #[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
    {
        let _ = (config_path, as_json);
        Err("ACP session persistence requires feature `memory-sqlite`".to_owned())
    }

    #[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
    {
        let (resolved_path, config) = mvp::config::load(config_path)?;
        let store =
            mvp::acp::AcpSqliteSessionStore::new(Some(config.memory.resolved_sqlite_path()));
        let sessions = mvp::acp::AcpSessionStore::list(&store)?;

        if as_json {
            let payload = json!({
                "config": resolved_path.display().to_string(),
                "sqlite_path": config.memory.resolved_sqlite_path().display().to_string(),
                "sessions": sessions
                    .iter()
                    .map(acp_session_metadata_json)
                    .collect::<Vec<_>>(),
            });
            let pretty = serde_json::to_string_pretty(&payload)
                .map_err(|error| format!("serialize ACP session output failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }

        println!(
            "config={} sqlite_path={}",
            resolved_path.display(),
            config.memory.resolved_sqlite_path().display()
        );
        if sessions.is_empty() {
            println!("sessions: (none)");
            return Ok(());
        }
        println!("sessions:");
        for session in sessions {
            println!(
                "- session_key={} backend={} conversation_id={} binding_route_session_id={} activation_origin={} state={} mode={} runtime_session_name={} last_activity_ms={} last_error={}",
                session.session_key,
                session.backend_id,
                session.conversation_id.as_deref().unwrap_or("(none)"),
                session
                    .binding
                    .as_ref()
                    .map(|binding| binding.route_session_id.as_str())
                    .unwrap_or("(none)"),
                session
                    .activation_origin
                    .map(mvp::acp::AcpRoutingOrigin::as_str)
                    .unwrap_or("(none)"),
                acp_session_state_label(session.state),
                session.mode.map(acp_session_mode_label).unwrap_or("(none)"),
                session.runtime_session_name,
                session.last_activity_ms,
                session.last_error.as_deref().unwrap_or("(none)")
            );
        }
        Ok(())
    }
}

pub async fn run_acp_doctor_cli(
    config_path: Option<&str>,
    backend_id: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let selection = mvp::acp::resolve_acp_backend_selection(&config);
    let backend = backend_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(selection.id.as_str());
    let report = mvp::acp::AcpSessionManager::default()
        .doctor(&config, Some(backend))
        .await?;

    if as_json {
        let payload = acp_doctor_json(
            resolved_path.display().to_string(),
            selection.id.as_str(),
            backend,
            &report,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP doctor output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "selected_backend={} requested_backend={} healthy={}",
        backend, backend, report.healthy
    );
    if report.diagnostics.is_empty() {
        println!("diagnostics: (none)");
        return Ok(());
    }
    println!("diagnostics:");
    for (key, value) in report.diagnostics {
        println!("- {}={}", key, value);
    }
    Ok(())
}

pub fn acp_doctor_json(
    config_path: impl Into<String>,
    _default_backend: &str,
    effective_backend: &str,
    report: &mvp::acp::AcpDoctorReport,
) -> Value {
    json!({
        "config": config_path.into(),
        "selected_backend": effective_backend,
        "requested_backend": effective_backend,
        "healthy": report.healthy,
        "diagnostics": report.diagnostics,
    })
}

pub async fn run_acp_status_cli(
    config_path: Option<&str>,
    session_key: Option<&str>,
    conversation_id: Option<&str>,
    route_session_id: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let resolved_session_key =
        resolve_acp_status_session_key(&config, session_key, conversation_id, route_session_id)?;
    let manager = mvp::acp::shared_acp_session_manager(&config)?;
    let status = manager
        .get_status(&config, resolved_session_key.as_str())
        .await?;

    if as_json {
        let config_display = resolved_path.display().to_string();
        let payload = gateway::read_models::build_acp_status_read_model(
            config_display.as_str(),
            session_key,
            conversation_id,
            route_session_id,
            resolved_session_key.as_str(),
            &status,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP status output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    if let Some(conversation_id) = conversation_id {
        println!("requested_conversation_id={conversation_id}");
    }
    if let Some(route_session_id) = route_session_id {
        println!("requested_route_session_id={route_session_id}");
    }
    if let Some(session_key) = session_key {
        println!("requested_session={session_key}");
    }
    println!("resolved_session_key={}", resolved_session_key);
    println!(
        "status=backend:{} state:{} mode:{} pending_turns:{} active_turn_id:{} conversation_id:{} binding_route_session_id:{} activation_origin:{} last_activity_ms:{} last_error:{}",
        status.backend_id,
        acp_session_state_label(status.state),
        status.mode.map(acp_session_mode_label).unwrap_or("(none)"),
        status.pending_turns,
        status.active_turn_id.as_deref().unwrap_or("(none)"),
        status.conversation_id.as_deref().unwrap_or("(none)"),
        status
            .binding
            .as_ref()
            .map(|binding| binding.route_session_id.as_str())
            .unwrap_or("(none)"),
        status
            .activation_origin
            .map(mvp::acp::AcpRoutingOrigin::as_str)
            .unwrap_or("(none)"),
        status.last_activity_ms,
        status.last_error.as_deref().unwrap_or("(none)")
    );
    Ok(())
}

pub async fn run_acp_observability_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let manager = mvp::acp::shared_acp_session_manager(&config)?;
    let snapshot = manager.observability_snapshot(&config).await?;

    if as_json {
        let config_display = resolved_path.display().to_string();
        let payload = gateway::read_models::build_acp_observability_read_model(
            config_display.as_str(),
            &snapshot,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP observability output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "runtime_cache=active_sessions:{} idle_ttl_ms:{} evicted_total:{} last_evicted_at_ms:{}",
        snapshot.runtime_cache.active_sessions,
        snapshot.runtime_cache.idle_ttl_ms,
        snapshot.runtime_cache.evicted_total,
        snapshot
            .runtime_cache
            .last_evicted_at_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "(none)".to_owned())
    );
    println!(
        "sessions=bound:{} unbound:{} activation_origins:{} backends:{}",
        snapshot.sessions.bound,
        snapshot.sessions.unbound,
        format_usize_rollup(&snapshot.sessions.activation_origin_counts),
        format_usize_rollup(&snapshot.sessions.backend_counts)
    );
    println!(
        "actors=active:{} queue_depth:{} waiting:{}",
        snapshot.actors.active, snapshot.actors.queue_depth, snapshot.actors.waiting
    );
    println!(
        "turns=active:{} queue_depth:{} completed:{} failed:{} average_latency_ms:{} max_latency_ms:{}",
        snapshot.turns.active,
        snapshot.turns.queue_depth,
        snapshot.turns.completed,
        snapshot.turns.failed,
        snapshot.turns.average_latency_ms,
        snapshot.turns.max_latency_ms
    );
    if snapshot.errors_by_code.is_empty() {
        println!("errors_by_code: (none)");
    } else {
        println!("errors_by_code:");
        for (key, value) in snapshot.errors_by_code {
            println!("- {}={}", key, value);
        }
    }
    Ok(())
}

pub fn resolve_acp_status_session_key(
    config: &mvp::config::LoongClawConfig,
    session_key: Option<&str>,
    conversation_id: Option<&str>,
    route_session_id: Option<&str>,
) -> CliResult<String> {
    let session_key = session_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let route_session_id = route_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    match (session_key, conversation_id, route_session_id) {
        (Some(session_key), None, None) => Ok(session_key),
        (None, Some(conversation_id), None) => {
            #[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
            {
                let _ = (config, conversation_id);
                Err("ACP conversation-id lookup requires feature `memory-sqlite`".to_owned())
            }

            #[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
            {
                let store = mvp::acp::AcpSqliteSessionStore::new(Some(
                    config.memory.resolved_sqlite_path(),
                ));
                let metadata = mvp::acp::AcpSessionStore::get_by_conversation_id(
                    &store,
                    conversation_id.as_str(),
                )?
                .ok_or_else(|| {
                    format!(
                        "ACP conversation `{}` is not registered in {}",
                        conversation_id,
                        config.memory.resolved_sqlite_path().display()
                    )
                })?;
                Ok(metadata.session_key)
            }
        }
        (None, None, Some(route_session_id)) => {
            #[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
            {
                let _ = (config, route_session_id);
                Err("ACP route-session-id lookup requires feature `memory-sqlite`".to_owned())
            }

            #[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
            {
                let store = mvp::acp::AcpSqliteSessionStore::new(Some(
                    config.memory.resolved_sqlite_path(),
                ));
                let metadata = mvp::acp::AcpSessionStore::get_by_binding_route_session_id(
                    &store,
                    route_session_id.as_str(),
                )?
                .ok_or_else(|| {
                    format!(
                        "ACP route session `{}` is not registered in {}",
                        route_session_id,
                        config.memory.resolved_sqlite_path().display()
                    )
                })?;
                Ok(metadata.session_key)
            }
        }
        (Some(_), Some(_), _)
        | (Some(_), _, Some(_))
        | (_, Some(_), Some(_)) => Err(
            "acp-status accepts exactly one of --session, --conversation-id, or --route-session-id"
                .to_owned(),
        ),
        (None, None, None) => Err(
            "acp-status requires --session <session_key>, --conversation-id <conversation_id>, or --route-session-id <route_session_id>"
                .to_owned(),
        ),
    }
}

pub async fn run_chat_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> CliResult<()> {
    let options = build_cli_chat_options(acp, acp_event_stream, acp_bootstrap_mcp_server, acp_cwd);
    mvp::chat::run_cli_chat(config_path, session, &options).await
}

pub async fn run_ask_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    message: &str,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> CliResult<()> {
    let options = build_cli_chat_options(acp, acp_event_stream, acp_bootstrap_mcp_server, acp_cwd);
    mvp::chat::run_cli_ask(config_path, session, message, &options).await
}

pub fn build_cli_chat_options(
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> mvp::chat::CliChatOptions {
    mvp::chat::CliChatOptions {
        acp_requested: acp,
        acp_event_stream,
        acp_bootstrap_mcp_servers: acp_bootstrap_mcp_server.to_vec(),
        acp_working_directory: acp_cwd
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from),
    }
}

pub fn run_acp_event_summary_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    limit: usize,
    as_json: bool,
) -> CliResult<()> {
    if limit == 0 {
        return Err("acp-event-summary limit must be >= 1".to_owned());
    }

    let (_, config) = mvp::config::load(config_path)?;
    let session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();

    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let turns = mvp::memory::window_direct(&session_id, limit, &mem_config)
            .map_err(|error| format!("load ACP event summary failed: {error}"))?;
        let summary = mvp::acp::summarize_turn_events(
            turns
                .iter()
                .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str())),
        );
        if as_json {
            let payload = acp_event_summary_json(&session_id, limit, &summary);
            let pretty = serde_json::to_string_pretty(&payload)
                .map_err(|error| format!("serialize ACP event summary failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }
        print!("{}", format_acp_event_summary(&session_id, limit, &summary));
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, session_id, as_json);
        Err("acp-event-summary requires memory-sqlite feature".to_owned())
    }
}

pub fn run_acp_dispatch_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    channel: Option<&str>,
    conversation_id: Option<&str>,
    account_id: Option<&str>,
    thread_id: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();
    let address = build_acp_dispatch_address(
        session_id.as_str(),
        channel,
        conversation_id,
        account_id,
        thread_id,
    )?;
    let decision = mvp::acp::evaluate_acp_conversation_dispatch_for_address(&config, &address)?;

    if as_json {
        let config_display = resolved_path.display().to_string();
        let payload = gateway::read_models::build_acp_dispatch_read_model(
            config_display.as_str(),
            &address,
            session_id.as_str(),
            &decision,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP dispatch output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "address=session:{} channel:{} account_id:{} conversation_id:{} thread_id:{}",
        address.session_id,
        address.channel_id.as_deref().unwrap_or("(none)"),
        address.account_id.as_deref().unwrap_or("(none)"),
        address.conversation_id.as_deref().unwrap_or("(none)"),
        address.thread_id.as_deref().unwrap_or("(none)")
    );
    println!(
        "dispatch=route_via_acp:{} reason:{} automatic_routing_origin:{} route_session_id:{} prefixed_agent_id:{} channel_id:{} account_id:{} conversation_id:{} thread_id:{}",
        decision.route_via_acp,
        decision.reason.as_str(),
        decision
            .automatic_routing_origin
            .map(mvp::acp::AcpRoutingOrigin::as_str)
            .unwrap_or("(none)"),
        decision.target.route_session_id,
        decision
            .target
            .prefixed_agent_id
            .as_deref()
            .unwrap_or("(none)"),
        decision.target.channel_id.as_deref().unwrap_or("(none)"),
        decision.target.account_id.as_deref().unwrap_or("(none)"),
        decision
            .target
            .conversation_id
            .as_deref()
            .unwrap_or("(none)"),
        decision.target.thread_id.as_deref().unwrap_or("(none)")
    );
    println!(
        "channel_path={}",
        if decision.target.channel_path.is_empty() {
            "(none)".to_owned()
        } else {
            decision.target.channel_path.join(":")
        }
    );
    Ok(())
}

pub fn build_acp_dispatch_address(
    session_id: &str,
    channel: Option<&str>,
    conversation_id: Option<&str>,
    account_id: Option<&str>,
    thread_id: Option<&str>,
) -> CliResult<mvp::conversation::ConversationSessionAddress> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("acp-dispatch requires a non-empty --session value".to_owned());
    }

    let channel = channel.map(str::trim).filter(|value| !value.is_empty());
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let account_id = account_id.map(str::trim).filter(|value| !value.is_empty());
    let thread_id = thread_id.map(str::trim).filter(|value| !value.is_empty());

    let channel = match channel {
        Some(channel) => channel,
        None => {
            if conversation_id.is_some() || account_id.is_some() || thread_id.is_some() {
                return Err(
                    "acp-dispatch requires --channel when using --conversation-id, --account-id, or --thread-id"
                        .to_owned(),
                );
            }
            return Ok(mvp::conversation::ConversationSessionAddress::from_session_id(session_id));
        }
    };

    let conversation_id = conversation_id.ok_or_else(|| {
        "acp-dispatch requires --conversation-id when --channel is provided".to_owned()
    })?;
    let mut address = mvp::conversation::ConversationSessionAddress::from_session_id(session_id)
        .with_channel_scope(channel, conversation_id);
    if let Some(account_id) = account_id {
        address = address.with_account_id(account_id);
    }
    if let Some(thread_id) = thread_id {
        address = address.with_thread_id(thread_id);
    }
    Ok(address)
}

pub fn run_safe_lane_summary_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    limit: usize,
    as_json: bool,
) -> CliResult<()> {
    if limit == 0 {
        return Err("safe-lane-summary limit must be >= 1".to_owned());
    }

    let (_, config) = mvp::config::load(config_path)?;
    let session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();

    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let turns = mvp::memory::window_direct(&session_id, limit, &mem_config)
            .map_err(|error| format!("load safe-lane summary failed: {error}"))?;
        let summary = mvp::conversation::summarize_safe_lane_events(
            turns
                .iter()
                .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str())),
        );
        if as_json {
            let payload = json!({
                "session": session_id,
                "limit": limit,
                "summary": summary,
            });
            let pretty = serde_json::to_string_pretty(&payload)
                .map_err(|error| format!("serialize safe-lane summary failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }

        let final_status = match summary.final_status {
            Some(mvp::conversation::SafeLaneFinalStatus::Succeeded) => "succeeded",
            Some(mvp::conversation::SafeLaneFinalStatus::Failed) => "failed",
            None => "unknown",
        };
        println!("safe_lane_summary session={} limit={}", session_id, limit);
        println!(
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
        );
        println!(
            "terminal status={} failure_code={} route_decision={} route_reason={}",
            final_status,
            summary.final_failure_code.as_deref().unwrap_or("-"),
            summary.final_route_decision.as_deref().unwrap_or("-"),
            summary.final_route_reason.as_deref().unwrap_or("-")
        );
        let route_reasons_rollup = if summary.route_reason_counts.is_empty() {
            "-".to_owned()
        } else {
            summary
                .route_reason_counts
                .iter()
                .map(|(key, value)| format!("{key}:{value}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        println!(
            "governor trigger_failed_threshold={} trigger_backpressure_threshold={} trigger_trend_threshold={} trigger_recovery_threshold={}",
            summary.session_governor_failed_threshold_triggered_events,
            summary.session_governor_backpressure_threshold_triggered_events,
            summary.session_governor_trend_threshold_triggered_events,
            summary.session_governor_recovery_threshold_triggered_events
        );
        println!(
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
            format_milli_ratio(summary.session_governor_latest_trend_failure_ewma_milli),
            format_milli_ratio(summary.session_governor_latest_trend_backpressure_ewma_milli),
            summary
                .session_governor_latest_recovery_success_streak
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_recovery_success_streak_threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned())
        );
        println!("rollup route_reasons={route_reasons_rollup}");
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, session_id, as_json);
        Err("safe-lane-summary requires memory-sqlite feature".to_owned())
    }
}

#[cfg(feature = "memory-sqlite")]
pub fn format_milli_ratio(value: Option<u32>) -> String {
    value
        .map(|raw| format!("{:.3}", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
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

pub const TELEGRAM_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_telegram_send_cli_impl,
};

pub const FEISHU_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_feishu_send_cli_impl,
};

pub const MATRIX_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_matrix_send_cli_impl,
};

pub const WECOM_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_wecom_send_cli_impl,
};

pub const DISCORD_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::DISCORD_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_discord_send_cli_impl,
};

pub const DINGTALK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::DINGTALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_dingtalk_send_cli_impl,
};

pub const SLACK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::SLACK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_slack_send_cli_impl,
};

pub const LINE_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_line_send_cli_impl,
};

pub const WHATSAPP_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_whatsapp_send_cli_impl,
};

pub const EMAIL_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::EMAIL_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_email_send_cli_impl,
};

pub const WEBHOOK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_webhook_send_cli_impl,
};

pub const GOOGLE_CHAT_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::GOOGLE_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_google_chat_send_cli_impl,
};

pub const TEAMS_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::TEAMS_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_teams_send_cli_impl,
};

pub const SIGNAL_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::SIGNAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_signal_send_cli_impl,
};

pub const TWITCH_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::TWITCH_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_twitch_send_cli_impl,
};

pub const MATTERMOST_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::MATTERMOST_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_mattermost_send_cli_impl,
};

pub const NEXTCLOUD_TALK_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::NEXTCLOUD_TALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_nextcloud_talk_send_cli_impl,
};

pub const SYNOLOGY_CHAT_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::SYNOLOGY_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_synology_chat_send_cli_impl,
};

pub const IRC_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::IRC_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_irc_send_cli_impl,
};

pub const IMESSAGE_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::IMESSAGE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_imessage_send_cli_impl,
};

pub const NOSTR_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: mvp::channel::NOSTR_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_nostr_send_cli_impl,
};

pub const TELEGRAM_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
    run: run_telegram_serve_cli_impl,
};

pub const FEISHU_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::FEISHU_COMMAND_FAMILY_DESCRIPTOR,
    run: run_feishu_serve_cli_impl,
};

pub const MATRIX_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::MATRIX_COMMAND_FAMILY_DESCRIPTOR,
    run: run_matrix_serve_cli_impl,
};

pub const WECOM_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::WECOM_COMMAND_FAMILY_DESCRIPTOR,
    run: run_wecom_serve_cli_impl,
};

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
        let target = args.target.unwrap_or_default();
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

pub fn run_feishu_serve_cli_impl(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        with_graceful_shutdown(mvp::channel::run_feishu_channel(
            args.config_path,
            args.account,
            args.bind_override,
            args.path_override,
        ))
        .await
    })
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

pub fn parse_json_payload(raw: &str, context: &str) -> CliResult<Value> {
    serde_json::from_str(raw).map_err(|error| format!("invalid JSON for {context}: {error}"))
}

pub fn build_runtime_snapshot_cli_json_payload(
    snapshot: &RuntimeSnapshotCliState,
) -> CliResult<Value> {
    let read_model = gateway::read_models::build_runtime_snapshot_read_model(snapshot);
    let payload = serde_json::to_value(read_model)
        .map_err(|error| format!("serialize runtime snapshot read model failed: {error}"))?;
    Ok(payload)
}

pub fn render_runtime_snapshot_text(snapshot: &RuntimeSnapshotCliState) -> String {
    let mut lines = vec![
        format!("config={}", snapshot.config),
        format!(
            "provider active_profile={} active_label=\"{}\" last_provider={}",
            snapshot.provider.active_profile_id,
            snapshot.provider.active_label,
            snapshot.provider.last_provider_id.as_deref().unwrap_or("-")
        ),
        format!(
            "provider saved_profiles={}",
            render_string_list(
                snapshot
                    .provider
                    .saved_profile_ids
                    .iter()
                    .map(String::as_str)
            )
        ),
    ];

    for profile in &snapshot.provider.profiles {
        lines.push(format!(
            "  profile {} active={} default_for_kind={} kind={} model={} wire_api={} credential_resolved={} auth_env={} endpoint={} models_endpoint={} temperature={} max_tokens={} timeout_ms={} retries={} headers={} preferred_models={}",
            profile.profile_id,
            profile.is_active,
            profile.default_for_kind,
            profile.kind.as_str(),
            profile.model,
            profile.wire_api.as_str(),
            profile.credential_resolved,
            profile.auth_env.as_deref().unwrap_or("-"),
            profile.endpoint,
            profile.models_endpoint,
            profile.temperature,
            profile
                .max_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            profile.request_timeout_ms,
            profile.retry_max_attempts,
            render_string_list(profile.header_names.iter().map(String::as_str)),
            render_string_list(profile.preferred_models.iter().map(String::as_str))
        ));
    }

    lines.push(format!(
        "context_engine selected={} source={} api_version={} capabilities={}",
        snapshot.context_engine.selected_metadata.id,
        snapshot.context_engine.selected.source.as_str(),
        snapshot.context_engine.selected_metadata.api_version,
        format_capability_names(&snapshot.context_engine.selected_metadata.capability_names())
    ));
    lines.push(format!(
        "context_engine compaction=enabled:{} min_messages:{} trigger_estimated_tokens:{} fail_open:{}",
        snapshot.context_engine.compaction.enabled,
        snapshot
            .context_engine
            .compaction
            .min_messages
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        snapshot
            .context_engine
            .compaction
            .trigger_estimated_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        snapshot.context_engine.compaction.fail_open
    ));
    lines.push(format!(
        "memory selected={} source={} api_version={} capabilities={} summary={}",
        snapshot.memory_system.selected_metadata.id,
        snapshot.memory_system.selected.source.as_str(),
        snapshot.memory_system.selected_metadata.api_version,
        format_capability_names(&snapshot.memory_system.selected_metadata.capability_names()),
        snapshot.memory_system.selected_metadata.summary
    ));
    lines.push(format!(
        "memory policy=backend:{} profile:{} mode:{} ingest_mode:{} fail_open:{} strict_mode_requested:{} strict_mode_active:{} effective_fail_open:{}",
        snapshot.memory_system.policy.backend.as_str(),
        snapshot.memory_system.policy.profile.as_str(),
        snapshot.memory_system.policy.mode.as_str(),
        snapshot.memory_system.policy.ingest_mode.as_str(),
        snapshot.memory_system.policy.fail_open,
        snapshot.memory_system.policy.strict_mode_requested,
        snapshot.memory_system.policy.strict_mode_active,
        snapshot.memory_system.policy.effective_fail_open
    ));
    lines.push(format!(
        "acp enabled={} selected={} source={} api_version={} capabilities={} dispatch_enabled={} routing={} thread_routing={} default_agent={} allowed_agents={} allowed_channels={} allowed_account_ids={} bootstrap_mcp_servers={} working_directory={}",
        snapshot.acp.control_plane.enabled,
        snapshot.acp.selected_metadata.id,
        snapshot.acp.selected.source.as_str(),
        snapshot.acp.selected_metadata.api_version,
        format_capability_names(&snapshot.acp.selected_metadata.capability_names()),
        snapshot.acp.control_plane.dispatch_enabled,
        snapshot.acp.control_plane.conversation_routing.as_str(),
        snapshot.acp.control_plane.thread_routing.as_str(),
        snapshot.acp.control_plane.default_agent,
        render_string_list(snapshot.acp.control_plane.allowed_agents.iter().map(String::as_str)),
        render_string_list(snapshot.acp.control_plane.allowed_channels.iter().map(String::as_str)),
        render_string_list(
            snapshot
                .acp
                .control_plane
                .allowed_account_ids
                .iter()
                .map(String::as_str)
        ),
        render_string_list(
            snapshot
                .acp
                .control_plane
                .bootstrap_mcp_servers
                .iter()
                .map(String::as_str)
        ),
        snapshot
            .acp
            .control_plane
            .working_directory
            .as_deref()
            .unwrap_or("-")
    ));
    lines.push(format!(
        "channels enabled={} service_enabled={} configured_accounts={} surfaces={}",
        render_string_list(snapshot.enabled_channel_ids.iter().map(String::as_str)),
        render_string_list(
            snapshot
                .enabled_service_channel_ids
                .iter()
                .map(String::as_str)
        ),
        snapshot.channels.channels.len(),
        snapshot.channels.channel_surfaces.len()
    ));
    for surface in &snapshot.channels.channel_surfaces {
        lines.push(format!(
            "  channel {} implementation_status={} configured_accounts={} default_configured_account={} aliases={}",
            surface.catalog.id,
            surface.catalog.implementation_status.as_str(),
            surface.configured_accounts.len(),
            surface
                .default_configured_account_id
                .as_deref()
                .unwrap_or("-"),
            render_string_list(surface.catalog.aliases.iter().copied())
        ));
    }
    lines.push(format!(
        "tool_runtime shell_default={} shell_allow={} shell_deny={} sessions_enabled={} messages_enabled={} delegate_enabled={}",
        shell_policy_default_str(snapshot.tool_runtime.shell_default_mode),
        render_string_list(snapshot.tool_runtime.shell_allow.iter().map(String::as_str)),
        render_string_list(snapshot.tool_runtime.shell_deny.iter().map(String::as_str)),
        snapshot.tool_runtime.sessions_enabled,
        snapshot.tool_runtime.messages_enabled,
        snapshot.tool_runtime.delegate_enabled
    ));
    lines.push(format!(
        "tool_runtime browser enabled={} tier={} max_sessions={} max_links={} max_text_chars={}",
        snapshot.tool_runtime.browser.enabled,
        snapshot.tool_runtime.browser_execution_security_tier(),
        snapshot.tool_runtime.browser.max_sessions,
        snapshot.tool_runtime.browser.max_links,
        snapshot.tool_runtime.browser.max_text_chars
    ));
    lines.push(format!(
        "tool_runtime browser_companion enabled={} ready={} tier={} command={} expected_version={}",
        snapshot.tool_runtime.browser_companion.enabled,
        snapshot.tool_runtime.browser_companion.ready,
        snapshot
            .tool_runtime
            .browser_companion_execution_security_tier(),
        snapshot
            .tool_runtime
            .browser_companion
            .command
            .as_deref()
            .unwrap_or("-"),
        snapshot
            .tool_runtime
            .browser_companion
            .expected_version
            .as_deref()
            .unwrap_or("-")
    ));
    lines.push(format!(
        "tool_runtime web_fetch enabled={} allow_private_hosts={} timeout_seconds={} max_bytes={} max_redirects={} allowed_domains={} blocked_domains={}",
        snapshot.tool_runtime.web_fetch.enabled,
        snapshot.tool_runtime.web_fetch.allow_private_hosts,
        snapshot.tool_runtime.web_fetch.timeout_seconds,
        snapshot.tool_runtime.web_fetch.max_bytes,
        snapshot.tool_runtime.web_fetch.max_redirects,
        render_string_list(snapshot.tool_runtime.web_fetch.allowed_domains.iter().map(String::as_str)),
        render_string_list(snapshot.tool_runtime.web_fetch.blocked_domains.iter().map(String::as_str))
    ));
    lines.push(format!(
        "tools visible_count={} capability_snapshot_sha256={} visible_names={}",
        snapshot.visible_tool_names.len(),
        snapshot.capability_snapshot_sha256,
        render_string_list(snapshot.visible_tool_names.iter().map(String::as_str))
    ));
    lines.push(format!(
        "external_skills inventory_status={} override_active={} enabled={} require_download_approval={} auto_expose_installed={} install_root={} resolved_skills={} shadowed_skills={} inventory_error={}",
        snapshot.external_skills.inventory_status.as_str(),
        snapshot.external_skills.override_active,
        snapshot.external_skills.policy.enabled,
        snapshot.external_skills.policy.require_download_approval,
        snapshot.external_skills.policy.auto_expose_installed,
        snapshot
            .external_skills
            .policy
            .install_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_owned()),
        snapshot.external_skills.resolved_skill_count,
        snapshot.external_skills.shadowed_skill_count,
        snapshot
            .external_skills
            .inventory_error
            .as_deref()
            .unwrap_or("-")
    ));

    if let Some(skills) = snapshot
        .external_skills
        .inventory
        .get("skills")
        .and_then(Value::as_array)
    {
        for skill in skills {
            lines.push(format!(
                "  external_skill {} scope={} active={} sha256={}",
                json_string_field(skill, "skill_id"),
                json_string_field(skill, "scope"),
                skill
                    .get("active")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                json_string_field(skill, "sha256")
            ));
        }
    }

    lines
        .into_iter()
        .chain([
            "capability_snapshot:".to_owned(),
            snapshot.capability_snapshot.clone(),
        ])
        .collect::<Vec<_>>()
        .join("\n")
}

fn runtime_snapshot_provider_json(snapshot: &RuntimeSnapshotProviderState) -> Value {
    json!({
        "active_profile_id": snapshot.active_profile_id,
        "active_label": snapshot.active_label,
        "last_provider_id": snapshot.last_provider_id,
        "saved_profile_ids": snapshot.saved_profile_ids,
        "profiles": snapshot
            .profiles
            .iter()
            .map(runtime_snapshot_provider_profile_json)
            .collect::<Vec<_>>(),
    })
}

fn runtime_snapshot_provider_profile_json(profile: &RuntimeSnapshotProviderProfileState) -> Value {
    json!({
        "profile_id": profile.profile_id,
        "is_active": profile.is_active,
        "default_for_kind": profile.default_for_kind,
        "kind": profile.kind.as_str(),
        "model": profile.model,
        "wire_api": profile.wire_api.as_str(),
        "base_url": profile.base_url,
        "endpoint": profile.endpoint,
        "models_endpoint": profile.models_endpoint,
        "protocol_family": profile.protocol_family,
        "credential_resolved": profile.credential_resolved,
        "auth_env": profile.auth_env,
        "reasoning_effort": profile.reasoning_effort,
        "temperature": profile.temperature,
        "max_tokens": profile.max_tokens,
        "request_timeout_ms": profile.request_timeout_ms,
        "retry_max_attempts": profile.retry_max_attempts,
        "header_names": profile.header_names,
        "preferred_models": profile.preferred_models,
    })
}

fn runtime_snapshot_context_engine_json(
    snapshot: &mvp::conversation::ContextEngineRuntimeSnapshot,
) -> Value {
    json!({
        "selected": context_engine_metadata_json(
            &snapshot.selected_metadata,
            Some(snapshot.selected.source.as_str())
        ),
        "available": snapshot
            .available
            .iter()
            .map(|metadata| context_engine_metadata_json(metadata, None))
            .collect::<Vec<_>>(),
        "compaction": {
            "enabled": snapshot.compaction.enabled,
            "min_messages": snapshot.compaction.min_messages,
            "trigger_estimated_tokens": snapshot.compaction.trigger_estimated_tokens,
            "fail_open": snapshot.compaction.fail_open,
        },
    })
}

fn runtime_snapshot_memory_system_json(
    snapshot: &mvp::memory::MemorySystemRuntimeSnapshot,
) -> Value {
    json!({
        "selected": memory_system_metadata_json(
            &snapshot.selected_metadata,
            Some(snapshot.selected.source.as_str())
        ),
        "available": snapshot
            .available
            .iter()
            .map(|metadata| memory_system_metadata_json(metadata, None))
            .collect::<Vec<_>>(),
        "policy": memory_system_policy_json(&snapshot.policy),
    })
}

fn runtime_snapshot_acp_json(snapshot: &mvp::acp::AcpRuntimeSnapshot) -> Value {
    json!({
        "enabled": snapshot.control_plane.enabled,
        "selected": acp_backend_metadata_json(
            &snapshot.selected_metadata,
            Some(snapshot.selected.source.as_str())
        ),
        "available": snapshot
            .available
            .iter()
            .map(|metadata| acp_backend_metadata_json(metadata, None))
            .collect::<Vec<_>>(),
        "control_plane": acp_control_plane_json(&snapshot.control_plane),
    })
}

fn runtime_snapshot_tool_runtime_json(
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Value {
    json!({
        "file_root": runtime
            .file_root
            .as_ref()
            .map(|path| path.display().to_string()),
        "shell": {
            "default_mode": shell_policy_default_str(runtime.shell_default_mode),
            "allow": runtime.shell_allow.iter().collect::<Vec<_>>(),
            "deny": runtime.shell_deny.iter().collect::<Vec<_>>(),
        },
        "sessions_enabled": runtime.sessions_enabled,
        "messages_enabled": runtime.messages_enabled,
        "delegate_enabled": runtime.delegate_enabled,
        "browser": {
            "enabled": runtime.browser.enabled,
            "execution_tier": runtime.browser_execution_security_tier().as_str(),
            "max_sessions": runtime.browser.max_sessions,
            "max_links": runtime.browser.max_links,
            "max_text_chars": runtime.browser.max_text_chars,
        },
        "browser_companion": {
            "enabled": runtime.browser_companion.enabled,
            "ready": runtime.browser_companion.ready,
            "execution_tier": runtime.browser_companion_execution_security_tier().as_str(),
            "command": runtime.browser_companion.command,
            "expected_version": runtime.browser_companion.expected_version,
        },
        "web_fetch": {
            "enabled": runtime.web_fetch.enabled,
            "allow_private_hosts": runtime.web_fetch.allow_private_hosts,
            "allowed_domains": runtime.web_fetch.allowed_domains.iter().collect::<Vec<_>>(),
            "blocked_domains": runtime.web_fetch.blocked_domains.iter().collect::<Vec<_>>(),
            "timeout_seconds": runtime.web_fetch.timeout_seconds,
            "max_bytes": runtime.web_fetch.max_bytes,
            "max_redirects": runtime.web_fetch.max_redirects,
        },
    })
}

fn runtime_snapshot_external_skills_json(snapshot: &RuntimeSnapshotExternalSkillsState) -> Value {
    json!({
        "policy": {
            "enabled": snapshot.policy.enabled,
            "require_download_approval": snapshot.policy.require_download_approval,
            "allowed_domains": snapshot.policy.allowed_domains.iter().collect::<Vec<_>>(),
            "blocked_domains": snapshot.policy.blocked_domains.iter().collect::<Vec<_>>(),
            "install_root": snapshot
                .policy
                .install_root
                .as_ref()
                .map(|path| path.display().to_string()),
            "auto_expose_installed": snapshot.policy.auto_expose_installed,
        },
        "override_active": snapshot.override_active,
        "inventory_status": snapshot.inventory_status.as_str(),
        "inventory_error": snapshot.inventory_error,
        "resolved_skill_count": snapshot.resolved_skill_count,
        "shadowed_skill_count": snapshot.shadowed_skill_count,
        "inventory": snapshot.inventory,
    })
}

fn shell_policy_default_str(
    mode: mvp::tools::shell_policy_ext::ShellPolicyDefault,
) -> &'static str {
    match mode {
        mvp::tools::shell_policy_ext::ShellPolicyDefault::Deny => "deny",
        mvp::tools::shell_policy_ext::ShellPolicyDefault::Allow => "allow",
    }
}

fn render_string_list<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
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
    let supported_pre_assembly_stage_families = metadata
        .supported_pre_assembly_stage_families
        .iter()
        .copied()
        .map(mvp::memory::MemoryStageFamily::as_str)
        .collect::<Vec<_>>();
    let mut payload = serde_json::Map::new();
    payload.insert("id".to_owned(), json!(metadata.id));
    payload.insert("api_version".to_owned(), json!(metadata.api_version));
    payload.insert(
        "capabilities".to_owned(),
        json!(metadata.capability_names()),
    );
    payload.insert(
        "supported_pre_assembly_stage_families".to_owned(),
        json!(supported_pre_assembly_stage_families),
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
        "policy": memory_system_policy_json(&snapshot.policy),
    })
}

pub fn render_memory_system_snapshot_text(
    config_path: &str,
    snapshot: &mvp::memory::MemorySystemRuntimeSnapshot,
) -> String {
    let selected_capabilities = snapshot.selected_metadata.capability_names();
    let selected_pre_assembly_stages = format_memory_stage_family_names(
        &snapshot
            .selected_metadata
            .supported_pre_assembly_stage_families,
    );
    let mut lines = vec![
        format!("config={config_path}"),
        format!(
            "selected={} source={} api_version={} capabilities={} pre_assembly_stages={} summary={}",
            snapshot.selected_metadata.id,
            snapshot.selected.source.as_str(),
            snapshot.selected_metadata.api_version,
            format_capability_names(&selected_capabilities),
            selected_pre_assembly_stages,
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
        let pre_assembly_stages =
            format_memory_stage_family_names(&metadata.supported_pre_assembly_stage_families);
        lines.push(format!(
            "- {} api_version={} capabilities={} pre_assembly_stages={} summary={}",
            metadata.id,
            metadata.api_version,
            format_capability_names(&capabilities),
            pre_assembly_stages,
            metadata.summary
        ));
    }

    lines.join("\n")
}

pub fn acp_backend_metadata_json(
    metadata: &mvp::acp::AcpBackendMetadata,
    source: Option<&str>,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("id".to_owned(), json!(metadata.id));
    payload.insert("api_version".to_owned(), json!(metadata.api_version));
    payload.insert(
        "capabilities".to_owned(),
        json!(metadata.capability_names()),
    );
    payload.insert("summary".to_owned(), json!(metadata.summary));
    if let Some(source) = source {
        payload.insert("source".to_owned(), json!(source));
    }
    Value::Object(payload)
}

pub fn acp_control_plane_json(snapshot: &mvp::acp::AcpControlPlaneSnapshot) -> Value {
    json!({
        "enabled": snapshot.enabled,
        "dispatch_enabled": snapshot.dispatch_enabled,
        "conversation_routing": snapshot.conversation_routing.as_str(),
        "allowed_channels": snapshot.allowed_channels,
        "allowed_account_ids": snapshot.allowed_account_ids,
        "bootstrap_mcp_servers": snapshot.bootstrap_mcp_servers,
        "working_directory": snapshot.working_directory,
        "thread_routing": snapshot.thread_routing.as_str(),
        "default_agent": snapshot.default_agent,
        "allowed_agents": snapshot.allowed_agents,
        "max_concurrent_sessions": snapshot.max_concurrent_sessions,
        "session_idle_ttl_ms": snapshot.session_idle_ttl_ms,
        "startup_timeout_ms": snapshot.startup_timeout_ms,
        "turn_timeout_ms": snapshot.turn_timeout_ms,
        "queue_owner_ttl_ms": snapshot.queue_owner_ttl_ms,
        "bindings_enabled": snapshot.bindings_enabled,
        "emit_runtime_events": snapshot.emit_runtime_events,
        "allow_mcp_server_injection": snapshot.allow_mcp_server_injection,
    })
}

pub fn acp_session_metadata_json(metadata: &mvp::acp::AcpSessionMetadata) -> Value {
    json!({
        "session_key": metadata.session_key,
        "conversation_id": metadata.conversation_id,
        "binding": metadata.binding.as_ref().map(acp_binding_scope_json),
        "activation_origin": metadata.activation_origin.map(mvp::acp::AcpRoutingOrigin::as_str),
        "provenance": acp_session_activation_provenance_json(metadata.activation_origin),
        "backend_id": metadata.backend_id,
        "runtime_session_name": metadata.runtime_session_name,
        "working_directory": metadata
            .working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
        "backend_session_id": metadata.backend_session_id,
        "agent_session_id": metadata.agent_session_id,
        "mode": metadata.mode.map(acp_session_mode_label),
        "state": acp_session_state_label(metadata.state),
        "last_activity_ms": metadata.last_activity_ms,
        "last_error": metadata.last_error,
    })
}

pub fn acp_session_status_json(status: &mvp::acp::AcpSessionStatus) -> Value {
    json!({
        "session_key": status.session_key,
        "backend_id": status.backend_id,
        "conversation_id": status.conversation_id,
        "binding": status.binding.as_ref().map(acp_binding_scope_json),
        "activation_origin": status.activation_origin.map(mvp::acp::AcpRoutingOrigin::as_str),
        "provenance": acp_session_activation_provenance_json(status.activation_origin),
        "state": acp_session_state_label(status.state),
        "mode": status.mode.map(acp_session_mode_label),
        "pending_turns": status.pending_turns,
        "active_turn_id": status.active_turn_id,
        "last_activity_ms": status.last_activity_ms,
        "last_error": status.last_error,
    })
}

pub fn acp_binding_scope_json(binding: &mvp::acp::AcpSessionBindingScope) -> Value {
    json!({
        "route_session_id": binding.route_session_id,
        "channel_id": binding.channel_id,
        "account_id": binding.account_id,
        "conversation_id": binding.conversation_id,
        "thread_id": binding.thread_id,
    })
}

pub fn acp_session_activation_provenance_json(origin: Option<mvp::acp::AcpRoutingOrigin>) -> Value {
    json!({
        "surface": "session_activation",
        "activation_origin": origin.map(mvp::acp::AcpRoutingOrigin::as_str),
    })
}

pub fn acp_dispatch_prediction_provenance_json(
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> Value {
    json!({
        "surface": "dispatch_prediction",
        "automatic_routing_origin": decision
            .automatic_routing_origin
            .map(mvp::acp::AcpRoutingOrigin::as_str),
    })
}

pub fn acp_turn_provenance_json(summary: &mvp::acp::AcpTurnEventSummary) -> Value {
    json!({
        "surface": "turn_execution",
        "last_routing_intent": summary.last_routing_intent,
        "last_routing_origin": summary.last_routing_origin,
        "routing_intent_counts": summary.routing_intent_counts,
        "routing_origin_counts": summary.routing_origin_counts,
    })
}

pub fn acp_dispatch_decision_json(
    session: &str,
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> Value {
    json!({
        "session": session,
        "decision": {
            "route_via_acp": decision.route_via_acp,
            "reason": decision.reason.as_str(),
            "automatic_routing_origin": decision
                .automatic_routing_origin
                .map(mvp::acp::AcpRoutingOrigin::as_str),
            "provenance": acp_dispatch_prediction_provenance_json(decision),
            "target": {
                "original_session_id": decision.target.original_session_id,
                "route_session_id": decision.target.route_session_id,
                "prefixed_agent_id": decision.target.prefixed_agent_id,
                "channel_id": decision.target.channel_id,
                "account_id": decision.target.account_id,
                "conversation_id": decision.target.conversation_id,
                "thread_id": decision.target.thread_id,
                "channel_path": decision.target.channel_path,
            }
        }
    })
}

pub fn acp_manager_observability_json(
    snapshot: &mvp::acp::AcpManagerObservabilitySnapshot,
) -> Value {
    json!({
        "runtime_cache": {
            "active_sessions": snapshot.runtime_cache.active_sessions,
            "idle_ttl_ms": snapshot.runtime_cache.idle_ttl_ms,
            "evicted_total": snapshot.runtime_cache.evicted_total,
            "last_evicted_at_ms": snapshot.runtime_cache.last_evicted_at_ms,
        },
        "sessions": {
            "bound": snapshot.sessions.bound,
            "unbound": snapshot.sessions.unbound,
            "activation_origin_counts": snapshot.sessions.activation_origin_counts,
            "provenance": {
                "surface": "session_activation_aggregate",
                "activation_origin_counts": snapshot.sessions.activation_origin_counts,
            },
            "backend_counts": snapshot.sessions.backend_counts,
        },
        "actors": {
            "active": snapshot.actors.active,
            "queue_depth": snapshot.actors.queue_depth,
            "waiting": snapshot.actors.waiting,
        },
        "turns": {
            "active": snapshot.turns.active,
            "queue_depth": snapshot.turns.queue_depth,
            "completed": snapshot.turns.completed,
            "failed": snapshot.turns.failed,
            "average_latency_ms": snapshot.turns.average_latency_ms,
            "max_latency_ms": snapshot.turns.max_latency_ms,
        },
        "errors_by_code": snapshot.errors_by_code,
    })
}

pub fn acp_event_summary_json(
    session: &str,
    limit: usize,
    summary: &mvp::acp::AcpTurnEventSummary,
) -> Value {
    json!({
        "session": session,
        "limit": limit,
        "provenance": acp_turn_provenance_json(summary),
        "summary": summary,
    })
}

pub fn format_acp_event_summary(
    session: &str,
    limit: usize,
    summary: &mvp::acp::AcpTurnEventSummary,
) -> String {
    format!(
        concat!(
            "acp_event_summary session={} limit={}\n",
            "records turn_event_records={} final_records={}\n",
            "events done={} error={} text={} usage_update={}\n",
            "turns succeeded={} cancelled={} failed={}\n",
            "latest backend_id={} agent_id={} routing_intent={} routing_origin={} session_key={} conversation_id={} binding_route_session_id={} channel_id={} account_id={} channel_conversation_id={} channel_thread_id={} trace_id={} source_message_id={} ack_cursor={} state={} stop_reason={} error={}\n",
            "rollup event_types={} stop_reasons={} routing_intents={} routing_origins={}\n"
        ),
        session,
        limit,
        summary.turn_event_records,
        summary.final_records,
        summary.done_events,
        summary.error_events,
        summary.text_events,
        summary.usage_update_events,
        summary.turns_succeeded,
        summary.turns_cancelled,
        summary.turns_failed,
        summary.last_backend_id.as_deref().unwrap_or("-"),
        summary.last_agent_id.as_deref().unwrap_or("-"),
        summary.last_routing_intent.as_deref().unwrap_or("-"),
        summary.last_routing_origin.as_deref().unwrap_or("-"),
        summary.last_session_key.as_deref().unwrap_or("-"),
        summary.last_conversation_id.as_deref().unwrap_or("-"),
        summary
            .last_binding_route_session_id
            .as_deref()
            .unwrap_or("-"),
        summary.last_channel_id.as_deref().unwrap_or("-"),
        summary.last_account_id.as_deref().unwrap_or("-"),
        summary
            .last_channel_conversation_id
            .as_deref()
            .unwrap_or("-"),
        summary.last_channel_thread_id.as_deref().unwrap_or("-"),
        summary.last_trace_id.as_deref().unwrap_or("-"),
        summary.last_source_message_id.as_deref().unwrap_or("-"),
        summary.last_ack_cursor.as_deref().unwrap_or("-"),
        summary.last_turn_state.as_deref().unwrap_or("-"),
        summary.last_stop_reason.as_deref().unwrap_or("-"),
        summary.last_error.as_deref().unwrap_or("-"),
        format_u32_rollup(&summary.event_type_counts),
        format_u32_rollup(&summary.stop_reason_counts),
        format_u32_rollup(&summary.routing_intent_counts),
        format_u32_rollup(&summary.routing_origin_counts)
    )
}

pub fn acp_session_mode_label(mode: mvp::acp::AcpSessionMode) -> &'static str {
    match mode {
        mvp::acp::AcpSessionMode::Interactive => "interactive",
        mvp::acp::AcpSessionMode::Background => "background",
        mvp::acp::AcpSessionMode::Review => "review",
    }
}

pub fn acp_session_state_label(state: mvp::acp::AcpSessionState) -> &'static str {
    match state {
        mvp::acp::AcpSessionState::Initializing => "initializing",
        mvp::acp::AcpSessionState::Ready => "ready",
        mvp::acp::AcpSessionState::Busy => "busy",
        mvp::acp::AcpSessionState::Cancelling => "cancelling",
        mvp::acp::AcpSessionState::Error => "error",
        mvp::acp::AcpSessionState::Closed => "closed",
    }
}

pub fn format_capability_names(names: &[&str]) -> String {
    if names.is_empty() {
        return "(none)".to_owned();
    }
    names.join(",")
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
