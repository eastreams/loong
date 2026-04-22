use super::*;

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
