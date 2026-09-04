//! Chat mode: one supervisor agent that delegates file work to one file_io
//! agent.

use std::sync::Arc;

use agent::{Agent, CancelActivePrompt, FileTools, Prompt, SwitchProvider};
use context::memory::MemoryStore;
use contracts::capability::Capability;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent};
use futures::StreamExt;
use kernel::{Facade, Kernel, policy::engine::PolicyEngine};
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
