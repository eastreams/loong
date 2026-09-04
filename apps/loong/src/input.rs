//! User input as a provider-like command source.

use tokio::io::{AsyncBufReadExt, BufReader, Lines, Stdin};

/// One parsed user interaction.
pub(crate) enum UserCommand {
    /// Send a prompt to the active agent or workflow.
    Prompt(String),
    /// Switch the model used by subsequent prompts.
    SwitchModel(String),
    /// End the interactive session.
    Quit,
}

/// Async source of cooked line input, parsed into [`UserCommand`]s.
///
/// This is the local analogue of a [`provider::Provider`]: it streams user
/// commands out of stdin instead of model replies out of an upstream.
pub(crate) struct StdinUserInput {
    lines: Lines<BufReader<Stdin>>,
}

impl StdinUserInput {
    pub(crate) fn new(stdin: Stdin) -> Self {
        Self {
            lines: BufReader::new(stdin).lines(),
        }
    }

    /// Reads lines until one parses into a command, then returns it.
    ///
    /// `None` means stdin reached EOF. Empty lines and invalid `/model`
    /// invocations are skipped with the same prompt/error behavior as the
    /// previous hand-written loops.
    pub(crate) async fn next(&mut self) -> Result<Option<UserCommand>, std::io::Error> {
        loop {
            super::show_prompt();
            let Some(line) = self.lines.next_line().await? else {
                return Ok(None);
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == "/quit" || line == "/exit" {
                return Ok(Some(UserCommand::Quit));
            }
            if let Some(model) = line.strip_prefix("/model") {
                let model = model.trim();
                if model.is_empty() {
                    eprintln!("usage: /model <model>");
                    continue;
                }
                return Ok(Some(UserCommand::SwitchModel(model.to_string())));
            }
            return Ok(Some(UserCommand::Prompt(line.to_string())));
        }
    }
}
