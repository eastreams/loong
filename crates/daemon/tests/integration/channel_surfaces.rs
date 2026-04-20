use super::*;

#[test]
fn render_channel_surfaces_text_reports_aliases_and_operation_health() {
    let mut config = mvp::config::LoongConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(loong_contracts::SecretRef::Inline(
        "123456:telegram-token".to_owned(),
    ));
    config.telegram.allowed_chat_ids = vec![1001];
    config.feishu.enabled = true;
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("app-secret".to_owned()));
    config.wecom.enabled = true;
    config.wecom.bot_id = Some(loong_contracts::SecretRef::Inline("bot_test".to_owned()));
    config.wecom.secret = Some(loong_contracts::SecretRef::Inline("secret_test".to_owned()));
    config.wecom.allowed_conversation_ids = vec!["group_demo".to_owned()];

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(
        rendered
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("LOONG")),
        "channel surface text should now use the shared compact header: {rendered}"
    );
    assert!(rendered.contains("channels"));
    assert!(rendered.contains("config=/tmp/loong.toml"));
    assert!(rendered.contains("Telegram [telegram]"));
    assert!(
        rendered.contains("capabilities=runtime_backed,multi_account,send,serve,runtime_tracking")
    );
    assert!(rendered.contains(&format!(
        "op send ({}) ready: ready target_kinds=conversation requirements=enabled,bot_token",
        channel_send_command("telegram")
    )));
    assert!(rendered.contains("Feishu/Lark [feishu]"));
    assert!(rendered.contains("implementation_status=runtime_backed"));
    assert!(
        rendered.contains("capabilities=runtime_backed,multi_account,send,serve,runtime_tracking")
    );
    assert!(rendered.contains(
        "onboarding strategy=manual_config status_command=\"loong doctor\" repair_command=\"loong doctor --fix\""
    ));
    assert!(rendered.contains("setup_hint=\"configure telegram bot credentials"));
    assert!(rendered.contains("target_kinds=receive_id,message_reply"));
    assert!(rendered.contains("configured_accounts=1"));
    assert!(rendered.contains("aliases=lark"));
    assert!(rendered.contains("account=feishu:cli_a1b2c3"));
    let feishu_section = rendered
        .split("Feishu/Lark [feishu]")
        .nth(1)
        .expect("feishu section should render");
    assert!(feishu_section.contains("policy conversation_key=allowed_chat_ids"));
    assert!(feishu_section.contains("sender_key=allowed_sender_ids"));
    assert!(feishu_section.contains("mention_required=false"));
    assert!(feishu_section.contains("senders=-"));
    assert!(rendered.contains(&format!(
        "op send ({}) ready: ready target_kinds=receive_id,message_reply requirements=enabled,app_id,app_secret",
        channel_send_command("feishu")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) misconfigured: allowed_chat_ids is empty target_kinds=message_reply requirements=enabled,app_id,app_secret,mode,allowed_chat_ids,allowed_sender_ids,verification_token,encrypt_key",
        channel_serve_command("feishu")
    )));
    assert!(rendered.contains("WeCom [wecom]"));
    assert!(rendered.contains("account=wecom:bot_test"));
    assert!(rendered.contains(
        "policy conversation_key=allowed_conversation_ids conversation_mode=exact_allowlist sender_key=allowed_sender_ids sender_mode=open mention_required=false conversations=group_demo senders=-"
    ));
    assert!(rendered.contains(&format!(
        "op send ({}) ready: ready target_kinds=conversation requirements=enabled,bot_id,secret,websocket_url",
        channel_send_command("wecom")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) ready: ready target_kinds=conversation requirements=enabled,bot_id,secret,allowed_conversation_ids,allowed_sender_ids,websocket_url,ping_interval_s",
        channel_serve_command("wecom")
    )));
    assert!(rendered.contains("running=false"));
}

#[test]
fn render_channel_surfaces_text_reports_configured_accounts_for_multi_account_channels() {
    let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
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

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(rendered.contains("configured_accounts=2"));
    assert!(rendered.contains("default_configured_account=work-bot"));
    assert!(rendered.contains("configured_account=work-bot"));
    assert!(rendered.contains("configured_account=personal"));
}

#[test]
fn render_channel_surfaces_text_reports_default_account_marker() {
    let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
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

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(rendered.contains("configured_account=work-bot"));
    assert!(rendered.contains("default_account=true"));
    assert!(rendered.contains("default_source=explicit_default"));
}

#[test]
fn render_channel_surfaces_text_reports_catalog_only_channels() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);
    let expected_summary = format!(
        "summary total_surfaces={} runtime_backed={} config_backed={} plugin_backed={} catalog_only={}",
        inventory.channel_surfaces.len(),
        inventory
            .channel_surfaces
            .iter()
            .filter(|surface| {
                surface.catalog.implementation_status
                    == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
            })
            .count(),
        inventory
            .channel_surfaces
            .iter()
            .filter(|surface| {
                surface.catalog.implementation_status
                    == mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
            })
            .count(),
        inventory
            .channel_surfaces
            .iter()
            .filter(|surface| {
                surface.catalog.implementation_status
                    == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
            })
            .count(),
        inventory
            .channel_surfaces
            .iter()
            .filter(|surface| {
                surface.catalog.implementation_status
                    == mvp::channel::ChannelCatalogImplementationStatus::Stub
            })
            .count()
    );

    assert!(rendered.contains(expected_summary.as_str()));
    assert!(rendered.contains("runtime-backed channels:"));
    assert!(rendered.contains("config-backed channels:"));
    assert!(rendered.contains("plugin-backed channels:"));
    assert!(rendered.contains("catalog-only channels:"));
    assert!(rendered.contains(
        "Discord [discord] implementation_status=config_backed selection_order=40 selection_label=\"community server bot\" capabilities=multi_account,send aliases=discord-bot transport=discord_http_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "blurb: Shipped Discord outbound message surface with config-backed direct sends; inbound gateway/runtime support remains planned."
    ));
    assert!(rendered.contains(&format!(
        "op send ({}) disabled: disabled by discord account configuration target_kinds=conversation requirements=enabled,bot_token",
        channel_send_command("discord")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) unsupported: discord serve runtime is not implemented yet target_kinds=conversation requirements=enabled,bot_token,application_id,allowed_guild_ids",
        channel_serve_command("discord")
    )));
    assert!(rendered.contains(
        "Slack [slack] implementation_status=config_backed selection_order=50 selection_label=\"workspace event bot\" capabilities=multi_account,send aliases=slack-bot transport=slack_web_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(&format!(
        "op send ({}) disabled: disabled by slack account configuration target_kinds=conversation requirements=enabled,bot_token",
        channel_send_command("slack")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) unsupported: slack serve runtime is not implemented yet target_kinds=conversation requirements=enabled,bot_token,app_token,signing_secret,allowed_channel_ids",
        channel_serve_command("slack")
    )));
    assert!(rendered.contains(
        "WhatsApp [whatsapp] implementation_status=runtime_backed selection_order=90 selection_label=\"business messaging app\" capabilities=runtime_backed,multi_account,send,serve,runtime_tracking aliases=wa,whatsapp-cloud transport=whatsapp_cloud_api target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(&format!(
        "op send ({}) disabled: disabled by whatsapp account configuration target_kinds=address requirements=enabled,access_token,phone_number_id",
        channel_send_command("whatsapp")
    )));
    assert!(rendered.contains(&format!(
        "op serve ({}) disabled: disabled by whatsapp account configuration target_kinds=address requirements=enabled,access_token,phone_number_id,verify_token,app_secret",
        channel_serve_command("whatsapp")
    )));
    assert!(rendered.contains(
        "LINE [line] implementation_status=runtime_backed selection_order=60 selection_label=\"consumer messaging bot\" capabilities=runtime_backed,multi_account,send,serve,runtime_tracking aliases=line-bot transport=line_messaging_api target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "DingTalk [dingtalk] implementation_status=config_backed selection_order=80 selection_label=\"group webhook bot\" capabilities=multi_account,send aliases=ding,ding-bot transport=dingtalk_custom_robot_webhook target_kinds=endpoint configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "Google Chat [google-chat] implementation_status=config_backed selection_order=120 selection_label=\"workspace space webhook\" capabilities=multi_account,send aliases=gchat,googlechat transport=google_chat_incoming_webhook target_kinds=endpoint configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (dingtalk-send) disabled: disabled by dingtalk account configuration target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "op send (google-chat-send) disabled: disabled by google_chat account configuration target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "op serve (google-chat-serve) unsupported: google chat incoming webhook surface is outbound-only target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "Signal [signal] implementation_status=config_backed selection_order=130 selection_label=\"private messenger bridge\" capabilities=multi_account,send aliases=signal-cli transport=signal_cli_rest_api target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (signal-send) disabled: disabled by signal account configuration target_kinds=address requirements=enabled,service_url,account"
    ));
    assert!(rendered.contains(
        "op serve (signal-serve) unsupported: signal serve runtime is not implemented yet target_kinds=address requirements=enabled,service_url,account"
    ));
    assert!(rendered.contains(
        "Microsoft Teams [teams] implementation_status=config_backed selection_order=140 selection_label=\"workspace webhook bot\" capabilities=multi_account,send aliases=msteams,ms-teams transport=microsoft_teams_incoming_webhook target_kinds=endpoint,conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (teams-send) disabled: disabled by teams account configuration target_kinds=endpoint requirements=enabled,webhook_url"
    ));
    assert!(rendered.contains(
        "op serve (teams-serve) unsupported: microsoft teams incoming webhook surface is outbound-only today target_kinds=conversation requirements=enabled,app_id,app_password,tenant_id,allowed_conversation_ids"
    ));
    assert!(rendered.contains(
        "Nextcloud Talk [nextcloud-talk] implementation_status=config_backed selection_order=160 selection_label=\"self-hosted room bot\" capabilities=multi_account,send aliases=nextcloud,nextcloudtalk transport=nextcloud_talk_bot_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (nextcloud-talk-send) disabled: disabled by nextcloud_talk account configuration target_kinds=conversation requirements=enabled,server_url,shared_secret"
    ));
    assert!(rendered.contains(
        "op serve (nextcloud-talk-serve) unsupported: nextcloud talk bot callback serve is not implemented yet target_kinds=conversation requirements=enabled,server_url,shared_secret"
    ));
    assert!(rendered.contains(
        "Synology Chat [synology-chat] implementation_status=config_backed selection_order=165 selection_label=\"nas webhook bot\" capabilities=multi_account,send aliases=synologychat,synochat transport=synology_chat_outgoing_incoming_webhooks target_kinds=address configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (synology-chat-send) disabled: disabled by synology_chat account configuration target_kinds=address requirements=enabled,incoming_url"
    ));
    assert!(rendered.contains(
        "op serve (synology-chat-serve) unsupported: synology chat outgoing webhook serve is not implemented yet target_kinds=address requirements=enabled,token,incoming_url,allowed_user_ids"
    ));
    assert!(rendered.contains(
        "iMessage [imessage] implementation_status=config_backed selection_order=180 selection_label=\"apple message bridge\" capabilities=multi_account,send aliases=bluebubbles,blue-bubbles transport=imessage_bridge_api target_kinds=conversation configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "op send (imessage-send) disabled: disabled by imessage account configuration target_kinds=conversation requirements=enabled,bridge_url,bridge_token"
    ));
    assert!(rendered.contains(
        "op serve (imessage-serve) unsupported: imessage bridge sync runtime is not implemented yet target_kinds=conversation requirements=enabled,bridge_url,bridge_token,allowed_chat_ids"
    ));
    assert!(rendered.contains(
        "Webhook [webhook] implementation_status=runtime_backed selection_order=110 selection_label=\"generic http integration\" capabilities=runtime_backed,multi_account,send,serve,runtime_tracking aliases=http-webhook transport=generic_webhook target_kinds=endpoint configured_accounts=1 default_configured_account=default"
    ));
    assert!(rendered.contains(
        "WebChat [webchat] implementation_status=stub selection_order=230 selection_label=\"embedded web inbox\""
    ));
    assert!(rendered.contains(
        "op send (webhook-send) disabled: disabled by webhook account configuration target_kinds=endpoint requirements=enabled,endpoint_url"
    ));
    assert!(rendered.contains(
        "op serve (webhook-serve) disabled: disabled by webhook account configuration target_kinds=endpoint requirements=enabled,signing_secret"
    ));
    assert!(rendered.contains(
        "onboarding strategy=manual_config status_command=\"loong doctor\" repair_command=\"loong doctor --fix\""
    ));
    assert!(rendered.contains(
        "setup_hint=\"configure discord bot credentials in loong.toml under discord or discord.accounts.<account>; outbound direct send is shipped, while gateway-based serve support remains planned\""
    ));
}

#[test]
fn render_channel_surfaces_text_groups_plugin_backed_channels_into_their_own_section() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    let plugin_section = rendered
        .split("plugin-backed channels:")
        .nth(1)
        .expect("plugin-backed channels section should exist");
    let plugin_section = plugin_section
        .split("catalog-only channels:")
        .next()
        .expect("plugin-backed section should precede catalog-only section");

    assert!(
        plugin_section.contains("Weixin [weixin]"),
        "plugin-backed section should include weixin: {plugin_section}"
    );
    assert!(
        plugin_section.contains("QQ Bot [qqbot]"),
        "plugin-backed section should include qqbot: {plugin_section}"
    );
    assert!(
        plugin_section.contains("OneBot [onebot]"),
        "plugin-backed section should include onebot: {plugin_section}"
    );
}

#[test]
fn render_channel_surfaces_text_reports_managed_plugin_bridge_discovery() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(
        rendered.contains("Weixin [weixin]"),
        "rendered channel surfaces should include the weixin surface: {rendered}"
    );
    assert!(
        rendered.contains(
            "managed_plugin_bridge_discovery status=not_configured managed_install_root=- scan_issue=- configured_plugin_id=- selected_plugin_id=- selection_status=- compatible=0 compatible_plugin_ids=- ambiguity_status=- incomplete=0 incompatible=0"
        ),
        "rendered channel surfaces should include managed discovery summaries: {rendered}"
    );
}

#[test]
fn render_channel_surfaces_text_reports_plugin_backed_stable_targets() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(
        rendered.contains(
            "stable_targets=\"weixin:<account>:contact:<id>[conversation]:direct contact conversation,weixin:<account>:room:<id>[conversation]:group room conversation\""
        ),
        "rendered channel surfaces should expose weixin stable target templates: {rendered}"
    );
    assert!(
        rendered.contains(
            "stable_targets=\"qqbot:<account>:c2c:<openid>[conversation]:direct message openid,qqbot:<account>:group:<openid>[conversation]:group openid,qqbot:<account>:channel:<id>[conversation]:guild channel id\""
        ),
        "rendered channel surfaces should expose qqbot stable target templates: {rendered}"
    );
    assert!(
        rendered
            .contains("account_scope_note=\"openids are scoped to the selected qq bot account\""),
        "rendered channel surfaces should expose qqbot account scope guidance: {rendered}"
    );
}

#[test]
fn render_channel_surfaces_text_reports_managed_plugin_bridge_ambiguity_and_setup_guidance() {
    let config = mvp::config::LoongConfig::default();
    let mut inventory = mvp::channel::channel_inventory(&config);
    let weixin_surface = inventory
        .channel_surfaces
        .iter_mut()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin_surface
        .plugin_bridge_discovery
        .as_mut()
        .expect("weixin managed discovery");

    discovery.status = mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound;
    discovery.selection_status =
        Some(mvp::channel::ChannelPluginBridgeSelectionStatus::NotConfigured);
    discovery.configured_plugin_id = None;
    discovery.selected_plugin_id = None;
    discovery.ambiguity_status =
        Some(mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins);
    discovery.compatible_plugins = 2;
    discovery.compatible_plugin_ids =
        vec!["weixin-bridge-a".to_owned(), "weixin-bridge-b".to_owned()];
    discovery.incomplete_plugins = 1;
    discovery.incompatible_plugins = 0;
    discovery.plugins = vec![mvp::channel::ChannelDiscoveredPluginBridge {
        plugin_id: "weixin-bridge-a".to_owned(),
        source_path: "/tmp/weixin-bridge-a/loong.plugin.json".to_owned(),
        package_root: "/tmp/weixin-bridge-a".to_owned(),
        package_manifest_path: Some("/tmp/weixin-bridge-a/loong.plugin.json".to_owned()),
        bridge_kind: "managed_connector".to_owned(),
        adapter_family: "channel-bridge".to_owned(),
        transport_family: Some("wechat_clawbot_ilink_bridge".to_owned()),
        target_contract: Some("weixin_reply_loop".to_owned()),
        account_scope: Some("shared".to_owned()),
        runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
        runtime_operations: vec![
            "send_message".to_owned(),
            "receive_batch".to_owned(),
        ],
        status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract,
        issues: vec!["example issue".to_owned()],
        missing_fields: vec!["metadata.transport_family".to_owned()],
        required_env_vars: vec!["WEIXIN_BRIDGE_URL".to_owned()],
        recommended_env_vars: vec!["WEIXIN_BRIDGE_ACCESS_TOKEN".to_owned()],
        required_config_keys: vec!["weixin.bridge_url".to_owned()],
        default_env_var: Some("WEIXIN_BRIDGE_URL".to_owned()),
        setup_docs_urls: vec!["https://example.test/docs/weixin-bridge".to_owned()],
        setup_remediation: Some(
            "Run the ClawBot setup flow before enabling this bridge.\nThen verify only one managed bridge remains.".to_owned(),
        ),
    }];

    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(
        rendered.contains("ambiguity_status=multiple_compatible_plugins"),
        "rendered channel surfaces should expose managed bridge ambiguity status: {rendered}"
    );
    assert!(
        rendered.contains("compatible_plugin_ids=weixin-bridge-a,weixin-bridge-b"),
        "rendered channel surfaces should expose managed bridge compatible plugin ids: {rendered}"
    );
    assert!(
        rendered.contains("required_env_vars=WEIXIN_BRIDGE_URL"),
        "rendered channel surfaces should expose managed bridge setup env requirements: {rendered}"
    );
    assert!(
        rendered.contains("setup_docs_urls=https://example.test/docs/weixin-bridge"),
        "rendered channel surfaces should expose managed bridge setup docs links: {rendered}"
    );
    assert!(
        rendered.contains(
            "setup_remediation=\"Run the ClawBot setup flow before enabling this bridge.\\nThen verify only one managed bridge remains.\""
        ),
        "rendered channel surfaces should expose managed bridge setup remediation text: {rendered}"
    );
}

#[test]
fn render_channel_surfaces_text_reports_plugin_bridge_account_summary_for_mixed_multi_account_surface()
 {
    let install_root = unique_temp_dir("text-render-managed-bridge-account-summary");
    let mut config = mixed_account_weixin_plugin_bridge_config();

    install_ready_weixin_managed_bridge(install_root.as_path());
    config.external_skills.install_root = Some(install_root.display().to_string());

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered = render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(
        rendered.contains("selected_plugin_id=weixin-managed-bridge"),
        "text rendering should keep the selected plugin identity visible: {rendered}"
    );
    assert!(
        rendered.contains("account_summary="),
        "text rendering should expose the bounded mixed-account summary line: {rendered}"
    );
    assert!(
        rendered.contains("configured_account=ops"),
        "text rendering should mention the ready default account in the mixed-account summary: {rendered}"
    );
    assert!(
        rendered.contains("(default): ready"),
        "text rendering should mark the default account as ready in the mixed-account summary: {rendered}"
    );
    assert!(
        rendered.contains("configured_account=backup"),
        "text rendering should mention blocked non-default accounts in the mixed-account summary: {rendered}"
    );
    assert!(
        rendered.contains("bridge_url is missing"),
        "text rendering should keep the blocking contract detail visible in the mixed-account summary: {rendered}"
    );
}

#[test]
fn render_channel_surfaces_text_escapes_untrusted_managed_bridge_values() {
    let config = mvp::config::LoongConfig::default();
    let mut inventory = mvp::channel::channel_inventory(&config);
    let weixin_surface = inventory
        .channel_surfaces
        .iter_mut()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin_surface
        .plugin_bridge_discovery
        .as_mut()
        .expect("weixin managed discovery");

    discovery.managed_install_root = Some("/tmp/managed bridge".to_owned());
    discovery.status = mvp::channel::ChannelPluginBridgeDiscoveryStatus::ScanFailed;
    discovery.scan_issue = Some("scan failed\nplease inspect".to_owned());
    discovery.compatible_plugin_ids = vec!["bridge\none".to_owned()];
    discovery.plugins = vec![mvp::channel::ChannelDiscoveredPluginBridge {
        plugin_id: "weixin bridge".to_owned(),
        source_path: "/tmp/plugin root/bridge\nplugin.json".to_owned(),
        package_root: "/tmp/plugin root".to_owned(),
        package_manifest_path: Some("/tmp/plugin root/manifest\tbridge.json".to_owned()),
        bridge_kind: "managed connector".to_owned(),
        adapter_family: "channel bridge".to_owned(),
        transport_family: Some("wechat clawbot".to_owned()),
        target_contract: Some("weixin\nreply".to_owned()),
        account_scope: Some("shared scope".to_owned()),
        runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
        runtime_operations: vec!["send_message".to_owned(), "receive_batch".to_owned()],
        status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract,
        issues: vec!["missing\nfield".to_owned()],
        missing_fields: vec!["metadata.transport family".to_owned()],
        required_env_vars: vec!["WEIXIN BRIDGE URL".to_owned()],
        recommended_env_vars: vec!["WEIXIN BRIDGE TOKEN".to_owned()],
        required_config_keys: vec!["weixin.bridge url".to_owned()],
        default_env_var: Some("WEIXIN DEFAULT ENV".to_owned()),
        setup_docs_urls: vec!["https://example.test/docs bridge".to_owned()],
        setup_remediation: Some("fix bridge\nthen retry".to_owned()),
    }];

    let rendered = loong_daemon::render_channel_surfaces_text("/tmp/loong.toml", &inventory);

    assert!(
        rendered.contains("managed_install_root=\"/tmp/managed bridge\""),
        "managed install root should be escaped when it contains spaces: {rendered}"
    );
    assert!(
        rendered.contains("scan_issue=\"scan failed\\nplease inspect\""),
        "scan issue should escape newlines: {rendered}"
    );
    assert!(
        rendered.contains("id=\"weixin bridge\""),
        "plugin id should be escaped when it contains spaces: {rendered}"
    );
    assert!(
        rendered.contains("target_contract=\"weixin\\nreply\""),
        "target contract should escape newlines: {rendered}"
    );
    assert!(
        rendered.contains("setup_docs_urls=\"https://example.test/docs bridge\""),
        "setup docs urls should be escaped when needed: {rendered}"
    );
    assert!(
        rendered.contains("setup_remediation=\"fix bridge\\nthen retry\""),
        "setup remediation should escape newlines: {rendered}"
    );
}

#[test]
fn build_channels_cli_json_payload_includes_operation_requirement_metadata() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("operations")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|operations| operations.first())
                        .and_then(|operation| operation.get("requirements"))
                        .and_then(serde_json::Value::as_array)
                        .map(|requirements| {
                            requirements
                                .iter()
                                .filter_map(|item| item.get("id"))
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["enabled", "bot_token"])
            })
    );

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("feishu")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("operations"))
                .and_then(serde_json::Value::as_array)
                .and_then(|operations| operations.get(1))
                .and_then(|operation| operation.get("requirements"))
                .and_then(serde_json::Value::as_array)
                .map(|requirements| {
                    requirements
                        .iter()
                        .filter_map(|item| item.get("id"))
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(vec![
                    "enabled",
                    "app_id",
                    "app_secret",
                    "mode",
                    "allowed_chat_ids",
                    "allowed_sender_ids",
                    "verification_token",
                    "encrypt_key",
                ])
    }));
}

#[test]
fn build_channels_cli_json_payload_includes_structured_channel_access_policy_summaries() {
    let mut config = mvp::config::LoongConfig::default();
    config.matrix.enabled = true;
    config.matrix.access_token = Some(loong_contracts::SecretRef::Inline(
        "matrix-token".to_owned(),
    ));
    config.matrix.base_url = Some("https://matrix.example.org".to_owned());
    config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
    config.matrix.allowed_sender_ids = vec!["@alice:example.org".to_owned()];

    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let access_policies = encoded["channel_access_policies"]
        .as_array()
        .expect("channel access policies array");

    assert!(access_policies.iter().any(|policy| {
        policy.get("channel_id").and_then(serde_json::Value::as_str) == Some("matrix")
            && policy
                .get("conversation_config_key")
                .and_then(serde_json::Value::as_str)
                == Some("allowed_room_ids")
            && policy
                .get("sender_config_key")
                .and_then(serde_json::Value::as_str)
                == Some("allowed_sender_ids")
            && policy
                .get("conversation_mode")
                .and_then(serde_json::Value::as_str)
                == Some("exact_allowlist")
            && policy
                .get("sender_mode")
                .and_then(serde_json::Value::as_str)
                == Some("exact_allowlist")
    }));
}

#[test]
fn build_channels_cli_json_payload_includes_onboarding_metadata() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("strategy"))
                        .and_then(serde_json::Value::as_str)
                        == Some("manual_config")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("status_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("repair_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor --fix")
            })
    );

    assert!(
        encoded["channel_surfaces"]
            .as_array()
            .expect("channel surfaces array")
            .iter()
            .any(|surface| {
                surface
                    .get("catalog")
                    .and_then(|catalog| catalog.get("id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("discord")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("strategy"))
                        .and_then(serde_json::Value::as_str)
                        == Some("manual_config")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("status_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("repair_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor --fix")
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_plugin_bridge_contracts() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("weixin")
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("manifest_channel_id"))
                        .and_then(serde_json::Value::as_str)
                        == Some("weixin")
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("required_setup_surface"))
                        .and_then(serde_json::Value::as_str)
                        == Some("channel")
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("runtime_owner"))
                        .and_then(serde_json::Value::as_str)
                        == Some("external_plugin")
            })
    );

    assert!(
        encoded["channel_surfaces"]
            .as_array()
            .expect("channel surfaces array")
            .iter()
            .any(|surface| {
                surface
                    .get("catalog")
                    .and_then(|catalog| catalog.get("id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("qqbot")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("plugin_bridge_contract"))
                        .and_then(|contract| contract.get("supported_operations"))
                        .and_then(serde_json::Value::as_array)
                        .map(|operations| {
                            operations
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["send", "serve"])
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_plugin_bridge_stable_targets() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("weixin")
                    && entry
                        .get("plugin_bridge_contract")
                        .and_then(|contract| contract.get("stable_targets"))
                        .and_then(serde_json::Value::as_array)
                        .map(|targets| {
                            targets
                                .iter()
                                .map(|target| {
                                    let template =
                                        target.get("template").and_then(serde_json::Value::as_str);
                                    let target_kind = target
                                        .get("target_kind")
                                        .and_then(serde_json::Value::as_str);
                                    let description = target
                                        .get("description")
                                        .and_then(serde_json::Value::as_str);
                                    (template, target_kind, description)
                                })
                                .collect::<Vec<_>>()
                        })
                        == Some(vec![
                            (
                                Some("weixin:<account>:contact:<id>"),
                                Some("conversation"),
                                Some("direct contact conversation"),
                            ),
                            (
                                Some("weixin:<account>:room:<id>"),
                                Some("conversation"),
                                Some("group room conversation"),
                            ),
                        ])
            })
    );

    assert!(
        encoded["channel_surfaces"]
            .as_array()
            .expect("channel surfaces array")
            .iter()
            .any(|surface| {
                surface
                    .get("catalog")
                    .and_then(|catalog| catalog.get("id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("qqbot")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("plugin_bridge_contract"))
                        .and_then(|contract| contract.get("account_scope_note"))
                        .and_then(serde_json::Value::as_str)
                        == Some("openids are scoped to the selected qq bot account")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("plugin_bridge_contract"))
                        .and_then(|contract| contract.get("stable_targets"))
                        .and_then(serde_json::Value::as_array)
                        .map(|targets| targets.len())
                        == Some(3)
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_managed_plugin_bridge_discovery() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert!(
        encoded["channel_surfaces"]
            .as_array()
            .expect("channel surfaces array")
            .iter()
            .any(|surface| {
                surface
                    .get("catalog")
                    .and_then(|catalog| catalog.get("id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("weixin")
                    && surface
                        .get("plugin_bridge_discovery")
                        .and_then(|discovery| discovery.get("status"))
                        .and_then(serde_json::Value::as_str)
                        == Some("not_configured")
                    && surface
                        .get("plugin_bridge_discovery")
                        .and_then(|discovery| discovery.get("compatible_plugins"))
                        .and_then(serde_json::Value::as_u64)
                        == Some(0)
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_managed_plugin_bridge_guidance_fields() {
    let config = mvp::config::LoongConfig::default();
    let mut inventory = mvp::channel::channel_inventory(&config);
    let weixin_surface = inventory
        .channel_surfaces
        .iter_mut()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin_surface
        .plugin_bridge_discovery
        .as_mut()
        .expect("weixin managed discovery");

    discovery.status = mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound;
    discovery.selection_status =
        Some(mvp::channel::ChannelPluginBridgeSelectionStatus::NotConfigured);
    discovery.configured_plugin_id = None;
    discovery.selected_plugin_id = None;
    discovery.ambiguity_status =
        Some(mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins);
    discovery.compatible_plugins = 2;
    discovery.compatible_plugin_ids =
        vec!["weixin-bridge-a".to_owned(), "weixin-bridge-b".to_owned()];
    discovery.plugins = vec![mvp::channel::ChannelDiscoveredPluginBridge {
        plugin_id: "weixin-bridge-a".to_owned(),
        source_path: "/tmp/weixin-bridge-a/loong.plugin.json".to_owned(),
        package_root: "/tmp/weixin-bridge-a".to_owned(),
        package_manifest_path: Some("/tmp/weixin-bridge-a/loong.plugin.json".to_owned()),
        bridge_kind: "managed_connector".to_owned(),
        adapter_family: "channel-bridge".to_owned(),
        transport_family: Some("wechat_clawbot_ilink_bridge".to_owned()),
        target_contract: Some("weixin_reply_loop".to_owned()),
        account_scope: Some("shared".to_owned()),
        runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
        runtime_operations: vec!["send_message".to_owned(), "receive_batch".to_owned()],
        status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleReady,
        issues: Vec::new(),
        missing_fields: Vec::new(),
        required_env_vars: vec!["WEIXIN_BRIDGE_URL".to_owned()],
        recommended_env_vars: vec!["WEIXIN_BRIDGE_ACCESS_TOKEN".to_owned()],
        required_config_keys: vec!["weixin.bridge_url".to_owned()],
        default_env_var: Some("WEIXIN_BRIDGE_URL".to_owned()),
        setup_docs_urls: vec!["https://example.test/docs/weixin-bridge".to_owned()],
        setup_remediation: Some(
            "Run the ClawBot setup flow before enabling this bridge.".to_owned(),
        ),
    }];

    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");
    let weixin = surfaces
        .iter()
        .find(|surface| {
            surface
                .get("catalog")
                .and_then(|catalog| catalog.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some("weixin")
        })
        .expect("weixin surface entry");

    assert_eq!(
        weixin["plugin_bridge_discovery"]["ambiguity_status"]
            .as_str()
            .expect("ambiguity_status should be string"),
        "multiple_compatible_plugins"
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["compatible_plugin_ids"]
            .as_array()
            .expect("compatible_plugin_ids should be array")
            .len(),
        2
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["plugins"][0]["setup_docs_urls"][0]
            .as_str()
            .expect("setup docs url should be string"),
        "https://example.test/docs/weixin-bridge"
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["plugins"][0]["setup_remediation"]
            .as_str()
            .expect("setup remediation should be string"),
        "Run the ClawBot setup flow before enabling this bridge."
    );
}

#[test]
fn build_channels_cli_json_payload_includes_duplicate_managed_bridge_selection_fields() {
    let config = mvp::config::LoongConfig::default();
    let mut inventory = mvp::channel::channel_inventory(&config);
    let weixin_surface = inventory
        .channel_surfaces
        .iter_mut()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin_surface
        .plugin_bridge_discovery
        .as_mut()
        .expect("weixin managed discovery");

    discovery.status = mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound;
    discovery.configured_plugin_id = Some("weixin-bridge-shared".to_owned());
    discovery.selected_plugin_id = None;
    discovery.selection_status =
        Some(mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginIdDuplicated);
    discovery.ambiguity_status = Some(
        mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::DuplicateCompatiblePluginIds,
    );
    discovery.compatible_plugins = 2;
    discovery.compatible_plugin_ids = vec![
        "weixin-bridge-shared".to_owned(),
        "weixin-bridge-shared".to_owned(),
    ];

    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");
    let weixin = surfaces
        .iter()
        .find(|surface| {
            surface
                .get("catalog")
                .and_then(|catalog| catalog.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some("weixin")
        })
        .expect("weixin surface entry");

    assert_eq!(
        weixin["plugin_bridge_discovery"]["configured_plugin_id"]
            .as_str()
            .expect("configured_plugin_id should be string"),
        "weixin-bridge-shared"
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["selection_status"]
            .as_str()
            .expect("selection_status should be string"),
        "configured_plugin_id_duplicated"
    );
    assert_eq!(
        weixin["plugin_bridge_discovery"]["ambiguity_status"]
            .as_str()
            .expect("ambiguity_status should be string"),
        "duplicate_compatible_plugin_ids"
    );
}

#[test]
fn build_channels_cli_json_payload_includes_plugin_bridge_account_summary_for_mixed_multi_account_surface()
 {
    let install_root = unique_temp_dir("channels-json-managed-bridge-account-summary");
    let mut config = mixed_account_weixin_plugin_bridge_config();

    install_ready_weixin_managed_bridge(install_root.as_path());
    config.external_skills.install_root = Some(install_root.display().to_string());

    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");
    let weixin = surfaces
        .iter()
        .find(|surface| {
            surface
                .get("catalog")
                .and_then(|catalog| catalog.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some("weixin")
        })
        .expect("weixin surface entry");
    let account_summary = weixin["plugin_bridge_account_summary"]
        .as_str()
        .expect("plugin bridge account summary should be string");

    assert_eq!(
        weixin["plugin_bridge_discovery"]["selected_plugin_id"]
            .as_str()
            .expect("selected_plugin_id should be string"),
        "weixin-managed-bridge"
    );
    assert!(
        account_summary.contains("configured_account=ops"),
        "channels json should mention the ready default account in the bounded summary: {weixin:#?}"
    );
    assert!(
        account_summary.contains("(default): ready"),
        "channels json should mark the default account as ready in the bounded summary: {weixin:#?}"
    );
    assert!(
        account_summary.contains("configured_account=backup"),
        "channels json should mention blocked non-default accounts in the bounded summary: {weixin:#?}"
    );
    assert!(
        account_summary.contains("bridge_url is missing"),
        "channels json should keep the blocking contract detail visible in the bounded summary: {weixin:#?}"
    );
    assert_eq!(account_summary, MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY);
}

#[test]
fn build_channels_cli_json_payload_includes_full_channel_catalog() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert_eq!(
        encoded.get("config").and_then(serde_json::Value::as_str),
        Some("/tmp/loong.toml")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("version"))
            .and_then(serde_json::Value::as_u64),
        Some(u64::from(CHANNELS_CLI_JSON_SCHEMA_VERSION))
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("primary_channel_view"))
            .and_then(serde_json::Value::as_str),
        Some("channel_surfaces")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("catalog_view"))
            .and_then(serde_json::Value::as_str),
        Some("channel_catalog")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("legacy_channel_views"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["channels", "catalog_only_channels"])
    );
    assert_eq!(
        encoded
            .get("summary")
            .and_then(|summary| summary.get("total_surface_count"))
            .and_then(serde_json::Value::as_u64),
        Some(inventory.channel_surfaces.len() as u64)
    );
    assert_eq!(
        encoded
            .get("summary")
            .and_then(|summary| summary.get("plugin_backed_surface_count"))
            .and_then(serde_json::Value::as_u64),
        Some(
            inventory
                .channel_surfaces
                .iter()
                .filter(|surface| {
                    surface.catalog.implementation_status
                        == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
                })
                .count() as u64
        )
    );
    assert_eq!(
        encoded
            .get("channel_catalog")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.channel_catalog.len())
    );
    assert_eq!(
        encoded
            .get("catalog_only_channels")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.catalog_only_channels.len())
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("matrix")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("wecom")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("discord")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
                    && entry
                        .get("operations")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.get("availability"))
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["implemented", "stub"])
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(40)
                    && entry
                        .get("selection_label")
                        .and_then(serde_json::Value::as_str)
                        == Some("community server bot")
                    && entry
                        .get("blurb")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|value| value.contains("config-backed direct sends"))
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("slack")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(50)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("whatsapp")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(90)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("webhook")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(110)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("google-chat")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(120)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("teams")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint", "conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(140)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("nextcloud-talk")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(160)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("imessage")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(180)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("synology-chat")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(165)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("signal")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(130)
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_grouped_channel_surfaces() {
    let _env = super::MigrationEnvironmentGuard::set(&[("TELEGRAM_BOT_TOKEN", None)]);

    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert_eq!(
        encoded
            .get("channel_surfaces")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.channel_surfaces.len())
    );

    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("telegram")
            && surface
                .get("default_configured_account_id")
                .and_then(serde_json::Value::as_str)
                == Some("default")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("telegram"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("telegram"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("slack")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("slack"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("slack"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(50)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("whatsapp")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("whatsapp"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("whatsapp"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(90)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("signal")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("signal"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("signal"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(130)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("wecom")
            && surface
                .get("default_configured_account_id")
                .and_then(serde_json::Value::as_str)
                == Some("default")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("wecom"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("wecom"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("matrix")
            && surface
                .get("default_configured_account_id")
                .and_then(serde_json::Value::as_str)
                == Some("default")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("matrix"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("matrix"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("discord")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("discord"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("discord"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(40)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("webhook")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("webhook"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("webchat")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(230)
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("webchat"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(0)
    }));
}
