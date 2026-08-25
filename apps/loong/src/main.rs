//! Minimal `loong` chat loop.
//!
//! This is a wiring-only CLI. It spawns one agent with an in-memory context
//! store and one OpenAI-compatible provider, then feeds stdin lines as user
//! prompts and prints streamed replies.

use std::env;
use std::io::Write;

use agent::{AgentProfile, FileIoAgent, FileIoProfile, Prompt, SwitchProvider};
use app::App;
use context::memory::MemoryStore;
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

    let profile = FileIoProfile;
    let kernel_owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let facade = Facade::for_owner(&kernel_owner, profile.capabilities());
    let mut app = App::new(facade);
    profile.register_tools(&mut app);
    let workspace_root = env_or("LOONG_WORKSPACE", ".");
    let owner = loac::spawn::<FileIoAgent<_, _>>((
        MemoryStore::new(),
        OpenAiProvider::new(config),
        app,
        workspace_root.into(),
        profile,
    ));
    let agent_ref = owner.actor_ref();

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    show_prompt();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "/quit" || line == "/exit" {
            break;
        }
        if let Some(new_model) = line.strip_prefix("/model ") {
            let new_model = new_model.trim();
            if new_model.is_empty() {
                eprintln!("usage: /model <model>");
                continue;
            }
            let config =
                OpenAiConfig::new(base_url.clone(), api_key.clone(), new_model.to_string());
            agent_ref
                .call(SwitchProvider(OpenAiProvider::new(config)))
                .await?;
            println!("switched to {new_model}");
            continue;
        }

        let mut reply = agent_ref.call(Prompt { text: line.clone() }).await?;

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
            Err(error) => eprintln!("\nstream error: {error}"),
        }
        println!();
        show_prompt();
    }

    let _ = owner.shutdown(Shutdown::Drain).await;
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
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
