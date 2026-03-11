#![allow(clippy::print_stdout, clippy::print_stderr)] // CLI daemon binary
#[cfg(test)]
use std::{collections::BTreeMap, time::Duration};
use std::{collections::BTreeSet, fs, path::Path, sync::Arc};

#[cfg(test)]
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(test)]
use kernel::{AuditEventKind, ExecutionRoute, HarnessKind, PluginBridgeKind, VerticalPackManifest};
use kernel::{Capability, ConnectorCommand, FixedClock, InMemoryAuditSink, TaskIntent};
use serde::Serialize;
use serde_json::{json, Value};
#[cfg(test)]
use sha2::{Digest, Sha256};
#[cfg(test)]
use tokio::time::sleep;

use loongclaw_app as mvp;
pub(crate) use loongclaw_spec::spec_execution::*;
pub(crate) use loongclaw_spec::spec_runtime::*;
use loongclaw_spec::{kernel_bootstrap, CliResult, DEFAULT_AGENT_ID, DEFAULT_PACK_ID};

use loongclaw_bench::{
    run_programmatic_pressure_baseline_lint_cli, run_programmatic_pressure_benchmark_cli,
    run_wasm_cache_benchmark_cli,
};
mod doctor_cli;
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
    name = "loongclawd",
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
    /// Generate a beginner-friendly TOML config and bootstrap local state
    Setup {
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
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
    /// Guided onboarding for fast first-chat setup with preflight diagnostics
    Onboard {
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
        #[arg(long, default_value_t = false)]
        non_interactive: bool,
        #[arg(long, default_value_t = false)]
        accept_risk: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        api_key_env: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        #[arg(long, default_value_t = false)]
        skip_model_probe: bool,
    },
    /// Run setup diagnostics and optionally apply safe config/path fixes
    Doctor {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        fix: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
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
    /// Start interactive CLI chat channel with sliding-window memory
    Chat {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
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
        Commands::Setup { output, force } => run_setup_cli(output.as_deref(), force),
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
            api_key_env,
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
                api_key_env,
                system_prompt,
                skip_model_probe,
            })
            .await
        }
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
        Commands::Chat { config, session } => {
            run_chat_cli(config.as_deref(), session.as_deref()).await
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
        .invoke_connector(
            DEFAULT_PACK_ID,
            &token,
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
        .invoke_connector(
            DEFAULT_PACK_ID,
            &token,
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
        .invoke_connector(
            DEFAULT_PACK_ID,
            &token,
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
    let report = execute_spec(spec, print_audit).await;
    let pretty = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("serialize spec run report failed: {error}"))?;
    println!("{pretty}");
    Ok(())
}

fn run_setup_cli(output: Option<&str>, force: bool) -> CliResult<()> {
    let path = mvp::config::write_template(output, force)?;
    #[cfg(feature = "memory-sqlite")]
    {
        let path_str = path
            .to_str()
            .ok_or_else(|| format!("config path is not valid UTF-8: {}", path.display()))?;
        let (_, parsed) = mvp::config::load(Some(path_str))?;
        let mem_config = mvp::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(parsed.memory.resolved_sqlite_path()),
        };
        let memory_db = mvp::memory::ensure_memory_db_ready(
            Some(parsed.memory.resolved_sqlite_path()),
            &mem_config,
        )
        .map_err(|error| format!("failed to bootstrap sqlite memory: {error}"))?;
        println!(
            "setup complete\n- config: {}\n- sqlite memory: {}",
            path.display(),
            memory_db.display()
        );
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        println!("setup complete\n- config: {}", path.display());
    }
    println!("next step: loongclawd chat --config {}", path.display());
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
    let snapshots = mvp::channel::channel_status_snapshots(&config);

    if as_json {
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "channels": snapshots,
        });
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize channel status output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_channel_snapshots_text(&resolved_path.display().to_string(), &snapshots)
    );
    Ok(())
}

fn render_channel_snapshots_text(
    config_path: &str,
    snapshots: &[mvp::channel::ChannelStatusSnapshot],
) -> String {
    let mut lines = vec![format!("config={config_path}")];
    for snapshot in snapshots {
        let aliases = if snapshot.aliases.is_empty() {
            "-".to_owned()
        } else {
            snapshot.aliases.join(",")
        };
        let api_base_url = snapshot.api_base_url.as_deref().unwrap_or("-");
        lines.push(format!(
            "{} [{}] configured_account={} default_account={} default_source={} compiled={} enabled={} aliases={} api_base_url={}",
            snapshot.label,
            snapshot.id,
            snapshot.configured_account_id,
            snapshot.is_default_account,
            snapshot.default_account_source.as_str(),
            snapshot.compiled,
            snapshot.enabled,
            aliases,
            api_base_url
        ));
        lines.push(format!("  transport={}", snapshot.transport));
        lines.push(format!(
            "  configured_account_label={}",
            snapshot.configured_account_label
        ));
        for note in &snapshot.notes {
            lines.push(format!("  note: {note}"));
        }
        for operation in &snapshot.operations {
            lines.push(format!(
                "  op {} ({}) {}: {}",
                operation.id,
                operation.command,
                operation.health.as_str(),
                operation.detail
            ));
            if let Some(runtime) = &operation.runtime {
                lines.push(format!(
                    "    runtime account={} account_id={} running={} stale={} busy={} active_runs={} instance_count={} running_instances={} stale_instances={} last_run_activity_at={} last_heartbeat_at={} pid={}",
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
                lines.push(format!("    issue: {issue}"));
            }
        }
    }
    lines.join("\n")
}

async fn run_chat_cli(config_path: Option<&str>, session: Option<&str>) -> CliResult<()> {
    mvp::chat::run_cli_chat(config_path, session).await
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
        let mem_config = mvp::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        };
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

fn format_milli_ratio(value: Option<u32>) -> String {
    value
        .map(|raw| format!("{:.3}", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
}

async fn run_telegram_serve_cli(
    config_path: Option<&str>,
    once: bool,
    account: Option<&str>,
) -> CliResult<()> {
    mvp::channel::run_telegram_channel(config_path, once, account).await
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
    mvp::channel::run_feishu_channel(config_path, account, bind_override, path_override).await
}

fn parse_json_payload(raw: &str, context: &str) -> CliResult<Value> {
    serde_json::from_str(raw).map_err(|error| format!("invalid JSON for {context}: {error}"))
}

fn read_spec_file(path: &str) -> CliResult<RunnerSpec> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read spec file {path}: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("failed to parse spec file {path}: {error}"))
}

fn write_json_file<T: Serialize>(path: &str, value: &T) -> CliResult<()> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize JSON value for output file failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create output directory failed: {error}"))?;
        }
    }
    fs::write(path, serialized)
        .map_err(|error| format!("write JSON output file failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn safe_lane_summary_cli_rejects_zero_limit() {
        let error = run_safe_lane_summary_cli(None, Some("session-a"), 0, false)
            .expect_err("zero limit must be rejected");
        assert!(error.contains(">= 1"));
    }
}
