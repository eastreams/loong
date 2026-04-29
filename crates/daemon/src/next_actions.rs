use std::collections::BTreeSet;
use std::ffi::OsStr;

use crate::personalize_presentation::personalize_action_label;
use loong_app as mvp;
use serde::Serialize;

pub use mvp::chat::DEFAULT_FIRST_PROMPT as DEFAULT_FIRST_ASK_MESSAGE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SetupNextActionKind {
    Ask,
    Chat,
    Personalize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedBridgeRuntimeAttentionSurface {
    id: &'static str,
    reasons: Vec<&'static str>,
    preferred_owner_pids: Vec<u32>,
    cleanup_owner_pids: Vec<u32>,
}

pub fn collect_setup_next_actions(
    config: &mvp::config::LoongConfig,
    config_path: &str,
) -> Vec<SetupNextAction> {
    let path_env = std::env::var_os("PATH");
    collect_setup_next_actions_with_path_env(config, config_path, path_env.as_deref())
}

pub(crate) fn collect_setup_next_actions_with_path_env(
    config: &mvp::config::LoongConfig,
    config_path: &str,
    path_env: Option<&OsStr>,
) -> Vec<SetupNextAction> {
    let mut actions = Vec::new();
    let channel_actions =
        crate::migration::channels::collect_channel_next_actions(config, config_path);
    let runtime_attention_plugin_bridge_surfaces =
        collect_runtime_attention_plugin_bridge_surfaces(config);
    let unresolved_plugin_bridge_surfaces =
        collect_unresolved_plugin_bridge_surface_ids(config, &channel_actions);
    let blocked_outbound_surfaces = collect_blocked_outbound_surface_ids(config);
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
            command: crate::cli_handoff::format_root_entry_with_config(config_path),
        });
        if should_suggest_personalization(config) {
            actions.push(SetupNextAction {
                kind: SetupNextActionKind::Personalize,
                channel_action_id: None,
                browser_preview_phase: None,
                label: personalize_action_label().to_owned(),
                command: crate::cli_handoff::format_subcommand_with_config(
                    "personalize",
                    config_path,
                ),
            });
        }
    }
    if !runtime_attention_plugin_bridge_surfaces.is_empty() {
        let doctor_action = build_managed_bridge_runtime_doctor_action(
            config_path,
            &runtime_attention_plugin_bridge_surfaces,
        );
        actions.push(doctor_action);
    }
    if !unresolved_plugin_bridge_surfaces.is_empty() {
        let doctor_action =
            build_managed_bridge_doctor_action(config_path, &unresolved_plugin_bridge_surfaces);
        actions.push(doctor_action);
    }
    if !blocked_outbound_surfaces.is_empty() {
        let doctor_action =
            build_outbound_channel_doctor_action(config_path, &blocked_outbound_surfaces);
        actions.push(doctor_action);
    }
    let channel_actions =
        normalize_channel_actions_for_outbound_doctor(channel_actions, &blocked_outbound_surfaces);
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

fn collect_unresolved_plugin_bridge_surface_ids(
    config: &mvp::config::LoongConfig,
    _channel_actions: &[crate::migration::channels::ChannelNextAction],
) -> Vec<&'static str> {
    unresolved_plugin_bridge_surface_ids(config)
}

fn collect_runtime_attention_plugin_bridge_surfaces(
    config: &mvp::config::LoongConfig,
) -> Vec<ManagedBridgeRuntimeAttentionSurface> {
    let inventory = mvp::channel::channel_inventory(config);

    inventory
        .channel_surfaces
        .into_iter()
        .filter(enabled_plugin_bridge_surface)
        .filter_map(|surface| {
            let reasons = plugin_bridge_surface_runtime_attention_reasons(&surface);
            if reasons.is_empty() {
                return None;
            }
            Some(ManagedBridgeRuntimeAttentionSurface {
                id: surface.catalog.id,
                reasons,
                preferred_owner_pids: collect_surface_preferred_runtime_owner_pids(&surface),
                cleanup_owner_pids: collect_surface_duplicate_runtime_cleanup_owner_pids(&surface),
            })
        })
        .collect()
}

fn unresolved_plugin_bridge_surface_ids(config: &mvp::config::LoongConfig) -> Vec<&'static str> {
    let inventory = mvp::channel::channel_inventory(config);
    let channel_checks = crate::migration::channels::collect_channel_preflight_checks(config);
    let unresolved_surface_names = channel_checks
        .into_iter()
        .filter(|check| check.level != crate::migration::channels::ChannelCheckLevel::Pass)
        .map(|check| check.name)
        .collect::<BTreeSet<_>>();

    inventory
        .channel_surfaces
        .into_iter()
        .filter(enabled_plugin_bridge_surface)
        .filter(|surface| {
            let surface_name = plugin_bridge_surface_name(surface.catalog.id);
            unresolved_surface_names.contains(surface_name)
        })
        .map(|surface| surface.catalog.id)
        .collect()
}

fn plugin_bridge_surface_runtime_attention_reasons(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<&'static str> {
    let mut reasons = BTreeSet::new();

    for snapshot in surface
        .configured_accounts
        .iter()
        .filter(|snapshot| snapshot.enabled)
    {
        for reason in channel_snapshot_runtime_attention_reasons(snapshot) {
            reasons.insert(reason);
        }
    }

    reasons.into_iter().collect()
}

fn channel_snapshot_runtime_attention_reasons(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
) -> Vec<&'static str> {
    let Some(runtime) = snapshot
        .operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID)
        .and_then(|operation| operation.runtime.as_ref())
    else {
        return Vec::new();
    };

    let mut reasons = Vec::new();
    if runtime.consecutive_failures > 0 {
        reasons.push("retrying");
    }
    if runtime.stale {
        reasons.push("stale");
    }
    if runtime.running_instances > 1 {
        reasons.push("duplicate_runtime_instances");
    }

    reasons
}

fn collect_surface_preferred_runtime_owner_pids(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<u32> {
    let mut owner_pids = BTreeSet::new();

    for snapshot in &surface.configured_accounts {
        let Some(runtime) = snapshot
            .operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID)
            .and_then(|operation| operation.runtime.as_ref())
        else {
            continue;
        };
        if runtime.duplicate_owner_pids.is_empty() {
            continue;
        }
        let Some(pid) = runtime.pid else {
            continue;
        };
        owner_pids.insert(pid);
    }

    owner_pids.into_iter().collect()
}

fn collect_surface_duplicate_runtime_cleanup_owner_pids(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<u32> {
    let mut owner_pids = BTreeSet::new();

    for snapshot in &surface.configured_accounts {
        let Some(runtime) = snapshot
            .operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID)
            .and_then(|operation| operation.runtime.as_ref())
        else {
            continue;
        };
        if runtime.duplicate_owner_pids.is_empty() {
            continue;
        }
        let preferred_pid = runtime.pid;
        for owner_pid in &runtime.duplicate_owner_pids {
            if Some(*owner_pid) == preferred_pid {
                continue;
            }
            owner_pids.insert(*owner_pid);
        }
    }

    owner_pids.into_iter().collect()
}

fn collect_blocked_outbound_surface_ids(config: &mvp::config::LoongConfig) -> Vec<&'static str> {
    let inventory = mvp::channel::channel_inventory(config);

    inventory
        .channel_surfaces
        .into_iter()
        .filter(crate::migration::channels::surface_is_outbound_only)
        .filter(|surface| {
            surface
                .configured_accounts
                .iter()
                .any(|snapshot| snapshot.enabled)
        })
        .filter(|surface| {
            surface
                .configured_accounts
                .iter()
                .filter(|snapshot| snapshot.enabled)
                .any(|snapshot| {
                    !snapshot
                        .operation(mvp::channel::CHANNEL_OPERATION_SEND_ID)
                        .is_some_and(|operation| {
                            operation.health == mvp::channel::ChannelOperationHealth::Ready
                        })
                })
        })
        .map(|surface| surface.catalog.id)
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

fn build_managed_bridge_doctor_action(
    config_path: &str,
    unresolved_surface_ids: &[&'static str],
) -> SetupNextAction {
    let command = crate::cli_handoff::format_subcommand_with_config("doctor", config_path);
    let label = managed_bridge_doctor_action_label(unresolved_surface_ids);

    SetupNextAction {
        kind: SetupNextActionKind::Doctor,
        channel_action_id: None,
        browser_preview_phase: None,
        label,
        command,
    }
}

fn build_managed_bridge_runtime_doctor_action(
    config_path: &str,
    runtime_attention_surfaces: &[ManagedBridgeRuntimeAttentionSurface],
) -> SetupNextAction {
    let command = crate::cli_handoff::format_subcommand_with_config("doctor", config_path);
    let label = managed_bridge_runtime_doctor_action_label(runtime_attention_surfaces);

    SetupNextAction {
        kind: SetupNextActionKind::Doctor,
        channel_action_id: None,
        browser_preview_phase: None,
        label,
        command,
    }
}

fn build_outbound_channel_doctor_action(
    config_path: &str,
    blocked_surface_ids: &[&'static str],
) -> SetupNextAction {
    let command = crate::cli_handoff::format_subcommand_with_config("doctor", config_path);
    let label = outbound_channel_doctor_action_label(blocked_surface_ids);

    SetupNextAction {
        kind: SetupNextActionKind::Doctor,
        channel_action_id: None,
        browser_preview_phase: None,
        label,
        command,
    }
}

fn managed_bridge_doctor_action_label(unresolved_surface_ids: &[&'static str]) -> String {
    if unresolved_surface_ids.len() == 1 {
        let surface_id = unresolved_surface_ids
            .first()
            .copied()
            .unwrap_or("managed bridge");
        return format!("verify {surface_id} managed bridge");
    }

    if unresolved_surface_ids.is_empty() {
        return "verify managed bridges".to_owned();
    }

    let rendered_surface_ids = unresolved_surface_ids.join(", ");
    let label = format!("verify managed bridges: {rendered_surface_ids}");

    label
}

fn outbound_channel_doctor_action_label(blocked_surface_ids: &[&'static str]) -> String {
    if blocked_surface_ids.len() == 1 {
        let surface_id = blocked_surface_ids
            .first()
            .copied()
            .unwrap_or("configured outbound channel");
        let label = mvp::config::channel_descriptor(surface_id)
            .map(|descriptor| descriptor.label)
            .unwrap_or(surface_id);
        return format!("verify {label} setup");
    }

    if blocked_surface_ids.is_empty() {
        return "verify configured outbound channels".to_owned();
    }

    "verify configured outbound channels".to_owned()
}

fn managed_bridge_runtime_doctor_action_label(
    runtime_attention_surfaces: &[ManagedBridgeRuntimeAttentionSurface],
) -> String {
    if runtime_attention_surfaces.len() == 1 {
        let surface = runtime_attention_surfaces.first().expect("one surface");
        let rendered_reasons = if surface.reasons.is_empty() {
            String::new()
        } else {
            format!(" ({})", surface.reasons.join(","))
        };
        let keep_suffix = if surface.reasons.contains(&"duplicate_runtime_instances") {
            let keep = if surface.preferred_owner_pids.len() == 1 {
                let pid = surface
                    .preferred_owner_pids
                    .first()
                    .copied()
                    .unwrap_or_default();
                format!(" keep pid={pid}")
            } else {
                String::new()
            };
            let cleanup = if surface.cleanup_owner_pids.is_empty() {
                String::new()
            } else {
                let rendered_cleanup = surface
                    .cleanup_owner_pids
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!(" cleanup pids={rendered_cleanup}")
            };
            format!("{keep}{cleanup}")
        } else {
            String::new()
        };
        return format!(
            "inspect {} managed bridge runtime{}{}",
            surface.id, rendered_reasons, keep_suffix
        );
    }

    if runtime_attention_surfaces.is_empty() {
        return "inspect managed bridge runtimes".to_owned();
    }

    let rendered_surface_ids = runtime_attention_surfaces
        .iter()
        .map(|surface| surface.id)
        .collect::<Vec<_>>()
        .join(", ");
    format!("inspect managed bridge runtimes: {rendered_surface_ids}")
}

#[cfg(test)]
pub(crate) fn is_managed_bridge_doctor_action(action: &SetupNextAction) -> bool {
    let is_doctor = action.kind == SetupNextActionKind::Doctor;
    let label = action.label.as_str();
    let is_managed_bridge_label = (label.starts_with("verify ") || label.starts_with("inspect "))
        && label.contains("managed bridge");

    is_doctor && is_managed_bridge_label
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

fn normalize_channel_actions_for_outbound_doctor(
    channel_actions: Vec<crate::migration::channels::ChannelNextAction>,
    blocked_outbound_surface_ids: &[&'static str],
) -> Vec<crate::migration::channels::ChannelNextAction> {
    channel_actions
        .into_iter()
        .map(|action| {
            normalize_channel_action_for_outbound_doctor(action, blocked_outbound_surface_ids)
        })
        .collect()
}

fn normalize_channel_action_for_outbound_doctor(
    mut action: crate::migration::channels::ChannelNextAction,
    blocked_outbound_surface_ids: &[&'static str],
) -> crate::migration::channels::ChannelNextAction {
    let is_configured_channels_action =
        action.id == crate::migration::channels::CONFIGURED_CHANNELS_ACTION_ID;

    if !is_configured_channels_action {
        return action;
    }

    let Some(normalized_label) =
        normalized_outbound_follow_up_label(action.label, blocked_outbound_surface_ids)
    else {
        return action;
    };

    action.label = Box::leak(normalized_label.into_boxed_str());

    action
}

fn normalized_outbound_follow_up_label(
    current_label: &str,
    blocked_outbound_surface_ids: &[&'static str],
) -> Option<String> {
    if blocked_outbound_surface_ids.is_empty() {
        return None;
    }

    if current_label == "review configured outbound channels" {
        return Some("inspect configured outbound channels".to_owned());
    }

    if blocked_outbound_surface_ids.len() == 1 {
        let channel_id = blocked_outbound_surface_ids.first().copied()?;
        let descriptor = mvp::config::channel_descriptor(channel_id)?;
        let review_label = format!("review {} setup", descriptor.label);

        if current_label == review_label {
            return Some(format!("inspect {}", descriptor.label));
        }

        return None;
    }

    None
}

fn should_suggest_personalization(config: &mvp::config::LoongConfig) -> bool {
    let personalization = config.memory.trimmed_personalization();
    let Some(personalization) = personalization else {
        return true;
    };
    if personalization.suppresses_suggestions() {
        return false;
    }
    !personalization.has_operator_preferences()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn write_runtime_attention_fixture(
        channel_id: &str,
        account_id: &str,
        process_id: u32,
        consecutive_failures: usize,
    ) {
        let runtime_dir = mvp::config::default_loong_home().join("channel-runtime");
        fs::create_dir_all(&runtime_dir).expect("create runtime dir");
        let runtime_path =
            runtime_dir.join(format!("{channel_id}-serve-{account_id}-{process_id}.json"));
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_millis() as u64;
        let payload = serde_json::json!({
            "running": true,
            "busy": false,
            "active_runs": 0,
            "consecutive_failures": consecutive_failures,
            "last_run_activity_at": now_ms.saturating_sub(500),
            "last_heartbeat_at": now_ms.saturating_sub(100),
            "last_failure_at": now_ms,
            "last_recovery_at": serde_json::Value::Null,
            "last_error": "temporary bridge timeout",
            "pid": process_id,
            "account_id": account_id,
            "account_label": account_id,
            "owner_token": serde_json::Value::Null
        });
        let encoded = serde_json::to_string_pretty(&payload).expect("encode runtime state");
        fs::write(runtime_path, encoded).expect("write runtime attention state");
    }

    fn write_managed_bridge_runtime_manifest(root: &Path, channel_id: &str) {
        let runtime_operations_json = serde_json::to_string(&vec![
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
        ])
        .expect("serialize runtime operations");
        let metadata = BTreeMap::from([
            ("bridge_kind".to_owned(), "http_json".to_owned()),
            ("adapter_family".to_owned(), "channel-bridge".to_owned()),
            (
                "transport_family".to_owned(),
                "wechat_clawbot_ilink_bridge".to_owned(),
            ),
            ("target_contract".to_owned(), "weixin_reply_loop".to_owned()),
            (
                "channel_runtime_contract".to_owned(),
                mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1.to_owned(),
            ),
            (
                "channel_runtime_operations_json".to_owned(),
                runtime_operations_json,
            ),
        ]);
        let plugin_id = format!("{channel_id}-managed-runtime");
        let manifest = crate::kernel::PluginManifest {
            api_version: Some("v1alpha1".to_owned()),
            version: Some("1.0.0".to_owned()),
            plugin_id: plugin_id.clone(),
            provider_id: format!("{channel_id}-managed-runtime-provider"),
            connector_name: format!("{channel_id}-managed-runtime-connector"),
            channel_id: Some(channel_id.to_owned()),
            endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
            capabilities: BTreeSet::new(),
            trust_tier: crate::kernel::PluginTrustTier::Unverified,
            metadata,
            summary: None,
            tags: Vec::new(),
            input_examples: Vec::new(),
            output_examples: Vec::new(),
            defer_loading: false,
            setup: Some(crate::kernel::PluginSetup {
                mode: crate::kernel::PluginSetupMode::MetadataOnly,
                surface: Some("channel".to_owned()),
                required_env_vars: Vec::new(),
                recommended_env_vars: Vec::new(),
                required_config_keys: Vec::new(),
                default_env_var: None,
                docs_urls: Vec::new(),
                remediation: None,
            }),
            slot_claims: Vec::new(),
            compatibility: None,
        };
        let plugin_directory = root.join(plugin_id);
        let manifest_path = plugin_directory.join("loong.plugin.json");
        let encoded_manifest =
            serde_json::to_string_pretty(&manifest).expect("serialize runtime manifest");

        fs::create_dir_all(&plugin_directory).expect("create runtime plugin directory");
        fs::write(&manifest_path, encoded_manifest).expect("write runtime plugin manifest");
    }

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
        assert_eq!(action.label, "choose a channel");
        assert_eq!(action.command, "loong channels --config '/tmp/loong.toml'");
    }

    #[test]
    fn collect_setup_next_actions_includes_personalize_after_chat_when_pending() {
        let config = mvp::config::LoongConfig::default();

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );

        assert_eq!(actions[0].kind, SetupNextActionKind::Ask);
        assert_eq!(actions[1].kind, SetupNextActionKind::Chat);
        assert_eq!(actions[2].kind, SetupNextActionKind::Personalize);
        assert_eq!(actions[2].label, personalize_action_label());
        assert_eq!(
            actions[2].command,
            "loong personalize --config '/tmp/loong.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_uses_root_entry_for_chat_followup() {
        let config = mvp::config::LoongConfig::default();

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );

        assert_eq!(actions[1].kind, SetupNextActionKind::Chat);
        assert_eq!(
            actions[1].command,
            "LOONG_CONFIG_PATH='/tmp/loong.toml' loong"
        );
    }

    #[test]
    fn collect_setup_next_actions_uses_plain_root_entry_for_default_config_path() {
        let mut env = crate::test_support::ScopedEnv::new();
        let home = unique_temp_dir("loong-next-actions-default-home");
        fs::create_dir_all(home.join(".loong")).expect("create default home");
        env.set("HOME", &home);
        env.remove("LOONG_HOME");
        env.remove("LOONG_CONFIG_PATH");

        let config = mvp::config::LoongConfig::default();
        let default_config_path = crate::resolved_default_entry_config_path();
        let actions = collect_setup_next_actions_with_path_env(
            &config,
            default_config_path.to_str().expect("utf8 config path"),
            Some(std::ffi::OsStr::new("")),
        );

        assert_eq!(actions[1].kind, SetupNextActionKind::Chat);
        assert_eq!(actions[1].command, "loong");
    }

    #[test]
    fn collect_setup_next_actions_omits_personalize_when_suppressed() {
        let mut config = mvp::config::LoongConfig::default();
        config.memory.personalization = Some(mvp::config::PersonalizationConfig {
            preferred_name: None,
            response_density: None,
            initiative_level: None,
            standing_boundaries: None,
            timezone: None,
            locale: None,
            prompt_state: mvp::config::PersonalizationPromptState::Suppressed,
            schema_version: 1,
            updated_at_epoch_seconds: Some(7),
        });

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            actions
                .iter()
                .all(|action| action.kind != SetupNextActionKind::Personalize),
            "suppressed personalization should not be suggested again: {actions:#?}"
        );
    }

    #[test]
    fn collect_setup_next_actions_omits_personalize_when_configured() {
        let mut config = mvp::config::LoongConfig::default();
        config.memory.personalization = Some(mvp::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(mvp::config::ResponseDensity::Balanced),
            initiative_level: Some(mvp::config::InitiativeLevel::Balanced),
            standing_boundaries: None,
            timezone: None,
            locale: None,
            prompt_state: mvp::config::PersonalizationPromptState::Configured,
            schema_version: 1,
            updated_at_epoch_seconds: Some(7),
        });

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            actions
                .iter()
                .all(|action| action.kind != SetupNextActionKind::Personalize),
            "configured personalization should not be suggested again: {actions:#?}"
        );
    }

    #[test]
    fn collect_setup_next_actions_promotes_browser_companion_preview_when_ready() {
        let root = unique_temp_dir("loong-next-actions-browser-companion");
        let install_root = root.join("managed-skills");
        write_file(
            &install_root,
            "browser-companion-preview/SKILL.md",
            "# Browser Companion Preview\n\nUse agent-browser through exec.\n",
        );
        let bin_dir = root.join("bin");
        write_fake_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_allow.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_eq!(actions[0].kind, SetupNextActionKind::Ask);
        assert_eq!(actions[1].kind, SetupNextActionKind::Chat);
        assert_eq!(actions[2].kind, SetupNextActionKind::Personalize);
        assert_channel_catalog_action(&actions[3]);
        assert_eq!(actions[4].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[4].browser_preview_phase,
            Some(BrowserPreviewActionPhase::Ready)
        );
        assert_eq!(actions[4].label, "browser companion preview");
        assert!(
            actions[4]
                .command
                .contains("Use the browser companion preview to open https://example.com"),
            "ready preview action should hand users into a task-shaped first browser recipe: {actions:#?}"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_setup_next_actions_guides_browser_preview_shell_unblock_when_hard_denied() {
        let root = unique_temp_dir("loong-next-actions-browser-companion-shell-deny");
        let install_root = root.join("managed-skills");
        write_file(
            &install_root,
            "browser-companion-preview/SKILL.md",
            "# Browser Companion Preview\n\nUse agent-browser through exec.\n",
        );
        let bin_dir = root.join("bin");
        write_fake_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_deny.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_eq!(actions[2].kind, SetupNextActionKind::Personalize);
        assert_channel_catalog_action(&actions[3]);
        assert_eq!(actions[4].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[4].browser_preview_phase,
            Some(BrowserPreviewActionPhase::Unblock)
        );
        assert_eq!(actions[4].label, "allow agent-browser");
        assert!(
            actions[4]
                .command
                .contains("remove `agent-browser` from [tools].shell_deny"),
            "shell hard-deny should produce an unblock step instead of looping back to enable-browser-preview: {actions:#?}"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_setup_next_actions_guides_browser_preview_enable_when_not_configured() {
        let root = unique_temp_dir("loong-next-actions-browser-companion-enable");
        let bin_dir = root.join("bin");
        write_fake_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongConfig::default();
        config.tools.file_root = Some(root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_eq!(actions[2].kind, SetupNextActionKind::Personalize);
        assert_channel_catalog_action(&actions[3]);
        assert_eq!(actions[4].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[4].browser_preview_phase,
            Some(BrowserPreviewActionPhase::Enable)
        );
        assert!(
            actions[4].command.contains("enable-browser-preview"),
            "browser preview enable action should point operators at the preview bootstrap command: {actions:#?}"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn collect_setup_next_actions_requires_an_executable_agent_browser_binary() {
        let root = unique_temp_dir("loong-next-actions-browser-companion-nonexec");
        let install_root = root.join("managed-skills");
        write_file(
            &install_root,
            "browser-companion-preview/SKILL.md",
            "# Browser Companion Preview\n\nUse agent-browser through exec.\n",
        );
        let bin_dir = root.join("bin");
        write_non_executable_agent_browser(&bin_dir);

        let mut config = mvp::config::LoongConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_allow.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(bin_dir.as_os_str()),
        );

        assert_eq!(actions[2].kind, SetupNextActionKind::Personalize);
        assert_channel_catalog_action(&actions[3]);
        assert_eq!(actions[4].kind, SetupNextActionKind::BrowserPreview);
        assert_eq!(
            actions[4].browser_preview_phase,
            Some(BrowserPreviewActionPhase::InstallRuntime)
        );
        assert_eq!(
            actions[4].label,
            format!("install {}", mvp::tools::BROWSER_COMPANION_COMMAND)
        );
        assert_eq!(
            actions[4].command,
            format!(
                "npm install -g {} && {} install",
                mvp::tools::BROWSER_COMPANION_COMMAND,
                mvp::tools::BROWSER_COMPANION_COMMAND
            )
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_setup_next_actions_labels_single_unresolved_plugin_bridge_surface() {
        let mut config = mvp::config::LoongConfig::default();
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "weixin-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );
        let doctor_action = actions
            .iter()
            .find(|action| action.kind == SetupNextActionKind::Doctor)
            .expect("managed bridge doctor action");

        assert_eq!(doctor_action.label, "verify weixin managed bridge");
        assert_eq!(
            doctor_action.command,
            "loong doctor --config '/tmp/loong.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_labels_single_runtime_attention_plugin_bridge_surface() {
        let home = unique_temp_dir("loong-next-actions-runtime-attention");
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("LOONG_HOME", home.as_os_str());
        write_runtime_attention_fixture("weixin", "default", 5151, 2);
        let plugin_root = unique_temp_dir("loong-next-actions-runtime-plugin-root");
        write_managed_bridge_runtime_manifest(plugin_root.as_path(), "weixin");

        let mut config = mvp::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![plugin_root.display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "weixin-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );
        let doctor_action = actions
            .iter()
            .find(|action| action.kind == SetupNextActionKind::Doctor)
            .expect("managed bridge runtime doctor action");

        assert_eq!(
            doctor_action.label,
            "inspect weixin managed bridge runtime (retrying)"
        );
        assert_eq!(
            doctor_action.command,
            "loong doctor --config '/tmp/loong.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_labels_multiple_unresolved_plugin_bridge_surfaces() {
        let mut config = mvp::config::LoongConfig::default();
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "weixin-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];
        config.qqbot.enabled = true;
        config.qqbot.app_id = Some(loong_contracts::SecretRef::Inline("10001".to_owned()));
        config.qqbot.client_secret = Some(loong_contracts::SecretRef::Inline(
            "qqbot-secret".to_owned(),
        ));
        config.qqbot.allowed_peer_ids = vec!["openid-alice".to_owned()];

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );
        let doctor_action = actions
            .iter()
            .find(|action| action.kind == SetupNextActionKind::Doctor)
            .expect("managed bridge doctor action");

        assert_eq!(doctor_action.label, "verify weixin managed bridge");
        assert_eq!(
            doctor_action.command,
            "loong doctor --config '/tmp/loong.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_keeps_outbound_follow_up_as_inspection_after_doctor_for_single_surface()
     {
        let mut config = mvp::config::LoongConfig::default();
        config.discord.enabled = true;
        config.discord.bot_token = None;
        config.discord.bot_token_env = None;

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );
        let labels = actions
            .iter()
            .map(|action| action.label.as_str())
            .collect::<Vec<_>>();

        assert!(
            labels.contains(&"verify Discord setup"),
            "blocked outbound-only surfaces should still expose a doctor-first handoff: {labels:?}"
        );
        assert!(
            labels.contains(&"inspect Discord"),
            "the secondary outbound-only handoff should be inspection rather than a duplicate review label: {labels:?}"
        );
        assert!(
            !labels.contains(&"review Discord setup"),
            "outbound-only follow-up actions should not duplicate the doctor wording: {labels:?}"
        );
    }

    #[test]
    fn collect_setup_next_actions_keeps_outbound_group_follow_up_as_inspection_after_doctor() {
        let mut config = mvp::config::LoongConfig::default();
        config.discord.enabled = true;
        config.discord.bot_token = Some(loong_contracts::SecretRef::Inline(
            "discord-token".to_owned(),
        ));
        config.slack.enabled = true;
        config.slack.bot_token = None;
        config.slack.bot_token_env = None;

        let actions = collect_setup_next_actions_with_path_env(
            &config,
            "/tmp/loong.toml",
            Some(std::ffi::OsStr::new("")),
        );
        let labels = actions
            .iter()
            .map(|action| action.label.as_str())
            .collect::<Vec<_>>();

        assert!(
            labels.contains(&"verify Slack setup"),
            "blocked outbound groups should still lead with the concrete doctor handoff: {labels:?}"
        );
        assert!(
            labels.contains(&"inspect configured outbound channels"),
            "blocked outbound groups should keep channels as an inspection handoff: {labels:?}"
        );
        assert!(
            !labels.contains(&"review configured outbound channels"),
            "grouped outbound follow-up should not repeat review wording once doctor already owns repair guidance: {labels:?}"
        );
    }

    #[test]
    fn is_managed_bridge_doctor_action_matches_single_surface_label() {
        let action = SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            channel_action_id: None,
            browser_preview_phase: None,
            label: "verify weixin managed bridge".to_owned(),
            command: "loong doctor --config '/tmp/loong.toml'".to_owned(),
        };

        assert!(is_managed_bridge_doctor_action(&action));
    }

    #[test]
    fn is_managed_bridge_doctor_action_matches_multi_surface_label() {
        let action = SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            channel_action_id: None,
            browser_preview_phase: None,
            label: "verify managed bridges: weixin, qqbot".to_owned(),
            command: "loong doctor --config '/tmp/loong.toml'".to_owned(),
        };

        assert!(is_managed_bridge_doctor_action(&action));
    }

    #[test]
    fn is_managed_bridge_doctor_action_matches_runtime_attention_label() {
        let action = SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            channel_action_id: None,
            browser_preview_phase: None,
            label: "inspect weixin managed bridge runtime (retrying)".to_owned(),
            command: "loong doctor --config '/tmp/loong.toml'".to_owned(),
        };

        assert!(is_managed_bridge_doctor_action(&action));
    }

    #[test]
    fn is_managed_bridge_doctor_action_rejects_unrelated_doctor_action() {
        let action = SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            channel_action_id: None,
            browser_preview_phase: None,
            label: "doctor".to_owned(),
            command: "loong doctor --config '/tmp/loong.toml'".to_owned(),
        };

        assert!(!is_managed_bridge_doctor_action(&action));
    }

    #[test]
    fn is_managed_bridge_doctor_action_rejects_non_doctor_action() {
        let action = SetupNextAction {
            kind: SetupNextActionKind::Channel,
            channel_action_id: Some(crate::migration::channels::CHANNEL_CATALOG_ACTION_ID),
            browser_preview_phase: None,
            label: "verify weixin managed bridge".to_owned(),
            command: "loong channels --config '/tmp/loong.toml'".to_owned(),
        };

        assert!(!is_managed_bridge_doctor_action(&action));
    }
}
