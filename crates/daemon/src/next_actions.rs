use std::ffi::OsStr;

use crate::personalize_presentation::personalize_action_label;
use crate::setup_boundary::SetupBoundaryKind;
use loong_app as mvp;
use serde::Serialize;

pub use mvp::chat::DEFAULT_FIRST_PROMPT as DEFAULT_FIRST_ASK_MESSAGE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SetupNextActionKind {
    Ask,
    Chat,
    Personalize,
    Channel,
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupNextAction {
    pub kind: SetupNextActionKind,
    pub channel_action_id: Option<&'static str>,
    pub label: String,
    pub command: String,
}

pub(crate) const fn setup_boundary_kind_for_action_kind(
    kind: SetupNextActionKind,
) -> SetupBoundaryKind {
    match kind {
        SetupNextActionKind::Ask => SetupBoundaryKind::Ask,
        SetupNextActionKind::Chat => SetupBoundaryKind::Chat,
        SetupNextActionKind::Personalize => SetupBoundaryKind::Personalize,
        SetupNextActionKind::Channel => SetupBoundaryKind::ChannelReview,
        SetupNextActionKind::Doctor => SetupBoundaryKind::Doctor,
    }
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
    _path_env: Option<&OsStr>,
) -> Vec<SetupNextAction> {
    let mut actions = Vec::new();
    let channel_actions =
        crate::migration::channels::collect_channel_next_actions(config, config_path);
    if config.cli.enabled {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Ask,
            channel_action_id: None,
            label: "first answer".to_owned(),
            command: crate::cli_handoff::format_ask_with_config(
                config_path,
                DEFAULT_FIRST_ASK_MESSAGE,
            ),
        });
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Chat,
            channel_action_id: None,
            label: "chat".to_owned(),
            command: crate::cli_handoff::format_root_entry_with_config(config_path),
        });
        if should_suggest_personalization(config) {
            actions.push(SetupNextAction {
                kind: SetupNextActionKind::Personalize,
                channel_action_id: None,
                label: personalize_action_label().to_owned(),
                command: crate::cli_handoff::format_subcommand_with_config(
                    "personalize",
                    config_path,
                ),
            });
        }
    }
    let channel_setup_actions = channel_actions
        .into_iter()
        .map(channel_next_action_to_setup_action);
    actions.extend(channel_setup_actions);
    if actions.is_empty() {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            channel_action_id: None,
            label: "doctor".to_owned(),
            command: crate::cli_handoff::format_subcommand_with_config("doctor", config_path),
        });
    }
    actions
}

fn channel_next_action_to_setup_action(
    action: crate::migration::channels::ChannelNextAction,
) -> SetupNextAction {
    SetupNextAction {
        kind: SetupNextActionKind::Channel,
        channel_action_id: Some(action.id),
        label: action.label.to_owned(),
        command: action.command,
    }
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
            pronouns: None,
            response_density: None,
            initiative_level: None,
            standing_boundaries: None,
            timezone: None,
            notes: None,
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
            pronouns: None,
            response_density: Some(mvp::config::ResponseDensity::Balanced),
            initiative_level: Some(mvp::config::InitiativeLevel::Balanced),
            standing_boundaries: None,
            timezone: None,
            notes: None,
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
    fn collect_setup_next_actions_uses_generic_channel_handoff_for_plugin_backed_channels() {
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
        let channel_action = actions
            .iter()
            .find(|action| action.kind == SetupNextActionKind::Channel)
            .expect("channel action");

        assert_eq!(channel_action.label, "review Weixin bridge");
        assert_eq!(
            channel_action.command,
            "loong channels --config '/tmp/loong.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_ignores_runtime_attention_bridge_state() {
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
        let channel_action = actions
            .iter()
            .find(|action| action.kind == SetupNextActionKind::Channel)
            .expect("channel action");

        assert_eq!(channel_action.label, "inspect Weixin bridge");
        assert_eq!(
            channel_action.command,
            "loong channels --config '/tmp/loong.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_uses_generic_channel_handoff_for_multiple_plugin_bridges() {
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
        let channel_action = actions
            .iter()
            .find(|action| action.kind == SetupNextActionKind::Channel)
            .expect("channel action");

        assert_eq!(channel_action.label, "review configured channels");
        assert_eq!(
            channel_action.command,
            "loong channels --config '/tmp/loong.toml'"
        );
    }

    #[test]
    fn collect_setup_next_actions_uses_generic_channel_handoff_for_single_outbound_surface() {
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
            labels.contains(&"review Discord setup"),
            "setup should keep only the generic channel handoff for single outbound surfaces: {labels:?}"
        );
    }

    #[test]
    fn collect_setup_next_actions_uses_generic_channel_handoff_for_outbound_groups() {
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
            labels.contains(&"review configured outbound channels"),
            "setup should keep only the generic grouped channel handoff for outbound groups: {labels:?}"
        );
    }
}
