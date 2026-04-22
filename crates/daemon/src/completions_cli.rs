use clap_complete::{Shell, generate};

use loong_spec::CliResult;

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
    render_completions_for_command(shell, crate::active_cli_command_name())
}

fn render_completions_for_command(shell: Shell, command_name: &'static str) -> CliResult<Vec<u8>> {
    let thread_builder = std::thread::Builder::new();
    let thread_builder = thread_builder.name("cli-completions-render".to_owned());
    let thread_builder = thread_builder.stack_size(CLI_COMPLETIONS_STACK_SIZE_BYTES);
    let join_handle = thread_builder
        .spawn(move || {
            let mut command = crate::build_cli_command(command_name);
            let hidden_root_subcommands = command
                .get_subcommands()
                .filter(|subcommand| subcommand.is_hide_set())
                .map(|subcommand| subcommand.get_name().to_owned())
                .collect::<Vec<_>>();

            let mut rendered = Vec::new();
            generate(shell, &mut command, command_name, &mut rendered);

            if matches!(shell, Shell::Bash) && !hidden_root_subcommands.is_empty() {
                let rendered_text =
                    String::from_utf8(rendered).expect("bash completions should be utf8");
                strip_hidden_root_bash_aliases(&rendered_text, &hidden_root_subcommands)
                    .into_bytes()
            } else {
                rendered
            }
        })
        .map_err(|error| format!("spawn completions render thread failed: {error}"))?;
    let rendered = join_handle
        .join()
        .map_err(|_panic| "completions render thread panicked".to_owned())?;
    Ok(rendered)
}

fn strip_hidden_root_bash_aliases(rendered: &str, hidden_root_subcommands: &[String]) -> String {
    let hidden = hidden_root_subcommands
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let line_count = rendered.lines().count();
    let mut filtered = Vec::with_capacity(line_count);
    let mut lines = rendered.lines().peekable();

    while let Some(line) = lines.next() {

        if should_strip_hidden_root_case_block(line, &hidden) {
            for candidate in lines.by_ref() {
                if candidate.trim() == ";;" {
                    break;
                }
            }
            continue;
        }

        filtered.push(filter_hidden_root_opts_line(line, &hidden));
    }

    filtered.join("\n")
}

fn should_strip_hidden_root_case_block(line: &str, hidden_root_subcommands: &[&str]) -> bool {
    hidden_root_subcommands.iter().any(|name| {
        line.contains(&format!("loong,{name})"))
            || line.contains(&format!("loong__subcmd__help,{name})"))
    })
}

fn filter_hidden_root_opts_line(line: &str, hidden_root_subcommands: &[&str]) -> String {
    let Some(opts_index) = line.find("opts=\"") else {
        return line.to_owned();
    };
    let value_start = opts_index + "opts=\"".len();
    let Some(relative_end) = line[value_start..].find('"') else {
        return line.to_owned();
    };
    let value_end = value_start + relative_end;
    let filtered_tokens = line[value_start..value_end]
        .split_whitespace()
        .filter(|token| !hidden_root_subcommands.iter().any(|hidden| token == hidden))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "{}{}{}",
        &line[..value_start],
        filtered_tokens,
        &line[value_end..]
    )
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
        let out = String::from_utf8(
            render_completions_for_command(Shell::Zsh, crate::CLI_COMMAND_NAME)
                .expect("generate zsh completions"),
        )
        .unwrap();
        let expected = format!("#compdef {}", crate::CLI_COMMAND_NAME);
        assert!(out.contains(&expected));
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
