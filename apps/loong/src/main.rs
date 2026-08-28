//! Minimal `loong` chat loop.
//!
//! This is a wiring-only CLI. It spawns two agents:
//! - a supervisor agent that answers the user directly and owns no file
//!   tools;
//! - a file I/O agent behind a named channel tool, so every file operation is
//!   delegated by the supervisor.
//!
//! Both agents use an in-memory context store and one OpenAI-compatible
//! provider. Stdin lines are fed to the supervisor as prompts and streamed
//! replies are printed.

use std::env;
use std::io::Write;
use std::sync::Arc;

use agent::{Agent, FileTools, Prompt, SwitchProvider};
use context::memory::MemoryStore;
use contracts::capability::Capability;
use contracts::provider::StreamItem;
use kernel::{Facade, Kernel, policy::engine::PolicyEngine};
use loac::Shutdown;
use provider_openai::{OpenAiConfig, OpenAiProvider};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = env_or("LOONG_OPENAI_BASE_URL", "https://api.openai.com/v1");
    let api_key = env_or("LOONG_OPENAI_API_KEY", "");
    let model = env_or("LOONG_OPENAI_MODEL", "gpt-4o-mini");

    let config = OpenAiConfig::new(base_url.clone(), api_key.clone(), model.clone());

    let kernel = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let facade = Facade::new(
        kernel.actor_ref(),
        [Capability::FsRead, Capability::FsWrite],
    );
    let workspace_root = env_or("LOONG_WORKSPACE", ".");

    let file_io = Agent::builder(facade.clone())
        .with(FileTools)
        .with_workspace_root(&workspace_root)
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

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    loop {
        show_prompt();
        let Some(line) = lines.next_line().await? else {
            break;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "/quit" || line == "/exit" {
            break;
        }
        if let Some(new_model) = line.strip_prefix("/model") {
            let new_model = new_model.trim();
            if new_model.is_empty() {
                eprintln!("usage: /model <model>");
                continue;
            }
            let config =
                OpenAiConfig::new(base_url.clone(), api_key.clone(), new_model.to_string());
            owner
                .call(SwitchProvider(OpenAiProvider::new(config.clone())))
                .await?;
            file_io
                .call(SwitchProvider(OpenAiProvider::new(config)))
                .await?;
            println!("switched to {new_model}");
            continue;
        }

        let mut reply = owner.call(Prompt { text: line.clone() }).await?;

        while let Some(item) = reply.recv().await {
            match item {
                StreamItem::Text { delta } => {
                    print!("{delta}");
                    let _ = std::io::stdout().flush();
                }
                StreamItem::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    println!("\n[tool_call {name}] id={id} args={arguments}");
                }
            }
        }

        match reply.finish().await? {
            Ok(()) => {}
            Err(error) => eprintln!("\nprompt error: {error}"),
        }
        println!();
    }

    let _ = owner.shutdown(Shutdown::Drain).await;
    let _ = file_io.shutdown(Shutdown::Drain).await;
    let _ = kernel.shutdown(Shutdown::Drain).await;
    Ok(())
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn show_prompt() {
    print!("loong> ");
    let _ = std::io::stdout().flush();
}
