//! Chat mode: one supervisor agent that delegates file work to one file_io
//! agent.

use std::{collections::HashMap, sync::Arc};

use agent::{CancelActivePrompt, ChannelTarget, Prompt, SwitchProvider};
use config::{AgentConfig, ProviderConfig, StoreConfig, ToolConfig};
use contracts::capability::{Capabilities, Capability};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent};
use futures::StreamExt;
use kernel::{Kernel, policy::engine::PolicyEngine};
use loac::Shutdown;
use provider_openai::{OpenAiConfig, OpenAiProvider};

use crate::{
    Cli, RawModeGuard,
    input::{StdinUserInput, UserCommand},
    print_stream_item,
};

pub(crate) async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = cli.model.clone();
    let config = OpenAiConfig::new(cli.base_url.clone(), cli.api_key.clone(), model.clone());
    let capabilities = [Capability::FsRead, Capability::FsWrite]
        .into_iter()
        .collect::<Capabilities>();

    let kernel = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());

    let file_io = AgentConfig {
        name: "file_io".into(),
        system_prompt: Some(
            "You are a file I/O agent. Use read_file and write_file for workspace files.".into(),
        ),
        workspace_root: cli.workspace.clone().into(),
        capabilities,
        store: StoreConfig::Memory,
        provider: ProviderConfig::OpenAi(config.clone()),
        tools: vec![ToolConfig::FileTools],
        channels: vec![],
    }
    .spawn_from_kernel(kernel.actor_ref(), &HashMap::new())?;

    let mut channels: HashMap<String, Arc<dyn ChannelTarget>> = HashMap::new();
    channels.insert("file_io".into(), Arc::new(file_io.actor_ref()));

    let owner = AgentConfig {
        name: "owner".into(),
        system_prompt: Some(
            "You are a supervisor agent. You do not read or write files yourself. \
             When the user needs file work, delegate it to the file_io agent by \
             calling the file_io tool with a clear instruction prompt in English, \
             then report its answer."
                .into(),
        ),
        workspace_root: cli.workspace.clone().into(),
        capabilities,
        store: StoreConfig::Memory,
        provider: ProviderConfig::OpenAi(config),
        tools: vec![],
        channels: vec!["file_io".into()],
    }
    .spawn_from_kernel(kernel.actor_ref(), &channels)?;

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
