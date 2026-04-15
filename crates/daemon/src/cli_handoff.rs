use loongclaw_app as mvp;

pub(crate) fn shell_quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn format_subcommand_with_config_for_command(
    command_name: &str,
    subcommand: &str,
    config_path: &str,
) -> String {
    format!(
        "{} {} --config {}",
        command_name,
        subcommand,
        shell_quote_argument(config_path)
    )
}

pub(crate) fn format_subcommand_with_config(subcommand: &str, config_path: &str) -> String {
    format_subcommand_with_config_for_command(
        mvp::config::active_cli_command_name(),
        subcommand,
        config_path,
    )
}

pub(crate) fn format_ask_with_config_for_command(
    command_name: &str,
    config_path: &str,
    message: &str,
) -> String {
    format!(
        "{} ask --config {} --message {}",
        command_name,
        shell_quote_argument(config_path),
        shell_quote_argument(message)
    )
}

pub(crate) fn format_ask_with_config(config_path: &str, message: &str) -> String {
    format_ask_with_config_for_command(mvp::config::active_cli_command_name(), config_path, message)
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
            "loong doctor --config '/tmp/loongclaw'\"'\"'s config.toml'"
        );
    }

    #[test]
    fn format_ask_with_config_shell_quotes_the_config_path() {
        assert_eq!(
            format_ask_with_config("/tmp/loongclaw's config.toml", "say it's ready"),
            "loong ask --config '/tmp/loongclaw'\"'\"'s config.toml' --message 'say it'\"'\"'s ready'"
        );
    }

    #[test]
    fn format_ask_with_config_shell_quotes_message_content() {
        assert_eq!(
            format_ask_with_config("/tmp/loongclaw.toml", "say \"hi\" and print $HOME"),
            "loong ask --config '/tmp/loongclaw.toml' --message 'say \"hi\" and print $HOME'"
        );
    }
}
