use std::path::Path;

use crate::config::LoongClawConfig;

pub fn initialize_runtime_environment(
    config: &LoongClawConfig,
    resolved_config_path: Option<&Path>,
) {
    match resolved_config_path {
        Some(path) => set_env_var("LOONGCLAW_CONFIG_PATH", path.display().to_string()),
        None => remove_env_var("LOONGCLAW_CONFIG_PATH"),
    }

    set_env_var(
        "LOONGCLAW_MEMORY_BACKEND",
        config.memory.resolved_backend().as_str(),
    );
    set_env_var(
        "LOONGCLAW_MEMORY_PROFILE",
        config.memory.resolved_profile().as_str(),
    );
    set_env_var(
        "LOONGCLAW_SQLITE_PATH",
        config.memory.resolved_sqlite_path().display().to_string(),
    );
    set_env_var(
        "LOONGCLAW_SLIDING_WINDOW",
        config.memory.sliding_window.to_string(),
    );
    set_env_var(
        "LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS",
        config.memory.summary_char_budget().to_string(),
    );
    match config.memory.trimmed_profile_note() {
        Some(profile_note) => set_env_var("LOONGCLAW_MEMORY_PROFILE_NOTE", profile_note),
        None => remove_env_var("LOONGCLAW_MEMORY_PROFILE_NOTE"),
    }

    set_env_var(
        "LOONGCLAW_SHELL_ALLOWLIST",
        config.tools.shell_allow.join(","),
    );
    set_env_var("LOONGCLAW_SHELL_DENY", config.tools.shell_deny.join(","));
    set_env_var(
        "LOONGCLAW_SHELL_DEFAULT_MODE",
        config.tools.shell_default_mode.as_str(),
    );
    set_env_var(
        "LOONGCLAW_FILE_ROOT",
        config.tools.resolved_file_root().display().to_string(),
    );
    set_env_var(
        "LOONGCLAW_EXTERNAL_SKILLS_ENABLED",
        bool_env(config.external_skills.enabled),
    );
    set_env_var(
        "LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
        bool_env(config.external_skills.require_download_approval),
    );
    set_env_var(
        "LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
        config
            .external_skills
            .normalized_allowed_domains()
            .join(","),
    );
    set_env_var(
        "LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
        config
            .external_skills
            .normalized_blocked_domains()
            .join(","),
    );
    match config.external_skills.resolved_install_root() {
        Some(path) => set_env_var(
            "LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT",
            path.display().to_string(),
        ),
        None => remove_env_var("LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT"),
    }
    set_env_var(
        "LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED",
        bool_env(config.external_skills.auto_expose_installed),
    );

    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig {
        shell_allow: config
            .tools
            .shell_allow
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect(),
        shell_deny: config
            .tools
            .shell_deny
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect(),
        file_root: Some(config.tools.resolved_file_root()),
        config_path: resolved_config_path.map(Path::to_path_buf),
        shell_default_mode: crate::tools::shell_policy_ext::ShellPolicyDefault::parse(
            &config.tools.shell_default_mode,
        ),
        external_skills: crate::tools::runtime_config::ExternalSkillsRuntimePolicy {
            enabled: config.external_skills.enabled,
            require_download_approval: config.external_skills.require_download_approval,
            allowed_domains: config
                .external_skills
                .normalized_allowed_domains()
                .into_iter()
                .collect(),
            blocked_domains: config
                .external_skills
                .normalized_blocked_domains()
                .into_iter()
                .collect(),
            install_root: config.external_skills.resolved_install_root(),
            auto_expose_installed: config.external_skills.auto_expose_installed,
        },
    };
    let _ = crate::tools::runtime_config::init_tool_runtime_config(tool_rt);

    let memory_rt =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let _ = crate::memory::runtime_config::init_memory_runtime_config(memory_rt);
}

fn bool_env(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn set_env_var(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    crate::process_env::set_var(key, value);
}

fn remove_env_var(key: &str) {
    crate::process_env::remove_var(key);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::{LoongClawConfig, MemoryProfile};

    use super::*;

    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[test]
    fn initialize_runtime_environment_exports_core_env_vars() {
        let _guard = env_lock().lock().expect("env lock");
        let mut config = LoongClawConfig::default();
        config.memory.profile = MemoryProfile::WindowPlusSummary;
        config.memory.summary_max_chars = 900;
        config.memory.profile_note = Some("Imported NanoBot preferences".to_owned());
        config.tools.file_root = Some("/tmp/loongclaw-runtime-file-root".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.allowed_domains = vec!["skills.sh".to_owned()];
        let config_path = PathBuf::from("/tmp/loongclaw-runtime-env.toml");

        initialize_runtime_environment(&config, Some(&config_path));

        assert_eq!(
            std::env::var("LOONGCLAW_CONFIG_PATH").ok().as_deref(),
            Some("/tmp/loongclaw-runtime-env.toml")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_MEMORY_PROFILE").ok().as_deref(),
            Some("window_plus_summary")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS")
                .ok()
                .as_deref(),
            Some("900")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_MEMORY_PROFILE_NOTE")
                .ok()
                .as_deref(),
            Some("Imported NanoBot preferences")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_FILE_ROOT").ok().as_deref(),
            Some("/tmp/loongclaw-runtime-file-root")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_EXTERNAL_SKILLS_ENABLED")
                .ok()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS")
                .ok()
                .as_deref(),
            Some("skills.sh")
        );

        crate::process_env::remove_var("LOONGCLAW_CONFIG_PATH");
        crate::process_env::remove_var("LOONGCLAW_MEMORY_BACKEND");
        crate::process_env::remove_var("LOONGCLAW_MEMORY_PROFILE");
        crate::process_env::remove_var("LOONGCLAW_SQLITE_PATH");
        crate::process_env::remove_var("LOONGCLAW_SLIDING_WINDOW");
        crate::process_env::remove_var("LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS");
        crate::process_env::remove_var("LOONGCLAW_MEMORY_PROFILE_NOTE");
        crate::process_env::remove_var("LOONGCLAW_SHELL_ALLOWLIST");
        crate::process_env::remove_var("LOONGCLAW_SHELL_DENY");
        crate::process_env::remove_var("LOONGCLAW_SHELL_DEFAULT_MODE");
        crate::process_env::remove_var("LOONGCLAW_FILE_ROOT");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_ENABLED");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT");
        crate::process_env::remove_var("LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED");
    }
}
