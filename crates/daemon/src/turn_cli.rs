use std::path::PathBuf;

use clap::{Args, Subcommand};

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

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractiveCliArgs {
    /// Path to the Loong config file, or omit to use normal config discovery
    #[arg(long)]
    pub config: Option<String>,
    /// Session id or selector such as `latest`; defaults to the normal CLI session
    #[arg(long)]
    pub session: Option<String>,
    /// Enable ACP bridge behavior for this interactive session
    #[arg(long, default_value_t = false)]
    pub acp: bool,
    /// Stream ACP turn events while interactive turns run
    #[arg(long, default_value_t = false)]
    pub acp_event_stream: bool,
    /// Bootstrap an MCP server before the ACP session starts; repeat to add more servers
    #[arg(long = "acp-bootstrap-mcp-server")]
    pub acp_bootstrap_mcp_server: Vec<String>,
    /// Working directory used for ACP and bootstrapped MCP server context
    #[arg(long = "acp-cwd")]
    pub acp_cwd: Option<String>,
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
    crate::task_execution::run_turn_cli(
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
