use std::path::PathBuf;

use crate::CliResult;
use crate::mvp;
use clap::Subcommand;
use loong_app_protocol::{
    AppProtocolWorkspaceContext, InteractiveShellRequest, OneshotTurnRequest,
    ProductionInteractiveExecutor, ProductionOneshotExecutor, execute_interactive_shell,
    execute_oneshot_turn, load_runtime_executor_config,
};

#[derive(Subcommand, Debug)]
pub enum TurnCommands {
    #[command(
        about = "Run one non-interactive assistant turn through the unified runtime",
        long_about = "Run one non-interactive assistant turn through the unified runtime.\n\nThis is the canonical one-shot turn entrypoint. It routes through the real agent runtime rather than the legacy demo harness path."
    )]
    Run {
        /// Path to the Loong config file, or omit to use normal config discovery
        #[arg(long)]
        config: Option<String>,
        /// Session id or selector such as `latest`; defaults to the normal CLI session
        #[arg(long)]
        session: Option<String>,
        /// User message to send through the canonical runtime turn entrypoint
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
}

pub async fn run_chat_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    _acp: bool,
    _acp_event_stream: bool,
    _acp_bootstrap_mcp_server: &[String],
    _acp_cwd: Option<&str>,
) -> CliResult<()> {
    run_spine_chat_cli(
        config_path,
        session,
        _acp,
        _acp_event_stream,
        _acp_bootstrap_mcp_server,
        _acp_cwd,
    )
    .await
}

async fn run_spine_chat_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    _acp: bool,
    _acp_event_stream: bool,
    _acp_bootstrap_mcp_server: &[String],
    _acp_cwd: Option<&str>,
) -> CliResult<()> {
    let protocol_request = InteractiveShellRequest {
        config_path: config_path.map(ToOwned::to_owned),
        session_hint: session.map(ToOwned::to_owned),
    };
    let runtime_executor_config = load_runtime_executor_config(config_path)?;
    let (_resolved_path, config) = load_chat_protocol_config(config_path)?;
    let host = mvp::runtime_protocol_host::LoongAppRuntimeProtocolHost::new();
    let executor = ProductionInteractiveExecutor::new(&host, runtime_executor_config);
    let workspace = migrated_turn_workspace_context(&config)?;
    let _execution = execute_interactive_shell(&protocol_request, workspace, &executor).await?;
    Ok(())
}

fn load_chat_protocol_config(
    config_path: Option<&str>,
) -> CliResult<(PathBuf, mvp::config::LoongConfig)> {
    let resolved_config_path = config_path
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let exists = resolved_config_path.try_exists().map_err(|error| {
        format!(
            "failed to access config path {}: {error}",
            resolved_config_path.display()
        )
    })?;

    if exists {
        return mvp::config::load(config_path);
    }

    Ok((resolved_config_path, mvp::config::LoongConfig::default()))
}

pub async fn run_ask_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    message: &str,
    _acp: bool,
    _acp_event_stream: bool,
    _acp_bootstrap_mcp_server: &[String],
    _acp_cwd: Option<&str>,
) -> CliResult<()> {
    run_spine_oneshot_cli(
        config_path,
        session,
        message,
        _acp,
        _acp_event_stream,
        _acp_bootstrap_mcp_server,
        _acp_cwd,
    )
    .await
}

pub async fn run_turn_run_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    message: &str,
    _acp: bool,
    _acp_event_stream: bool,
    _acp_bootstrap_mcp_server: &[String],
    _acp_cwd: Option<&str>,
) -> CliResult<()> {
    run_spine_oneshot_cli(
        config_path,
        session,
        message,
        _acp,
        _acp_event_stream,
        _acp_bootstrap_mcp_server,
        _acp_cwd,
    )
    .await
}

async fn run_spine_oneshot_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    message: &str,
    _acp: bool,
    _acp_event_stream: bool,
    _acp_bootstrap_mcp_server: &[String],
    _acp_cwd: Option<&str>,
) -> CliResult<()> {
    let protocol_request = OneshotTurnRequest {
        config_path: config_path.map(ToOwned::to_owned),
        session_hint: session.map(ToOwned::to_owned),
        message: message.to_owned(),
    };
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let host = mvp::runtime_protocol_host::LoongAppRuntimeProtocolHost::new();
    let runtime_executor_config = loong_app_protocol::RuntimeExecutorConfig {
        requested_config_path: config_path.map(ToOwned::to_owned),
        resolved_config_path: resolved_path,
        runtime_workspace_root: None,
        latest_session_selector: Some("latest".to_owned()),
    };
    let executor = ProductionOneshotExecutor::new(&host, runtime_executor_config);
    let workspace = migrated_turn_workspace_context(&config)?;
    let execution = execute_oneshot_turn(&protocol_request, workspace, &executor).await?;
    println!("{}", execution.output_text);
    Ok(())
}

fn migrated_turn_workspace_context(
    config: &mvp::config::LoongConfig,
) -> CliResult<AppProtocolWorkspaceContext> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let workspace_root = config
        .tools
        .configured_runtime_workspace_root()
        .or_else(|| config.tools.configured_file_root())
        .unwrap_or_else(|| cwd.clone());
    let workspace_root = dunce::canonicalize(&workspace_root).unwrap_or(workspace_root);
    let repo_root =
        resolve_git_repo_root(workspace_root.as_path()).unwrap_or_else(|_| workspace_root.clone());
    let worktree_root = workspace_root.clone();
    Ok(AppProtocolWorkspaceContext::new(
        workspace_root.clone(),
        repo_root,
        worktree_root,
        cwd,
        current_branch_identity(&workspace_root),
    ))
}

fn current_branch_identity(workspace_root: &std::path::Path) -> String {
    std::process::Command::new("git")
        .args(["-C"])
        .arg(workspace_root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn resolve_git_repo_root(base_root: &std::path::Path) -> Result<PathBuf, String> {
    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(base_root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("spawn git command failed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let display_path = base_root.display();
        return Err(format!(
            "resolve git repo root from `{display_path}` failed: {stderr}"
        ));
    }

    let raw_stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed_stdout = raw_stdout.trim();
    if trimmed_stdout.is_empty() {
        let display_path = base_root.display();
        return Err(format!(
            "resolve git repo root from `{display_path}` returned empty output"
        ));
    }

    Ok(PathBuf::from(trimmed_stdout))
}
