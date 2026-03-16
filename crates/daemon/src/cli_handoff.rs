use loongclaw_app as mvp;

pub(crate) const DEFAULT_SETUP_SMOKE_TEST_MESSAGE: &str = "say hello and verify this setup";

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn format_subcommand_with_config(subcommand: &str, config_path: &str) -> String {
    format!(
        "{} {} --config {}",
        mvp::config::CLI_COMMAND_NAME,
        subcommand,
        shell_quote(config_path)
    )
}

pub(crate) fn format_ask_with_config(config_path: &str, message: &str) -> String {
    format!(
        "{} ask --config {} --message {}",
        mvp::config::CLI_COMMAND_NAME,
        shell_quote(config_path),
        shell_quote(message)
    )
}

#[cfg(test)]
mod tests {
    use super::{format_ask_with_config, shell_quote};

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("/tmp/o'hara.toml"), "'/tmp/o'\"'\"'hara.toml'");
    }

    #[test]
    fn format_ask_with_config_shell_quotes_message() {
        assert_eq!(
            format_ask_with_config("/tmp/loongclaw.toml", "say it's ready"),
            "loongclaw ask --config '/tmp/loongclaw.toml' --message 'say it'\"'\"'s ready'"
        );
    }
}
