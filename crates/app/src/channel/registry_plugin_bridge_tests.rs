use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use tempfile::TempDir;

fn sample_channel_bridge_manifest(
    channel_id: Option<&str>,
    setup_surface: Option<&str>,
) -> loongclaw_kernel::PluginManifest {
    sample_channel_bridge_manifest_with_metadata(channel_id, setup_surface, BTreeMap::new())
}

fn sample_channel_bridge_manifest_with_metadata(
    channel_id: Option<&str>,
    setup_surface: Option<&str>,
    metadata: BTreeMap<String, String>,
) -> loongclaw_kernel::PluginManifest {
    let normalized_channel_id = channel_id.map(str::to_owned);
    let normalized_setup_surface = setup_surface.map(str::to_owned);

    let setup = normalized_setup_surface.map(|surface| loongclaw_kernel::PluginSetup {
        mode: loongclaw_kernel::PluginSetupMode::MetadataOnly,
        surface: Some(surface),
        required_env_vars: Vec::new(),
        recommended_env_vars: Vec::new(),
        required_config_keys: Vec::new(),
        default_env_var: None,
        docs_urls: Vec::new(),
        remediation: None,
    });

    loongclaw_kernel::PluginManifest {
        api_version: Some("v1alpha1".to_owned()),
        version: Some("1.0.0".to_owned()),
        plugin_id: "sample-bridge".to_owned(),
        provider_id: "sample-provider".to_owned(),
        connector_name: "sample-connector".to_owned(),
        channel_id: normalized_channel_id,
        endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
        capabilities: BTreeSet::new(),
        trust_tier: loongclaw_kernel::PluginTrustTier::Unverified,
        metadata,
        summary: None,
        tags: Vec::new(),
        input_examples: Vec::new(),
        output_examples: Vec::new(),
        defer_loading: false,
        setup,
        slot_claims: Vec::new(),
        compatibility: None,
    }
}

fn sample_channel_bridge_manifest_with_setup(
    channel_id: Option<&str>,
    metadata: BTreeMap<String, String>,
    setup: Option<loongclaw_kernel::PluginSetup>,
) -> loongclaw_kernel::PluginManifest {
    let normalized_channel_id = channel_id.map(str::to_owned);

    loongclaw_kernel::PluginManifest {
        api_version: Some("v1alpha1".to_owned()),
        version: Some("1.0.0".to_owned()),
        plugin_id: "sample-bridge".to_owned(),
        provider_id: "sample-provider".to_owned(),
        connector_name: "sample-connector".to_owned(),
        channel_id: normalized_channel_id,
        endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
        capabilities: BTreeSet::new(),
        trust_tier: loongclaw_kernel::PluginTrustTier::Unverified,
        metadata,
        summary: None,
        tags: Vec::new(),
        input_examples: Vec::new(),
        output_examples: Vec::new(),
        defer_loading: false,
        setup,
        slot_claims: Vec::new(),
        compatibility: None,
    }
}

fn sample_channel_bridge_manifest_with_plugin_id(
    plugin_id: &str,
    channel_id: Option<&str>,
    metadata: BTreeMap<String, String>,
    setup: Option<loongclaw_kernel::PluginSetup>,
) -> loongclaw_kernel::PluginManifest {
    let mut manifest = sample_channel_bridge_manifest_with_setup(channel_id, metadata, setup);

    manifest.plugin_id = plugin_id.to_owned();

    manifest
}

fn write_plugin_package_manifest(
    root: &Path,
    directory_name: &str,
    manifest: &loongclaw_kernel::PluginManifest,
) {
    let plugin_directory = root.join(directory_name);
    let manifest_path = plugin_directory.join("loongclaw.plugin.json");
    let encoded_manifest =
        serde_json::to_string_pretty(manifest).expect("serialize plugin package manifest");

    fs::create_dir_all(&plugin_directory).expect("create plugin package directory");
    fs::write(&manifest_path, encoded_manifest).expect("write plugin package manifest");
}

fn compatible_bridge_metadata(
    transport_family: &str,
    target_contract: &str,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();

    metadata.insert("adapter_family".to_owned(), "channel-bridge".to_owned());
    metadata.insert("transport_family".to_owned(), transport_family.to_owned());
    metadata.insert("target_contract".to_owned(), target_contract.to_owned());

    metadata
}

fn bridge_setup_with_docs_and_remediation(
    surface: &str,
    docs_urls: Vec<&str>,
    remediation: Option<&str>,
) -> loongclaw_kernel::PluginSetup {
    let normalized_docs_urls = docs_urls.into_iter().map(str::to_owned).collect();
    let normalized_remediation = remediation.map(str::to_owned);

    loongclaw_kernel::PluginSetup {
        mode: loongclaw_kernel::PluginSetupMode::MetadataOnly,
        surface: Some(surface.to_owned()),
        required_env_vars: Vec::new(),
        recommended_env_vars: Vec::new(),
        required_config_keys: Vec::new(),
        default_env_var: None,
        docs_urls: normalized_docs_urls,
        remediation: normalized_remediation,
    }
}

#[test]
fn resolve_channel_catalog_entry_exposes_plugin_bridge_contracts() {
    let telegram = resolve_channel_catalog_entry("telegram").expect("telegram entry");
    let weixin = resolve_channel_catalog_entry("weixin").expect("weixin entry");
    let qqbot = resolve_channel_catalog_entry("qqbot").expect("qqbot entry");
    let onebot = resolve_channel_catalog_entry("onebot").expect("onebot entry");

    assert_eq!(telegram.plugin_bridge_contract, None);

    let weixin_contract = weixin
        .plugin_bridge_contract
        .as_ref()
        .expect("weixin plugin bridge contract");
    assert_eq!(weixin_contract.manifest_channel_id, "weixin");
    assert_eq!(weixin_contract.required_setup_surface, "channel");
    assert_eq!(weixin_contract.runtime_owner, "external_plugin");
    assert_eq!(weixin_contract.supported_operations, vec!["send", "serve"]);
    assert_eq!(
        weixin_contract.recommended_metadata_keys,
        vec![
            "bridge_kind",
            "adapter_family",
            "entrypoint",
            "transport_family",
            "target_contract",
            "account_scope",
        ]
    );

    let qqbot_contract = qqbot
        .plugin_bridge_contract
        .as_ref()
        .expect("qqbot plugin bridge contract");
    assert_eq!(qqbot_contract.manifest_channel_id, "qqbot");

    let onebot_contract = onebot
        .plugin_bridge_contract
        .as_ref()
        .expect("onebot plugin bridge contract");
    assert_eq!(onebot_contract.manifest_channel_id, "onebot");
}

#[test]
fn resolve_channel_catalog_entry_exposes_plugin_bridge_stable_targets() {
    let weixin = resolve_channel_catalog_entry("wechat").expect("weixin entry");
    let qqbot = resolve_channel_catalog_entry("qq").expect("qqbot entry");
    let onebot = resolve_channel_catalog_entry("onebot-v11").expect("onebot entry");

    let weixin_contract = weixin
        .plugin_bridge_contract
        .as_ref()
        .expect("weixin plugin bridge contract");
    let qqbot_contract = qqbot
        .plugin_bridge_contract
        .as_ref()
        .expect("qqbot plugin bridge contract");
    let onebot_contract = onebot
        .plugin_bridge_contract
        .as_ref()
        .expect("onebot plugin bridge contract");

    assert_eq!(
        weixin_contract
            .stable_targets
            .iter()
            .map(|target| { (target.template, target.target_kind, target.description,) })
            .collect::<Vec<_>>(),
        vec![
            (
                "weixin:<account>:contact:<id>",
                ChannelCatalogTargetKind::Conversation,
                "direct contact conversation",
            ),
            (
                "weixin:<account>:room:<id>",
                ChannelCatalogTargetKind::Conversation,
                "group room conversation",
            ),
        ]
    );
    assert_eq!(weixin_contract.account_scope_note, None);

    assert_eq!(
        qqbot_contract
            .stable_targets
            .iter()
            .map(|target| { (target.template, target.target_kind, target.description,) })
            .collect::<Vec<_>>(),
        vec![
            (
                "qqbot:<account>:c2c:<openid>",
                ChannelCatalogTargetKind::Conversation,
                "direct message openid",
            ),
            (
                "qqbot:<account>:group:<openid>",
                ChannelCatalogTargetKind::Conversation,
                "group openid",
            ),
            (
                "qqbot:<account>:channel:<id>",
                ChannelCatalogTargetKind::Conversation,
                "guild channel id",
            ),
        ]
    );
    assert_eq!(
        qqbot_contract.account_scope_note,
        Some("openids are scoped to the selected qq bot account")
    );

    assert_eq!(
        onebot_contract
            .stable_targets
            .iter()
            .map(|target| { (target.template, target.target_kind, target.description,) })
            .collect::<Vec<_>>(),
        vec![
            (
                "onebot:<account>:private:<user_id>",
                ChannelCatalogTargetKind::Conversation,
                "private conversation user id",
            ),
            (
                "onebot:<account>:group:<group_id>",
                ChannelCatalogTargetKind::Conversation,
                "group conversation id",
            ),
        ]
    );
    assert_eq!(
        onebot_contract.account_scope_note,
        Some("keep <account> stable so personal-account bridge routes stay unambiguous")
    );
}

#[test]
fn validate_plugin_channel_bridge_manifest_reports_contract_mismatches() {
    let compatible_manifest = sample_channel_bridge_manifest(Some("weixin"), Some("channel"));
    let compatible_validation = validate_plugin_channel_bridge_manifest(&compatible_manifest)
        .expect("compatible channel bridge validation");
    assert_eq!(
        compatible_validation.status,
        ChannelPluginBridgeManifestStatus::Compatible
    );
    assert_eq!(compatible_validation.channel_id, "weixin");
    assert!(compatible_validation.issues.is_empty());
    assert_eq!(
        compatible_validation.recommended_metadata_keys,
        vec![
            "bridge_kind",
            "adapter_family",
            "entrypoint",
            "transport_family",
            "target_contract",
            "account_scope",
        ]
    );

    let unknown_manifest = sample_channel_bridge_manifest(Some("unknown-bridge"), Some("channel"));
    let unknown_validation = validate_plugin_channel_bridge_manifest(&unknown_manifest)
        .expect("unknown channel bridge validation");
    assert_eq!(
        unknown_validation.status,
        ChannelPluginBridgeManifestStatus::UnknownChannel
    );

    let runtime_backed_manifest = sample_channel_bridge_manifest(Some("telegram"), Some("channel"));
    let runtime_backed_validation =
        validate_plugin_channel_bridge_manifest(&runtime_backed_manifest)
            .expect("runtime-backed channel validation");
    assert_eq!(
        runtime_backed_validation.status,
        ChannelPluginBridgeManifestStatus::UnsupportedChannelSurface
    );

    let missing_surface_manifest = sample_channel_bridge_manifest(Some("qqbot"), None);
    let missing_surface_validation =
        validate_plugin_channel_bridge_manifest(&missing_surface_manifest)
            .expect("missing setup surface validation");
    assert_eq!(
        missing_surface_validation.status,
        ChannelPluginBridgeManifestStatus::MissingSetupSurface
    );
}

#[test]
fn channel_inventory_marks_managed_bridge_discovery_unavailable_when_install_root_is_unset() {
    let config = LoongClawConfig::default();
    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");

    assert_eq!(
        discovery.status,
        ChannelPluginBridgeDiscoveryStatus::NotConfigured
    );
    assert!(discovery.managed_install_root.is_none());
    assert!(discovery.scan_issue.is_none());
    assert!(discovery.plugins.is_empty());
}

#[test]
fn channel_inventory_reports_managed_bridge_plugin_statuses_per_surface() {
    let install_root = TempDir::new().expect("create managed install root");
    let compatible_manifest = sample_channel_bridge_manifest_with_metadata(
        Some("weixin"),
        Some("channel"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut incomplete_metadata = compatible_bridge_metadata(
        "qq_official_bot_gateway_or_plugin_bridge",
        "qqbot_reply_loop",
    );
    let removed_transport_family = incomplete_metadata.remove("transport_family");
    let incomplete_manifest = sample_channel_bridge_manifest_with_metadata(
        Some("qqbot"),
        Some("channel"),
        incomplete_metadata,
    );
    let incompatible_manifest = sample_channel_bridge_manifest(Some("onebot"), Some("tool"));
    let mut config = LoongClawConfig::default();

    assert_eq!(
        removed_transport_family.as_deref(),
        Some("qq_official_bot_gateway_or_plugin_bridge")
    );

    write_plugin_package_manifest(
        install_root.path(),
        "weixin-compatible",
        &compatible_manifest,
    );
    write_plugin_package_manifest(
        install_root.path(),
        "qqbot-incomplete",
        &incomplete_manifest,
    );
    write_plugin_package_manifest(
        install_root.path(),
        "onebot-incompatible",
        &incompatible_manifest,
    );

    config.external_skills.install_root = Some(install_root.path().display().to_string());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let qqbot = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "qqbot")
        .expect("qqbot surface");
    let onebot = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "onebot")
        .expect("onebot surface");
    let weixin_discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");
    let qqbot_discovery = qqbot
        .plugin_bridge_discovery
        .as_ref()
        .expect("qqbot managed discovery");
    let onebot_discovery = onebot
        .plugin_bridge_discovery
        .as_ref()
        .expect("onebot managed discovery");

    assert_eq!(
        weixin_discovery.status,
        ChannelPluginBridgeDiscoveryStatus::MatchesFound
    );
    assert_eq!(weixin_discovery.compatible_plugins, 1);
    assert_eq!(weixin_discovery.incomplete_plugins, 0);
    assert_eq!(weixin_discovery.incompatible_plugins, 0);
    assert_eq!(weixin_discovery.plugins.len(), 1);
    assert_eq!(
        weixin_discovery.plugins[0].status,
        ChannelDiscoveredPluginBridgeStatus::CompatibleReady
    );

    assert_eq!(
        qqbot_discovery.status,
        ChannelPluginBridgeDiscoveryStatus::MatchesFound
    );
    assert_eq!(qqbot_discovery.compatible_plugins, 0);
    assert_eq!(qqbot_discovery.incomplete_plugins, 1);
    assert_eq!(qqbot_discovery.incompatible_plugins, 0);
    assert_eq!(qqbot_discovery.plugins.len(), 1);
    assert_eq!(
        qqbot_discovery.plugins[0].status,
        ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract
    );
    assert_eq!(
        qqbot_discovery.plugins[0].missing_fields,
        vec!["metadata.transport_family".to_owned()]
    );

    assert_eq!(
        onebot_discovery.status,
        ChannelPluginBridgeDiscoveryStatus::MatchesFound
    );
    assert_eq!(onebot_discovery.compatible_plugins, 0);
    assert_eq!(onebot_discovery.incomplete_plugins, 0);
    assert_eq!(onebot_discovery.incompatible_plugins, 1);
    assert_eq!(onebot_discovery.plugins.len(), 1);
    assert_eq!(
        onebot_discovery.plugins[0].status,
        ChannelDiscoveredPluginBridgeStatus::UnsupportedChannelSurface
    );
}

#[test]
fn channel_inventory_reports_managed_bridge_scan_failures() {
    let missing_install_root = std::env::temp_dir().join(format!(
        "loongclaw-missing-managed-bridge-root-{}",
        std::process::id()
    ));
    let expected_install_root = missing_install_root.display().to_string();
    let mut config = LoongClawConfig::default();

    let _ = fs::remove_dir_all(&missing_install_root);

    config.external_skills.install_root = Some(expected_install_root.clone());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");

    assert_eq!(
        discovery.status,
        ChannelPluginBridgeDiscoveryStatus::ScanFailed
    );
    assert_eq!(
        discovery.managed_install_root.as_deref(),
        Some(expected_install_root.as_str())
    );
    assert!(discovery.scan_issue.is_some());
}

#[test]
fn channel_inventory_reports_managed_bridge_ambiguity_and_setup_guidance() {
    let install_root = TempDir::new().expect("create managed install root");
    let compatible_setup = bridge_setup_with_docs_and_remediation(
        "channel",
        vec!["https://example.test/docs/weixin-bridge"],
        Some("Run the ClawBot setup flow before enabling this bridge."),
    );
    let first_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-a",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(compatible_setup.clone()),
    );
    let second_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-b",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(compatible_setup),
    );
    let mut config = LoongClawConfig::default();

    write_plugin_package_manifest(install_root.path(), "weixin-bridge-a", &first_manifest);
    write_plugin_package_manifest(install_root.path(), "weixin-bridge-b", &second_manifest);

    config.external_skills.install_root = Some(install_root.path().display().to_string());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");
    let first_plugin = discovery.plugins.first().expect("first discovered plugin");

    assert_eq!(
        discovery.ambiguity_status,
        Some(ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins)
    );
    assert_eq!(discovery.compatible_plugin_ids.len(), 2);
    assert_eq!(
        first_plugin.setup_docs_urls,
        vec!["https://example.test/docs/weixin-bridge".to_owned()]
    );
    assert_eq!(
        first_plugin.setup_remediation.as_deref(),
        Some("Run the ClawBot setup flow before enabling this bridge.")
    );
}

#[test]
fn channel_inventory_reports_duplicate_compatible_plugin_ids_as_distinct_ambiguity() {
    let install_root = TempDir::new().expect("create managed install root");
    let first_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-shared",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let second_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-shared",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let mut config = LoongClawConfig::default();

    write_plugin_package_manifest(install_root.path(), "weixin-bridge-a", &first_manifest);
    write_plugin_package_manifest(install_root.path(), "weixin-bridge-b", &second_manifest);

    config.external_skills.install_root = Some(install_root.path().display().to_string());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");

    assert_eq!(
        discovery.ambiguity_status,
        Some(ChannelPluginBridgeDiscoveryAmbiguityStatus::DuplicateCompatiblePluginIds)
    );
    assert_eq!(
        discovery.selection_status,
        Some(ChannelPluginBridgeSelectionStatus::NotConfigured)
    );
    assert_eq!(
        discovery.compatible_plugin_ids,
        vec![
            "weixin-bridge-shared".to_owned(),
            "weixin-bridge-shared".to_owned()
        ]
    );
}

#[test]
fn channel_inventory_reports_duplicate_configured_plugin_id_selection_failure() {
    let install_root = TempDir::new().expect("create managed install root");
    let first_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-shared",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let second_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-shared",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let mut config = LoongClawConfig::default();

    write_plugin_package_manifest(install_root.path(), "weixin-bridge-a", &first_manifest);
    write_plugin_package_manifest(install_root.path(), "weixin-bridge-b", &second_manifest);

    config.external_skills.install_root = Some(install_root.path().display().to_string());
    config.weixin.managed_bridge_plugin_id = Some("weixin-bridge-shared".to_owned());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");

    assert_eq!(discovery.ambiguity_status, None);
    assert_eq!(
        discovery.selection_status,
        Some(ChannelPluginBridgeSelectionStatus::ConfiguredPluginIdDuplicated)
    );
    assert_eq!(
        discovery.configured_plugin_id.as_deref(),
        Some("weixin-bridge-shared")
    );
    assert_eq!(discovery.selected_plugin_id, None);
}

#[test]
fn channel_inventory_resolves_managed_bridge_selection_from_configured_plugin_id() {
    let install_root = TempDir::new().expect("create managed install root");
    let first_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-a",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let second_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-b",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let mut config = LoongClawConfig::default();

    write_plugin_package_manifest(install_root.path(), "weixin-bridge-a", &first_manifest);
    write_plugin_package_manifest(install_root.path(), "weixin-bridge-b", &second_manifest);

    config.external_skills.install_root = Some(install_root.path().display().to_string());
    config.weixin.managed_bridge_plugin_id = Some("weixin-bridge-b".to_owned());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");

    assert_eq!(discovery.ambiguity_status, None);
    assert_eq!(
        discovery.selection_status,
        Some(ChannelPluginBridgeSelectionStatus::SelectedCompatible)
    );
    assert_eq!(
        discovery.configured_plugin_id.as_deref(),
        Some("weixin-bridge-b")
    );
    assert_eq!(
        discovery.selected_plugin_id.as_deref(),
        Some("weixin-bridge-b")
    );
}

#[test]
fn channel_inventory_reports_missing_configured_managed_bridge_plugin_id() {
    let install_root = TempDir::new().expect("create managed install root");
    let first_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-a",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let second_manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-b",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let mut config = LoongClawConfig::default();

    write_plugin_package_manifest(install_root.path(), "weixin-bridge-a", &first_manifest);
    write_plugin_package_manifest(install_root.path(), "weixin-bridge-b", &second_manifest);

    config.external_skills.install_root = Some(install_root.path().display().to_string());
    config.weixin.managed_bridge_plugin_id = Some("missing-plugin".to_owned());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");

    assert_eq!(discovery.ambiguity_status, None);
    assert_eq!(
        discovery.selection_status,
        Some(ChannelPluginBridgeSelectionStatus::ConfiguredPluginNotFound)
    );
    assert_eq!(
        discovery.configured_plugin_id.as_deref(),
        Some("missing-plugin")
    );
    assert_eq!(discovery.selected_plugin_id, None);
}

#[test]
fn channel_inventory_marks_single_compatible_plugin_without_explicit_selection() {
    let install_root = TempDir::new().expect("create managed install root");
    let manifest = sample_channel_bridge_manifest_with_plugin_id(
        "weixin-bridge-only",
        Some("weixin"),
        compatible_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        Some(bridge_setup_with_docs_and_remediation(
            "channel",
            Vec::new(),
            None,
        )),
    );
    let mut config = LoongClawConfig::default();

    write_plugin_package_manifest(install_root.path(), "weixin-bridge-only", &manifest);

    config.external_skills.install_root = Some(install_root.path().display().to_string());

    let inventory = channel_inventory(&config);
    let weixin = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let discovery = weixin
        .plugin_bridge_discovery
        .as_ref()
        .expect("weixin managed discovery");

    assert_eq!(discovery.ambiguity_status, None);
    assert_eq!(
        discovery.selection_status,
        Some(ChannelPluginBridgeSelectionStatus::SingleCompatibleMatch)
    );
    assert_eq!(discovery.configured_plugin_id, None);
    assert_eq!(
        discovery.selected_plugin_id.as_deref(),
        Some("weixin-bridge-only")
    );
}
