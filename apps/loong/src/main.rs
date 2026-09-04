//! Minimal `loong` CLI.
//!
//! Two modes:
//! - `chat`: one supervisor agent that delegates file work to one file_io
//!   agent;
//! - `workflow`: planner / reviewer / workers workflow.

use std::io::Write;
use std::sync::Arc;

use agent::{Agent, CancelActivePrompt, FileTools, Prompt, SwitchProvider};
use clap::{Parser, Subcommand};
use context::memory::MemoryStore;
use contracts::capability::Capability;
use contracts::provider::StreamItem;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent};
use crossterm::terminal;
use futures::StreamExt;
use kernel::{Facade, Kernel, policy::engine::PolicyEngine};
use loac::Shutdown;
use provider_openai::{OpenAiConfig, OpenAiProvider};

mod input;
mod workflow;

use input::{StdinUserInput, UserCommand};
use workflow::{RunGoal, Workflow};

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
        Command::Chat => run_chat(cli).await,
        Command::Workflow {
            max_workers,
            max_review_rounds,
            max_llm_retries,
        } => run_workflow(cli, max_workers, max_review_rounds, max_llm_retries).await,
    }
}

async fn run_chat(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = cli.model.clone();
    let config = OpenAiConfig::new(cli.base_url.clone(), cli.api_key.clone(), model.clone());

    let kernel = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let facade = Facade::new(
        kernel.actor_ref(),
        [Capability::FsRead, Capability::FsWrite],
    );

    let file_io = Agent::builder(facade.clone())
        .with(FileTools)
        .with_workspace_root(&cli.workspace)
        .with_system_prompt(
            "You are a file I/O agent. Use read_file and write_file for workspace files.",
        )
        .with_store(MemoryStore::new())
        .with_provider(OpenAiProvider::new(config.clone()))
        .build()?
        .spawn();

    let owner = Agent::builder(facade)
        .with_channel("file_io", Arc::new(file_io.actor_ref()))
        .with_system_prompt(
            "You are a supervisor agent. You do not read or write files yourself. \
             When the user needs file work, delegate it to the file_io agent by \
             calling the file_io tool with a clear instruction prompt in English, \
             then report its answer.",
        )
        .with_store(MemoryStore::new())
        .with_provider(OpenAiProvider::new(config))
        .build()?
        .spawn();

    let mut input = StdinUserInput::new(tokio::io::stdin());

    while let Some(command) = input.next().await? {
        match command {
            UserCommand::Quit => break,
            UserCommand::SwitchModel(new_model) => {
                model = new_model.clone();
                let config =
                    OpenAiConfig::new(cli.base_url.clone(), cli.api_key.clone(), model.clone());
                owner
                    .call(SwitchProvider(Arc::new(OpenAiProvider::new(
                        config.clone(),
                    ))))
                    .await?;
                file_io
                    .call(SwitchProvider(Arc::new(OpenAiProvider::new(config))))
                    .await?;
                println!("switched to {new_model}");
            }
            UserCommand::Prompt(text) => {
                let mut reply = owner.call(Prompt { text }).await?;

                {
                    // Raw mode lets us read an ESC keypress while the reply streams.
                    // The guard restores cooked mode at the end of this block even if
                    // streaming is cancelled or a read fails. When stdin is not a TTY
                    // (for example piped input), raw mode is unavailable and the
                    // fallback below streams without ESC handling.
                    let raw_mode = RawModeGuard::enter().ok();

                    if let Some(_raw_mode) = raw_mode {
                        let mut keys = EventStream::new();

                        loop {
                            tokio::select! {
                                maybe_item = reply.recv() => {
                                    let Some(item) = maybe_item else {
                                        break;
                                    };
                                    print_stream_item(item);
                                }
                                Some(Ok(Event::Key(KeyEvent { code: KeyCode::Esc, .. }))) = keys.next() => {
                                    // Idempotent: cancelling an already-cancelled or
                                    // finished prompt is a no-op in the agent.
                                    owner.call(CancelActivePrompt).await?;
                                    println!("\n[interrupted]");
                                }
                            }
                        }
                    } else {
                        while let Some(item) = reply.recv().await {
                            print_stream_item(item);
                        }
                    }

                    match reply.finish().await? {
                        Ok(()) => {}
                        Err(error) => eprintln!("\nprompt error: {error}"),
                    }
                    println!();
                }
            }
        }
    }

    let _ = owner.shutdown(Shutdown::Drain).await;
    let _ = file_io.shutdown(Shutdown::Drain).await;
    let _ = kernel.shutdown(Shutdown::Drain).await;
    Ok(())
}

async fn run_workflow(
    cli: Cli,
    max_workers: usize,
    max_review_rounds: usize,
    max_llm_retries: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = cli.model.clone();

    let kernel = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());

    let mut input = StdinUserInput::new(tokio::io::stdin());

    while let Some(command) = input.next().await? {
        match command {
            UserCommand::Quit => break,
            UserCommand::SwitchModel(new_model) => {
                model = new_model.clone();
                println!("switched to {new_model}");
            }
            UserCommand::Prompt(text) => {
                let facade = Facade::new(
                    kernel.actor_ref(),
                    [
                        Capability::FsRead,
                        Capability::FsWrite,
                        Capability::SpawnSubagent,
                    ],
                );
                let provider = Arc::new(OpenAiProvider::new(OpenAiConfig::new(
                    cli.base_url.clone(),
                    cli.api_key.clone(),
                    model.clone(),
                )));

                let workflow = Workflow::builder(facade, provider, &cli.workspace)
                    .with_max_workers(max_workers)
                    .with_max_review_rounds(max_review_rounds)
                    .with_max_llm_retries(max_llm_retries);
                let owner = workflow.spawn();

                let mut reply = match owner.call(RunGoal { goal: text }).await {
                    Ok(reply) => reply,
                    Err(error) => {
                        eprintln!("workflow error: {error}");
                        let _ = owner.shutdown(Shutdown::Drain).await;
                        continue;
                    }
                };

                while let Some(item) = reply.recv().await {
                    print_stream_item(item);
                }
                match reply.finish().await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!("workflow error: {error}"),
                    Err(error) => eprintln!("workflow error: {error}"),
                }
                println!();
                let _ = owner.shutdown(Shutdown::Drain).await;
            }
        }
    }

    let _ = kernel.shutdown(Shutdown::Drain).await;
    Ok(())
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
