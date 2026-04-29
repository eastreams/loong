use std::path::Path;

use loong_app as mvp;

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

pub(crate) fn format_root_entry_with_config(config_path: &str) -> String {
    let command_name = mvp::config::active_cli_command_name();
    if Path::new(config_path) == crate::resolved_default_entry_config_path().as_path() {
        return command_name.to_owned();
    }

    format!(
        "LOONG_CONFIG_PATH={} {}",
        shell_quote_argument(config_path),
        command_name
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
    use super::{
        format_ask_with_config, format_root_entry_with_config, format_subcommand_with_config,
        shell_quote_argument,
    };
    use crate::test_support::ScopedEnv;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn shell_quote_argument_escapes_single_quotes() {
        assert_eq!(
            shell_quote_argument("/tmp/loong's config.toml"),
            "'/tmp/loong'\"'\"'s config.toml'"
        );
    }

    #[test]
    fn format_subcommand_with_config_shell_quotes_the_config_path() {
        assert_eq!(
            format_subcommand_with_config("doctor", "/tmp/loong's config.toml"),
            "loong doctor --config '/tmp/loong'\"'\"'s config.toml'"
        );
    }

    #[test]
    fn format_ask_with_config_shell_quotes_the_config_path() {
        assert_eq!(
            format_ask_with_config("/tmp/loong's config.toml", "say it's ready"),
            "loong ask --config '/tmp/loong'\"'\"'s config.toml' --message 'say it'\"'\"'s ready'"
        );
    }

    #[test]
    fn format_ask_with_config_shell_quotes_message_content() {
        assert_eq!(
            format_ask_with_config("/tmp/loong.toml", "say \"hi\" and print $HOME"),
            "loong ask --config '/tmp/loong.toml' --message 'say \"hi\" and print $HOME'"
        );
    }

    #[test]
    fn format_root_entry_with_config_prefers_plain_root_command_for_default_path() {
        let mut env = ScopedEnv::new();
        let home = std::env::temp_dir().join(format!(
            "loong-cli-handoff-home-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(home.join(".loong")).expect("create home config dir");
        env.set("HOME", &home);
        env.remove("LOONG_HOME");
        env.remove("LOONG_CONFIG_PATH");

        let default_config_path = crate::resolved_default_entry_config_path();
        assert_eq!(
            format_root_entry_with_config(default_config_path.to_str().expect("utf8 config path")),
            "loong"
        );
    }

    #[test]
    fn format_root_entry_with_config_uses_env_override_for_non_default_paths() {
        assert_eq!(
            format_root_entry_with_config("/tmp/loong's config.toml"),
            "LOONG_CONFIG_PATH='/tmp/loong'\"'\"'s config.toml' loong"
        );
    }
}
