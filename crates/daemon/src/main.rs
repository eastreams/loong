#[cfg(test)]
use std::{collections::BTreeMap, time::Duration};
use std::{collections::BTreeSet, fs, path::Path, sync::Arc};

#[cfg(test)]
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use clap::{Parser, Subcommand};
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
};
#[cfg(test)]
pub(crate) use loongclaw_spec::programmatic::{
    acquire_programmatic_circuit_slot, record_programmatic_circuit_outcome,
};
#[cfg(test)]
mod tests;

#[derive(Parser, Debug)]
#[command(name = "loongclawd", about = "LoongClaw low-level runtime daemon")]
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
    /// Generate a beginner-friendly TOML config and bootstrap local state
    Setup {
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
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
    /// Run Telegram channel polling/response loop
    TelegramServe {
        #[arg(long)]
        config: Option<String>,
        #[arg(long, default_value_t = false)]
        once: bool,
    },
    /// Send one Feishu message or card
    FeishuSend {
        #[arg(long)]
        config: Option<String>,
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
        bind: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
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
        Commands::Setup { output, force } => run_setup_cli(output.as_deref(), force),
        Commands::ListModels { config, json } => run_list_models_cli(config.as_deref(), json).await,
        Commands::Chat { config, session } => {
            run_chat_cli(config.as_deref(), session.as_deref()).await
        }
        Commands::TelegramServe { config, once } => {
            run_telegram_serve_cli(config.as_deref(), once).await
        }
        Commands::FeishuSend {
            config,
            receive_id,
            text,
            card,
        } => run_feishu_send_cli(config.as_deref(), &receive_id, &text, card).await,
        Commands::FeishuServe { config, bind, path } => {
            run_feishu_serve_cli(config.as_deref(), bind.as_deref(), path.as_deref()).await
        }
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
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
        payload: json!({"repo": "chumyin/LoongClaw"}),
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
        let memory_db =
            mvp::memory::ensure_memory_db_ready(Some(parsed.memory.resolved_sqlite_path()))
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

async fn run_chat_cli(config_path: Option<&str>, session: Option<&str>) -> CliResult<()> {
    mvp::chat::run_cli_chat(config_path, session).await
}

async fn run_telegram_serve_cli(config_path: Option<&str>, once: bool) -> CliResult<()> {
    mvp::channel::run_telegram_channel(config_path, once).await
}

async fn run_feishu_send_cli(
    config_path: Option<&str>,
    receive_id: &str,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    mvp::channel::run_feishu_send(config_path, receive_id, text, as_card).await
}

async fn run_feishu_serve_cli(
    config_path: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    mvp::channel::run_feishu_channel(config_path, bind_override, path_override).await
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
