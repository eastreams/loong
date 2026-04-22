use super::*;

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
