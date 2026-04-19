use super::*;
use kernel::{
    Capability, PluginBridgeKind, PluginCompatibility, PluginCompatibilityMode,
    PluginContractDialect, PluginDescriptor, PluginIR, PluginManifest, PluginRuntimeProfile,
    PluginScanReport, PluginSetup, PluginSetupMode, PluginSlotClaim, PluginSlotMode,
    PluginSourceKind, PluginTranslationReport, PluginTrustTier,
};
use std::collections::{BTreeMap, BTreeSet};

fn test_descriptor(source_kind: PluginSourceKind) -> PluginDescriptor {
    let path = match source_kind {
        PluginSourceKind::PackageManifest => "/tmp/pkg/loong.plugin.json".to_owned(),
        PluginSourceKind::EmbeddedSource => "/tmp/pkg/plugin.py".to_owned(),
    };
    let package_manifest_path = match source_kind {
        PluginSourceKind::PackageManifest => Some(path.clone()),
        PluginSourceKind::EmbeddedSource => None,
    };
    let language = match source_kind {
        PluginSourceKind::PackageManifest => "manifest".to_owned(),
        PluginSourceKind::EmbeddedSource => "py".to_owned(),
    };

    PluginDescriptor {
        path,
        source_kind,
        dialect: match source_kind {
            PluginSourceKind::PackageManifest => PluginContractDialect::LoongPackageManifest,
            PluginSourceKind::EmbeddedSource => PluginContractDialect::LoongEmbeddedSource,
        },
        dialect_version: Some("v1alpha1".to_owned()),
        compatibility_mode: PluginCompatibilityMode::Native,
        package_root: "/tmp/pkg".to_owned(),
        package_manifest_path,
        language,
        manifest: PluginManifest {
            api_version: Some("v1alpha1".to_owned()),
            version: Some("0.3.0".to_owned()),
            plugin_id: "search-plugin".to_owned(),
            provider_id: "search-provider".to_owned(),
            connector_name: "search-connector".to_owned(),
            channel_id: Some("primary".to_owned()),
            endpoint: Some("https://example.com/search".to_owned()),
            capabilities: BTreeSet::from([Capability::InvokeConnector]),
            trust_tier: PluginTrustTier::VerifiedCommunity,
            metadata: BTreeMap::new(),
            summary: Some("Search plugin".to_owned()),
            tags: vec!["search".to_owned()],
            input_examples: Vec::new(),
            output_examples: Vec::new(),
            defer_loading: false,
            setup: Some(PluginSetup {
                mode: PluginSetupMode::MetadataOnly,
                surface: Some("web_search".to_owned()),
                required_env_vars: vec!["TAVILY_API_KEY".to_owned()],
                recommended_env_vars: vec!["TEAM_TAVILY_KEY".to_owned()],
                required_config_keys: vec!["tools.web_search.default_provider".to_owned()],
                default_env_var: Some("TAVILY_API_KEY".to_owned()),
                docs_urls: vec!["https://docs.example.com/tavily".to_owned()],
                remediation: Some("set a Tavily credential before enabling search".to_owned()),
            }),
            slot_claims: vec![PluginSlotClaim {
                slot: "provider:web_search".to_owned(),
                key: "tavily".to_owned(),
                mode: PluginSlotMode::Exclusive,
            }],
            compatibility: Some(PluginCompatibility {
                host_api: Some("loong-plugin/v1".to_owned()),
                host_version_req: Some(">=0.1.0-alpha.1".to_owned()),
            }),
        },
    }
}

fn test_translation(descriptor: &PluginDescriptor) -> PluginTranslationReport {
    PluginTranslationReport {
        translated_plugins: 1,
        bridge_distribution: BTreeMap::from([("http_json".to_owned(), 1)]),
        entries: vec![PluginIR {
            manifest_api_version: descriptor.manifest.api_version.clone(),
            plugin_version: descriptor.manifest.version.clone(),
            dialect: descriptor.dialect,
            dialect_version: descriptor.dialect_version.clone(),
            compatibility_mode: descriptor.compatibility_mode,
            plugin_id: descriptor.manifest.plugin_id.clone(),
            provider_id: descriptor.manifest.provider_id.clone(),
            connector_name: descriptor.manifest.connector_name.clone(),
            channel_id: descriptor.manifest.channel_id.clone(),
            endpoint: descriptor.manifest.endpoint.clone(),
            capabilities: descriptor.manifest.capabilities.clone(),
            trust_tier: descriptor.manifest.trust_tier,
            metadata: descriptor.manifest.metadata.clone(),
            source_path: descriptor.path.clone(),
            source_kind: descriptor.source_kind,
            package_root: descriptor.package_root.clone(),
            package_manifest_path: descriptor.package_manifest_path.clone(),
            diagnostic_findings: Vec::new(),
            setup: descriptor.manifest.setup.clone(),
            channel_bridge: None,
            slot_claims: descriptor.manifest.slot_claims.clone(),
            compatibility: descriptor.manifest.compatibility.clone(),
            runtime: PluginRuntimeProfile {
                source_language: descriptor.language.clone(),
                bridge_kind: PluginBridgeKind::HttpJson,
                adapter_family: "http-adapter".to_owned(),
                entrypoint_hint: "https://example.com/search".to_owned(),
            },
        }],
    }
}

fn test_channel_bridge_descriptor(source_kind: PluginSourceKind) -> PluginDescriptor {
    let mut descriptor = test_descriptor(source_kind);

    descriptor.manifest.plugin_id = "weixin-clawbot-bridge".to_owned();
    descriptor.manifest.provider_id = "weixin-bridge".to_owned();
    descriptor.manifest.connector_name = "weixin-clawbot-http".to_owned();
    descriptor.manifest.channel_id = Some("weixin".to_owned());
    descriptor.manifest.endpoint = Some("http://127.0.0.1:8091/bridge".to_owned());
    descriptor.manifest.metadata = BTreeMap::from([
        (
            "transport_family".to_owned(),
            "wechat_clawbot_ilink_bridge".to_owned(),
        ),
        (
            "target_contract".to_owned(),
            "weixin:<account>:contact:<id> | weixin:<account>:room:<id>".to_owned(),
        ),
        ("account_scope".to_owned(), "multi_account".to_owned()),
    ]);
    descriptor.manifest.setup = Some(PluginSetup {
        mode: PluginSetupMode::MetadataOnly,
        surface: Some("channel".to_owned()),
        required_env_vars: vec!["WEIXIN_BRIDGE_URL".to_owned()],
        recommended_env_vars: vec!["WEIXIN_BRIDGE_ACCESS_TOKEN".to_owned()],
        required_config_keys: vec![
            "weixin.enabled".to_owned(),
            "weixin.bridge_url".to_owned(),
            "weixin.bridge_access_token".to_owned(),
        ],
        default_env_var: Some("WEIXIN_BRIDGE_URL".to_owned()),
        docs_urls: vec!["https://docs.example.com/weixin-bridge".to_owned()],
        remediation: Some("configure the sanctioned weixin bridge contract".to_owned()),
    });

    descriptor
}

fn test_channel_bridge_translation(descriptor: &PluginDescriptor) -> PluginTranslationReport {
    PluginTranslationReport {
        translated_plugins: 1,
        bridge_distribution: BTreeMap::from([("http_json".to_owned(), 1)]),
        entries: vec![PluginIR {
            manifest_api_version: descriptor.manifest.api_version.clone(),
            plugin_version: descriptor.manifest.version.clone(),
            dialect: descriptor.dialect,
            dialect_version: descriptor.dialect_version.clone(),
            compatibility_mode: descriptor.compatibility_mode,
            plugin_id: descriptor.manifest.plugin_id.clone(),
            provider_id: descriptor.manifest.provider_id.clone(),
            connector_name: descriptor.manifest.connector_name.clone(),
            channel_id: descriptor.manifest.channel_id.clone(),
            endpoint: descriptor.manifest.endpoint.clone(),
            capabilities: descriptor.manifest.capabilities.clone(),
            trust_tier: descriptor.manifest.trust_tier,
            metadata: descriptor.manifest.metadata.clone(),
            source_path: descriptor.path.clone(),
            source_kind: descriptor.source_kind,
            package_root: descriptor.package_root.clone(),
            package_manifest_path: descriptor.package_manifest_path.clone(),
            diagnostic_findings: Vec::new(),
            setup: descriptor.manifest.setup.clone(),
            channel_bridge: Some(kernel::PluginChannelBridgeContract {
                channel_id: Some("weixin".to_owned()),
                setup_surface: Some("channel".to_owned()),
                transport_family: Some("wechat_clawbot_ilink_bridge".to_owned()),
                target_contract: Some(
                    "weixin:<account>:contact:<id> | weixin:<account>:room:<id>".to_owned(),
                ),
                account_scope: Some("multi_account".to_owned()),
                runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
                runtime_operations: vec!["send_message".to_owned(), "receive_batch".to_owned()],
                runtime_metadata_issues: Vec::new(),
                readiness: kernel::PluginChannelBridgeReadiness {
                    ready: true,
                    missing_fields: Vec::new(),
                },
            }),
            slot_claims: descriptor.manifest.slot_claims.clone(),
            compatibility: descriptor.manifest.compatibility.clone(),
            runtime: PluginRuntimeProfile {
                source_language: descriptor.language.clone(),
                bridge_kind: PluginBridgeKind::HttpJson,
                adapter_family: "channel-bridge".to_owned(),
                entrypoint_hint: "http://127.0.0.1:8091/bridge".to_owned(),
            },
        }],
    }
}

#[test]
fn enrich_scan_report_adds_package_manifest_provenance_and_setup_metadata() {
    let descriptor = test_descriptor(PluginSourceKind::PackageManifest);
    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![descriptor.clone()],
    };
    let translation = test_translation(&descriptor);

    let enriched = enrich_scan_report_with_translation(&report, &translation, None);
    let metadata = &enriched.descriptors[0].manifest.metadata;

    assert_eq!(
        metadata.get("plugin_source_kind").map(String::as_str),
        Some("package_manifest")
    );
    assert_eq!(
        metadata.get("plugin_trust_tier").map(String::as_str),
        Some("verified-community")
    );
    assert_eq!(
        metadata.get("plugin_package_root").map(String::as_str),
        Some("/tmp/pkg")
    );
    assert_eq!(
        metadata
            .get("plugin_provenance_summary")
            .map(String::as_str),
        Some("package_manifest:/tmp/pkg/loong.plugin.json")
    );
    assert_eq!(
        metadata
            .get("plugin_package_manifest_path")
            .map(String::as_str),
        Some("/tmp/pkg/loong.plugin.json")
    );
    assert_eq!(
        metadata.get("plugin_setup_mode").map(String::as_str),
        Some("metadata_only")
    );
    assert_eq!(
        metadata.get("plugin_setup_surface").map(String::as_str),
        Some("web_search")
    );
    assert_eq!(
        metadata
            .get("plugin_setup_default_env_var")
            .map(String::as_str),
        Some("TAVILY_API_KEY")
    );
    assert_eq!(
        metadata
            .get("plugin_setup_required_env_vars_json")
            .map(String::as_str),
        Some("[\"TAVILY_API_KEY\"]")
    );
    assert_eq!(
        metadata.get("plugin_slot_claims_json").map(String::as_str),
        Some("[{\"slot\":\"provider:web_search\",\"key\":\"tavily\",\"mode\":\"exclusive\"}]")
    );
    assert_eq!(
        metadata
            .get("plugin_manifest_api_version")
            .map(String::as_str),
        Some("v1alpha1")
    );
    assert_eq!(
        metadata.get("plugin_version").map(String::as_str),
        Some("0.3.0")
    );
    assert_eq!(
        metadata
            .get("plugin_compatibility_host_api")
            .map(String::as_str),
        Some("loong-plugin/v1")
    );
    assert_eq!(
        metadata
            .get("plugin_compatibility_host_version_req")
            .map(String::as_str),
        Some(">=0.1.0-alpha.1")
    );
}

#[test]
fn enrich_scan_report_omits_package_manifest_path_for_source_fallback() {
    let descriptor = test_descriptor(PluginSourceKind::EmbeddedSource);
    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![descriptor.clone()],
    };
    let translation = test_translation(&descriptor);

    let enriched = enrich_scan_report_with_translation(&report, &translation, None);
    let metadata = &enriched.descriptors[0].manifest.metadata;

    assert_eq!(
        metadata.get("plugin_source_kind").map(String::as_str),
        Some("embedded_source")
    );
    assert_eq!(
        metadata.get("plugin_trust_tier").map(String::as_str),
        Some("verified-community")
    );
    assert_eq!(
        metadata.get("plugin_package_root").map(String::as_str),
        Some("/tmp/pkg")
    );
    assert_eq!(
        metadata
            .get("plugin_provenance_summary")
            .map(String::as_str),
        Some("embedded_source:/tmp/pkg/plugin.py")
    );
    assert_eq!(
        metadata.get("plugin_setup_mode").map(String::as_str),
        Some("metadata_only")
    );
    assert!(
        !metadata.contains_key("plugin_package_manifest_path"),
        "source fallback should not synthesize a package manifest path"
    );
}

#[test]
fn enrich_scan_report_overwrites_forged_package_manifest_provenance_metadata() {
    let mut descriptor = test_descriptor(PluginSourceKind::PackageManifest);

    descriptor.manifest.metadata.insert(
        "plugin_source_path".to_owned(),
        "/forged/source-path".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_source_kind".to_owned(),
        "embedded_source".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_package_root".to_owned(),
        "/forged/package-root".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_package_manifest_path".to_owned(),
        "/forged/package-manifest".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_provenance_summary".to_owned(),
        "forged:summary".to_owned(),
    );
    descriptor
        .manifest
        .metadata
        .insert("plugin_trust_tier".to_owned(), "unverified".to_owned());

    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![descriptor.clone()],
    };
    let translation = test_translation(&descriptor);

    let enriched = enrich_scan_report_with_translation(&report, &translation, None);
    let metadata = &enriched.descriptors[0].manifest.metadata;

    assert_eq!(
        metadata.get("plugin_source_path").map(String::as_str),
        Some("/tmp/pkg/loong.plugin.json")
    );
    assert_eq!(
        metadata.get("plugin_source_kind").map(String::as_str),
        Some("package_manifest")
    );
    assert_eq!(
        metadata.get("plugin_package_root").map(String::as_str),
        Some("/tmp/pkg")
    );
    assert_eq!(
        metadata
            .get("plugin_provenance_summary")
            .map(String::as_str),
        Some("package_manifest:/tmp/pkg/loong.plugin.json")
    );
    assert_eq!(
        metadata.get("plugin_trust_tier").map(String::as_str),
        Some("verified-community")
    );
    assert_eq!(
        metadata
            .get("plugin_package_manifest_path")
            .map(String::as_str),
        Some("/tmp/pkg/loong.plugin.json")
    );
}

#[test]
fn enrich_scan_report_clears_forged_package_manifest_path_for_source_fallback() {
    let mut descriptor = test_descriptor(PluginSourceKind::EmbeddedSource);

    descriptor.manifest.metadata.insert(
        "plugin_source_path".to_owned(),
        "/forged/source-path".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_source_kind".to_owned(),
        "package_manifest".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_package_root".to_owned(),
        "/forged/package-root".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_package_manifest_path".to_owned(),
        "/forged/package-manifest".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_provenance_summary".to_owned(),
        "forged:summary".to_owned(),
    );
    descriptor
        .manifest
        .metadata
        .insert("plugin_trust_tier".to_owned(), "official".to_owned());

    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![descriptor.clone()],
    };
    let translation = test_translation(&descriptor);

    let enriched = enrich_scan_report_with_translation(&report, &translation, None);
    let metadata = &enriched.descriptors[0].manifest.metadata;

    assert_eq!(
        metadata.get("plugin_source_path").map(String::as_str),
        Some("/tmp/pkg/plugin.py")
    );
    assert_eq!(
        metadata.get("plugin_source_kind").map(String::as_str),
        Some("embedded_source")
    );
    assert_eq!(
        metadata.get("plugin_package_root").map(String::as_str),
        Some("/tmp/pkg")
    );
    assert_eq!(
        metadata
            .get("plugin_provenance_summary")
            .map(String::as_str),
        Some("embedded_source:/tmp/pkg/plugin.py")
    );
    assert_eq!(
        metadata.get("plugin_trust_tier").map(String::as_str),
        Some("verified-community")
    );
    assert!(
        !metadata.contains_key("plugin_package_manifest_path"),
        "source fallback should remove forged package manifest paths"
    );
}

#[test]
fn enrich_scan_report_overrides_conflicting_ad_hoc_setup_metadata() {
    let mut descriptor = test_descriptor(PluginSourceKind::PackageManifest);
    descriptor
        .manifest
        .metadata
        .insert("plugin_setup_mode".to_owned(), "governed_entry".to_owned());
    descriptor.manifest.metadata.insert(
        "plugin_setup_surface".to_owned(),
        "legacy_surface".to_owned(),
    );
    descriptor.manifest.metadata.insert(
        "plugin_setup_required_env_vars_json".to_owned(),
        "[\"LEGACY_KEY\"]".to_owned(),
    );

    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![descriptor.clone()],
    };
    let translation = test_translation(&descriptor);

    let enriched = enrich_scan_report_with_translation(&report, &translation, None);
    let metadata = &enriched.descriptors[0].manifest.metadata;

    assert_eq!(
        metadata.get("plugin_setup_mode").map(String::as_str),
        Some("metadata_only")
    );
    assert_eq!(
        metadata.get("plugin_setup_surface").map(String::as_str),
        Some("web_search")
    );
    assert_eq!(
        metadata
            .get("plugin_setup_required_env_vars_json")
            .map(String::as_str),
        Some("[\"TAVILY_API_KEY\"]")
    );
}

#[test]
fn enrich_scan_report_adds_channel_bridge_contract_metadata() {
    let descriptor = test_channel_bridge_descriptor(PluginSourceKind::PackageManifest);
    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![descriptor.clone()],
    };
    let translation = test_channel_bridge_translation(&descriptor);

    let enriched = enrich_scan_report_with_translation(&report, &translation, None);
    let metadata = &enriched.descriptors[0].manifest.metadata;

    assert_eq!(
        metadata.get("plugin_channel_id").map(String::as_str),
        Some("weixin")
    );
    assert_eq!(
        metadata
            .get("plugin_channel_bridge_transport_family")
            .map(String::as_str),
        Some("wechat_clawbot_ilink_bridge")
    );
    assert_eq!(
        metadata
            .get("plugin_channel_bridge_target_contract")
            .map(String::as_str),
        Some("weixin:<account>:contact:<id> | weixin:<account>:room:<id>")
    );
    assert_eq!(
        metadata
            .get("plugin_channel_bridge_account_scope")
            .map(String::as_str),
        Some("multi_account")
    );
    assert_eq!(
        metadata
            .get("plugin_channel_bridge_ready")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        metadata
            .get("plugin_channel_bridge_missing_fields_json")
            .map(String::as_str),
        Some("[]")
    );
}
