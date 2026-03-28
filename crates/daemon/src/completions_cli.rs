use clap::CommandFactory;
use clap_complete::{Shell, generate};

use loongclaw_spec::CliResult;

const CLI_COMPLETIONS_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

pub struct CompletionsCommandOptions {
    pub shell: Shell,
}

/// Generate completions into an arbitrary writer — enables unit testing without stdout capture.
pub fn generate_completions(shell: Shell, writer: &mut dyn std::io::Write) -> CliResult<()> {
    let rendered = render_completions(shell)?;
    writer
        .write_all(rendered.as_slice())
        .map_err(|error| format!("write generated completions failed: {error}"))?;
    Ok(())
}

fn render_completions(shell: Shell) -> CliResult<Vec<u8>> {
    let thread_builder = std::thread::Builder::new();
    let thread_builder = thread_builder.name("cli-completions-render".to_owned());
    let thread_builder = thread_builder.stack_size(CLI_COMPLETIONS_STACK_SIZE_BYTES);
    let join_handle = thread_builder
        .spawn(move || {
            let mut rendered = Vec::new();
            let mut command = crate::Cli::command();
            generate(shell, &mut command, crate::CLI_COMMAND_NAME, &mut rendered);
            rendered
        })
        .map_err(|error| format!("spawn completions render thread failed: {error}"))?;
    let rendered = join_handle
        .join()
        .map_err(|_panic| "completions render thread panicked".to_owned())?;
    Ok(rendered)
}

pub fn run_completions_cli(options: CompletionsCommandOptions) -> CliResult<()> {
    generate_completions(options.shell, &mut std::io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_bash_non_empty() {
        let mut buf = Vec::new();
        generate_completions(Shell::Bash, &mut buf).expect("generate bash completions");
        assert!(!buf.is_empty());
    }

    #[test]
    fn completions_zsh_contains_binary_name() {
        let mut buf = Vec::new();
        generate_completions(Shell::Zsh, &mut buf).expect("generate zsh completions");
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("loongclaw"));
    }

    #[test]
    fn completions_fish_non_empty() {
        let mut buf = Vec::new();
        generate_completions(Shell::Fish, &mut buf).expect("generate fish completions");
        assert!(!buf.is_empty());
    }

    #[test]
    fn completions_powershell_non_empty() {
        let mut buf = Vec::new();
        generate_completions(Shell::PowerShell, &mut buf).expect("generate powershell completions");
        assert!(!buf.is_empty());
    }

    #[test]
    fn completions_elvish_non_empty() {
        let mut buf = Vec::new();
        generate_completions(Shell::Elvish, &mut buf).expect("generate elvish completions");
        assert!(!buf.is_empty());
    }

    #[test]
    fn run_completions_cli_returns_ok() {
        let result = run_completions_cli(CompletionsCommandOptions { shell: Shell::Fish });
        assert!(result.is_ok());
    }
}
