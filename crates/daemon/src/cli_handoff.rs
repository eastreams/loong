use loongclaw_app as mvp;

pub(crate) fn shell_quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn format_subcommand_with_config(subcommand: &str, config_path: &str) -> String {
    format!(
        "{} {} --config {}",
        mvp::config::CLI_COMMAND_NAME,
        subcommand,
        shell_quote_argument(config_path)
    )
}

pub(crate) fn format_ask_with_config(config_path: &str, message: &str) -> String {
    format!(
        "{} ask --config {} --message \"{}\"",
        mvp::config::CLI_COMMAND_NAME,
        shell_quote_argument(config_path),
        message
    )
}

#[cfg(test)]
mod tests {
    use super::{format_ask_with_config, format_subcommand_with_config, shell_quote_argument};

    #[test]
    fn shell_quote_argument_escapes_single_quotes() {
        assert_eq!(
            shell_quote_argument("/tmp/loongclaw's config.toml"),
            "'/tmp/loongclaw'\"'\"'s config.toml'"
        );
    }

    #[test]
    fn format_subcommand_with_config_shell_quotes_the_config_path() {
        assert_eq!(
            format_subcommand_with_config("doctor", "/tmp/loongclaw's config.toml"),
            "loongclaw doctor --config '/tmp/loongclaw'\"'\"'s config.toml'"
        );
    }

    #[test]
    fn format_ask_with_config_shell_quotes_the_config_path() {
        assert_eq!(
            format_ask_with_config("/tmp/loongclaw's config.toml", "say it's ready"),
            "loongclaw ask --config '/tmp/loongclaw'\"'\"'s config.toml' --message \"say it's ready\""
        );
    }
}
