use super::*;
use crate::config::FeishuChannelServeMode;
use crate::channel::registry::runtime_backed::TELEGRAM_ONBOARDING_DESCRIPTOR;

#[test]
fn normalize_channel_platform_maps_lark_alias_to_feishu() {
    assert_eq!(
        normalize_channel_platform("lark"),
        Some(ChannelPlatform::Feishu)
    );
    assert_eq!(
        normalize_channel_platform(" TELEGRAM "),
        Some(ChannelPlatform::Telegram)
    );
    assert_eq!(normalize_channel_platform("discord"), None);
}

#[test]
fn resolve_channel_selection_order_uses_registry_metadata() {
    assert_eq!(resolve_channel_selection_order("telegram"), Some(10));
    assert_eq!(resolve_channel_selection_order("discord-bot"), Some(40));
    assert_eq!(resolve_channel_selection_order(" DISCORD-BOT "), Some(40));
    assert_eq!(resolve_channel_selection_order("unknown"), None);
}

#[test]
fn normalize_channel_catalog_id_maps_runtime_and_stub_aliases() {
    assert_eq!(normalize_channel_catalog_id("lark"), Some("feishu"));
    assert_eq!(normalize_channel_catalog_id(" TELEGRAM "), Some("telegram"));
    assert_eq!(normalize_channel_catalog_id("discord-bot"), Some("discord"));
    assert_eq!(normalize_channel_catalog_id("slack"), Some("slack"));
    assert_eq!(normalize_channel_catalog_id("gchat"), Some("google-chat"));
    assert_eq!(normalize_channel_catalog_id("wechat"), Some("weixin"));
    assert_eq!(normalize_channel_catalog_id("wx"), Some("weixin"));
    assert_eq!(normalize_channel_catalog_id("qq"), Some("qqbot"));
    assert_eq!(normalize_channel_catalog_id("onebot-v11"), Some("onebot"));
    assert_eq!(
        normalize_channel_catalog_id("synochat"),
        Some("synology-chat")
    );
    assert_eq!(
        normalize_channel_catalog_id("bluebubbles"),
        Some("imessage")
    );
    assert_eq!(normalize_channel_catalog_id("urbit"), Some("tlon"));
    assert_eq!(normalize_channel_catalog_id("web-ui"), Some("webchat"));
    assert_eq!(normalize_channel_catalog_id("unknown"), None);
}

#[test]
fn runtime_backed_channel_registry_descriptors_only_include_runtime_backed_surfaces() {
    let runtime_backed = runtime_backed_channel_registry_descriptors();

    assert_eq!(
        runtime_backed
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>(),
        vec![
            "telegram", "feishu", "matrix", "wecom", "qqbot", "line", "whatsapp", "webhook"
        ]
    );
    assert!(
        runtime_backed
            .iter()
            .all(|descriptor| descriptor.runtime.is_some())
    );
}

#[test]
fn resolve_channel_runtime_command_descriptor_returns_runtime_surface_metadata() {
    let telegram = resolve_channel_runtime_command_descriptor("telegram")
        .expect("telegram runtime command descriptor");
    let lark = resolve_channel_runtime_command_descriptor("lark").expect("lark runtime descriptor");
    let line = resolve_channel_runtime_command_descriptor("line").expect("line runtime descriptor");
    let wecom =
        resolve_channel_runtime_command_descriptor("wecom").expect("wecom runtime descriptor");
    let webhook =
        resolve_channel_runtime_command_descriptor("webhook").expect("webhook runtime descriptor");

    assert_eq!(telegram.channel_id, "telegram");
    assert_eq!(telegram.platform, ChannelPlatform::Telegram);
    assert_eq!(telegram.serve_bootstrap_agent_id, "channel-telegram");

    assert_eq!(lark.channel_id, "feishu");
    assert_eq!(lark.platform, ChannelPlatform::Feishu);
    assert_eq!(lark.serve_bootstrap_agent_id, "channel-feishu");

    assert_eq!(line.channel_id, "line");
    assert_eq!(line.platform, ChannelPlatform::Line);
    assert_eq!(line.serve_bootstrap_agent_id, "channel-line");

    assert_eq!(wecom.channel_id, "wecom");
    assert_eq!(wecom.platform, ChannelPlatform::Wecom);
    assert_eq!(wecom.serve_bootstrap_agent_id, "channel-wecom");

    assert_eq!(webhook.channel_id, "webhook");
    assert_eq!(webhook.platform, ChannelPlatform::Webhook);
    assert_eq!(webhook.serve_bootstrap_agent_id, "channel-webhook");
}

#[test]
fn resolve_channel_runtime_command_descriptor_skips_stub_surfaces() {
    assert_eq!(resolve_channel_runtime_command_descriptor("discord"), None);
    assert_eq!(
        resolve_channel_runtime_command_descriptor("slack-bot"),
        None
    );
}

#[test]
fn resolve_channel_catalog_command_family_descriptor_includes_matrix_runtime_channel() {
    let matrix = resolve_channel_catalog_command_family_descriptor("matrix")
        .expect("matrix catalog command family");

    assert_eq!(matrix.channel_id, "matrix");
    assert_eq!(matrix.send.id, CHANNEL_OPERATION_SEND_ID);
    assert_eq!(matrix.send.command, "channels send matrix");
    assert_eq!(matrix.serve.id, CHANNEL_OPERATION_SERVE_ID);
    assert_eq!(matrix.serve.command, "channels serve matrix");
    assert_eq!(
        matrix.default_send_target_kind,
        ChannelCatalogTargetKind::Conversation
    );
}

#[test]
fn resolve_channel_catalog_command_family_descriptor_includes_wecom_runtime_channel() {
    let wecom = resolve_channel_catalog_command_family_descriptor("wecom")
        .expect("wecom catalog command family");

    assert_eq!(wecom.channel_id, "wecom");
    assert_eq!(wecom.send.id, CHANNEL_OPERATION_SEND_ID);
    assert_eq!(wecom.send.command, "channels send wecom");
    assert_eq!(wecom.serve.id, CHANNEL_OPERATION_SERVE_ID);
    assert_eq!(wecom.serve.command, "channels serve wecom");
    assert_eq!(
        wecom.default_send_target_kind,
        ChannelCatalogTargetKind::Conversation
    );
}

#[test]
fn resolve_channel_catalog_command_family_descriptor_rejects_unknown_channels() {
    assert_eq!(
        resolve_channel_catalog_command_family_descriptor("unknown-channel"),
        None
    );
}

#[test]
fn resolve_channel_command_family_descriptor_returns_runtime_send_and_serve_metadata() {
    let telegram = resolve_channel_command_family_descriptor("telegram")
        .expect("telegram command family descriptor");
    let lark = resolve_channel_command_family_descriptor("lark").expect("lark family descriptor");
    let telegram_catalog = resolve_channel_catalog_command_family_descriptor("telegram")
        .expect("telegram catalog family");
    let lark_catalog =
        resolve_channel_catalog_command_family_descriptor("lark").expect("lark catalog family");

    assert_eq!(telegram.runtime.channel_id, "telegram");
    assert_eq!(telegram.runtime.platform, ChannelPlatform::Telegram);
    assert_eq!(telegram.catalog, telegram_catalog);
    assert_eq!(telegram.catalog.send.id, CHANNEL_OPERATION_SEND_ID);
    assert_eq!(telegram.catalog.send.command, "channels send telegram");
    assert_eq!(telegram.catalog.serve.id, CHANNEL_OPERATION_SERVE_ID);
    assert_eq!(telegram.catalog.serve.command, "channels serve telegram");
    assert_eq!(
        telegram.catalog.send.default_target_kind(),
        Some(telegram.catalog.default_send_target_kind)
    );

    assert_eq!(lark.runtime.channel_id, "feishu");
    assert_eq!(lark.runtime.platform, ChannelPlatform::Feishu);
    assert_eq!(lark.catalog, lark_catalog);
    assert_eq!(lark.catalog.send.command, "channels send feishu");
    assert_eq!(lark.catalog.serve.command, "channels serve feishu");
    assert_eq!(
        lark.catalog.send.default_target_kind(),
        Some(lark.catalog.default_send_target_kind)
    );
}

#[test]
fn resolve_channel_command_family_descriptor_skips_stub_surfaces() {
    assert_eq!(resolve_channel_command_family_descriptor("discord"), None);
    assert_eq!(resolve_channel_command_family_descriptor("slack-bot"), None);
}

#[test]
fn resolve_channel_operation_descriptor_combines_catalog_and_doctor_metadata() {
    let lark_serve = resolve_channel_operation_descriptor("lark", CHANNEL_OPERATION_SERVE_ID)
        .expect("lark serve descriptor");
    assert_eq!(lark_serve.operation.command, "channels serve feishu");
    assert_eq!(
        lark_serve
            .doctor
            .expect("lark serve doctor metadata")
            .checks
            .iter()
            .map(|check| check.name)
            .collect::<Vec<_>>(),
        vec!["feishu inbound transport", "feishu serve runtime"]
    );

    let discord_send =
        resolve_channel_operation_descriptor("discord-bot", CHANNEL_OPERATION_SEND_ID)
            .expect("discord send descriptor");
    assert_eq!(discord_send.operation.command, "channels send discord");
    assert_eq!(discord_send.doctor, None);

    assert_eq!(
        resolve_channel_operation_descriptor("telegram", "unknown"),
        None
    );
}

#[test]
fn resolve_channel_catalog_entry_returns_config_backed_metadata_for_alias_lookup() {
    let discord = resolve_channel_catalog_entry("discord-bot").expect("discord entry");
    let encoded = serde_json::to_value(&discord).expect("serialize discord entry");

    assert_eq!(discord.id, "discord");
    assert_eq!(discord.selection_order, 40);
    assert_eq!(discord.selection_label, "community server bot");
    assert!(discord.blurb.contains("outbound message surface"));
    assert_eq!(
        discord.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(discord.transport, "discord_http_api");
    assert_eq!(discord.operations[0].command, "channels send discord");
    assert_eq!(discord.operations[1].command, "discord-serve");
    assert_eq!(
        encoded
            .get("operations")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("availability"))
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["implemented", "stub"])
    );
    assert_eq!(
        encoded
            .get("onboarding")
            .and_then(|onboarding| onboarding.get("strategy"))
            .and_then(serde_json::Value::as_str),
        Some("manual_config")
    );
    assert!(
        encoded.get("plugin_bridge_contract").is_none(),
        "config-backed catalog entries should omit plugin bridge contract when absent: {encoded}"
    );
}

#[test]
fn resolve_channel_catalog_entry_exposes_onboarding_contracts() {
    let telegram = resolve_channel_catalog_entry("telegram").expect("telegram entry");
    let lark = resolve_channel_catalog_entry("lark").expect("lark entry");
    let discord = resolve_channel_catalog_entry("discord").expect("discord entry");
    let weixin = resolve_channel_catalog_entry("wechat").expect("weixin entry");
    let qqbot = resolve_channel_catalog_entry("qq").expect("qqbot entry");
    let onebot = resolve_channel_catalog_entry("onebot-v11").expect("onebot entry");

    assert_eq!(
        telegram.onboarding.strategy,
        ChannelOnboardingStrategy::PluginBridge
    );
    assert_eq!(telegram.onboarding.status_command, "loong doctor");
    assert_eq!(telegram.onboarding.repair_command, None);
    assert!(telegram.onboarding.setup_hint.contains("bridge plugin"));

    assert_eq!(
        lark.onboarding.strategy,
        ChannelOnboardingStrategy::PluginBridge
    );
    assert_eq!(lark.onboarding.status_command, "loong doctor");
    assert_eq!(lark.onboarding.repair_command, None);
    assert!(lark.onboarding.setup_hint.contains("bridge plugin"));

    assert_eq!(
        discord.onboarding.strategy,
        ChannelOnboardingStrategy::ManualConfig
    );
    assert_eq!(
        discord.onboarding.repair_command,
        Some("loong doctor --fix")
    );
    assert!(
        discord
            .onboarding
            .setup_hint
            .contains("outbound direct send is shipped")
    );

    assert_eq!(weixin.onboarding.strategy.as_str(), "plugin_bridge");
    assert_eq!(weixin.onboarding.status_command, "loong doctor");
    assert_eq!(
        weixin.onboarding.repair_command,
        Some("loong weixin onboard")
    );
    assert!(weixin.onboarding.setup_hint.contains("ClawBot"));
    assert!(
        weixin
            .onboarding
            .setup_hint
            .contains("loong weixin onboard")
    );

    assert_eq!(qqbot.onboarding.strategy.as_str(), "plugin_bridge");
    assert_eq!(qqbot.onboarding.status_command, "loong doctor");
    assert_eq!(qqbot.onboarding.repair_command, None);
    assert!(qqbot.onboarding.setup_hint.contains("qqbot"));
    assert!(qqbot.onboarding.setup_hint.contains("client_secret"));
    assert!(qqbot.onboarding.setup_hint.contains("allowed_peer_ids"));

    let qqbot_send_requirements = qqbot
        .operation(CHANNEL_OPERATION_SEND_ID)
        .expect("qqbot send operation")
        .requirements
        .iter()
        .map(|requirement| requirement.id)
        .collect::<Vec<_>>();
    assert_eq!(
        qqbot_send_requirements,
        vec!["enabled", "app_id", "client_secret"]
    );
    let qqbot_serve_requirements = qqbot
        .operation(CHANNEL_OPERATION_SERVE_ID)
        .expect("qqbot serve operation")
        .requirements
        .iter()
        .map(|requirement| requirement.id)
        .collect::<Vec<_>>();
    assert_eq!(
        qqbot_serve_requirements,
        vec!["enabled", "app_id", "client_secret", "allowed_peer_ids"]
    );

    assert_eq!(onebot.onboarding.strategy.as_str(), "plugin_bridge");
    assert_eq!(onebot.onboarding.status_command, "loong doctor");
    assert_eq!(onebot.onboarding.repair_command, None);
    assert!(onebot.onboarding.setup_hint.contains("OneBot"));
}

#[test]
fn resolve_channel_doctor_operation_spec_uses_registry_metadata() {
    let telegram =
        resolve_channel_doctor_operation_spec("telegram", "serve").expect("telegram spec");
    assert_eq!(
        telegram
            .checks
            .iter()
            .map(|check| (check.name, check.trigger))
            .collect::<Vec<_>>(),
        vec![
            (
                "telegram channel",
                ChannelDoctorCheckTrigger::OperationHealth,
            ),
            (
                "telegram channel runtime",
                ChannelDoctorCheckTrigger::ReadyRuntime,
            ),
        ]
    );

    let feishu_send =
        resolve_channel_doctor_operation_spec("feishu", "send").expect("feishu send spec");
    assert_eq!(
        feishu_send
            .checks
            .iter()
            .map(|check| (check.name, check.trigger))
            .collect::<Vec<_>>(),
        vec![("feishu channel", ChannelDoctorCheckTrigger::OperationHealth)]
    );

    let lark_serve =
        resolve_channel_doctor_operation_spec("lark", "serve").expect("lark serve spec");
    assert_eq!(
        lark_serve
            .checks
            .iter()
            .map(|check| (check.name, check.trigger))
            .collect::<Vec<_>>(),
        vec![
            (
                "feishu inbound transport",
                ChannelDoctorCheckTrigger::OperationHealth,
            ),
            (
                "feishu serve runtime",
                ChannelDoctorCheckTrigger::ReadyRuntime,
            ),
        ]
    );

    assert_eq!(
        resolve_channel_doctor_operation_spec("discord", "serve"),
        None
    );
    assert_eq!(
        resolve_channel_doctor_operation_spec("telegram", "send"),
        None
    );

    let weixin_send =
        resolve_channel_doctor_operation_spec("weixin", "send").expect("weixin send spec");
    let weixin_send_checks = weixin_send
        .checks
        .iter()
        .map(|check| (check.name, check.trigger))
        .collect::<Vec<_>>();
    assert_eq!(
        weixin_send_checks,
        vec![(
            "weixin bridge send contract",
            ChannelDoctorCheckTrigger::PluginBridgeHealth,
        )]
    );

    let qqbot_serve =
        resolve_channel_doctor_operation_spec("qqbot", "serve").expect("qqbot serve spec");
    let qqbot_serve_checks = qqbot_serve
        .checks
        .iter()
        .map(|check| (check.name, check.trigger))
        .collect::<Vec<_>>();
    assert_eq!(
        qqbot_serve_checks,
        vec![
            ("qqbot channel", ChannelDoctorCheckTrigger::OperationHealth,),
            (
                "qqbot serve runtime",
                ChannelDoctorCheckTrigger::ReadyRuntime,
            ),
        ]
    );

    let onebot_serve =
        resolve_channel_doctor_operation_spec("onebot", "serve").expect("onebot serve spec");
    let onebot_serve_checks = onebot_serve
        .checks
        .iter()
        .map(|check| (check.name, check.trigger))
        .collect::<Vec<_>>();
    assert_eq!(
        onebot_serve_checks,
        vec![
            (
                "onebot bridge serve contract",
                ChannelDoctorCheckTrigger::PluginBridgeHealth,
            ),
            (
                "onebot bridge serve runtime",
                ChannelDoctorCheckTrigger::ReadyRuntime,
            ),
        ]
    );
}

#[test]
fn channel_catalog_keeps_lark_alias_under_feishu_surface() {
    let catalog = list_channel_catalog();
    let feishu = catalog
        .iter()
        .find(|entry| entry.id == "feishu")
        .expect("feishu catalog entry");
    let encoded = serde_json::to_value(feishu).expect("serialize feishu entry");

    assert_eq!(
        feishu.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(feishu.aliases, vec!["lark"]);
    assert_eq!(feishu.operations.len(), 2);
    assert_eq!(feishu.operations[0].command, "channels send feishu");
    assert_eq!(feishu.operations[1].command, "channels serve feishu");
    assert_eq!(
        encoded
            .get("operations")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("availability"))
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["managed_bridge", "managed_bridge"])
    );
}

#[test]
fn channel_catalog_includes_discord_and_slack_config_backed_surfaces() {
    let catalog = list_channel_catalog();
    let telegram = catalog
        .iter()
        .find(|entry| entry.id == "telegram")
        .expect("telegram catalog entry");
    let matrix = catalog
        .iter()
        .find(|entry| entry.id == "matrix")
        .expect("matrix catalog entry");
    let discord = catalog
        .iter()
        .find(|entry| entry.id == "discord")
        .expect("discord catalog entry");
    let slack = catalog
        .iter()
        .find(|entry| entry.id == "slack")
        .expect("slack catalog entry");
    let telegram_json = serde_json::to_value(telegram).expect("serialize telegram entry");
    let discord_json = serde_json::to_value(discord).expect("serialize discord entry");
    let slack_json = serde_json::to_value(slack).expect("serialize slack entry");

    assert_eq!(telegram.operations.len(), 2);
    assert_eq!(telegram.operations[0].command, "channels send telegram");
    assert_eq!(telegram.operations[1].command, "channels serve telegram");
    assert_eq!(
        matrix.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(
        matrix.transport,
        "matrix_client_server_sync_or_plugin_bridge"
    );
    assert!(matrix.aliases.is_empty());
    assert_eq!(matrix.operations.len(), 2);
    assert_eq!(matrix.operations[0].command, "channels send matrix");
    assert_eq!(matrix.operations[1].command, "channels serve matrix");
    assert_eq!(
        discord.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(discord.transport, "discord_http_api");
    assert_eq!(discord.aliases, vec!["discord-bot"]);
    assert_eq!(discord.selection_order, 40);
    assert_eq!(discord.selection_label, "community server bot");
    assert_eq!(discord.operations.len(), 2);
    assert_eq!(discord.operations[0].command, "channels send discord");
    assert_eq!(discord.operations[1].command, "discord-serve");

    assert_eq!(
        slack.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(slack.transport, "slack_web_api");
    assert_eq!(slack.aliases, vec!["slack-bot"]);
    assert_eq!(slack.operations.len(), 2);
    assert_eq!(slack.operations[0].command, "slack-send");
    assert_eq!(slack.operations[1].command, "slack-serve");
    assert_eq!(
        telegram_json
            .get("capabilities")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec![
            "plugin_backed",
            "multi_account",
            "send",
            "serve",
            "runtime_tracking",
        ])
    );
    assert_eq!(
        serde_json::to_value(matrix)
            .expect("serialize matrix entry")
            .get("capabilities")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec![
            "plugin_backed",
            "multi_account",
            "send",
            "serve",
            "runtime_tracking",
        ])
    );
    assert_eq!(
        discord_json
            .get("capabilities")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["multi_account", "send"])
    );
    assert_eq!(
        slack_json
            .get("capabilities")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["multi_account", "send"])
    );
}

#[test]
fn channel_catalog_operations_expose_requirement_metadata() {
    let catalog = list_channel_catalog();
    let telegram = catalog
        .iter()
        .find(|entry| entry.id == "telegram")
        .expect("telegram catalog entry");
    let feishu = catalog
        .iter()
        .find(|entry| entry.id == "feishu")
        .expect("feishu catalog entry");
    let discord = catalog
        .iter()
        .find(|entry| entry.id == "discord")
        .expect("discord catalog entry");
    let line = catalog
        .iter()
        .find(|entry| entry.id == "line")
        .expect("line catalog entry");
    let google_chat = catalog
        .iter()
        .find(|entry| entry.id == "google-chat")
        .expect("google chat catalog entry");
    let twitch = catalog
        .iter()
        .find(|entry| entry.id == "twitch")
        .expect("twitch catalog entry");
    let teams = catalog
        .iter()
        .find(|entry| entry.id == "teams")
        .expect("teams catalog entry");
    let mattermost = catalog
        .iter()
        .find(|entry| entry.id == "mattermost")
        .expect("mattermost catalog entry");
    let nextcloud_talk = catalog
        .iter()
        .find(|entry| entry.id == "nextcloud-talk")
        .expect("nextcloud talk catalog entry");
    let synology_chat = catalog
        .iter()
        .find(|entry| entry.id == "synology-chat")
        .expect("synology chat catalog entry");
    let imessage = catalog
        .iter()
        .find(|entry| entry.id == "imessage")
        .expect("imessage catalog entry");
    let matrix = catalog
        .iter()
        .find(|entry| entry.id == "matrix")
        .expect("matrix catalog entry");

    assert_eq!(
        telegram.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "bot_token"]
    );
    assert_eq!(
        telegram.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec![
            "enabled",
            "bot_token",
            "allowed_chat_ids",
            "allowed_sender_ids",
            "require_mention"
        ]
    );
    assert_eq!(
        telegram.operations[0].requirements[1].default_env_var,
        Some("TELEGRAM_BOT_TOKEN")
    );
    assert_eq!(
        telegram.operations[0].requirements[1].env_pointer_paths,
        &[
            "telegram.bot_token_env",
            "telegram.accounts.<account>.bot_token_env",
        ]
    );

    assert_eq!(
        feishu.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "app_id", "app_secret"]
    );
    assert_eq!(
        feishu.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec![
            "enabled",
            "app_id",
            "app_secret",
            "mode",
            "allowed_chat_ids",
            "allowed_sender_ids",
            "verification_token",
            "encrypt_key",
        ]
    );
    assert_eq!(
        feishu.operations[1].requirements[6].default_env_var,
        Some("FEISHU_VERIFICATION_TOKEN")
    );
    assert_eq!(
        feishu.operations[1].requirements[7].default_env_var,
        Some("FEISHU_ENCRYPT_KEY")
    );
    assert_eq!(
        matrix.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec![
            "enabled",
            "access_token",
            "base_url",
            "allowed_room_ids",
            "allowed_sender_ids",
            "require_mention",
            "user_id",
        ]
    );

    assert_eq!(
        discord.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "bot_token"]
    );
    assert_eq!(
        discord.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec![
            "enabled",
            "bot_token",
            "application_id",
            "allowed_guild_ids"
        ]
    );
    assert_eq!(
        discord.operations[1].requirements[2].default_env_var,
        Some("DISCORD_APPLICATION_ID")
    );

    assert_eq!(
        line.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "channel_access_token"]
    );
    assert_eq!(
        line.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "channel_access_token", "channel_secret"]
    );
    assert_eq!(
        line.operations[0].requirements[1].default_env_var,
        Some("LINE_CHANNEL_ACCESS_TOKEN")
    );
    assert_eq!(
        line.operations[1].requirements[2].default_env_var,
        Some("LINE_CHANNEL_SECRET")
    );

    let dingtalk = catalog
        .iter()
        .find(|entry| entry.id == "dingtalk")
        .expect("dingtalk catalog entry");

    assert_eq!(
        dingtalk.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "webhook_url"]
    );
    assert_eq!(
        dingtalk.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "webhook_url", "secret"]
    );
    assert_eq!(
        dingtalk.operations[0].requirements[1].default_env_var,
        Some("DINGTALK_WEBHOOK_URL")
    );
    assert_eq!(
        dingtalk.operations[1].requirements[2].default_env_var,
        Some("DINGTALK_SECRET")
    );

    assert_eq!(
        google_chat.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "webhook_url"]
    );
    assert_eq!(
        google_chat.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "webhook_url"]
    );
    assert_eq!(
        google_chat.operations[0].requirements[1].default_env_var,
        Some("GOOGLE_CHAT_WEBHOOK_URL")
    );

    assert_eq!(
        teams.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "webhook_url"]
    );
    assert_eq!(
        teams.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec![
            "enabled",
            "app_id",
            "app_password",
            "tenant_id",
            "allowed_conversation_ids",
        ]
    );
    assert_eq!(
        teams.operations[0].requirements[1].default_env_var,
        Some("TEAMS_WEBHOOK_URL")
    );
    assert_eq!(
        teams.operations[1].requirements[1].default_env_var,
        Some("TEAMS_APP_ID")
    );
    assert_eq!(
        teams.operations[1].requirements[2].default_env_var,
        Some("TEAMS_APP_PASSWORD")
    );
    assert_eq!(
        teams.operations[1].requirements[3].default_env_var,
        Some("TEAMS_TENANT_ID")
    );

    assert_eq!(
        mattermost.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "server_url", "bot_token"]
    );
    assert_eq!(
        mattermost.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "server_url", "bot_token", "allowed_channel_ids"]
    );
    assert_eq!(
        mattermost.operations[0].requirements[1].default_env_var,
        Some("MATTERMOST_SERVER_URL")
    );
    assert_eq!(
        mattermost.operations[0].requirements[2].default_env_var,
        Some("MATTERMOST_BOT_TOKEN")
    );

    assert_eq!(
        nextcloud_talk.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "server_url", "shared_secret"]
    );
    assert_eq!(
        nextcloud_talk.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "server_url", "shared_secret"]
    );
    assert_eq!(
        nextcloud_talk.operations[0].requirements[1].default_env_var,
        Some("NEXTCLOUD_TALK_SERVER_URL")
    );
    assert_eq!(
        nextcloud_talk.operations[0].requirements[2].default_env_var,
        Some("NEXTCLOUD_TALK_SHARED_SECRET")
    );

    assert_eq!(
        twitch.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "access_token"]
    );
    assert_eq!(
        twitch.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "access_token", "channel_names"]
    );
    assert_eq!(
        twitch.operations[0].requirements[1].default_env_var,
        Some("TWITCH_ACCESS_TOKEN")
    );
    assert_eq!(
        twitch.operations[0].requirements[1].env_pointer_paths,
        &[
            "twitch.access_token_env",
            "twitch.accounts.<account>.access_token_env",
        ]
    );

    assert_eq!(
        synology_chat.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "incoming_url"]
    );
    assert_eq!(
        synology_chat.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "token", "incoming_url", "allowed_user_ids"]
    );
    assert_eq!(
        synology_chat.operations[0].requirements[1].default_env_var,
        Some("SYNOLOGY_CHAT_INCOMING_URL")
    );
    assert_eq!(
        synology_chat.operations[1].requirements[1].default_env_var,
        Some("SYNOLOGY_CHAT_TOKEN")
    );

    assert_eq!(
        imessage.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "bridge_url", "bridge_token"]
    );
    assert_eq!(
        imessage.operations[1]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "bridge_url", "bridge_token", "allowed_chat_ids"]
    );
    assert_eq!(
        imessage.operations[0].requirements[1].default_env_var,
        Some("IMESSAGE_BRIDGE_URL")
    );
    assert_eq!(
        imessage.operations[0].requirements[2].default_env_var,
        Some("IMESSAGE_BRIDGE_TOKEN")
    );
}

#[test]
fn channel_catalog_operations_expose_supported_target_kinds() {
    let catalog = list_channel_catalog();
    let telegram = catalog
        .iter()
        .find(|entry| entry.id == "telegram")
        .expect("telegram catalog entry");
    let feishu = catalog
        .iter()
        .find(|entry| entry.id == "feishu")
        .expect("feishu catalog entry");
    let discord = catalog
        .iter()
        .find(|entry| entry.id == "discord")
        .expect("discord catalog entry");
    let teams = catalog
        .iter()
        .find(|entry| entry.id == "teams")
        .expect("teams catalog entry");
    let email = catalog
        .iter()
        .find(|entry| entry.id == "email")
        .expect("email catalog entry");
    let nextcloud_talk = catalog
        .iter()
        .find(|entry| entry.id == "nextcloud-talk")
        .expect("nextcloud talk catalog entry");
    let signal = catalog
        .iter()
        .find(|entry| entry.id == "signal")
        .expect("signal catalog entry");
    let twitch = catalog
        .iter()
        .find(|entry| entry.id == "twitch")
        .expect("twitch catalog entry");
    let synology_chat = catalog
        .iter()
        .find(|entry| entry.id == "synology-chat")
        .expect("synology chat catalog entry");
    let irc = catalog
        .iter()
        .find(|entry| entry.id == "irc")
        .expect("irc catalog entry");
    let imessage = catalog
        .iter()
        .find(|entry| entry.id == "imessage")
        .expect("imessage catalog entry");

    assert_eq!(
        telegram.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        telegram.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        feishu.operations[0].supported_target_kinds,
        &[
            ChannelCatalogTargetKind::ReceiveId,
            ChannelCatalogTargetKind::MessageReply,
        ]
    );
    assert_eq!(
        feishu.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::MessageReply]
    );
    assert_eq!(
        discord.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        discord.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        teams.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Endpoint]
    );
    assert_eq!(
        teams.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        email.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        email.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        nextcloud_talk.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        nextcloud_talk.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        signal.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        signal.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        twitch.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        twitch.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        synology_chat.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        synology_chat.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        irc.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        irc.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        imessage.operations[0].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        imessage.operations[1].supported_target_kinds,
        &[ChannelCatalogTargetKind::Conversation]
    );
}

#[test]
fn channel_catalog_operation_exposes_default_target_kind_from_metadata() {
    let telegram =
        resolve_channel_catalog_operation("telegram", "send").expect("telegram send operation");
    let feishu =
        resolve_channel_catalog_operation("feishu", "send").expect("feishu send operation");
    let nextcloud_talk = resolve_channel_catalog_operation("nextcloud-talk", "send")
        .expect("nextcloud talk send operation");
    let webhook =
        resolve_channel_catalog_operation("webhook", "send").expect("webhook send operation");
    let signal =
        resolve_channel_catalog_operation("signal", "send").expect("signal send operation");
    let email = resolve_channel_catalog_operation("email", "send").expect("email send operation");
    let teams = resolve_channel_catalog_operation("teams", "send").expect("teams send operation");
    let synology_chat = resolve_channel_catalog_operation("synology-chat", "send")
        .expect("synology chat send operation");
    let imessage =
        resolve_channel_catalog_operation("imessage", "send").expect("imessage send operation");

    assert_eq!(
        telegram.default_target_kind(),
        Some(ChannelCatalogTargetKind::Conversation)
    );
    assert!(telegram.supports_target_kind(ChannelCatalogTargetKind::Conversation));
    assert_eq!(
        feishu.default_target_kind(),
        Some(ChannelCatalogTargetKind::ReceiveId)
    );
    assert!(feishu.supports_target_kind(ChannelCatalogTargetKind::ReceiveId));
    assert!(feishu.supports_target_kind(ChannelCatalogTargetKind::MessageReply));
    assert!(!feishu.supports_target_kind(ChannelCatalogTargetKind::Conversation));
    assert_eq!(
        nextcloud_talk.default_target_kind(),
        Some(ChannelCatalogTargetKind::Conversation)
    );
    assert!(nextcloud_talk.supports_target_kind(ChannelCatalogTargetKind::Conversation));
    assert_eq!(
        webhook.default_target_kind(),
        Some(ChannelCatalogTargetKind::Endpoint)
    );
    assert!(webhook.supports_target_kind(ChannelCatalogTargetKind::Endpoint));
    assert!(!webhook.supports_target_kind(ChannelCatalogTargetKind::Conversation));
    assert_eq!(
        signal.default_target_kind(),
        Some(ChannelCatalogTargetKind::Address)
    );
    assert!(signal.supports_target_kind(ChannelCatalogTargetKind::Address));
    assert_eq!(
        email.default_target_kind(),
        Some(ChannelCatalogTargetKind::Address)
    );
    assert!(email.supports_target_kind(ChannelCatalogTargetKind::Address));
    assert_eq!(
        teams.default_target_kind(),
        Some(ChannelCatalogTargetKind::Endpoint)
    );
    assert!(teams.supports_target_kind(ChannelCatalogTargetKind::Endpoint));
    assert_eq!(
        synology_chat.default_target_kind(),
        Some(ChannelCatalogTargetKind::Address)
    );
    assert!(synology_chat.supports_target_kind(ChannelCatalogTargetKind::Address));
    assert_eq!(
        imessage.default_target_kind(),
        Some(ChannelCatalogTargetKind::Conversation)
    );
    assert!(imessage.supports_target_kind(ChannelCatalogTargetKind::Conversation));
}

#[test]
fn multi_target_channel_catalog_operations_declare_explicit_default_target_kind() {
    let catalog = list_channel_catalog();
    for entry in catalog {
        for operation in entry.operations {
            if operation.supported_target_kinds.len() < 2 {
                continue;
            }

            let default_target_kind = operation.default_target_kind.unwrap_or_else(|| {
                panic!(
                    "{}:{} must declare an explicit default_target_kind",
                    entry.id, operation.id
                )
            });

            assert!(
                operation.supports_target_kind(default_target_kind),
                "{}:{} default target kind must be supported by the operation",
                entry.id,
                operation.id
            );
        }
    }
}

#[test]
fn channel_catalog_surfaces_expose_union_of_supported_target_kinds() {
    let catalog = list_channel_catalog();
    let telegram = catalog
        .iter()
        .find(|entry| entry.id == "telegram")
        .expect("telegram catalog entry");
    let feishu = catalog
        .iter()
        .find(|entry| entry.id == "feishu")
        .expect("feishu catalog entry");
    let discord = catalog
        .iter()
        .find(|entry| entry.id == "discord")
        .expect("discord catalog entry");
    let teams = catalog
        .iter()
        .find(|entry| entry.id == "teams")
        .expect("teams catalog entry");
    let email = catalog
        .iter()
        .find(|entry| entry.id == "email")
        .expect("email catalog entry");
    let nextcloud_talk = catalog
        .iter()
        .find(|entry| entry.id == "nextcloud-talk")
        .expect("nextcloud talk catalog entry");
    let signal = catalog
        .iter()
        .find(|entry| entry.id == "signal")
        .expect("signal catalog entry");
    let synology_chat = catalog
        .iter()
        .find(|entry| entry.id == "synology-chat")
        .expect("synology chat catalog entry");
    let irc = catalog
        .iter()
        .find(|entry| entry.id == "irc")
        .expect("irc catalog entry");
    let imessage = catalog
        .iter()
        .find(|entry| entry.id == "imessage")
        .expect("imessage catalog entry");

    assert_eq!(
        telegram.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        feishu.supported_target_kinds,
        vec![
            ChannelCatalogTargetKind::ReceiveId,
            ChannelCatalogTargetKind::MessageReply,
        ]
    );
    assert_eq!(
        discord.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        teams.supported_target_kinds,
        vec![
            ChannelCatalogTargetKind::Endpoint,
            ChannelCatalogTargetKind::Conversation,
        ]
    );
    assert_eq!(
        email.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        nextcloud_talk.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        signal.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        synology_chat.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        irc.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        imessage.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
}

#[test]
fn channel_catalog_includes_irc_config_backed_surface() {
    let catalog = list_channel_catalog();
    let irc = catalog
        .iter()
        .find(|entry| entry.id == "irc")
        .expect("irc catalog entry");

    assert_eq!(
        irc.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(irc.selection_order, 170);
    assert_eq!(irc.transport, "irc_socket");
    assert_eq!(irc.aliases, Vec::<&str>::new());
    assert_eq!(irc.operations[0].command, "channels send irc");
    assert_eq!(irc.operations[1].command, "irc-serve");
    assert_eq!(
        irc.operations[0]
            .requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        vec!["enabled", "server", "nickname"]
    );
}

#[test]
fn catalog_only_channel_entries_include_stub_surfaces_for_default_config() {
    let config = LoongConfig::default();
    let snapshots = channel_status_snapshots(&config);
    let catalog_only = catalog_only_channel_entries(&snapshots);
    let webchat = catalog_only
        .iter()
        .find(|entry| entry.id == "webchat")
        .expect("webchat catalog entry");

    assert_eq!(
        catalog_only
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec!["zalo", "zalo-personal", "webchat"]
    );
    assert!(!catalog_only.iter().any(|entry| entry.id == "discord"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "slack"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "line"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "dingtalk"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "whatsapp"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "email"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "webhook"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "google-chat"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "signal"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "irc"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "nostr"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "twitch"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "tlon"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "teams"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "mattermost"));
    assert!(
        !catalog_only
            .iter()
            .any(|entry| entry.id == "nextcloud-talk")
    );
    assert!(!catalog_only.iter().any(|entry| entry.id == "synology-chat"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "imessage"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "weixin"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "qqbot"));
    assert!(!catalog_only.iter().any(|entry| entry.id == "onebot"));
    assert_eq!(webchat.operations[1].command, "webchat-serve");
}

#[test]
fn channel_inventory_combines_runtime_and_catalog_surfaces() {
    let config = LoongConfig::default();
    let inventory = channel_inventory(&config);

    assert_eq!(
        inventory
            .channels
            .iter()
            .map(|snapshot| snapshot.id)
            .collect::<Vec<_>>(),
        vec![
            "telegram",
            "feishu",
            "matrix",
            "wecom",
            "qqbot",
            "weixin",
            "onebot",
            "whatsapp-personal",
            "discord",
            "slack",
            "line",
            "dingtalk",
            "whatsapp",
            "email",
            "webhook",
            "google-chat",
            "signal",
            "twitch",
            "teams",
            "mattermost",
            "nextcloud-talk",
            "synology-chat",
            "irc",
            "imessage",
            "nostr",
            "tlon",
        ]
    );
    assert_eq!(
        inventory
            .catalog_only_channels
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec!["zalo", "zalo-personal", "webchat"]
    );
    assert_eq!(
        inventory
            .channel_catalog
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![
            "telegram",
            "feishu",
            "matrix",
            "wecom",
            "qqbot",
            "weixin",
            "onebot",
            "whatsapp-personal",
            "discord",
            "slack",
            "line",
            "dingtalk",
            "whatsapp",
            "email",
            "webhook",
            "google-chat",
            "signal",
            "twitch",
            "teams",
            "mattermost",
            "nextcloud-talk",
            "synology-chat",
            "irc",
            "imessage",
            "nostr",
            "tlon",
            "zalo",
            "zalo-personal",
            "webchat",
        ]
    );
}

#[test]
fn channel_catalog_includes_dingtalk_and_google_chat_config_backed_webhook_surfaces() {
    let catalog = list_channel_catalog();
    let dingtalk = catalog
        .iter()
        .find(|entry| entry.id == "dingtalk")
        .expect("dingtalk catalog entry");
    let google_chat = catalog
        .iter()
        .find(|entry| entry.id == "google-chat")
        .expect("google chat catalog entry");

    assert_eq!(
        dingtalk.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(dingtalk.selection_order, 80);
    assert_eq!(dingtalk.aliases, vec!["ding", "ding-bot"]);
    assert_eq!(dingtalk.transport, "dingtalk_custom_robot_webhook");
    assert_eq!(
        dingtalk.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Endpoint]
    );
    assert_eq!(dingtalk.operations[0].command, "channels send dingtalk");
    assert_eq!(dingtalk.operations[1].command, "dingtalk-serve");
    assert_eq!(
        dingtalk.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        dingtalk.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );

    assert_eq!(
        google_chat.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(google_chat.selection_order, 120);
    assert_eq!(google_chat.aliases, vec!["gchat", "googlechat"]);
    assert_eq!(google_chat.transport, "google_chat_incoming_webhook");
    assert_eq!(
        google_chat.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Endpoint]
    );
    assert_eq!(
        google_chat.operations[0].command,
        "channels send google-chat"
    );
    assert_eq!(google_chat.operations[1].command, "google-chat-serve");
    assert_eq!(
        google_chat.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        google_chat.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );
}

#[test]
fn channel_catalog_includes_email_config_backed_smtp_surface() {
    let catalog = list_channel_catalog();
    let email = catalog
        .iter()
        .find(|entry| entry.id == "email")
        .expect("email catalog entry");

    assert_eq!(
        email.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(email.selection_order, 100);
    assert_eq!(email.aliases, vec!["smtp", "imap"]);
    assert_eq!(email.transport, "smtp_imap");
    assert_eq!(
        email.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Address]
    );
    assert_eq!(email.operations[0].command, "channels send email");
    assert_eq!(email.operations[1].command, "email-serve");
    assert_eq!(
        email.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        email.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );
}

#[test]
fn channel_catalog_includes_nextcloud_talk_config_backed_bot_surface() {
    let catalog = list_channel_catalog();
    let nextcloud_talk = catalog
        .iter()
        .find(|entry| entry.id == "nextcloud-talk")
        .expect("nextcloud talk catalog entry");
    let synology_chat = catalog
        .iter()
        .find(|entry| entry.id == "synology-chat")
        .expect("synology chat catalog entry");

    assert_eq!(
        nextcloud_talk.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(nextcloud_talk.selection_order, 160);
    assert_eq!(nextcloud_talk.aliases, vec!["nextcloud", "nextcloudtalk"]);
    assert_eq!(nextcloud_talk.transport, "nextcloud_talk_bot_api");
    assert_eq!(
        nextcloud_talk.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Conversation]
    );
    assert_eq!(
        nextcloud_talk.operations[0].command,
        "channels send nextcloud-talk"
    );
    assert_eq!(nextcloud_talk.operations[1].command, "nextcloud-talk-serve");
    assert_eq!(
        nextcloud_talk.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        nextcloud_talk.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );

    assert_eq!(
        synology_chat.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(synology_chat.selection_order, 165);
    assert_eq!(synology_chat.aliases, vec!["synologychat", "synochat"]);
    assert_eq!(
        synology_chat.transport,
        "synology_chat_outgoing_incoming_webhooks"
    );
    assert_eq!(
        synology_chat.supported_target_kinds,
        vec![ChannelCatalogTargetKind::Address]
    );
    assert_eq!(
        synology_chat.operations[0].command,
        "channels send synology-chat"
    );
    assert_eq!(synology_chat.operations[1].command, "synology-chat-serve");
    assert_eq!(
        synology_chat.operations[0].availability,
        ChannelCatalogOperationAvailability::Implemented
    );
    assert_eq!(
        synology_chat.operations[1].availability,
        ChannelCatalogOperationAvailability::Stub
    );
}

#[test]
fn channel_status_snapshots_redact_webhook_channel_status_urls() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
            "dingtalk": {
                "enabled": true,
                "webhook_url": "https://oapi.dingtalk.com/robot/send?access_token=secret-token"
            },
            "google_chat": {
                "enabled": true,
                "webhook_url": "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=secret-key&token=secret-token"
            },
            "teams": {
                "enabled": true,
                "webhook_url": "https://outlook.office.com/webhook/abc123/IncomingWebhook/demo?tenant=secret-tenant&auth=secret-auth"
            },
            "synology_chat": {
                "enabled": true,
                "incoming_url": "https://chat.example.test/webapi/entry.cgi?api=SYNO.Chat.External&method=incoming&version=2&token=secret-token"
            }
        }))
        .expect("deserialize webhook channel config");

    let snapshots = channel_status_snapshots(&config);
    let dingtalk = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "dingtalk")
        .expect("dingtalk snapshot");
    let google_chat = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "google-chat")
        .expect("google chat snapshot");
    let teams = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "teams")
        .expect("teams snapshot");
    let synology_chat = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "synology-chat")
        .expect("synology chat snapshot");

    assert_eq!(
        dingtalk.api_base_url.as_deref(),
        Some("https://oapi.dingtalk.com/robot/send")
    );
    assert_eq!(
        google_chat.api_base_url.as_deref(),
        Some("https://chat.googleapis.com/v1/spaces/AAAA/messages")
    );
    assert_eq!(
        teams.api_base_url.as_deref(),
        Some("https://outlook.office.com/")
    );
    assert_eq!(
        synology_chat.api_base_url.as_deref(),
        Some("https://chat.example.test/webapi/entry.cgi")
    );
}

#[test]
fn channel_status_snapshots_redact_generic_webhook_path_segments() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "webhook": {
            "enabled": true,
            "endpoint_url": "https://hooks.example.test/customer/secret-token/send?trace=secret"
        }
    }))
    .expect("deserialize generic webhook config");

    let webhook = channel_status_snapshots(&config)
        .into_iter()
        .find(|snapshot| snapshot.id == "webhook")
        .expect("generic webhook snapshot");

    assert_eq!(
        webhook.api_base_url.as_deref(),
        Some("https://hooks.example.test/")
    );
}

#[test]
fn email_channel_status_snapshot_reports_smtp_readiness() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "email": {
            "enabled": true,
            "smtp_host": "smtps://smtp.example.test:465?auth=plain",
            "smtp_username": "mailer@example.test",
            "smtp_password": "top-secret",
            "from_address": "Loong <ops@example.test>"
        }
    }))
    .expect("deserialize email channel config");

    let snapshots = channel_status_snapshots(&config);
    let email = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "email")
        .expect("email snapshot");
    let send_operation = email
        .operation(CHANNEL_OPERATION_SEND_ID)
        .expect("email send operation");
    let serve_operation = email
        .operation(CHANNEL_OPERATION_SERVE_ID)
        .expect("email serve operation");

    assert_eq!(
        email.api_base_url.as_deref(),
        Some("smtps://smtp.example.test:465")
    );
    assert!(
        email
            .notes
            .iter()
            .any(|note| note == "from_address=Loong <ops@example.test>")
    );
    assert_eq!(send_operation.health, ChannelOperationHealth::Ready);
    assert_eq!(serve_operation.health, ChannelOperationHealth::Unsupported);
}

#[test]
fn webhook_status_snapshot_rejects_invalid_auth_header_values() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "webhook": {
            "enabled": true,
            "endpoint_url": "https://hooks.example.test/send",
            "auth_token": "token-123",
            "auth_token_prefix": "Bearer\n",
            "signing_secret": "signing-secret"
        }
    }))
    .expect("deserialize generic webhook config");

    let webhook = channel_status_snapshots(&config)
        .into_iter()
        .find(|snapshot| snapshot.id == "webhook")
        .expect("generic webhook snapshot");
    let send = webhook
        .operation(CHANNEL_OPERATION_SEND_ID)
        .expect("webhook send operation");
    let serve = webhook
        .operation(CHANNEL_OPERATION_SERVE_ID)
        .expect("webhook serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("auth header value is invalid")),
        "unexpected issues: {:?}",
        send.issues
    );
    assert_eq!(serve.health, ChannelOperationHealth::Ready);
}

#[test]
fn webhook_status_snapshot_requires_signing_secret_for_serve() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "webhook": {
            "enabled": true,
            "endpoint_url": "https://hooks.example.test/send"
        }
    }))
    .expect("deserialize generic webhook config");

    let webhook = channel_status_snapshots(&config)
        .into_iter()
        .find(|snapshot| snapshot.id == "webhook")
        .expect("generic webhook snapshot");
    let serve = webhook
        .operation(CHANNEL_OPERATION_SERVE_ID)
        .expect("webhook serve operation");

    assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("signing_secret")),
        "unexpected serve issues: {:?}",
        serve.issues
    );
}

#[test]
fn wecom_status_rejects_non_websocket_endpoint_schemes() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "wecom": {
            "enabled": true,
            "bot_id": "wx-bot-id",
            "secret": "wx-secret",
            "allowed_conversation_ids": ["conv-1"],
            "websocket_url": "https://wecom.example.test/aibot"
        }
    }))
    .expect("deserialize wecom config");

    let snapshots = channel_status_snapshots(&config);
    let wecom = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "wecom")
        .expect("wecom snapshot");
    let send = wecom.operation("send").expect("wecom send operation");
    let serve = wecom.operation("serve").expect("wecom serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("websocket_url must use ws or wss")),
        "send issues should reject non-websocket schemes: {:?}",
        send.issues
    );
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("websocket_url must use ws or wss")),
        "serve issues should reject non-websocket schemes: {:?}",
        serve.issues
    );
}

#[test]
fn channel_inventory_exposes_grouped_channel_surfaces() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.remove("TELEGRAM_BOT_TOKEN");

    let config = LoongConfig::default();
    let inventory = channel_inventory(&config);

    assert_eq!(
        inventory
            .channel_surfaces
            .iter()
            .map(|surface| surface.catalog.id)
            .collect::<Vec<_>>(),
        vec![
            "telegram",
            "feishu",
            "matrix",
            "wecom",
            "qqbot",
            "weixin",
            "onebot",
            "whatsapp-personal",
            "discord",
            "slack",
            "line",
            "dingtalk",
            "whatsapp",
            "email",
            "webhook",
            "google-chat",
            "signal",
            "twitch",
            "teams",
            "mattermost",
            "nextcloud-talk",
            "synology-chat",
            "irc",
            "imessage",
            "nostr",
            "tlon",
            "zalo",
            "zalo-personal",
            "webchat",
        ]
    );

    let telegram = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "telegram")
        .expect("telegram surface");
    assert_eq!(
        telegram.catalog.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(telegram.configured_accounts.len(), 1);
    assert_eq!(
        telegram.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(telegram.configured_accounts[0].id, "telegram");

    let discord = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "discord")
        .expect("discord surface");
    assert_eq!(
        discord.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(discord.configured_accounts.len(), 1);
    assert_eq!(
        discord.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(discord.configured_accounts[0].id, "discord");
    assert!(
        inventory
            .channel_access_policies
            .iter()
            .any(|policy| policy.channel_id == "telegram"
                && policy.configured_account_id == "default"
                && policy.conversation_config_key == "allowed_chat_ids"
                && policy.sender_config_key == "allowed_sender_ids"),
        "channel inventory should expose structured access policy for telegram"
    );
    let discord_encoded = serde_json::to_value(discord).expect("serialize discord channel surface");
    assert!(
        discord_encoded.get("plugin_bridge_discovery").is_none(),
        "channel surface output should omit plugin bridge discovery when absent: {discord_encoded}"
    );
    assert!(
        discord_encoded
            .get("catalog")
            .and_then(serde_json::Value::as_object)
            .map(|catalog| !catalog.contains_key("plugin_bridge_contract"))
            .unwrap_or(false),
        "channel surface catalog output should omit plugin bridge contract when absent: {discord_encoded}"
    );

    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    assert_eq!(
        weixin.catalog.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(weixin.configured_accounts.len(), 1);
    assert_eq!(
        weixin.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(weixin.configured_accounts[0].id, "weixin");
    assert_eq!(
        weixin.configured_accounts[0].configured_account_id,
        "default"
    );

    let qqbot = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "qqbot")
        .expect("qqbot surface");
    assert_eq!(
        qqbot.catalog.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(qqbot.configured_accounts.len(), 1);
    assert_eq!(
        qqbot.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(qqbot.configured_accounts[0].id, "qqbot");
    assert_eq!(
        qqbot.configured_accounts[0].configured_account_id,
        "default"
    );

    let onebot = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "onebot")
        .expect("onebot surface");
    assert_eq!(
        onebot.catalog.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(onebot.configured_accounts.len(), 1);
    assert_eq!(
        onebot.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(onebot.configured_accounts[0].id, "onebot");
    assert_eq!(
        onebot.configured_accounts[0].configured_account_id,
        "default"
    );

    let line = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "line")
        .expect("line surface");
    assert_eq!(
        line.catalog.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(line.configured_accounts.len(), 1);
    assert_eq!(
        line.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(line.configured_accounts[0].id, "line");

    let wecom = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "wecom")
        .expect("wecom surface");
    assert_eq!(
        wecom.catalog.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(wecom.configured_accounts.len(), 1);
    assert_eq!(
        wecom.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(wecom.configured_accounts[0].id, "wecom");

    let webhook = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "webhook")
        .expect("webhook surface");
    assert_eq!(
        webhook.catalog.implementation_status,
        ChannelCatalogImplementationStatus::PluginBacked
    );
    assert_eq!(webhook.configured_accounts.len(), 1);
    assert_eq!(
        webhook.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(webhook.configured_accounts[0].id, "webhook");

    let mattermost = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "mattermost")
        .expect("mattermost surface");
    assert_eq!(
        mattermost.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(mattermost.configured_accounts.len(), 1);
    assert_eq!(
        mattermost.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(mattermost.configured_accounts[0].id, "mattermost");

    let teams = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "teams")
        .expect("teams surface");
    assert_eq!(
        teams.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(teams.configured_accounts.len(), 1);
    assert_eq!(
        teams.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(teams.configured_accounts[0].id, "teams");

    let synology_chat = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "synology-chat")
        .expect("synology chat surface");
    assert_eq!(
        synology_chat.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(synology_chat.configured_accounts.len(), 1);
    assert_eq!(
        synology_chat.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(synology_chat.configured_accounts[0].id, "synology-chat");

    let imessage = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "imessage")
        .expect("imessage surface");
    assert_eq!(
        imessage.catalog.implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(imessage.configured_accounts.len(), 1);
    assert_eq!(
        imessage.default_configured_account_id.as_deref(),
        Some("default")
    );
    assert_eq!(imessage.configured_accounts[0].id, "imessage");

    let webchat = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "webchat")
        .expect("webchat surface");
    assert_eq!(
        webchat.catalog.implementation_status,
        ChannelCatalogImplementationStatus::Stub
    );
    assert_eq!(webchat.catalog.aliases, vec!["browser-chat", "web-ui"]);
    assert!(webchat.configured_accounts.is_empty());
}

#[test]
fn catalog_only_channel_entries_skip_platforms_that_already_have_status_snapshots() {
    let catalog = vec![
        ChannelCatalogEntry {
            id: "telegram",
            label: "Telegram",
            selection_order: 10,
            selection_label: "personal and group chat bot",
            blurb: "Shipped Telegram Bot API surface with direct send and reply-loop runtime support.",
            implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
            capabilities: vec![
                ChannelCapability::RuntimeBacked,
                ChannelCapability::Send,
                ChannelCapability::Serve,
                ChannelCapability::RuntimeTracking,
            ],
            aliases: vec![],
            transport: "telegram_bot_api_polling",
            onboarding: TELEGRAM_ONBOARDING_DESCRIPTOR,
            plugin_bridge_contract: None,
            supported_target_kinds: vec![ChannelCatalogTargetKind::Conversation],
            operations: vec![
                ChannelCatalogOperation {
                    id: "send",
                    label: "direct send",
                    command: "channels send telegram",
                    availability: ChannelCatalogOperationAvailability::Implemented,
                    tracks_runtime: false,
                    requirements: &[],
                    default_target_kind: None,
                    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                },
                ChannelCatalogOperation {
                    id: "serve",
                    label: "reply loop",
                    command: "channels serve telegram",
                    availability: ChannelCatalogOperationAvailability::Implemented,
                    tracks_runtime: true,
                    requirements: &[],
                    default_target_kind: None,
                    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                },
            ],
        },
        ChannelCatalogEntry {
            id: "discord",
            label: "Discord",
            selection_order: 40,
            selection_label: "community server bot",
            blurb: "Shipped Discord outbound message surface with config-backed direct sends; inbound gateway/runtime support remains planned.",
            implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
            capabilities: vec![ChannelCapability::MultiAccount, ChannelCapability::Send],
            aliases: vec![],
            transport: "discord_http_api",
            onboarding: DISCORD_ONBOARDING_DESCRIPTOR,
            plugin_bridge_contract: None,
            supported_target_kinds: vec![ChannelCatalogTargetKind::Conversation],
            operations: vec![
                ChannelCatalogOperation {
                    id: "send",
                    label: "direct send",
                    command: "channels send discord",
                    availability: ChannelCatalogOperationAvailability::Implemented,
                    tracks_runtime: false,
                    requirements: &[],
                    default_target_kind: None,
                    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                },
                ChannelCatalogOperation {
                    id: "serve",
                    label: "reply loop",
                    command: "discord-serve",
                    availability: ChannelCatalogOperationAvailability::Stub,
                    tracks_runtime: false,
                    requirements: &[],
                    default_target_kind: None,
                    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                },
            ],
        },
    ];
    let snapshots = vec![ChannelStatusSnapshot {
        id: "telegram",
        configured_account_id: "default".to_owned(),
        configured_account_label: "default".to_owned(),
        is_default_account: true,
        default_account_source: ChannelDefaultAccountSelectionSource::Fallback,
        label: "Telegram",
        aliases: vec![],
        transport: "telegram_bot_api_polling",
        compiled: true,
        enabled: false,
        api_base_url: Some("https://api.telegram.org".to_owned()),
        notes: vec![],
        reserved_runtime_fields: Vec::new(),
        operations: vec![ChannelOperationStatus {
            id: "serve",
            label: "reply loop",
            command: "channels serve telegram",
            health: ChannelOperationHealth::Disabled,
            detail: "disabled".to_owned(),
            issues: vec![],
            runtime: None,
        }],
    }];

    let catalog_only = catalog_only_channel_entries_from(&catalog, &snapshots);

    assert_eq!(catalog_only.len(), 1);
    assert_eq!(catalog_only[0].id, "discord");
    assert_eq!(
        catalog_only[0].implementation_status,
        ChannelCatalogImplementationStatus::ConfigBacked
    );
    assert_eq!(
        catalog_only[0].operations[0].command,
        "channels send discord"
    );
}

#[test]
fn shipped_channel_registry_descriptors_define_snapshot_builders() {
    for descriptor in sorted_channel_registry_descriptors() {
        let requires_snapshot_builder = matches!(
            descriptor.implementation_status,
            ChannelCatalogImplementationStatus::RuntimeBacked
                | ChannelCatalogImplementationStatus::ConfigBacked
                | ChannelCatalogImplementationStatus::PluginBacked
        );
        if !requires_snapshot_builder {
            continue;
        }

        assert!(
            descriptor.snapshot_builder.is_some(),
            "built-in shipped channel `{}` must define a snapshot builder",
            descriptor.id
        );
    }
}

#[test]
fn channel_registry_stays_physically_sorted_by_selection_order() {
    for descriptor_pair in CHANNEL_REGISTRY.windows(2) {
        let first_descriptor = &descriptor_pair[0];
        let second_descriptor = &descriptor_pair[1];

        assert!(
            first_descriptor.selection_order <= second_descriptor.selection_order,
            "channel registry order drifted: {} ({}) appears before {} ({})",
            first_descriptor.id,
            first_descriptor.selection_order,
            second_descriptor.id,
            second_descriptor.selection_order
        );
    }
}

#[test]
fn telegram_status_reports_ready_when_token_and_allowlist_are_configured() {
    let mut config = LoongConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loong_contracts::SecretRef::Inline(
        "123456:token".to_owned(),
    ));
    config.telegram.allowed_chat_ids = vec![123];

    let snapshots = channel_status_snapshots(&config);
    let telegram = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "telegram")
        .expect("telegram snapshot");
    let serve = telegram
        .operation("serve")
        .expect("telegram serve operation");

    assert_eq!(serve.health, ChannelOperationHealth::Ready);
    assert!(serve.is_ready());
    assert_eq!(
        telegram.api_base_url.as_deref(),
        Some("https://api.telegram.org")
    );
    assert!(!serve.runtime.as_ref().expect("telegram runtime").running);
}

#[test]
fn telegram_status_splits_direct_send_and_reply_loop_readiness() {
    let mut config = LoongConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loong_contracts::SecretRef::Inline(
        "123456:token".to_owned(),
    ));

    let snapshots = channel_status_snapshots(&config);
    let telegram = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "telegram")
        .expect("telegram snapshot");
    let send = telegram.operation("send").expect("telegram send operation");
    let serve = telegram
        .operation("serve")
        .expect("telegram serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("allowed_chat_ids")),
        "serve issues should mention allowlist"
    );
    assert!(send.runtime.is_none());
    assert_eq!(
        serve
            .runtime
            .as_ref()
            .expect("telegram runtime")
            .active_runs,
        0
    );
}

#[test]
fn feishu_status_splits_direct_send_and_webhook_readiness() {
    let mut config = LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.mode = Some(FeishuChannelServeMode::Webhook);
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("app-id".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("app-secret".to_owned()));

    let snapshots = channel_status_snapshots(&config);
    let feishu = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "feishu")
        .expect("feishu snapshot");
    let send = feishu.operation("send").expect("feishu send operation");
    let serve = feishu.operation("serve").expect("feishu serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("allowed_chat_ids")),
        "serve issues should mention allowlist"
    );
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("verification_token")),
        "serve issues should mention verification token"
    );
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("encrypt_key")),
        "serve issues should mention encrypt key"
    );
    assert!(send.runtime.is_none());
    assert_eq!(
        serve.runtime.as_ref().expect("serve runtime").active_runs,
        0
    );
}

#[test]
fn matrix_status_requires_user_id_when_ignoring_self_messages() {
    let mut config = LoongConfig::default();
    config.matrix.enabled = true;
    config.matrix.access_token = Some(loong_contracts::SecretRef::Inline(
        "matrix-token".to_owned(),
    ));
    config.matrix.base_url = Some("https://matrix.example.org".to_owned());
    config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
    config.matrix.ignore_self_messages = true;

    let snapshots = channel_status_snapshots(&config);
    let matrix = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "matrix")
        .expect("matrix snapshot");
    let send = matrix.operation("send").expect("matrix send operation");
    let serve = matrix.operation("serve").expect("matrix serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
    assert!(
        serve.issues.iter().any(|issue| issue.contains("user_id")),
        "serve issues should require user_id when ignore_self_messages is enabled"
    );
}

#[test]
fn discord_status_splits_config_backed_send_and_stub_serve() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.remove(DISCORD_BOT_TOKEN_ENV);
    let mut config = LoongConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token_env = None;

    let snapshots = channel_status_snapshots(&config);
    let discord = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "discord")
        .expect("discord snapshot");
    let send = discord.operation("send").expect("discord send operation");
    let serve = discord.operation("serve").expect("discord serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues.iter().any(|issue| issue.contains("bot_token")),
        "send issues should mention the missing discord bot token"
    );
    assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("not implemented")),
        "serve issues should explain that discord serve is not implemented"
    );
    assert_eq!(
        discord.api_base_url.as_deref(),
        Some("https://discord.com/api/v10")
    );
    assert!(send.runtime.is_none());
    assert!(serve.runtime.is_none());
}

#[test]
fn discord_status_rejects_non_http_api_base_url() {
    let mut config = LoongConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token = Some(loong_contracts::SecretRef::Inline(
        "discord-token".to_owned(),
    ));
    config.discord.api_base_url = Some("file:///tmp/discord-api".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let discord = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "discord")
        .expect("discord snapshot");
    let send = discord.operation("send").expect("discord send operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("requires http or https")),
        "send issues should reject non-http discord api base urls"
    );
}

#[test]
fn discord_status_rejects_api_base_url_with_query_string() {
    let mut config = LoongConfig::default();
    config.discord.enabled = true;
    config.discord.bot_token = Some(loong_contracts::SecretRef::Inline(
        "discord-token".to_owned(),
    ));
    config.discord.api_base_url = Some("https://discord.com/api/v10?debug=1".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let discord = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "discord")
        .expect("discord snapshot");
    let send = discord.operation("send").expect("discord send operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("must not include a query string")),
        "send issues should reject discord api base urls with query strings: {send:#?}"
    );
}

#[test]
fn discord_status_notes_reserved_future_runtime_fields_when_present() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "discord": {
            "enabled": true,
            "bot_token": "discord-token",
            "application_id": "discord-application-id",
            "allowed_guild_ids": ["guild-a", "guild-b"]
        }
    }))
    .expect("deserialize discord config");

    let snapshots = channel_status_snapshots(&config);
    let discord = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "discord")
        .expect("discord snapshot");

    assert!(
        discord
            .notes
            .iter()
            .any(|note| note == "reserved_runtime_field=application_id"),
        "discord status notes should preserve configured future runtime fields: {discord:#?}"
    );
    assert!(
        discord
            .notes
            .iter()
            .any(|note| note == "reserved_runtime_field=allowed_guild_ids:2"),
        "discord status notes should record allowed_guild_ids count when reserved runtime fields are configured: {discord:#?}"
    );
    assert_eq!(
        discord.reserved_runtime_fields,
        vec![
            "application_id".to_owned(),
            "allowed_guild_ids:2".to_owned()
        ],
        "discord status snapshots should expose structured reserved runtime fields alongside operator notes: {discord:#?}"
    );
}

#[test]
fn slack_status_reports_ready_send_and_stub_serve() {
    let mut config = LoongConfig::default();
    config.slack.enabled = true;
    config.slack.bot_token = Some(loong_contracts::SecretRef::Inline(
        "xoxb-test-token".to_owned(),
    ));

    let snapshots = channel_status_snapshots(&config);
    let slack = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "slack")
        .expect("slack snapshot");
    let send = slack.operation("send").expect("slack send operation");
    let serve = slack.operation("serve").expect("slack serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
    assert_eq!(slack.api_base_url.as_deref(), Some("https://slack.com/api"));
    assert!(send.runtime.is_none());
    assert!(serve.runtime.is_none());
}

#[test]
fn slack_status_rejects_api_base_url_with_fragment() {
    let mut config = LoongConfig::default();
    config.slack.enabled = true;
    config.slack.bot_token = Some(loong_contracts::SecretRef::Inline(
        "xoxb-test-token".to_owned(),
    ));
    config.slack.api_base_url = Some("https://slack.com/api#fragment".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let slack = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "slack")
        .expect("slack snapshot");
    let send = slack.operation("send").expect("slack send operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("must not include a fragment")),
        "send issues should reject slack api base urls with fragments: {send:#?}"
    );
}

#[test]
fn line_status_reports_ready_send_and_stub_serve() {
    let mut config = LoongConfig::default();
    config.line.enabled = true;
    config.line.channel_access_token = Some(loong_contracts::SecretRef::Inline(
        "line-access-token".to_owned(),
    ));

    let snapshots = channel_status_snapshots(&config);
    let line = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "line")
        .expect("line snapshot");
    let send = line.operation("send").expect("line send operation");
    let serve = line.operation("serve").expect("line serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue == "channel_secret is missing"),
        "unexpected serve issues: {:?}",
        serve.issues
    );
    assert_eq!(
        line.api_base_url.as_deref(),
        Some("https://api.line.me/v2/bot")
    );
    assert!(send.runtime.is_none());
    assert!(serve.runtime.is_some());
}

#[test]
fn irc_status_reports_ready_send_and_planned_serve() {
    let mut config = LoongConfig::default();
    config.irc.enabled = true;
    config.irc.server = Some("ircs://irc.example.test:6697".to_owned());
    config.irc.nickname = Some("loong".to_owned());
    config.irc.username = Some("loong".to_owned());
    config.irc.channel_names = vec!["#ops".to_owned()];

    let snapshots = channel_status_snapshots(&config);
    let irc = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "irc")
        .expect("irc snapshot");
    let send = irc.operation("send").expect("irc send operation");
    let serve = irc.operation("serve").expect("irc serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
    assert_eq!(
        irc.api_base_url.as_deref(),
        Some("ircs://irc.example.test:6697")
    );
    assert!(
        irc.notes.iter().any(|note| note == "nickname=loong"),
        "irc notes should include the resolved nickname"
    );
    assert!(
        irc.notes.iter().any(|note| note == "server_transport=ircs"),
        "irc notes should include the parsed transport"
    );
    assert!(
        irc.notes.iter().any(|note| note == "channel_names=#ops"),
        "irc notes should include configured channel names"
    );
    assert!(send.runtime.is_none());
    assert!(serve.runtime.is_none());
}

#[test]
fn irc_status_formats_ipv6_server_endpoint_with_brackets() {
    let mut config = LoongConfig::default();
    config.irc.enabled = true;
    config.irc.server = Some("ircs://[2001:db8::42]:6697".to_owned());
    config.irc.nickname = Some("loong".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let irc = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "irc")
        .expect("irc snapshot");

    assert_eq!(
        irc.api_base_url.as_deref(),
        Some("ircs://[2001:db8::42]:6697")
    );
}

#[test]
fn whatsapp_status_reports_ready_send_when_access_token_and_phone_number_id_are_configured() {
    let mut config = LoongConfig::default();
    config.whatsapp.enabled = true;
    config.whatsapp.access_token = Some(loong_contracts::SecretRef::Inline(
        "whatsapp-access-token".to_owned(),
    ));
    config.whatsapp.phone_number_id = Some("1234567890".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let whatsapp = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "whatsapp")
        .expect("whatsapp snapshot");
    let send = whatsapp.operation("send").expect("whatsapp send operation");
    let serve = whatsapp
        .operation("serve")
        .expect("whatsapp serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("verify_token")),
        "serve issues should mention the missing verify token"
    );
    assert!(
        serve
            .issues
            .iter()
            .any(|issue| issue.contains("app_secret")),
        "serve issues should mention the missing app secret"
    );
    assert_eq!(
        whatsapp.api_base_url.as_deref(),
        Some("https://graph.facebook.com/v25.0")
    );
    assert!(
        whatsapp
            .notes
            .iter()
            .any(|note| note == "phone_number_id=1234567890"),
        "status notes should expose the resolved phone number id"
    );
    assert!(send.runtime.is_none());
    assert!(serve.runtime.is_some());
}

#[test]
fn mattermost_status_reports_ready_send_and_stub_serve() {
    let mut config = LoongConfig::default();
    config.mattermost.enabled = true;
    config.mattermost.server_url = Some("https://mattermost.example.test".to_owned());
    config.mattermost.bot_token = Some(loong_contracts::SecretRef::Inline(
        "mattermost-bot-token".to_owned(),
    ));

    let snapshots = channel_status_snapshots(&config);
    let mattermost = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "mattermost")
        .expect("mattermost snapshot");
    let send = mattermost
        .operation("send")
        .expect("mattermost send operation");
    let serve = mattermost
        .operation("serve")
        .expect("mattermost serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
    assert_eq!(
        mattermost.api_base_url.as_deref(),
        Some("https://mattermost.example.test")
    );
    assert!(send.runtime.is_none());
    assert!(serve.runtime.is_none());
}

#[test]
fn feishu_websocket_status_uses_websocket_requirements() {
    let mut config = LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("app-id".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("app-secret".to_owned()));
    config.feishu.mode = Some(crate::config::FeishuChannelServeMode::Websocket);
    config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];

    let snapshots = channel_status_snapshots(&config);
    let feishu = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "feishu")
        .expect("feishu snapshot");
    let serve = feishu.operation("serve").expect("feishu serve operation");

    assert_eq!(serve.health, ChannelOperationHealth::Ready);
    assert!(
        serve
            .issues
            .iter()
            .all(|issue| !issue.contains("verification_token")),
        "websocket mode must not require a webhook verification token"
    );
    assert!(
        serve
            .issues
            .iter()
            .all(|issue| !issue.contains("encrypt_key")),
        "websocket mode must not require a webhook encrypt key"
    );
    assert!(
        feishu.notes.iter().any(|note| note == "mode=websocket"),
        "status notes should surface the configured feishu serve mode"
    );
    assert!(
        feishu
            .notes
            .iter()
            .all(|note| !note.starts_with("webhook_bind=")),
        "websocket mode notes should not imply a webhook bind address is active"
    );
    assert!(
        feishu
            .notes
            .iter()
            .all(|note| !note.starts_with("webhook_path=")),
        "websocket mode notes should not imply a webhook callback path is active"
    );
}

#[test]
fn channel_status_snapshots_merge_runtime_activity_for_serve_operations() {
    let mut config = LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("app-id".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("app-secret".to_owned()));
    config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
    config.feishu.verification_token = Some(loong_contracts::SecretRef::Inline("token".to_owned()));
    config.feishu.encrypt_key = Some(loong_contracts::SecretRef::Inline("encrypt".to_owned()));

    let runtime_dir = temp_runtime_dir("registry-runtime");
    let now = now_ms();
    state::write_runtime_state_for_test(
        runtime_dir.as_path(),
        ChannelPlatform::Feishu,
        "serve",
        true,
        true,
        2,
        Some(now.saturating_sub(1_000)),
        Some(now.saturating_sub(500)),
        Some(4242),
    )
    .expect("write runtime state");

    let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
    let feishu = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "feishu")
        .expect("feishu snapshot");
    let serve = feishu.operation("serve").expect("feishu serve operation");
    let runtime = serve.runtime.as_ref().expect("runtime info");

    assert!(runtime.running);
    assert!(!runtime.stale);
    assert!(runtime.busy);
    assert_eq!(runtime.active_runs, 2);
    assert_eq!(runtime.pid, Some(4242));
}

#[test]
fn channel_status_snapshots_report_resolved_account_identity_in_notes() {
    let mut config = LoongConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loong_contracts::SecretRef::Inline(
        "123456:token".to_owned(),
    ));
    config.telegram.allowed_chat_ids = vec![123];

    let snapshots = channel_status_snapshots(&config);
    let telegram = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "telegram")
        .expect("telegram snapshot");

    assert!(
        telegram
            .notes
            .iter()
            .any(|note| note.contains("account_id=bot_123456")),
        "telegram notes should expose the resolved account id"
    );
}

#[test]
fn channel_status_snapshots_report_telegram_acp_bootstrap_mcp_servers_in_notes() {
    let mut config = LoongConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loong_contracts::SecretRef::Inline(
        "123456:token".to_owned(),
    ));
    config.telegram.allowed_chat_ids = vec![123];
    config.telegram.acp.bootstrap_mcp_servers = vec!["filesystem".to_owned()];
    config.telegram.acp.working_directory = Some(" /workspace/telegram ".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let telegram = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "telegram")
        .expect("telegram snapshot");

    assert!(
        telegram
            .notes
            .iter()
            .any(|note| note == "acp_bootstrap_mcp_servers=filesystem"),
        "telegram notes should expose configured ACP bootstrap MCP servers"
    );
    assert!(
        telegram
            .notes
            .iter()
            .any(|note| note == "acp_working_directory=/workspace/telegram"),
        "telegram notes should expose configured ACP working directory"
    );
}

#[test]
fn channel_status_snapshots_report_feishu_acp_bootstrap_mcp_servers_in_notes() {
    let mut config = LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("app-secret".to_owned()));
    config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
    config.feishu.verification_token = Some(loong_contracts::SecretRef::Inline("token".to_owned()));
    config.feishu.encrypt_key = Some(loong_contracts::SecretRef::Inline("encrypt".to_owned()));
    config.feishu.acp.bootstrap_mcp_servers = vec!["search".to_owned()];
    config.feishu.acp.working_directory = Some("/workspace/feishu".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let feishu = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "feishu")
        .expect("feishu snapshot");

    assert!(
        feishu
            .notes
            .iter()
            .any(|note| note == "acp_bootstrap_mcp_servers=search"),
        "feishu notes should expose configured ACP bootstrap MCP servers"
    );
    assert!(
        feishu
            .notes
            .iter()
            .any(|note| note == "acp_working_directory=/workspace/feishu"),
        "feishu notes should expose configured ACP working directory"
    );
}

#[test]
fn channel_status_snapshots_attach_account_identity_to_runtime_view() {
    let mut config = LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("app-secret".to_owned()));
    config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
    config.feishu.verification_token = Some(loong_contracts::SecretRef::Inline("token".to_owned()));
    config.feishu.encrypt_key = Some(loong_contracts::SecretRef::Inline("encrypt".to_owned()));

    let runtime_dir = temp_runtime_dir("registry-account-runtime");
    let now = now_ms();
    state::write_runtime_state_for_test_with_account_and_pid(
        runtime_dir.as_path(),
        ChannelPlatform::Feishu,
        "serve",
        "feishu_cli_a1b2c3",
        4242,
        true,
        true,
        2,
        Some(now.saturating_sub(1_000)),
        Some(now.saturating_sub(500)),
        Some(4242),
    )
    .expect("write runtime state");

    let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
    let feishu = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "feishu")
        .expect("feishu snapshot");
    let serve = feishu.operation("serve").expect("feishu serve operation");
    let runtime = serve.runtime.as_ref().expect("runtime info");

    assert_eq!(runtime.account_id.as_deref(), Some("feishu_cli_a1b2c3"));
    assert_eq!(runtime.account_label.as_deref(), Some("feishu:cli_a1b2c3"));
}

#[test]
fn channel_status_snapshots_preserve_runtime_instance_counts() {
    let mut config = LoongConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loong_contracts::SecretRef::Inline(
        "123456:token".to_owned(),
    ));
    config.telegram.allowed_chat_ids = vec![123];

    let runtime_dir = temp_runtime_dir("registry-duplicate-runtime");
    let now = now_ms();
    state::write_runtime_state_for_test_with_account_and_pid(
        runtime_dir.as_path(),
        ChannelPlatform::Telegram,
        "serve",
        "bot_123456",
        1001,
        true,
        true,
        1,
        Some(now.saturating_sub(300)),
        Some(now.saturating_sub(100)),
        Some(1001),
    )
    .expect("write first runtime state");
    state::write_runtime_state_for_test_with_account_and_pid(
        runtime_dir.as_path(),
        ChannelPlatform::Telegram,
        "serve",
        "bot_123456",
        1002,
        true,
        false,
        0,
        Some(now.saturating_sub(200)),
        Some(now.saturating_sub(50)),
        Some(1002),
    )
    .expect("write second runtime state");

    let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
    let telegram = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "telegram")
        .expect("telegram snapshot");
    let serve = telegram
        .operation("serve")
        .expect("telegram serve operation");
    let runtime = serve.runtime.as_ref().expect("runtime info");

    assert_eq!(runtime.instance_count, 2);
    assert_eq!(runtime.running_instances, 2);
    assert_eq!(runtime.stale_instances, 0);
}

#[test]
fn multi_account_registry_emits_one_snapshot_per_configured_account() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "telegram": {
            "enabled": true,
            "default_account": "Work Bot",
            "allowed_chat_ids": [1001],
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token": "123456:token-work",
                    "allowed_chat_ids": [2002]
                },
                "Personal": {
                    "bot_token": "654321:token-personal",
                    "allowed_chat_ids": [3003]
                }
            }
        }
    }))
    .expect("deserialize multi-account config");

    let telegram = channel_status_snapshots(&config)
        .into_iter()
        .filter(|snapshot| snapshot.id == "telegram")
        .collect::<Vec<_>>();

    assert_eq!(telegram.len(), 2);
    assert_eq!(telegram[0].configured_account_id, "personal");
    assert_eq!(telegram[1].configured_account_id, "work-bot");
    assert!(
        telegram[1]
            .notes
            .iter()
            .any(|note| note == "configured_account_id=work-bot")
    );
    assert!(
        telegram[1]
            .notes
            .iter()
            .any(|note| note == "account_id=ops-bot")
    );
}

#[test]
fn multi_account_registry_marks_default_configured_account() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "telegram": {
            "enabled": true,
            "default_account": "Work Bot",
            "allowed_chat_ids": [1001],
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token": "123456:token-work",
                    "allowed_chat_ids": [2002]
                },
                "Personal": {
                    "bot_token": "654321:token-personal",
                    "allowed_chat_ids": [3003]
                }
            }
        }
    }))
    .expect("deserialize multi-account config");

    let telegram = channel_status_snapshots(&config)
        .into_iter()
        .filter(|snapshot| snapshot.id == "telegram")
        .collect::<Vec<_>>();
    let encoded = serde_json::to_value(&telegram).expect("serialize telegram snapshots");

    assert!(
        telegram[1]
            .notes
            .iter()
            .any(|note| note == "default_account=true")
    );
    assert_eq!(
        encoded[0]
            .get("is_default_account")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        encoded[1]
            .get("is_default_account")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        encoded[1]
            .get("default_account_source")
            .and_then(serde_json::Value::as_str),
        Some("explicit_default")
    );
}

#[test]
fn multi_account_registry_records_fallback_default_account_source() {
    let config: LoongConfig = serde_json::from_value(serde_json::json!({
        "telegram": {
            "enabled": true,
            "accounts": {
                "Work": {
                    "bot_token": "123456:token-work",
                    "allowed_chat_ids": [2002]
                },
                "Alerts": {
                    "bot_token": "654321:token-alerts",
                    "allowed_chat_ids": [3003]
                }
            }
        }
    }))
    .expect("deserialize multi-account config");

    let telegram = channel_status_snapshots(&config)
        .into_iter()
        .filter(|snapshot| snapshot.id == "telegram")
        .collect::<Vec<_>>();

    assert!(telegram[0].is_default_account);
    assert_eq!(
        telegram[0].default_account_source,
        ChannelDefaultAccountSelectionSource::Fallback
    );
    assert!(
        telegram[0]
            .notes
            .iter()
            .any(|note| note == "default_account_source=fallback")
    );
}

fn temp_runtime_dir(suffix: &str) -> std::path::PathBuf {
    let unique = format!(
        "loong-channel-registry-{suffix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    std::env::temp_dir().join(unique)
}
