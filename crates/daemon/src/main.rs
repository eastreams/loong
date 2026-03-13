#![allow(clippy::print_stdout, clippy::print_stderr)] // CLI daemon binary
#[cfg(test)]
use std::time::Duration;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::Arc,
};

#[cfg(test)]
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
#[cfg(test)]
use clap::CommandFactory;
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(test)]
use kernel::{AuditEventKind, ExecutionRoute, HarnessKind, PluginBridgeKind, VerticalPackManifest};
use kernel::{Capability, ConnectorCommand, FixedClock, InMemoryAuditSink, TaskIntent};
use serde::Serialize;
use serde_json::{Value, json};
#[cfg(test)]
use sha2::{Digest, Sha256};
#[cfg(test)]
use tokio::time::sleep;

use loongclaw_app as mvp;
pub(crate) use loongclaw_spec::spec_execution::*;
pub(crate) use loongclaw_spec::spec_runtime::*;
use loongclaw_spec::{CliResult, DEFAULT_AGENT_ID, DEFAULT_PACK_ID, kernel_bootstrap};

use loongclaw_bench::{
    run_programmatic_pressure_baseline_lint_cli, run_programmatic_pressure_benchmark_cli,
    run_wasm_cache_benchmark_cli,
};
mod doctor_cli;
mod import_claw_cli;
mod onboard_cli;
#[cfg(test)]
pub(crate) use loongclaw_spec::programmatic::{
    acquire_programmatic_circuit_slot, record_programmatic_circuit_outcome,
};
#[cfg(test)]
mod tests;

const PUBLIC_GITHUB_REPO: &str = "loongclaw-ai/loongclaw";

#[derive(Parser, Debug)]
#[command(
    name = "loongclaw",
    about = "LoongClaw low-level runtime daemon",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
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
    },
    /// Run a full workflow from a JSON spec (task/connector/runtime/tool/memory)
    RunSpec {
        #[arg(long)]
        spec: String,
        #[arg(long, default_value_t = false)]
        print_audit: bool,
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
    /// Guided onboarding for a fast first chat with preflight diagnostics
    Onboard {
        /// Target config output path (defaults to ~/.loongclaw/config.toml)
        #[arg(long)]
        output: Option<String>,
        /// Overwrite existing output config if present
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Run onboarding in non-interactive mode using provided flags/defaults
        #[arg(long, default_value_t = false)]
        non_interactive: bool,
        /// Accept generated config values that may require manual hardening later
        #[arg(long, default_value_t = false)]
        accept_risk: bool,
        /// Provider identifier to prefill in generated config
        #[arg(long)]
        provider: Option<String>,
        /// Model identifier to prefill in generated config
        #[arg(long)]
        model: Option<String>,
        /// Environment variable name holding the provider API key
        #[arg(long, alias = "api-key-env")]
        api_key: Option<String>,
        /// Default personality profile for the generated config
        #[arg(long)]
        personality: Option<String>,
        /// Default memory profile for the generated config
        #[arg(long)]
        memory_profile: Option<String>,
        /// Override system prompt text in generated config
        #[arg(long)]
        system_prompt: Option<String>,
        /// Skip provider model probe during onboarding diagnostics
        #[arg(long, default_value_t = false)]
        skip_model_probe: bool,
    },
    /// Import prompt/identity traits from another claw workspace into LoongClaw config
    ImportClaw {
        /// Source config file to import from
        #[arg(long)]
        input: Option<String>,
        /// Target config file to write import plan/apply result
        #[arg(long)]
        output: Option<String>,
        /// Explicit source kind (auto-detected when omitted)
        #[arg(long)]
        source: Option<String>,
        /// Import mode (`plan` preview or `apply` mutation)
        #[arg(long, value_enum, default_value = "plan")]
        mode: import_claw_cli::ImportClawMode,
        /// Emit machine-readable JSON output
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Source selection identifier (alias: --selection-id)
        #[arg(long, visible_alias = "selection-id")]
        source_id: Option<String>,
        /// Merge memory profiles conservatively to avoid destructive overwrites
        #[arg(long, default_value_t = false)]
        safe_profile_merge: bool,
        /// Primary source identifier for multi-source imports
        #[arg(long, visible_alias = "primary-selection-id")]
        primary_source_id: Option<String>,
        /// Apply external-skills import plan when running in apply mode
        #[arg(long, default_value_t = false)]
        apply_external_skills_plan: bool,
        /// Force apply even when importer detects non-fatal safeguards
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Run configuration diagnostics and optionally apply safe config/path fixes
    Doctor {
        /// Config file path to validate (defaults to auto-discovery)
        #[arg(long)]
        config: Option<String>,
        /// Apply safe auto-fixes for detected diagnostics
        #[arg(long, default_value_t = false)]
        fix: bool,
        /// Emit machine-readable JSON diagnostics
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Skip provider model probing during diagnostics
        #[arg(long, default_value_t = false)]
        skip_model_probe: bool,
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
    /// List available conversation context engines and selected runtime engine
    ListContextEngines {
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
        receive_id: String,
        #[arg(long)]
        text: String,
        #[arg(long, default_value_t = false)]
        card: bool,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ValidateConfigOutput {
    Text,
    Json,
    ProblemJson,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Commands::Demo) {
        Commands::Demo => run_demo().await,
        Commands::RunTask { objective, payload } => run_task_cli(&objective, &payload).await,
        Commands::InvokeConnector { operation, payload } => {
            invoke_connector_cli(&operation, &payload).await
        }
        Commands::AuditDemo => run_audit_demo().await,
        Commands::InitSpec { output } => init_spec_cli(&output),
        Commands::RunSpec { spec, print_audit } => run_spec_cli(&spec, print_audit).await,
        Commands::BenchmarkProgrammaticPressure {
            matrix,
            baseline,
            output,
            enforce_gate,
            preflight_fail_on_warnings,
        } => {
            run_programmatic_pressure_benchmark_cli(
                &matrix,
                baseline.as_deref(),
                &output,
                enforce_gate,
                preflight_fail_on_warnings,
            )
            .await
        }
        Commands::BenchmarkProgrammaticPressureLint {
            matrix,
            baseline,
            output,
            enforce_gate,
            fail_on_warnings,
        } => run_programmatic_pressure_baseline_lint_cli(
            &matrix,
            baseline.as_deref(),
            &output,
            enforce_gate,
            fail_on_warnings,
        ),
        Commands::BenchmarkWasmCache {
            wasm,
            output,
            cold_iterations,
            hot_iterations,
            warmup_iterations,
            enforce_gate,
            min_speedup_ratio,
        } => run_wasm_cache_benchmark_cli(
            &wasm,
            &output,
            cold_iterations,
            hot_iterations,
            warmup_iterations,
            enforce_gate,
            min_speedup_ratio,
        ),
        Commands::ValidateConfig {
            config,
            json,
            output,
            locale,
            fail_on_diagnostics,
        } => run_validate_config_cli(
            config.as_deref(),
            json,
            output,
            &locale,
            fail_on_diagnostics,
        ),
        Commands::Onboard {
            output,
            force,
            non_interactive,
            accept_risk,
            provider,
            model,
            api_key,
            personality,
            memory_profile,
            system_prompt,
            skip_model_probe,
        } => {
            onboard_cli::run_onboard_cli(onboard_cli::OnboardCommandOptions {
                output,
                force,
                non_interactive,
                accept_risk,
                provider,
                model,
                api_key,
                personality,
                memory_profile,
                system_prompt,
                skip_model_probe,
            })
            .await
        }
        Commands::ImportClaw {
            input,
            output,
            source,
            mode,
            json,
            source_id,
            safe_profile_merge,
            primary_source_id,
            apply_external_skills_plan,
            force,
        } => import_claw_cli::run_import_claw_cli(import_claw_cli::ImportClawCommandOptions {
            input,
            output,
            source,
            mode,
            json,
            source_id,
            safe_profile_merge,
            primary_source_id,
            apply_external_skills_plan,
            force,
        }),
        Commands::Doctor {
            config,
            fix,
            json,
            skip_model_probe,
        } => {
            doctor_cli::run_doctor_cli(doctor_cli::DoctorCommandOptions {
                config,
                fix,
                json,
                skip_model_probe,
            })
            .await
        }
        Commands::Channels { config, json } => run_channels_cli(config.as_deref(), json),
        Commands::ListModels { config, json } => run_list_models_cli(config.as_deref(), json).await,
        Commands::ListContextEngines { config, json } => {
            run_list_context_engines_cli(config.as_deref(), json)
        }
        Commands::ListAcpBackends { config, json } => {
            run_list_acp_backends_cli(config.as_deref(), json)
        }
        Commands::ListAcpSessions { config, json } => {
            run_list_acp_sessions_cli(config.as_deref(), json)
        }
        Commands::AcpStatus {
            config,
            session,
            conversation_id,
            route_session_id,
            json,
        } => {
            run_acp_status_cli(
                config.as_deref(),
                session.as_deref(),
                conversation_id.as_deref(),
                route_session_id.as_deref(),
                json,
            )
            .await
        }
        Commands::AcpObservability { config, json } => {
            run_acp_observability_cli(config.as_deref(), json).await
        }
        Commands::AcpEventSummary {
            config,
            session,
            limit,
            json,
        } => run_acp_event_summary_cli(config.as_deref(), session.as_deref(), limit, json),
        Commands::AcpDispatch {
            config,
            session,
            channel,
            conversation_id,
            account_id,
            thread_id,
            json,
        } => run_acp_dispatch_cli(
            config.as_deref(),
            session.as_deref(),
            channel.as_deref(),
            conversation_id.as_deref(),
            account_id.as_deref(),
            thread_id.as_deref(),
            json,
        ),
        Commands::AcpDoctor {
            config,
            backend,
            json,
        } => run_acp_doctor_cli(config.as_deref(), backend.as_deref(), json).await,
        Commands::Chat {
            config,
            session,
            acp,
            acp_event_stream,
            acp_bootstrap_mcp_server,
            acp_cwd,
        } => {
            run_chat_cli(
                config.as_deref(),
                session.as_deref(),
                acp,
                acp_event_stream,
                &acp_bootstrap_mcp_server,
                acp_cwd.as_deref(),
            )
            .await
        }
        Commands::SafeLaneSummary {
            config,
            session,
            limit,
            json,
        } => run_safe_lane_summary_cli(config.as_deref(), session.as_deref(), limit, json),
        Commands::TelegramServe {
            config,
            once,
            account,
        } => run_telegram_serve_cli(config.as_deref(), once, account.as_deref()).await,
        Commands::FeishuSend {
            config,
            account,
            receive_id,
            text,
            card,
        } => {
            run_feishu_send_cli(
                config.as_deref(),
                account.as_deref(),
                &receive_id,
                &text,
                card,
            )
            .await
        }
        Commands::FeishuServe {
            config,
            account,
            bind,
            path,
        } => {
            run_feishu_serve_cli(
                config.as_deref(),
                account.as_deref(),
                bind.as_deref(),
                path.as_deref(),
            )
            .await
        }
    };
    if let Err(error) = result {
        // startup error reporting
        #[allow(clippy::print_stderr)]
        {
            eprintln!("error: {error}");
        }
        std::process::exit(2);
    }
}

async fn run_demo() -> CliResult<()> {
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

async fn run_task_cli(objective: &str, payload_raw: &str) -> CliResult<()> {
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

async fn invoke_connector_cli(operation: &str, payload_raw: &str) -> CliResult<()> {
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

async fn run_audit_demo() -> CliResult<()> {
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

fn init_spec_cli(output_path: &str) -> CliResult<()> {
    let spec = RunnerSpec::template();
    write_json_file(output_path, &spec)?;
    println!("spec template written to {}", output_path);
    Ok(())
}

async fn run_spec_cli(spec_path: &str, print_audit: bool) -> CliResult<()> {
    let spec = read_spec_file(spec_path)?;
    let report = execute_spec(&spec, print_audit).await;
    let pretty = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("serialize spec run report failed: {error}"))?;
    println!("{pretty}");
    Ok(())
}

fn run_validate_config_cli(
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

    match output {
        ValidateConfigOutput::Text => {
            if diagnostics.is_empty() {
                println!("config={} valid=true", resolved_path.display());
            } else {
                println!(
                    "config={} valid=false diagnostics={}",
                    resolved_path.display(),
                    diagnostics_count
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
                "valid": diagnostics.is_empty(),
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
                    "locale": normalized_locale,
                    "supported_locales": supported_locales.clone(),
                    "diagnostics_schema_version": 1,
                    "errors": [],
                })
            } else {
                json!({
                    "type": "urn:loongclaw:problem:config.validation_failed",
                    "title": "Configuration Validation Failed",
                    "detail": format!("{} configuration diagnostic(s) were reported.", diagnostics_count),
                    "instance": resolved_path.display().to_string(),
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

fn resolve_validate_output(
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

async fn run_list_models_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
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

fn run_channels_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
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

#[derive(Debug, Clone, Serialize)]
struct ChannelsCliJsonSchema {
    version: u32,
    primary_channel_view: &'static str,
    catalog_view: &'static str,
    legacy_channel_views: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize)]
struct ChannelsCliJsonPayload {
    config: String,
    schema: ChannelsCliJsonSchema,
    channels: Vec<mvp::channel::ChannelStatusSnapshot>,
    catalog_only_channels: Vec<mvp::channel::ChannelCatalogEntry>,
    channel_catalog: Vec<mvp::channel::ChannelCatalogEntry>,
    channel_surfaces: Vec<mvp::channel::ChannelSurface>,
}

const CHANNELS_CLI_JSON_SCHEMA_VERSION: u32 = 1;
const CHANNELS_CLI_JSON_LEGACY_VIEWS: &[&str] = &["channels", "catalog_only_channels"];

fn build_channels_cli_json_payload(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> ChannelsCliJsonPayload {
    ChannelsCliJsonPayload {
        config: config_path.to_owned(),
        schema: ChannelsCliJsonSchema {
            version: CHANNELS_CLI_JSON_SCHEMA_VERSION,
            primary_channel_view: "channel_surfaces",
            catalog_view: "channel_catalog",
            legacy_channel_views: CHANNELS_CLI_JSON_LEGACY_VIEWS,
        },
        channels: inventory.channels.clone(),
        catalog_only_channels: inventory.catalog_only_channels.clone(),
        channel_catalog: inventory.channel_catalog.clone(),
        channel_surfaces: inventory.channel_surfaces.clone(),
    }
}

fn render_channel_surfaces_text(
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
                lines.push(format!(
                    "    op {} ({}) {}: {}",
                    operation.id,
                    operation.command,
                    operation.health.as_str(),
                    operation.detail
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
            for operation in &surface.catalog.operations {
                lines.push(format!(
                    "  catalog op {} ({}) tracks_runtime={}",
                    operation.id, operation.command, operation.tracks_runtime
                ));
            }
        }
    }
    lines.join("\n")
}

fn push_channel_surface_header(lines: &mut Vec<String>, surface: &mvp::channel::ChannelSurface) {
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
    lines.push(format!(
        "{} [{}] implementation_status={} capabilities={} aliases={} transport={} configured_accounts={} default_configured_account={}",
        surface.catalog.label,
        surface.catalog.id,
        surface.catalog.implementation_status.as_str(),
        capabilities,
        aliases,
        surface.catalog.transport,
        surface.configured_accounts.len(),
        surface
            .default_configured_account_id
            .as_deref()
            .unwrap_or("-")
    ));
}

fn run_list_context_engines_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
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

fn run_list_acp_backends_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
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

fn run_list_acp_sessions_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
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

async fn run_acp_doctor_cli(
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

fn acp_doctor_json(
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

async fn run_acp_status_cli(
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
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "requested_session": session_key,
            "requested_conversation_id": conversation_id,
            "requested_route_session_id": route_session_id,
            "resolved_session_key": resolved_session_key,
            "status": acp_session_status_json(&status),
        });
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

async fn run_acp_observability_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let manager = mvp::acp::shared_acp_session_manager(&config)?;
    let snapshot = manager.observability_snapshot(&config).await?;

    if as_json {
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "snapshot": acp_manager_observability_json(&snapshot),
        });
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

fn resolve_acp_status_session_key(
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

async fn run_chat_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> CliResult<()> {
    let options = mvp::chat::CliChatOptions {
        acp_requested: acp,
        acp_event_stream,
        acp_bootstrap_mcp_servers: acp_bootstrap_mcp_server.to_vec(),
        acp_working_directory: acp_cwd
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from),
    };
    mvp::chat::run_cli_chat(config_path, session, &options).await
}

fn run_acp_event_summary_cli(
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

fn run_acp_dispatch_cli(
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
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "address": {
                "session_id": address.session_id,
                "channel_id": address.channel_id,
                "account_id": address.account_id,
                "conversation_id": address.conversation_id,
                "thread_id": address.thread_id,
            },
            "dispatch": acp_dispatch_decision_json(session_id.as_str(), &decision),
        });
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

fn build_acp_dispatch_address(
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

fn run_safe_lane_summary_cli(
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
fn format_milli_ratio(value: Option<u32>) -> String {
    value
        .map(|raw| format!("{:.3}", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
}

async fn with_graceful_shutdown<F>(serve_future: F) -> CliResult<()>
where
    F: std::future::Future<Output = CliResult<()>>,
{
    tokio::select! {
        result = serve_future => result,
        result = wait_for_shutdown_signal() => result,
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> CliResult<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|error| format!("failed to register SIGTERM handler: {error}"))?;

    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.map_err(|error| format!("failed to register Ctrl-C handler: {error}"))?;
            eprintln!("\nReceived Ctrl-C, shutting down gracefully...");
            Ok(())
        }
        _ = sigterm.recv() => {
            eprintln!("\nReceived SIGTERM, shutting down gracefully...");
            Ok(())
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> CliResult<()> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("failed to register Ctrl-C handler: {error}"))?;
    eprintln!("\nReceived Ctrl-C, shutting down gracefully...");
    Ok(())
}

async fn run_telegram_serve_cli(
    config_path: Option<&str>,
    once: bool,
    account: Option<&str>,
) -> CliResult<()> {
    with_graceful_shutdown(mvp::channel::run_telegram_channel(
        config_path,
        once,
        account,
    ))
    .await
}

async fn run_feishu_send_cli(
    config_path: Option<&str>,
    account: Option<&str>,
    receive_id: &str,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    mvp::channel::run_feishu_send(config_path, account, receive_id, text, as_card).await
}

async fn run_feishu_serve_cli(
    config_path: Option<&str>,
    account: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    with_graceful_shutdown(mvp::channel::run_feishu_channel(
        config_path,
        account,
        bind_override,
        path_override,
    ))
    .await
}

fn parse_json_payload(raw: &str, context: &str) -> CliResult<Value> {
    serde_json::from_str(raw).map_err(|error| format!("invalid JSON for {context}: {error}"))
}

fn context_engine_metadata_json(
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

fn acp_backend_metadata_json(
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

#[cfg_attr(test, allow(dead_code))]
fn acp_control_plane_json(snapshot: &mvp::acp::AcpControlPlaneSnapshot) -> Value {
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

#[cfg_attr(test, allow(dead_code))]
fn acp_session_metadata_json(metadata: &mvp::acp::AcpSessionMetadata) -> Value {
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

#[cfg_attr(test, allow(dead_code))]
fn acp_session_status_json(status: &mvp::acp::AcpSessionStatus) -> Value {
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

#[cfg_attr(test, allow(dead_code))]
fn acp_binding_scope_json(binding: &mvp::acp::AcpSessionBindingScope) -> Value {
    json!({
        "route_session_id": binding.route_session_id,
        "channel_id": binding.channel_id,
        "account_id": binding.account_id,
        "conversation_id": binding.conversation_id,
        "thread_id": binding.thread_id,
    })
}

#[cfg_attr(test, allow(dead_code))]
fn acp_session_activation_provenance_json(origin: Option<mvp::acp::AcpRoutingOrigin>) -> Value {
    json!({
        "surface": "session_activation",
        "activation_origin": origin.map(mvp::acp::AcpRoutingOrigin::as_str),
    })
}

#[cfg_attr(test, allow(dead_code))]
fn acp_dispatch_prediction_provenance_json(
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> Value {
    json!({
        "surface": "dispatch_prediction",
        "automatic_routing_origin": decision
            .automatic_routing_origin
            .map(mvp::acp::AcpRoutingOrigin::as_str),
    })
}

#[cfg_attr(test, allow(dead_code))]
fn acp_turn_provenance_json(summary: &mvp::acp::AcpTurnEventSummary) -> Value {
    json!({
        "surface": "turn_execution",
        "last_routing_intent": summary.last_routing_intent,
        "last_routing_origin": summary.last_routing_origin,
        "routing_intent_counts": summary.routing_intent_counts,
        "routing_origin_counts": summary.routing_origin_counts,
    })
}

#[cfg_attr(test, allow(dead_code))]
fn acp_dispatch_decision_json(
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

#[cfg_attr(test, allow(dead_code))]
fn acp_manager_observability_json(snapshot: &mvp::acp::AcpManagerObservabilitySnapshot) -> Value {
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

#[cfg_attr(test, allow(dead_code))]
fn acp_event_summary_json(
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

#[cfg_attr(test, allow(dead_code))]
fn format_acp_event_summary(
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

#[cfg_attr(test, allow(dead_code))]
fn acp_session_mode_label(mode: mvp::acp::AcpSessionMode) -> &'static str {
    match mode {
        mvp::acp::AcpSessionMode::Interactive => "interactive",
        mvp::acp::AcpSessionMode::Background => "background",
        mvp::acp::AcpSessionMode::Review => "review",
    }
}

#[cfg_attr(test, allow(dead_code))]
fn acp_session_state_label(state: mvp::acp::AcpSessionState) -> &'static str {
    match state {
        mvp::acp::AcpSessionState::Initializing => "initializing",
        mvp::acp::AcpSessionState::Ready => "ready",
        mvp::acp::AcpSessionState::Busy => "busy",
        mvp::acp::AcpSessionState::Cancelling => "cancelling",
        mvp::acp::AcpSessionState::Error => "error",
        mvp::acp::AcpSessionState::Closed => "closed",
    }
}

fn format_capability_names(names: &[&str]) -> String {
    if names.is_empty() {
        return "(none)".to_owned();
    }
    names.join(",")
}

fn format_u32_rollup(values: &BTreeMap<String, u32>) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_usize_rollup(values: &BTreeMap<String, usize>) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn read_spec_file(path: &str) -> CliResult<RunnerSpec> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read spec file {path}: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("failed to parse spec file {path}: {error}"))
}

fn write_json_file<T: Serialize>(path: &str, value: &T) -> CliResult<()> {
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

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn root_help_uses_onboarding_language() {
        let mut command = Cli::command();
        let mut rendered = Vec::new();
        command
            .write_long_help(&mut rendered)
            .expect("render root help");
        let help = String::from_utf8(rendered).expect("help is valid utf-8");

        assert!(help.contains("onboarding"));
        assert!(!help.contains("setup"));
    }

    #[test]
    fn setup_subcommand_is_removed() {
        let error = Cli::try_parse_from(["loongclaw", "setup"])
            .expect_err("`setup` should no longer parse as a valid subcommand");
        assert!(
            error
                .to_string()
                .contains("unrecognized subcommand 'setup'")
        );
    }

    #[test]
    fn safe_lane_summary_cli_rejects_zero_limit() {
        let error = run_safe_lane_summary_cli(None, Some("session-a"), 0, false)
            .expect_err("zero limit must be rejected");
        assert!(error.contains(">= 1"));
    }

    #[test]
    fn onboard_cli_accepts_generic_api_key_flag() {
        let cli = Cli::try_parse_from([
            "loongclaw",
            "onboard",
            "--non-interactive",
            "--accept-risk",
            "--api-key",
            "${OPENAI_API_KEY}",
        ])
        .expect("`--api-key` should parse");

        match cli.command {
            Some(Commands::Onboard { api_key, .. }) => {
                assert_eq!(api_key.as_deref(), Some("${OPENAI_API_KEY}"));
            }
            other => panic!("unexpected command parsed: {other:?}"),
        }
    }

    #[test]
    fn onboard_cli_keeps_legacy_api_key_env_alias() {
        let cli = Cli::try_parse_from([
            "loongclaw",
            "onboard",
            "--non-interactive",
            "--accept-risk",
            "--api-key-env",
            "OPENAI_API_KEY",
        ])
        .expect("legacy `--api-key-env` alias should still parse");

        match cli.command {
            Some(Commands::Onboard { api_key, .. }) => {
                assert_eq!(api_key.as_deref(), Some("OPENAI_API_KEY"));
            }
            other => panic!("unexpected command parsed: {other:?}"),
        }
    }

    #[test]
    fn acp_event_summary_cli_rejects_zero_limit() {
        let error = run_acp_event_summary_cli(None, Some("session-a"), 0, false)
            .expect_err("zero limit must be rejected");
        assert!(error.contains(">= 1"));
    }

    #[test]
    fn build_acp_dispatch_address_requires_channel_for_structured_scope() {
        let error = build_acp_dispatch_address("opaque-session", None, Some("oc_123"), None, None)
            .expect_err("structured scope without channel must be rejected");
        assert!(error.contains("--channel"));
    }

    #[test]
    fn build_acp_dispatch_address_builds_structured_scope() {
        let address = build_acp_dispatch_address(
            "opaque-session",
            Some("Feishu"),
            Some("oc_123"),
            Some("LARK PROD"),
            Some("om_thread_1"),
        )
        .expect("structured scope should build");

        assert_eq!(address.session_id, "opaque-session");
        assert_eq!(address.channel_id.as_deref(), Some("feishu"));
        assert_eq!(address.account_id.as_deref(), Some("lark-prod"));
        assert_eq!(address.conversation_id.as_deref(), Some("oc_123"));
        assert_eq!(address.thread_id.as_deref(), Some("om_thread_1"));
    }

    #[test]
    fn format_u32_rollup_uses_dash_for_empty_map() {
        let rendered = format_u32_rollup(&BTreeMap::new());
        assert_eq!(rendered, "-");
    }

    #[test]
    fn format_acp_event_summary_includes_routing_intent_and_provenance() {
        let rendered = format_acp_event_summary(
            "telegram:42",
            120,
            &mvp::acp::AcpTurnEventSummary {
                turn_event_records: 4,
                final_records: 2,
                done_events: 2,
                error_events: 1,
                text_events: 1,
                usage_update_events: 1,
                turns_succeeded: 1,
                turns_cancelled: 1,
                turns_failed: 0,
                event_type_counts: BTreeMap::from([
                    ("done".to_owned(), 2u32),
                    ("text".to_owned(), 1u32),
                ]),
                stop_reason_counts: BTreeMap::from([
                    ("completed".to_owned(), 1u32),
                    ("cancelled".to_owned(), 1u32),
                ]),
                routing_intent_counts: BTreeMap::from([("explicit".to_owned(), 2u32)]),
                routing_origin_counts: BTreeMap::from([("explicit_request".to_owned(), 2u32)]),
                last_backend_id: Some("acpx".to_owned()),
                last_agent_id: Some("codex".to_owned()),
                last_session_key: Some("agent:codex:telegram:42".to_owned()),
                last_conversation_id: Some("telegram:42".to_owned()),
                last_binding_route_session_id: Some("telegram:bot_123456:42".to_owned()),
                last_channel_id: Some("telegram".to_owned()),
                last_account_id: Some("bot_123456".to_owned()),
                last_channel_conversation_id: Some("42".to_owned()),
                last_channel_thread_id: None,
                last_routing_intent: Some("explicit".to_owned()),
                last_routing_origin: Some("explicit_request".to_owned()),
                last_trace_id: Some("trace-123".to_owned()),
                last_source_message_id: Some("message-42".to_owned()),
                last_ack_cursor: Some("cursor-9".to_owned()),
                last_turn_state: Some("ready".to_owned()),
                last_stop_reason: Some("cancelled".to_owned()),
                last_error: Some("permission denied".to_owned()),
            },
        );

        assert!(rendered.contains("acp_event_summary session=telegram:42 limit=120"));
        assert!(rendered.contains("routing_intent=explicit"));
        assert!(rendered.contains("routing_origin=explicit_request"));
        assert!(rendered.contains("routing_intents=explicit:2"));
        assert!(rendered.contains("routing_origins=explicit_request:2"));
        assert!(rendered.contains("trace_id=trace-123"));
        assert!(rendered.contains("source_message_id=message-42"));
        assert!(rendered.contains("ack_cursor=cursor-9"));
    }

    #[test]
    fn chat_cli_accepts_acp_runtime_option_flags() {
        let cli = Cli::try_parse_from([
            "loongclaw",
            "chat",
            "--session",
            "telegram:42",
            "--acp",
            "--acp-event-stream",
            "--acp-bootstrap-mcp-server",
            "filesystem",
            "--acp-bootstrap-mcp-server",
            "search",
            "--acp-cwd",
            "/workspace/project",
        ])
        .expect("chat CLI should parse ACP runtime option flags");

        match cli.command {
            Some(Commands::Chat {
                session,
                acp,
                acp_event_stream,
                acp_bootstrap_mcp_server,
                acp_cwd,
                ..
            }) => {
                assert_eq!(session.as_deref(), Some("telegram:42"));
                assert!(acp);
                assert!(acp_event_stream);
                assert_eq!(
                    acp_bootstrap_mcp_server,
                    vec!["filesystem".to_owned(), "search".to_owned()]
                );
                assert_eq!(acp_cwd.as_deref(), Some("/workspace/project"));
            }
            other => panic!("unexpected command parse result: {other:?}"),
        }
    }
}
