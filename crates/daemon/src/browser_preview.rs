use std::{env, ffi::OsStr, path::PathBuf};

use loong_app as mvp;

pub(crate) const BROWSER_PREVIEW_SKILL_ID: &str = mvp::tools::BROWSER_COMPANION_PREVIEW_SKILL_ID;
pub(crate) const BROWSER_PREVIEW_ENABLE_LABEL: &str = "enable browser preview";
pub(crate) const BROWSER_PREVIEW_UNBLOCK_LABEL: &str = "allow agent-browser";
pub(crate) const BROWSER_PREVIEW_READY_LABEL: &str = "browser companion preview";
const BROWSER_PREVIEW_INSTALL_COMMAND: &str =
    "npm install -g agent-browser && agent-browser install";
const BROWSER_PREVIEW_VERIFY_COMMAND: &str = "agent-browser open example.com";
const BROWSER_PREVIEW_RECIPES: &[(&str, &str)] = &[
    (
        "summarize a page",
        "Use the browser companion preview to open https://example.com, snapshot the page, and summarize what is visible.",
    ),
    (
        "extract page text",
        "Use the browser companion preview to open https://example.com, extract the main page text, and return the key points.",
    ),
    (
        "follow a link",
        "Use the browser companion preview to open https://example.com, click the first relevant link, wait for navigation, and summarize the result.",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserPreviewRecipeCommand {
    pub(crate) label: String,
    pub(crate) command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserPreviewState {
    pub(crate) runtime_enabled: bool,
    pub(crate) auto_expose_installed: bool,
    pub(crate) skill_installed: bool,
    pub(crate) shell_allowed: bool,
    pub(crate) explicit_shell_deny: bool,
    pub(crate) runtime_available: bool,
}

impl BrowserPreviewState {
    pub(crate) fn ready(&self) -> bool {
        self.runtime_enabled
            && self.auto_expose_installed
            && self.skill_installed
            && self.shell_allowed
            && self.runtime_available
    }

    pub(crate) fn needs_enable_command(&self) -> bool {
        !self.explicit_shell_deny
            && (!self.runtime_enabled
                || !self.auto_expose_installed
                || !self.skill_installed
                || !self.shell_allowed)
    }

    pub(crate) fn needs_shell_unblock(&self) -> bool {
        self.explicit_shell_deny
    }

    pub(crate) fn needs_runtime_install(&self) -> bool {
        !self.runtime_available
    }

    fn has_preview_intent(&self) -> bool {
        self.runtime_enabled
            || self.auto_expose_installed
            || self.skill_installed
            || self.shell_allowed
    }
}

pub(crate) fn inspect_browser_preview_state(
    config: &mvp::config::LoongConfig,
) -> BrowserPreviewState {
    let path_env = env::var_os("PATH");
    inspect_browser_preview_state_with_path_env(config, path_env.as_deref())
}

pub(crate) fn inspect_browser_preview_state_with_path_env(
    config: &mvp::config::LoongConfig,
    path_env: Option<&OsStr>,
) -> BrowserPreviewState {
    BrowserPreviewState {
        runtime_enabled: config.external_skills.enabled,
        auto_expose_installed: config.external_skills.auto_expose_installed,
        skill_installed: bundled_skill_install_path(config).is_file(),
        shell_allowed: shell_policy_allows_command(config, mvp::tools::BROWSER_COMPANION_COMMAND),
        explicit_shell_deny: shell_policy_explicitly_denies_command(
            config,
            mvp::tools::BROWSER_COMPANION_COMMAND,
        ),
        runtime_available: command_on_path(mvp::tools::BROWSER_COMPANION_COMMAND, path_env),
    }
}

pub(crate) fn browser_preview_enable_command(config_path: &str) -> String {
    crate::cli_handoff::format_subcommand_with_config("skills enable-browser-preview", config_path)
}

pub(crate) fn browser_preview_unblock_command(config_path: &str) -> String {
    format!(
        "edit {} and remove `agent-browser` from [tools].shell_deny",
        crate::cli_handoff::shell_quote_argument(config_path)
    )
}

pub(crate) fn browser_preview_install_command() -> &'static str {
    BROWSER_PREVIEW_INSTALL_COMMAND
}

pub(crate) fn browser_preview_verify_command() -> &'static str {
    BROWSER_PREVIEW_VERIFY_COMMAND
}

pub(crate) fn browser_preview_install_step() -> String {
    format!(
        "Install browser preview runtime: {}",
        browser_preview_install_command()
    )
}

pub(crate) fn browser_preview_verify_step() -> String {
    format!(
        "Verify browser preview runtime: {}",
        browser_preview_verify_command()
    )
}

pub(crate) fn browser_preview_recipe_commands(
    config_path: &str,
) -> Vec<BrowserPreviewRecipeCommand> {
    BROWSER_PREVIEW_RECIPES
        .iter()
        .map(|(label, message)| BrowserPreviewRecipeCommand {
            label: (*label).to_owned(),
            command: browser_preview_ask_command(config_path, message),
        })
        .collect()
}

pub(crate) fn browser_preview_ready_command(config_path: &str) -> String {
    browser_preview_recipe_commands(config_path)
        .into_iter()
        .next()
        .map(|recipe| recipe.command)
        .unwrap_or_else(|| browser_preview_ask_command(config_path, "Use the browser companion preview to open https://example.com and summarize the result."))
}

pub(crate) fn ensure_browser_preview_config(config: &mut mvp::config::LoongConfig) -> bool {
    let mut updated = false;

    if config
        .tools
        .file_root
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        config.tools.file_root = Some(
            mvp::config::default_loong_home()
                .join("workspace")
                .display()
                .to_string(),
        );
        updated = true;
    }

    if !config.external_skills.enabled {
        config.external_skills.enabled = true;
        updated = true;
    }
    if !config.external_skills.auto_expose_installed {
        config.external_skills.auto_expose_installed = true;
        updated = true;
    }

    let command = mvp::tools::BROWSER_COMPANION_COMMAND;
    let shell_denied = shell_policy_explicitly_denies_command(config, command);
    let shell_default_allow = config
        .tools
        .shell_default_mode
        .eq_ignore_ascii_case("allow");
    let shell_allowed = config
        .tools
        .shell_allow
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(command));
    if !shell_denied && !shell_default_allow && !shell_allowed {
        config.tools.shell_allow.push(command.to_owned());
        updated = true;
    }

    updated
}

pub(crate) fn bundled_skill_install_path(config: &mvp::config::LoongConfig) -> PathBuf {
    let install_root = config
        .external_skills
        .resolved_install_root()
        .unwrap_or_else(|| {
            config
                .tools
                .resolved_file_root()
                .join("external-skills-installed")
        });
    install_root.join(BROWSER_PREVIEW_SKILL_ID).join("SKILL.md")
}

pub(crate) fn shell_policy_allows_command(
    config: &mvp::config::LoongConfig,
    command: &str,
) -> bool {
    if shell_policy_explicitly_denies_command(config, command) {
        return false;
    }
    if config
        .tools
        .shell_allow
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(command))
    {
        return true;
    }
    config
        .tools
        .shell_default_mode
        .eq_ignore_ascii_case("allow")
}

pub(crate) fn shell_policy_explicitly_denies_command(
    config: &mvp::config::LoongConfig,
    command: &str,
) -> bool {
    config
        .tools
        .shell_deny
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(command))
}

pub(crate) fn browser_preview_check(
    config: &mvp::config::LoongConfig,
    path_env: Option<&OsStr>,
) -> Option<crate::doctor_cli::DoctorCheck> {
    let state = inspect_browser_preview_state_with_path_env(config, path_env);
    if !state.has_preview_intent() {
        return None;
    }

    if state.ready() {
        return Some(crate::doctor_cli::DoctorCheck {
            name: "browser companion preview".to_owned(),
            level: crate::doctor_cli::DoctorCheckLevel::Pass,
            detail: "managed preview is ready".to_owned(),
        });
    }

    let mut missing = Vec::new();
    if !state.runtime_enabled {
        missing.push("external skills runtime is disabled");
    }
    if !state.auto_expose_installed {
        missing.push("installed skills are not auto-exposed");
    }
    if !state.skill_installed {
        missing.push("helper skill is not installed");
    }
    if state.explicit_shell_deny {
        missing.push("shell policy hard-denies `agent-browser`");
    } else if !state.shell_allowed {
        missing.push("shell policy does not allow `agent-browser`");
    }
    if !state.runtime_available {
        missing.push("`agent-browser` is not on PATH");
    }

    Some(crate::doctor_cli::DoctorCheck {
        name: "browser companion preview".to_owned(),
        level: crate::doctor_cli::DoctorCheckLevel::Warn,
        detail: format!("not ready ({})", missing.join("; ")),
    })
}

fn command_on_path(command: &str, path_env: Option<&OsStr>) -> bool {
    let Some(path_env) = path_env else {
        return false;
    };
    env::split_paths(path_env).any(|dir| {
        command_candidates(command)
            .into_iter()
            .map(|candidate| dir.join(&candidate))
            .any(|candidate| command_candidate_is_available(&candidate))
    })
}

fn command_candidates(command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        vec![
            command.to_owned(),
            format!("{command}.exe"),
            format!("{command}.cmd"),
            format!("{command}.bat"),
            format!("{command}.com"),
        ]
    }
    #[cfg(not(windows))]
    {
        vec![command.to_owned()]
    }
}

fn browser_preview_ask_command(config_path: &str, message: &str) -> String {
    crate::cli_handoff::format_ask_with_config(config_path, message)
}
#[cfg(unix)]
fn command_candidate_is_available(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn command_candidate_is_available(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::{
        browser_preview_enable_command, browser_preview_ready_command,
        browser_preview_unblock_command,
    };

    #[test]
    fn browser_preview_commands_shell_escape_config_paths() {
        let config_path = "/tmp/loong's config.toml";

        assert_eq!(
            browser_preview_enable_command(config_path),
            "loong skills enable-browser-preview --config '/tmp/loong'\"'\"'s config.toml'"
        );
        assert_eq!(
            browser_preview_unblock_command(config_path),
            "edit '/tmp/loong'\"'\"'s config.toml' and remove `agent-browser` from [tools].shell_deny"
        );
        assert!(
            browser_preview_ready_command(config_path)
                .starts_with("loong ask --config '/tmp/loong'\"'\"'s config.toml' --message "),
            "ready command should quote the config path for copy-paste safety"
        );
    }
}
