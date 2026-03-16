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
        "LOONGCLAW_TOOL_SESSIONS_ENABLED",
        bool_env(config.tools.sessions.enabled),
    );
    set_env_var(
        "LOONGCLAW_TOOL_MESSAGES_ENABLED",
        bool_env(config.tools.messages.enabled),
    );
    set_env_var(
        "LOONGCLAW_TOOL_DELEGATE_ENABLED",
        bool_env(config.tools.delegate.enabled),
    );
    set_env_var(
        "LOONGCLAW_BROWSER_ENABLED",
        bool_env(config.tools.browser.enabled),
    );
    set_env_var(
        "LOONGCLAW_BROWSER_MAX_SESSIONS",
        config.tools.browser.max_sessions.to_string(),
    );
    set_env_var(
        "LOONGCLAW_BROWSER_MAX_LINKS",
        config.tools.browser.max_links.to_string(),
    );
    set_env_var(
        "LOONGCLAW_BROWSER_MAX_TEXT_CHARS",
        config.tools.browser.max_text_chars.to_string(),
    );
    set_env_var(
        "LOONGCLAW_BROWSER_COMPANION_ENABLED",
        bool_env(config.tools.browser_companion.enabled),
    );
    match config.tools.browser_companion.command.as_deref() {
        Some(command) => set_env_var("LOONGCLAW_BROWSER_COMPANION_COMMAND", command),
        None => remove_env_var("LOONGCLAW_BROWSER_COMPANION_COMMAND"),
    }
    match config.tools.browser_companion.expected_version.as_deref() {
        Some(expected_version) => set_env_var(
            "LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION",
            expected_version,
        ),
        None => remove_env_var("LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION"),
    }
    set_env_var(
        "LOONGCLAW_WEB_FETCH_ENABLED",
        bool_env(config.tools.web.enabled),
    );
    set_env_var(
        "LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
        bool_env(config.tools.web.allow_private_hosts),
    );
    set_env_var(
        "LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS",
        config.tools.web.normalized_allowed_domains().join(","),
    );
    set_env_var(
        "LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS",
        config.tools.web.normalized_blocked_domains().join(","),
    );
    set_env_var(
        "LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS",
        config.tools.web.timeout_seconds.to_string(),
    );
    set_env_var(
        "LOONGCLAW_WEB_FETCH_MAX_BYTES",
        config.tools.web.max_bytes.to_string(),
    );
    set_env_var(
        "LOONGCLAW_WEB_FETCH_MAX_REDIRECTS",
        config.tools.web.max_redirects.to_string(),
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

    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        resolved_config_path,
    );
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
        config.tools.browser.enabled = false;
        config.tools.browser.max_sessions = 4;
        config.tools.browser.max_links = 12;
        config.tools.browser.max_text_chars = 2048;
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some("loongclaw-browser-companion".to_owned());
        config.tools.browser_companion.expected_version = Some("1.2.3".to_owned());
        config.tools.web.enabled = false;
        config.tools.web.allow_private_hosts = true;
        config.tools.web.allowed_domains = vec!["docs.example.com".to_owned()];
        config.tools.web.blocked_domains = vec!["internal.example".to_owned()];
        config.tools.web.timeout_seconds = 9;
        config.tools.web.max_bytes = 262_144;
        config.tools.web.max_redirects = 1;
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
        assert_eq!(
            std::env::var("LOONGCLAW_BROWSER_ENABLED").ok().as_deref(),
            Some("false")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_BROWSER_MAX_SESSIONS")
                .ok()
                .as_deref(),
            Some("4")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_BROWSER_MAX_LINKS").ok().as_deref(),
            Some("12")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_BROWSER_MAX_TEXT_CHARS")
                .ok()
                .as_deref(),
            Some("2048")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_BROWSER_COMPANION_ENABLED")
                .ok()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_BROWSER_COMPANION_COMMAND")
                .ok()
                .as_deref(),
            Some("loongclaw-browser-companion")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION")
                .ok()
                .as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_WEB_FETCH_ENABLED").ok().as_deref(),
            Some("false")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS")
                .ok()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS")
                .ok()
                .as_deref(),
            Some("docs.example.com")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS")
                .ok()
                .as_deref(),
            Some("internal.example")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS")
                .ok()
                .as_deref(),
            Some("9")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_WEB_FETCH_MAX_BYTES")
                .ok()
                .as_deref(),
            Some("262144")
        );
        assert_eq!(
            std::env::var("LOONGCLAW_WEB_FETCH_MAX_REDIRECTS")
                .ok()
                .as_deref(),
            Some("1")
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
        crate::process_env::remove_var("LOONGCLAW_BROWSER_ENABLED");
        crate::process_env::remove_var("LOONGCLAW_BROWSER_MAX_SESSIONS");
        crate::process_env::remove_var("LOONGCLAW_BROWSER_MAX_LINKS");
        crate::process_env::remove_var("LOONGCLAW_BROWSER_MAX_TEXT_CHARS");
        crate::process_env::remove_var("LOONGCLAW_BROWSER_COMPANION_ENABLED");
        crate::process_env::remove_var("LOONGCLAW_BROWSER_COMPANION_COMMAND");
        crate::process_env::remove_var("LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION");
        crate::process_env::remove_var("LOONGCLAW_WEB_FETCH_ENABLED");
        crate::process_env::remove_var("LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS");
        crate::process_env::remove_var("LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS");
        crate::process_env::remove_var("LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS");
        crate::process_env::remove_var("LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS");
        crate::process_env::remove_var("LOONGCLAW_WEB_FETCH_MAX_BYTES");
        crate::process_env::remove_var("LOONGCLAW_WEB_FETCH_MAX_REDIRECTS");
    }
}
