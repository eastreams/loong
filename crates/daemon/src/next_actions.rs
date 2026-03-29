use std::ffi::OsStr;

use loongclaw_app as mvp;

pub use mvp::chat::DEFAULT_FIRST_PROMPT as DEFAULT_FIRST_ASK_MESSAGE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupNextActionKind {
    Ask,
    Chat,
    Channel,
    BrowserPreview,
    Doctor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserPreviewActionPhase {
    Ready,
    Unblock,
    Enable,
    InstallRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupNextAction {
    pub kind: SetupNextActionKind,
    pub channel_action_id: Option<&'static str>,
    pub browser_preview_phase: Option<BrowserPreviewActionPhase>,
    pub label: String,
    pub command: String,
}

pub fn collect_setup_next_actions(
    config: &mvp::config::LoongClawConfig,
    config_path: &str,
) -> Vec<SetupNextAction> {
    let path_env = std::env::var_os("PATH");
    collect_setup_next_actions_with_path_env(config, config_path, path_env.as_deref())
}

pub(crate) fn collect_setup_next_actions_with_path_env(
    config: &mvp::config::LoongClawConfig,
    config_path: &str,
    path_env: Option<&OsStr>,
) -> Vec<SetupNextAction> {
    let mut actions = Vec::new();
    let browser_preview =
        crate::browser_preview::inspect_browser_preview_state_with_path_env(config, path_env);
    if config.cli.enabled {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Ask,
            channel_action_id: None,
            browser_preview_phase: None,
            label: "first answer".to_owned(),
            command: crate::cli_handoff::format_ask_with_config(
                config_path,
                DEFAULT_FIRST_ASK_MESSAGE,
            ),
        });
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Chat,
            channel_action_id: None,
            browser_preview_phase: None,
            label: "chat".to_owned(),
            command: crate::cli_handoff::format_subcommand_with_config("chat", config_path),
        });
    }
    actions.extend(
        crate::migration::channels::collect_channel_next_actions(config, config_path)
            .into_iter()
            .map(|action| SetupNextAction {
                kind: SetupNextActionKind::Channel,
                channel_action_id: Some(action.id),
                browser_preview_phase: None,
                label: action.label.to_owned(),
                command: action.command,
            }),
    );
    if config.cli.enabled {
        let preview_action = if browser_preview.ready() {
            Some(SetupNextAction {
                kind: SetupNextActionKind::BrowserPreview,
                channel_action_id: None,
                browser_preview_phase: Some(BrowserPreviewActionPhase::Ready),
                label: crate::browser_preview::BROWSER_PREVIEW_READY_LABEL.to_owned(),
                command: crate::browser_preview::browser_preview_ready_command(config_path),
            })
        } else if browser_preview.needs_shell_unblock() {
            Some(SetupNextAction {
                kind: SetupNextActionKind::BrowserPreview,
                channel_action_id: None,
                browser_preview_phase: Some(BrowserPreviewActionPhase::Unblock),
                label: crate::browser_preview::BROWSER_PREVIEW_UNBLOCK_LABEL.to_owned(),
                command: crate::browser_preview::browser_preview_unblock_command(config_path),
            })
        } else if browser_preview.needs_enable_command() {
            Some(SetupNextAction {
                kind: SetupNextActionKind::BrowserPreview,
                channel_action_id: None,
                browser_preview_phase: Some(BrowserPreviewActionPhase::Enable),
                label: crate::browser_preview::BROWSER_PREVIEW_ENABLE_LABEL.to_owned(),
                command: crate::browser_preview::browser_preview_enable_command(config_path),
            })
        } else if browser_preview.needs_runtime_install() {
            Some(SetupNextAction {
                kind: SetupNextActionKind::BrowserPreview,
                channel_action_id: None,
                browser_preview_phase: Some(BrowserPreviewActionPhase::InstallRuntime),
                label: format!("install {}", mvp::tools::BROWSER_COMPANION_COMMAND),
                command: crate::browser_preview::browser_preview_install_command().to_owned(),
            })
        } else {
            None
        };
        if let Some(action) = preview_action {
            actions.push(action);
        }
    }
    if actions.is_empty() {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            channel_action_id: None,
            browser_preview_phase: None,
            label: "doctor".to_owned(),
            command: crate::cli_handoff::format_subcommand_with_config("doctor", config_path),
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    #[cfg(unix)]
    fn write_fake_agent_browser(bin_dir: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let path = bin_dir.join("agent-browser");
        fs::create_dir_all(bin_dir).expect("create bin dir");
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write fake agent-browser");
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("set executable bit");
    }

    #[cfg(windows)]
    fn write_fake_agent_browser(bin_dir: &Path) {
        fs::create_dir_all(bin_dir).expect("create bin dir");
        fs::write(bin_dir.join("agent-browser.exe"), b"").expect("write fake agent-browser");
    }

    #[cfg(unix)]
    fn write_non_executable_agent_browser(bin_dir: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let path = bin_dir.join("agent-browser");
        fs::create_dir_all(bin_dir).expect("create bin dir");
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write fake agent-browser");
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&path, permissions).expect("clear executable bit");
    }

    fn assert_channel_catalog_action(action: &SetupNextAction) {
        assert_eq!(action.kind, SetupNextActionKind::Channel);
        assert_eq!(
            action.channel_action_id,
            Some(crate::migration::channels::CHANNEL_CATALOG_ACTION_ID)
        );
        assert_eq!(action.browser_preview_phase, None);
        assert_eq!(action.label, "channels");
        assert_eq!(
            action.command,
            "loong channels --config '/tmp/loongclaw.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_promotes_browser_companion_preview_when_ready() {
        let root = unique_temp_dir("loongclaw-next-actions-browser-companion");
        let install_root = root.join("managed-skills");
        write_file(
            &install_root,
            "browser-companion-preview/SKILL.md",
            "# Browser Companion Preview\n\nUse agent-browser through shell.exec.\n",
        );
        let bin_dir = root.join("bin");
        write_fake_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_allow.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loongclaw.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_eq!(actions[0].kind, SetupNextActionKind::Ask);
        assert_eq!(actions[1].kind, SetupNextActionKind::Chat);
        assert_channel_catalog_action(&actions[2]);
        assert_eq!(actions[3].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[3].browser_preview_phase,
            Some(BrowserPreviewActionPhase::Ready)
        );
        assert_eq!(actions[3].label, "browser companion preview");
        assert!(
            actions[3]
                .command
                .contains("Use the browser companion preview to open https://example.com"),
            "ready preview action should hand users into a task-shaped first browser recipe: {actions:#?}"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_setup_next_actions_guides_browser_preview_shell_unblock_when_hard_denied() {
        let root = unique_temp_dir("loongclaw-next-actions-browser-companion-shell-deny");
        let install_root = root.join("managed-skills");
        write_file(
            &install_root,
            "browser-companion-preview/SKILL.md",
            "# Browser Companion Preview\n\nUse agent-browser through shell.exec.\n",
        );
        let bin_dir = root.join("bin");
        write_fake_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_deny.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loongclaw.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_channel_catalog_action(&actions[2]);
        assert_eq!(actions[3].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[3].browser_preview_phase,
            Some(BrowserPreviewActionPhase::Unblock)
        );
        assert_eq!(actions[3].label, "allow agent-browser");
        assert!(
            actions[3]
                .command
                .contains("remove `agent-browser` from [tools].shell_deny"),
            "shell hard-deny should produce an unblock step instead of looping back to enable-browser-preview: {actions:#?}"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_setup_next_actions_guides_browser_preview_enable_when_not_configured() {
        let root = unique_temp_dir("loongclaw-next-actions-browser-companion-enable");
        let bin_dir = root.join("bin");
        write_fake_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.file_root = Some(root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loongclaw.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_channel_catalog_action(&actions[2]);
        assert_eq!(actions[3].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[3].browser_preview_phase,
            Some(BrowserPreviewActionPhase::Enable)
        );
        assert!(
            actions[3].command.contains("enable-browser-preview"),
            "browser preview enable action should point operators at the preview bootstrap command: {actions:#?}"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn collect_setup_next_actions_requires_an_executable_agent_browser_binary() {
        let root = unique_temp_dir("loongclaw-next-actions-browser-companion-nonexec");
        let install_root = root.join("managed-skills");
        write_file(
            &install_root,
            "browser-companion-preview/SKILL.md",
            "# Browser Companion Preview\n\nUse agent-browser through shell.exec.\n",
        );
        let bin_dir = root.join("bin");
        write_non_executable_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_allow.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loongclaw.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_channel_catalog_action(&actions[2]);
        assert_eq!(actions[3].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[3].browser_preview_phase,
            Some(BrowserPreviewActionPhase::InstallRuntime)
        );
        assert_eq!(
            actions[3].label,
            format!("install {}", mvp::tools::BROWSER_COMPANION_COMMAND)
        );
        assert_eq!(
            actions[3].command,
            format!(
                "npm install -g {} && {} install",
                mvp::tools::BROWSER_COMPANION_COMMAND,
                mvp::tools::BROWSER_COMPANION_COMMAND
            )
        );

        fs::remove_dir_all(&root).ok();
    }
}
