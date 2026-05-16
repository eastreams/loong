use std::path::PathBuf;

use async_trait::async_trait;
use clap::Subcommand;
use loong_app_protocol::{
    AppProtocolInteractiveExecutor, AppProtocolOneshotExecutor, AppProtocolRuntimeExecutorRequest,
    AppProtocolRuntimeExecutorResult, AppProtocolRuntimeInteractiveExecutorRequest,
    AppProtocolRuntimeInteractiveExecutorResult, AppProtocolWorkspaceContext,
    InteractiveShellRequest, OneshotTurnRequest, execute_interactive_shell, execute_oneshot_turn,
    render_oneshot_turn_output,
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
    if !configured_chat_path_exists(config_path)? {
        let options =
            build_cli_chat_options(acp, acp_event_stream, acp_bootstrap_mcp_server, acp_cwd);
        return mvp::chat::run_cli_chat(config_path, session, &options).await;
    }

    run_spine_chat_cli(
        config_path,
        session,
        acp,
        acp_event_stream,
        acp_bootstrap_mcp_server,
        acp_cwd,
    )
    .await
}

async fn run_spine_chat_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    acp: bool,
    acp_event_stream: bool,
    acp_bootstrap_mcp_server: &[String],
    acp_cwd: Option<&str>,
) -> CliResult<()> {
    let protocol_request = InteractiveShellRequest {
        config_path: config_path.map(ToOwned::to_owned),
        session_hint: session.map(ToOwned::to_owned),
        acp,
        acp_event_stream,
        acp_bootstrap_mcp_servers: acp_bootstrap_mcp_server.to_vec(),
        acp_cwd: acp_cwd.map(ToOwned::to_owned),
    };
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let executor = LegacyInteractiveExecutor::new(resolved_path, config);
    let workspace = migrated_turn_workspace_context(executor.config())?;
    let _execution = execute_interactive_shell(&protocol_request, workspace, &executor).await?;
    Ok(())
}

fn configured_chat_path_exists(config_path: Option<&str>) -> CliResult<bool> {
    let resolved_config_path = config_path
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let exists = resolved_config_path.try_exists().map_err(|error| {
        format!(
            "failed to access config path {}: {error}",
            resolved_config_path.display()
        )
    })?;
    Ok(exists)
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
            turn_mode: mvp::agent_runtime::AgentTurnMode::Oneshot,
            metadata: std::collections::BTreeMap::new(),
            ..Default::default()
        };
        let projection_request = mvp::turn_gateway::TurnGatewayRequest {
            address: mvp::conversation::ConversationSessionAddress::from_session_id(
                request
                    .session_hint
                    .clone()
                    .unwrap_or_else(|| "default".to_owned()),
            ),
            message: turn_request.message.clone(),
            metadata: turn_request.metadata.clone(),
            turn_mode: turn_request.turn_mode,
            acp_routing_intent: if request.acp {
                mvp::acp::AcpRoutingIntent::Explicit
            } else {
                mvp::acp::AcpRoutingIntent::Automatic
            },
            acp_event_stream: request.acp_event_stream,
            acp_bootstrap_mcp_servers: request.acp_bootstrap_mcp_servers.clone(),
            acp_cwd: request.acp_cwd.clone(),
            live_surface_enabled: false,
            ingress: None,
            observer: None,
            provenance: mvp::turn_gateway::TurnGatewayProvenance::default(),
            provider_error_mode: mvp::conversation::ProviderErrorMode::InlineMessage,
            retry_progress: None,
        };
        let turn_options =
            mvp::turn_gateway::build_turn_execution_options(&projection_request, None);
        let result = turn_service
            .execute(request.session_hint.as_deref(), &turn_request, turn_options)
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

// Transitional Phase 3 adapter.
// Delete condition: once `loong-runtime` owns the real interactive shell
// executor without calling back into `loong-app`, remove this daemon-side
// bridge and route `chat` directly through the runtime-owned implementation.
struct LegacyInteractiveExecutor {
    resolved_path: PathBuf,
    config: mvp::config::LoongConfig,
}

impl LegacyInteractiveExecutor {
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
impl AppProtocolInteractiveExecutor for LegacyInteractiveExecutor {
    async fn run_interactive(
        &self,
        request: AppProtocolRuntimeInteractiveExecutorRequest,
    ) -> Result<AppProtocolRuntimeInteractiveExecutorResult, String> {
        let options = build_cli_chat_options(
            request.acp,
            request.acp_event_stream,
            &request.acp_bootstrap_mcp_servers,
            request.acp_cwd.as_deref(),
        );
        let resolved_path = self.resolved_path.to_string_lossy().into_owned();
        mvp::chat::run_cli_chat(
            Some(resolved_path.as_str()),
            request.session_hint.as_deref(),
            &options,
        )
        .await?;

        Ok(AppProtocolRuntimeInteractiveExecutorResult {
            session_id: resolved_interactive_session_id(
                self.config(),
                request.session_hint.as_deref(),
            )?,
            exit_state: "completed".to_owned(),
        })
    }
}

fn resolved_interactive_session_id(
    config: &mvp::config::LoongConfig,
    session_hint: Option<&str>,
) -> CliResult<String> {
    let session_id = session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");

    #[cfg(feature = "memory-sqlite")]
    if session_id == mvp::session::LATEST_SESSION_SELECTOR {
        let memory_config =
            mvp::session::store::SessionStoreConfig::from_memory_config(&config.memory);
        let latest_session_id = mvp::session::latest_resumable_root_session_id(&memory_config)?
            .ok_or_else(|| {
                "CLI session selector `latest` did not find any resumable root session".to_owned()
            })?;
        return Ok(latest_session_id);
    }

    Ok(session_id.to_owned())
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
