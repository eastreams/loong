use std::path::Path;

use crate::config::LoongConfig;

/// Mirror config-backed runtime knobs into process environment and singleton
/// runtime caches.
///
/// Use this before spawning child processes or surfaces that still consume
/// `LOONG_*` environment variables. This helper intentionally does not
/// bootstrap a kernel token, choose a chat session, or validate durable chat
/// state; higher-level entrypoints compose those steps separately.
pub fn initialize_runtime_environment(config: &LoongConfig, resolved_config_path: Option<&Path>) {
    match resolved_config_path {
        Some(path) => {
            let value = path.display().to_string();
            set_env_var("LOONG_CONFIG_PATH", value.clone());
            set_env_var("LOONG_CONFIG_PATH", value);
        }
        None => {
            remove_env_var("LOONG_CONFIG_PATH");
            remove_env_var("LOONG_CONFIG_PATH");
        }
    }

    set_env_var(
        "LOONG_MEMORY_BACKEND",
        config.memory.resolved_backend().as_str(),
    );
    set_env_var(
        "LOONG_MEMORY_PROFILE",
        config.memory.resolved_profile().as_str(),
    );
    set_env_var(
        "LOONG_SQLITE_PATH",
        config.memory.resolved_sqlite_path().display().to_string(),
    );
    set_env_var(
        "LOONG_SLIDING_WINDOW",
        config.memory.sliding_window.to_string(),
    );
    set_env_var(
        "LOONG_MEMORY_SUMMARY_MAX_CHARS",
        config.memory.summary_char_budget().to_string(),
    );
    match config.memory.trimmed_profile_note() {
        Some(profile_note) => set_env_var("LOONG_MEMORY_PROFILE_NOTE", profile_note),
        None => remove_env_var("LOONG_MEMORY_PROFILE_NOTE"),
    }

    set_env_var("LOONG_SHELL_ALLOWLIST", config.tools.shell_allow.join(","));
    set_env_var("LOONG_SHELL_DENY", config.tools.shell_deny.join(","));
    set_env_var(
        "LOONG_SHELL_DEFAULT_MODE",
        config.tools.shell_default_mode.as_str(),
    );
    let configured_file_root = config.tools.configured_file_root();
    match configured_file_root {
        Some(configured_file_root) => {
            let configured_file_root_text = configured_file_root.display().to_string();
            set_env_var("LOONG_FILE_ROOT", configured_file_root_text);
        }
        None => remove_env_var("LOONG_FILE_ROOT"),
    }
    let workspace_root = std::env::current_dir()
        .ok()
        .unwrap_or_else(|| config.tools.resolved_file_root());
    let workspace_root = dunce::canonicalize(&workspace_root).unwrap_or(workspace_root);
    set_env_var("LOONG_WORKSPACE_ROOT", workspace_root.display().to_string());
    set_env_var(
        "LOONG_TOOL_SESSIONS_ENABLED",
        bool_env(config.tools.sessions.enabled),
    );
    set_env_var(
        "LOONG_TOOL_SESSIONS_ALLOW_MUTATION",
        bool_env(config.tools.sessions.allow_mutation),
    );
    set_env_var(
        "LOONG_TOOL_MESSAGES_ENABLED",
        bool_env(config.tools.messages.enabled),
    );
    set_env_var(
        "LOONG_TOOL_DELEGATE_ENABLED",
        bool_env(config.tools.delegate.enabled),
    );
    set_env_var(
        "LOONG_BROWSER_ENABLED",
        bool_env(config.tools.browser.enabled),
    );
    set_env_var(
        "LOONG_BROWSER_MAX_SESSIONS",
        config.tools.browser.max_sessions.to_string(),
    );
    set_env_var(
        "LOONG_BROWSER_MAX_LINKS",
        config.tools.browser.max_links.to_string(),
    );
    set_env_var(
        "LOONG_BROWSER_MAX_TEXT_CHARS",
        config.tools.browser.max_text_chars.to_string(),
    );
    set_env_var(
        "LOONG_BROWSER_COMPANION_ENABLED",
        bool_env(config.tools.browser_companion.enabled),
    );
    set_env_var(
        "LOONG_BROWSER_COMPANION_TIMEOUT_SECONDS",
        config.tools.browser_companion.timeout_seconds.to_string(),
    );
    match normalized_optional_str(config.tools.browser_companion.command.as_deref()) {
        Some(command) => set_env_var("LOONG_BROWSER_COMPANION_COMMAND", command),
        None => remove_env_var("LOONG_BROWSER_COMPANION_COMMAND"),
    }
    match normalized_optional_str(config.tools.browser_companion.expected_version.as_deref()) {
        Some(expected_version) => {
            set_env_var("LOONG_BROWSER_COMPANION_EXPECTED_VERSION", expected_version)
        }
        None => remove_env_var("LOONG_BROWSER_COMPANION_EXPECTED_VERSION"),
    }
    set_env_var(
        "LOONG_WEB_FETCH_ENABLED",
        bool_env(config.tools.web.enabled),
    );
    set_env_var(
        "LOONG_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
        bool_env(config.tools.web.allow_private_hosts),
    );
    set_env_var(
        "LOONG_WEB_FETCH_ALLOWED_DOMAINS",
        config.tools.web.normalized_allowed_domains().join(","),
    );
    set_env_var(
        "LOONG_WEB_FETCH_BLOCKED_DOMAINS",
        config.tools.web.normalized_blocked_domains().join(","),
    );
    set_env_var(
        "LOONG_WEB_FETCH_TIMEOUT_SECONDS",
        config.tools.web.timeout_seconds.to_string(),
    );
    set_env_var(
        "LOONG_WEB_FETCH_MAX_BYTES",
        config.tools.web.max_bytes.to_string(),
    );
    set_env_var(
        "LOONG_WEB_FETCH_MAX_REDIRECTS",
        config.tools.web.max_redirects.to_string(),
    );
    set_env_var(
        "LOONG_EXTERNAL_SKILLS_ENABLED",
        bool_env(config.external_skills.enabled),
    );
    set_env_var(
        "LOONG_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
        bool_env(config.external_skills.require_download_approval),
    );
    set_env_var(
        "LOONG_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
        config
            .external_skills
            .normalized_allowed_domains()
            .join(","),
    );
    set_env_var(
        "LOONG_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
        config
            .external_skills
            .normalized_blocked_domains()
            .join(","),
    );
    match config.external_skills.resolved_install_root() {
        Some(path) => set_env_var(
            "LOONG_EXTERNAL_SKILLS_INSTALL_ROOT",
            path.display().to_string(),
        ),
        None => remove_env_var("LOONG_EXTERNAL_SKILLS_INSTALL_ROOT"),
    }
    set_env_var(
        "LOONG_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED",
        bool_env(config.external_skills.auto_expose_installed),
    );

    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
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

fn normalized_optional_str(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
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

    use crate::config::{LoongConfig, MemoryProfile};
    use crate::test_support::ScopedEnv;

    use super::*;

    fn clear_runtime_environment_exports(env: &mut ScopedEnv) {
        for key in [
            "LOONG_CONFIG_PATH",
            "LOONG_MEMORY_BACKEND",
            "LOONG_MEMORY_PROFILE",
            "LOONG_SQLITE_PATH",
            "LOONG_SLIDING_WINDOW",
            "LOONG_MEMORY_SUMMARY_MAX_CHARS",
            "LOONG_MEMORY_PROFILE_NOTE",
            "LOONG_SHELL_ALLOWLIST",
            "LOONG_SHELL_DENY",
            "LOONG_SHELL_DEFAULT_MODE",
            "LOONG_FILE_ROOT",
            "LOONG_WORKSPACE_ROOT",
            "LOONG_TOOL_SESSIONS_ALLOW_MUTATION",
            "LOONG_EXTERNAL_SKILLS_ENABLED",
            "LOONG_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
            "LOONG_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
            "LOONG_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
            "LOONG_EXTERNAL_SKILLS_INSTALL_ROOT",
            "LOONG_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED",
            "LOONG_BROWSER_ENABLED",
            "LOONG_BROWSER_MAX_SESSIONS",
            "LOONG_BROWSER_MAX_LINKS",
            "LOONG_BROWSER_MAX_TEXT_CHARS",
            "LOONG_BROWSER_COMPANION_ENABLED",
            "LOONG_BROWSER_COMPANION_TIMEOUT_SECONDS",
            "LOONG_BROWSER_COMPANION_COMMAND",
            "LOONG_BROWSER_COMPANION_EXPECTED_VERSION",
            "LOONG_WEB_FETCH_ENABLED",
            "LOONG_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
            "LOONG_WEB_FETCH_ALLOWED_DOMAINS",
            "LOONG_WEB_FETCH_BLOCKED_DOMAINS",
            "LOONG_WEB_FETCH_TIMEOUT_SECONDS",
            "LOONG_WEB_FETCH_MAX_BYTES",
            "LOONG_WEB_FETCH_MAX_REDIRECTS",
        ] {
            env.remove(key);
        }
    }

    #[test]
    fn initialize_runtime_environment_exports_core_env_vars() {
        let mut env = ScopedEnv::new();
        clear_runtime_environment_exports(&mut env);
        let mut config = LoongConfig::default();
        config.memory.profile = MemoryProfile::WindowPlusSummary;
        config.memory.summary_max_chars = 900;
        config.memory.profile_note = Some("Imported NanoBot preferences".to_owned());
        config.tools.file_root = Some("/tmp/loong-runtime-file-root".to_owned());
        config.tools.sessions.allow_mutation = true;
        config.tools.browser.enabled = false;
        config.tools.browser.max_sessions = 4;
        config.tools.browser.max_links = 12;
        config.tools.browser.max_text_chars = 2048;
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some("loong-browser-companion".to_owned());
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
        let config_path = PathBuf::from("/tmp/loong-runtime-env.toml");

        initialize_runtime_environment(&config, Some(&config_path));

        assert_eq!(
            std::env::var("LOONG_CONFIG_PATH").ok().as_deref(),
            Some("/tmp/loong-runtime-env.toml")
        );
        assert_eq!(
            std::env::var("LOONG_MEMORY_PROFILE").ok().as_deref(),
            Some("window_plus_summary")
        );
        assert_eq!(
            std::env::var("LOONG_MEMORY_SUMMARY_MAX_CHARS")
                .ok()
                .as_deref(),
            Some("900")
        );
        assert_eq!(
            std::env::var("LOONG_MEMORY_PROFILE_NOTE").ok().as_deref(),
            Some("Imported NanoBot preferences")
        );
        assert_eq!(
            std::env::var("LOONG_FILE_ROOT").ok().as_deref(),
            Some("/tmp/loong-runtime-file-root")
        );
        let expected_workspace_root =
            std::env::current_dir().expect("current_dir should resolve during runtime env tests");
        let expected_workspace_root =
            dunce::canonicalize(&expected_workspace_root).unwrap_or(expected_workspace_root);
        let expected_workspace_root = expected_workspace_root.display().to_string();
        assert_eq!(
            std::env::var("LOONG_WORKSPACE_ROOT").ok().as_deref(),
            Some(expected_workspace_root.as_str())
        );
        assert_eq!(
            std::env::var("LOONG_TOOL_SESSIONS_ALLOW_MUTATION")
                .ok()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            std::env::var("LOONG_EXTERNAL_SKILLS_ENABLED")
                .ok()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            std::env::var("LOONG_EXTERNAL_SKILLS_ALLOWED_DOMAINS")
                .ok()
                .as_deref(),
            Some("skills.sh")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_ENABLED").ok().as_deref(),
            Some("false")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_MAX_SESSIONS").ok().as_deref(),
            Some("4")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_MAX_LINKS").ok().as_deref(),
            Some("12")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_MAX_TEXT_CHARS")
                .ok()
                .as_deref(),
            Some("2048")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_COMPANION_ENABLED")
                .ok()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_COMPANION_TIMEOUT_SECONDS")
                .ok()
                .as_deref(),
            Some("30")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_COMPANION_COMMAND")
                .ok()
                .as_deref(),
            Some("loong-browser-companion")
        );
        assert_eq!(
            std::env::var("LOONG_BROWSER_COMPANION_EXPECTED_VERSION")
                .ok()
                .as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            std::env::var("LOONG_WEB_FETCH_ENABLED").ok().as_deref(),
            Some("false")
        );
        assert_eq!(
            std::env::var("LOONG_WEB_FETCH_ALLOW_PRIVATE_HOSTS")
                .ok()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            std::env::var("LOONG_WEB_FETCH_ALLOWED_DOMAINS")
                .ok()
                .as_deref(),
            Some("docs.example.com")
        );
        assert_eq!(
            std::env::var("LOONG_WEB_FETCH_BLOCKED_DOMAINS")
                .ok()
                .as_deref(),
            Some("internal.example")
        );
        assert_eq!(
            std::env::var("LOONG_WEB_FETCH_TIMEOUT_SECONDS")
                .ok()
                .as_deref(),
            Some("9")
        );
        assert_eq!(
            std::env::var("LOONG_WEB_FETCH_MAX_BYTES").ok().as_deref(),
            Some("262144")
        );
        assert_eq!(
            std::env::var("LOONG_WEB_FETCH_MAX_REDIRECTS")
                .ok()
                .as_deref(),
            Some("1")
        );
    }

    #[test]
    fn initialize_runtime_environment_drops_blank_browser_companion_metadata() {
        let mut env = ScopedEnv::new();
        clear_runtime_environment_exports(&mut env);
        let mut config = LoongConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.timeout_seconds = 7;
        config.tools.browser_companion.command = Some("   ".to_owned());
        config.tools.browser_companion.expected_version = Some("\n\t".to_owned());

        initialize_runtime_environment(&config, None);

        assert_eq!(
            std::env::var("LOONG_BROWSER_COMPANION_TIMEOUT_SECONDS")
                .ok()
                .as_deref(),
            Some("7")
        );
        assert_eq!(std::env::var("LOONG_BROWSER_COMPANION_COMMAND").ok(), None);
        assert_eq!(
            std::env::var("LOONG_BROWSER_COMPANION_EXPECTED_VERSION").ok(),
            None
        );
    }

    #[test]
    fn initialize_runtime_environment_leaves_file_root_unset_when_not_configured() {
        let mut env = ScopedEnv::new();
        clear_runtime_environment_exports(&mut env);
        env.set("LOONG_FILE_ROOT", "/tmp/stale-root");
        let config = LoongConfig::default();

        initialize_runtime_environment(&config, None);

        assert_eq!(std::env::var("LOONG_FILE_ROOT").ok(), None);
    }
}
