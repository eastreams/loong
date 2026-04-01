use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn sample_channel_bridge_manifest(
    channel_id: Option<&str>,
    setup_surface: Option<&str>,
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
        plugin_id: "sample-bridge".to_owned(),
        provider_id: "sample-provider".to_owned(),
        connector_name: "sample-connector".to_owned(),
        channel_id: normalized_channel_id,
        endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
        capabilities: BTreeSet::new(),
        metadata: BTreeMap::new(),
        summary: None,
        tags: Vec::new(),
        input_examples: Vec::new(),
        output_examples: Vec::new(),
        defer_loading: false,
        setup,
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
