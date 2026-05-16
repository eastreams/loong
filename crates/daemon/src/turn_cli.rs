use std::path::PathBuf;

use async_trait::async_trait;
use clap::Subcommand;
use loong_app_protocol::{
    AppProtocolOneshotExecutor, AppProtocolRuntimeExecutorRequest,
    AppProtocolRuntimeExecutorResult, AppProtocolWorkspaceContext, OneshotTurnRequest,
    execute_oneshot_turn, render_oneshot_turn_output,
};

use crate::CliResult;
use crate::mvp;

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
    run_spine_oneshot_cli(
        config_path,
        session,
        message,
        acp,
        acp_event_stream,
        acp_bootstrap_mcp_server,
        acp_cwd,
    )
    .await
}

pub async fn run_turn_run_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    message: &str,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> CliResult<()> {
    run_spine_oneshot_cli(
        config_path,
        session,
        message,
        acp,
        acp_event_stream,
        acp_bootstrap_mcp_server,
        acp_cwd,
    )
    .await
}

async fn run_spine_oneshot_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    message: &str,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> CliResult<()> {
    let protocol_request = OneshotTurnRequest {
        config_path: config_path.map(ToOwned::to_owned),
        session_hint: session.map(ToOwned::to_owned),
        message: message.to_owned(),
        acp,
        acp_event_stream,
        acp_bootstrap_mcp_servers: acp_bootstrap_mcp_server.to_vec(),
        acp_cwd: acp_cwd.map(ToOwned::to_owned),
    };
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let executor = LegacyOneshotExecutor::new(resolved_path, config);
    let workspace = migrated_turn_workspace_context(executor.config())?;
    let execution = execute_oneshot_turn(&protocol_request, workspace, &executor).await?;
    println!("{}", render_oneshot_turn_output(&execution));
    Ok(())
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
            .map(PathBuf::from),
    }
}

// Transitional Phase 3 adapter.
// Delete condition: once `loong-runtime` owns the real one-shot executor
// without calling back into `loong-app`, remove this daemon-side bridge and
// route `turn run` directly through the runtime-owned implementation.
struct LegacyOneshotExecutor {
    resolved_path: PathBuf,
    config: mvp::config::LoongConfig,
}

impl LegacyOneshotExecutor {
    fn new(resolved_path: PathBuf, config: mvp::config::LoongConfig) -> Self {
        Self {
            resolved_path,
            config,
        }
    }

    fn config(&self) -> &mvp::config::LoongConfig {
        &self.config
    }
}

#[async_trait]
impl AppProtocolOneshotExecutor for LegacyOneshotExecutor {
    async fn execute(
        &self,
        request: AppProtocolRuntimeExecutorRequest,
    ) -> Result<AppProtocolRuntimeExecutorResult, String> {
        let turn_service = mvp::agent_runtime::TurnExecutionService::new(
            self.resolved_path.clone(),
            self.config.clone(),
        );
        let turn_request = mvp::agent_runtime::AgentTurnRequest {
            message: request.message,
            turn_mode: if request.acp {
                mvp::agent_runtime::AgentTurnMode::Acp
            } else {
                mvp::agent_runtime::AgentTurnMode::Oneshot
            },
            metadata: std::collections::BTreeMap::new(),
            acp: request.acp,
            acp_event_stream: request.acp_event_stream,
            acp_bootstrap_mcp_servers: request.acp_bootstrap_mcp_servers,
            acp_cwd: request.acp_cwd,
            ..Default::default()
        };
        let result = turn_service
            .execute(
                request.session_hint.as_deref(),
                &turn_request,
                mvp::agent_runtime::TurnExecutionOptions::default(),
            )
            .await?;

        Ok(AppProtocolRuntimeExecutorResult {
            session_id: result.session_id,
            output_text: result.output_text,
            state: result.state,
            stop_reason: result.stop_reason,
            event_count: result.event_count,
        })
    }
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
