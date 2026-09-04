//! Minimal `loong` CLI.
//!
//! Two modes:
//! - `chat`: one supervisor agent that delegates file work to one file_io
//!   agent;
//! - `workflow`: planner / reviewer / workers workflow.

use std::io::Write;

use clap::{Parser, Subcommand};
use contracts::provider::StreamItem;
use crossterm::terminal;

mod chat;
mod input;
mod workflow;

#[derive(Parser)]
#[command(name = "loong")]
struct Cli {
    /// OpenAI-compatible base URL.
    #[arg(
        long,
        env = "LOONG_OPENAI_BASE_URL",
        default_value = "https://api.openai.com/v1"
    )]
    base_url: String,

    /// OpenAI-compatible API key.
    #[arg(long, env = "LOONG_OPENAI_API_KEY", default_value = "")]
    api_key: String,

    /// Model name.
    #[arg(long, env = "LOONG_OPENAI_MODEL", default_value = "gpt-4o-mini")]
    model: String,

    /// Workspace root for file tools.
    #[arg(long, env = "LOONG_WORKSPACE", default_value = ".")]
    workspace: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Clone, Subcommand)]
enum Command {
    /// One supervisor agent that delegates file work to one file_io agent.
    Chat,
    /// Planner / reviewer / workers workflow.
    Workflow {
        /// Maximum number of workers a plan may request.
        #[arg(long, default_value_t = 8)]
        max_workers: usize,

        /// Maximum plan/final review rounds.
        #[arg(long, default_value_t = 3)]
        max_review_rounds: usize,

        /// Maximum retries when the planner/reviewer returns invalid JSON or
        /// an invalid plan.
        #[arg(long, default_value_t = 3)]
        max_llm_retries: usize,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let command = cli.command.clone().unwrap_or(Command::Chat);

    match command {
        Command::Chat => chat::run(cli).await,
        Command::Workflow {
            max_workers,
            max_review_rounds,
            max_llm_retries,
        } => workflow::run(cli, max_workers, max_review_rounds, max_llm_retries).await,
    }
}

fn show_prompt() {
    print!("loong> ");
    let _ = std::io::stdout().flush();
}

fn print_stream_item(item: StreamItem) {
    match item {
        StreamItem::Text { delta } => {
            print!("{delta}");
            let _ = std::io::stdout().flush();
        }
        StreamItem::ReasoningDelta { .. } => {}
        StreamItem::ToolCall {
            id,
            name,
            arguments,
        } => {
            println!("\n[tool_call {name}] id={id} args={arguments}");
        }
    }
}

/// RAII guard that leaves the terminal in raw mode until it is dropped.
///
/// Raw mode is only active while a reply streams. Line input outside this
/// block runs in the usual cooked mode.
struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}
