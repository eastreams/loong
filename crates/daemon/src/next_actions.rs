use std::collections::BTreeSet;
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
    let channel_actions =
        crate::migration::channels::collect_channel_next_actions(config, config_path);
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
    if should_add_managed_bridge_doctor_action(config, &channel_actions) {
        let doctor_action = build_managed_bridge_doctor_action(config_path);
        actions.push(doctor_action);
    }
    let channel_setup_actions = channel_actions
        .into_iter()
        .map(channel_next_action_to_setup_action);
    actions.extend(channel_setup_actions);
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

fn should_add_managed_bridge_doctor_action(
    config: &mvp::config::LoongClawConfig,
    channel_actions: &[crate::migration::channels::ChannelNextAction],
) -> bool {
    let has_catalog_only_channel_handoff = channel_actions_are_catalog_only(channel_actions);

    if !has_catalog_only_channel_handoff {
        return false;
    }

    has_unresolved_plugin_bridge_preflight(config)
}

fn channel_actions_are_catalog_only(
    channel_actions: &[crate::migration::channels::ChannelNextAction],
) -> bool {
    if channel_actions.len() != 1 {
        return false;
    }

    let Some(action) = channel_actions.first() else {
        return false;
    };

    action.id == crate::migration::channels::CHANNEL_CATALOG_ACTION_ID
}

fn has_unresolved_plugin_bridge_preflight(config: &mvp::config::LoongClawConfig) -> bool {
    let plugin_bridge_surface_names = collect_enabled_plugin_bridge_surface_names(config);

    if plugin_bridge_surface_names.is_empty() {
        return false;
    }

    let channel_checks = crate::migration::channels::collect_channel_preflight_checks(config);

    channel_checks.into_iter().any(|check| {
        let check_name = check.name;
        let check_is_plugin_bridge_surface = plugin_bridge_surface_names.contains(check_name);
        let check_needs_review = check.level != crate::migration::channels::ChannelCheckLevel::Pass;

        check_is_plugin_bridge_surface && check_needs_review
    })
}

fn collect_enabled_plugin_bridge_surface_names(
    config: &mvp::config::LoongClawConfig,
) -> BTreeSet<&'static str> {
    let inventory = mvp::channel::channel_inventory(config);

    inventory
        .channel_surfaces
        .into_iter()
        .filter(enabled_plugin_bridge_surface)
        .map(|surface| plugin_bridge_surface_name(surface.catalog.id))
        .collect()
}

fn enabled_plugin_bridge_surface(surface: &mvp::channel::ChannelSurface) -> bool {
    let has_plugin_bridge_contract = surface.catalog.plugin_bridge_contract.is_some();

    if !has_plugin_bridge_contract {
        return false;
    }

    surface
        .configured_accounts
        .iter()
        .any(|snapshot| snapshot.enabled)
}

fn plugin_bridge_surface_name(channel_id: &'static str) -> &'static str {
    let descriptor = mvp::config::channel_descriptor(channel_id);

    match descriptor {
        Some(descriptor) => descriptor.surface_label,
        None => channel_id,
    }
}

fn build_managed_bridge_doctor_action(config_path: &str) -> SetupNextAction {
    let command = crate::cli_handoff::format_subcommand_with_config("doctor", config_path);

    SetupNextAction {
        kind: SetupNextActionKind::Doctor,
        channel_action_id: None,
        browser_preview_phase: None,
        label: "verify managed bridges".to_owned(),
        command,
    }
}

fn channel_next_action_to_setup_action(
    action: crate::migration::channels::ChannelNextAction,
) -> SetupNextAction {
    SetupNextAction {
        kind: SetupNextActionKind::Channel,
        channel_action_id: Some(action.id),
        browser_preview_phase: None,
        label: action.label.to_owned(),
        command: action.command,
    }
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
