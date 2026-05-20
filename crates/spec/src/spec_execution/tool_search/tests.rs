use super::*;
use kernel::{
    Capability, IntegrationCatalog, PluginActivationCandidate, PluginActivationPlan,
    PluginActivationStatus, PluginBridgeKind, PluginCompatibilityMode, PluginContractDialect,
    PluginDescriptor, PluginDiagnosticCode, PluginDiagnosticFinding, PluginDiagnosticPhase,
    PluginDiagnosticSeverity, PluginIR, PluginManifest, PluginRuntimeProfile, PluginSetup,
    PluginSetupMode, PluginSetupReadinessContext, PluginSlotClaim, PluginSlotMode,
    PluginSourceKind, PluginTranslationReport, PluginTrustTier, ProviderConfig,
};
use std::collections::{BTreeMap, BTreeSet};

fn test_channel_bridge_descriptor() -> PluginDescriptor {
    PluginDescriptor {
        path: "/tmp/weixin/loong.plugin.json".to_owned(),
        source_kind: PluginSourceKind::PackageManifest,
        dialect: PluginContractDialect::LoongPackageManifest,
        dialect_version: Some("v1alpha1".to_owned()),
        compatibility_mode: PluginCompatibilityMode::Native,
        package_root: "/tmp/weixin".to_owned(),
        package_manifest_path: Some("/tmp/weixin/loong.plugin.json".to_owned()),
        language: "manifest".to_owned(),
        manifest: PluginManifest {
            api_version: Some("v1alpha1".to_owned()),
            version: Some("0.3.0".to_owned()),
            plugin_id: "weixin-clawbot-bridge".to_owned(),
            provider_id: "weixin-bridge".to_owned(),
            connector_name: "weixin-clawbot-http".to_owned(),
            channel_id: Some("weixin".to_owned()),
            endpoint: Some("http://127.0.0.1:8091/bridge".to_owned()),
            capabilities: BTreeSet::from([Capability::InvokeConnector]),
            trust_tier: PluginTrustTier::VerifiedCommunity,
            metadata: BTreeMap::from([
                (
                    "transport_family".to_owned(),
                    "wechat_clawbot_ilink_bridge".to_owned(),
                ),
                (
                    "target_contract".to_owned(),
                    "weixin:<account>:contact:<id> | weixin:<account>:room:<id>".to_owned(),
                ),
                ("account_scope".to_owned(), "multi_account".to_owned()),
            ]),
            summary: Some("Weixin bridge".to_owned()),
            tags: vec!["weixin".to_owned(), "bridge".to_owned()],
            input_examples: Vec::new(),
            output_examples: Vec::new(),
            defer_loading: false,
            setup: Some(PluginSetup {
                mode: PluginSetupMode::MetadataOnly,
                surface: Some("channel".to_owned()),
                required_env_vars: vec!["WEIXIN_BRIDGE_URL".to_owned()],
                recommended_env_vars: vec!["WEIXIN_BRIDGE_ACCESS_TOKEN".to_owned()],
                required_config_keys: vec!["weixin.enabled".to_owned()],
                default_env_var: Some("WEIXIN_BRIDGE_URL".to_owned()),
                docs_urls: vec!["https://docs.example.com/weixin-bridge".to_owned()],
                remediation: Some("configure the sanctioned weixin bridge contract".to_owned()),
            }),
            slot_claims: vec![PluginSlotClaim {
                slot: "channel:weixin".to_owned(),
                key: "bridge".to_owned(),
                mode: PluginSlotMode::Exclusive,
            }],
            compatibility: None,
        },
    }
}

fn test_channel_bridge_translation() -> PluginTranslationReport {
    let descriptor = test_channel_bridge_descriptor();
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
                runtime_operations: vec!["send_message".to_owned()],
                runtime_metadata_issues: Vec::new(),
                readiness: kernel::PluginChannelBridgeReadiness {
                    ready: true,
                    missing_fields: Vec::new(),
                },
            }),
            slot_claims: descriptor.manifest.slot_claims.clone(),
            compatibility: descriptor.manifest.compatibility.clone(),
            runtime: PluginRuntimeProfile {
                source_language: descriptor.language,
                bridge_kind: PluginBridgeKind::HttpJson,
                adapter_family: "channel-bridge".to_owned(),
                entrypoint_hint: "http://127.0.0.1:8091/bridge".to_owned(),
            },
        }],
    }
}

#[test]
fn execute_tool_search_surfaces_plugin_provenance_and_setup_metadata() {
    let mut catalog = IntegrationCatalog::new();
    let provider = ProviderConfig {
        provider_id: "tavily".to_owned(),
        connector_name: "tavily-http".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "tavily-search".to_owned()),
            (
                "plugin_source_path".to_owned(),
                "/tmp/tavily/loong.plugin.json".to_owned(),
            ),
            (
                "plugin_source_kind".to_owned(),
                "package_manifest".to_owned(),
            ),
            ("plugin_package_root".to_owned(), "/tmp/tavily".to_owned()),
            (
                "plugin_package_manifest_path".to_owned(),
                "/tmp/tavily/loong.plugin.json".to_owned(),
            ),
            (
                "plugin_provenance_summary".to_owned(),
                "package_manifest:/tmp/tavily/loong.plugin.json".to_owned(),
            ),
            ("plugin_trust_tier".to_owned(), "official".to_owned()),
            (
                "plugin_manifest_api_version".to_owned(),
                "v1alpha1".to_owned(),
            ),
            ("plugin_version".to_owned(), "0.3.0".to_owned()),
            ("plugin_setup_mode".to_owned(), "metadata_only".to_owned()),
            ("plugin_setup_surface".to_owned(), "web_search".to_owned()),
            (
                "plugin_setup_required_env_vars_json".to_owned(),
                "[\"TAVILY_API_KEY\"]".to_owned(),
            ),
            (
                "plugin_setup_recommended_env_vars_json".to_owned(),
                "[\"TEAM_TAVILY_KEY\"]".to_owned(),
            ),
            (
                "plugin_setup_required_config_keys_json".to_owned(),
                "[\"tools.web_search.default_provider\"]".to_owned(),
            ),
            (
                "plugin_setup_default_env_var".to_owned(),
                "TAVILY_API_KEY".to_owned(),
            ),
            (
                "plugin_setup_docs_urls_json".to_owned(),
                "[\"https://docs.example.com/tavily\"]".to_owned(),
            ),
            (
                "plugin_setup_remediation".to_owned(),
                "set a Tavily credential before enabling search".to_owned(),
            ),
            (
                "plugin_slot_claims_json".to_owned(),
                "[{\"slot\":\"provider:web_search\",\"key\":\"tavily\",\"mode\":\"exclusive\"}]"
                    .to_owned(),
            ),
            (
                "plugin_compatibility_host_api".to_owned(),
                "loong-plugin/v1".to_owned(),
            ),
            (
                "plugin_compatibility_host_version_req".to_owned(),
                ">=0.1.0-alpha.1".to_owned(),
            ),
            ("bridge_kind".to_owned(), "http_json".to_owned()),
        ]),
    };
    catalog.upsert_provider(provider);

    let activation_plans = vec![PluginActivationPlan {
        total_plugins: 1,
        ready_plugins: 0,
        setup_incomplete_plugins: 0,
        blocked_plugins: 1,
        candidates: vec![PluginActivationCandidate {
            plugin_id: "tavily-search".to_owned(),
            source_path: "/tmp/tavily/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            package_root: "/tmp/tavily".to_owned(),
            package_manifest_path: Some("/tmp/tavily/loong.plugin.json".to_owned()),
            trust_tier: kernel::PluginTrustTier::Official,
            compatibility_mode: PluginCompatibilityMode::Native,
            compatibility_shim: None,
            compatibility_shim_support: None,
            compatibility_shim_support_mismatch_reasons: Vec::new(),
            bridge_kind: PluginBridgeKind::HttpJson,
            adapter_family: "http-adapter".to_owned(),
            slot_claims: vec![PluginSlotClaim {
                slot: "provider:web_search".to_owned(),
                key: "tavily".to_owned(),
                mode: PluginSlotMode::Exclusive,
            }],
            diagnostic_findings: vec![PluginDiagnosticFinding {
                code: PluginDiagnosticCode::SlotClaimConflict,
                severity: PluginDiagnosticSeverity::Error,
                phase: PluginDiagnosticPhase::Activation,
                blocking: true,
                plugin_id: Some("tavily-search".to_owned()),
                source_path: Some("/tmp/tavily/loong.plugin.json".to_owned()),
                source_kind: Some(PluginSourceKind::PackageManifest),
                field_path: Some("slot_claims".to_owned()),
                message: "slot claim `provider:web_search`:`tavily` conflicts with existing plugin `web-search`".to_owned(),
                remediation: Some("choose a different slot or relax ownership intentionally".to_owned()),
            }],
            status: PluginActivationStatus::BlockedSlotClaimConflict,
            reason: "slot claim `provider:web_search`:`tavily` conflicts with existing plugin `web-search`".to_owned(),
            missing_required_env_vars: Vec::new(),
            missing_required_config_keys: Vec::new(),
            bootstrap_hint: "register http".to_owned(),
        }],
    }];
    let setup_readiness_context = PluginSetupReadinessContext::default();
    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &setup_readiness_context,
        &activation_plans,
        "TAVILY_API_KEY",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert!(!report.trust_filter_summary.applied);
    assert_eq!(
        report.results[0].manifest_api_version.as_deref(),
        Some("v1alpha1")
    );
    assert_eq!(report.results[0].plugin_version.as_deref(), Some("0.3.0"));
    assert_eq!(
        report.results[0].source_kind.as_deref(),
        Some("package_manifest")
    );
    assert_eq!(report.results[0].package_root.as_deref(), Some("/tmp/tavily"));
    assert_eq!(
        report.results[0].package_manifest_path.as_deref(),
        Some("/tmp/tavily/loong.plugin.json")
    );
    assert!(report.results[0].compatibility_shim.is_none());
    assert_eq!(
        report.results[0].provenance_summary.as_deref(),
        Some("package_manifest:/tmp/tavily/loong.plugin.json")
    );
    assert_eq!(report.results[0].trust_tier.as_deref(), Some("official"));
    assert_eq!(
        report.results[0].setup_mode.as_deref(),
        Some("metadata_only")
    );
    assert_eq!(
        report.results[0].setup_surface.as_deref(),
        Some("web_search")
    );
    assert_eq!(
        report.results[0].setup_default_env_var.as_deref(),
        Some("TAVILY_API_KEY")
    );
    assert_eq!(
        report.results[0].setup_required_env_vars,
        vec!["TAVILY_API_KEY".to_owned()]
    );
    assert!(!report.results[0].setup_ready);
    assert_eq!(
        report.results[0].missing_required_env_vars,
        vec!["TAVILY_API_KEY".to_owned()]
    );
    assert_eq!(
        report.results[0].missing_required_config_keys,
        vec!["tools.web_search.default_provider".to_owned()]
    );
    assert_eq!(
        report.results[0].slot_claims,
        vec![PluginSlotClaim {
            slot: "provider:web_search".to_owned(),
            key: "tavily".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }]
    );
    assert_eq!(
        report.results[0]
            .compatibility
            .as_ref()
            .and_then(|compatibility| compatibility.host_api.as_deref()),
        Some("loong-plugin/v1")
    );
    assert_eq!(
        report.results[0]
            .compatibility
            .as_ref()
            .and_then(|compatibility| compatibility.host_version_req.as_deref()),
        Some(">=0.1.0-alpha.1")
    );
    assert_eq!(
        report.results[0].activation_status.as_deref(),
        Some("blocked_slot_claim_conflict")
    );
    assert!(
        report.results[0]
            .activation_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("provider:web_search"))
    );
    assert_eq!(report.results[0].diagnostic_findings.len(), 1);
    assert_eq!(
        report.results[0].diagnostic_findings[0].code,
        PluginDiagnosticCode::SlotClaimConflict
    );
    assert_eq!(
        report.results[0].diagnostic_findings[0].phase,
        PluginDiagnosticPhase::Activation
    );
    assert!(report.results[0].diagnostic_findings[0].blocking);
}

#[test]
fn execute_tool_search_surfaces_verified_activation_attestation_for_loaded_plugins() {
    let contract = crate::spec_runtime::PluginActivationRuntimeContract {
        plugin_id: "openclaw-weather".to_owned(),
        source_path: "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
        source_kind: PluginSourceKind::PackageManifest,
        dialect: PluginContractDialect::OpenClawModernManifest,
        dialect_version: Some("openclaw.plugin.json".to_owned()),
        compatibility_mode: PluginCompatibilityMode::OpenClawModern,
        compatibility_shim: Some(kernel::PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        }),
        bridge_kind: PluginBridgeKind::ProcessStdio,
        adapter_family: "openclaw-modern-compat".to_owned(),
        entrypoint_hint: "stdin/stdout::invoke".to_owned(),
        source_language: "javascript".to_owned(),
        compatibility: None,
    };
    let raw_contract = crate::spec_runtime::plugin_activation_runtime_contract_json(&contract)
        .expect("encode activation contract");
    let checksum =
        crate::spec_runtime::activation_runtime_contract_checksum_hex(raw_contract.as_bytes());

    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "openclaw-weather".to_owned(),
        connector_name: "weather".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "openclaw-weather".to_owned()),
            (
                "plugin_source_path".to_owned(),
                "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            ),
            (
                "plugin_dialect".to_owned(),
                "openclaw_modern_manifest".to_owned(),
            ),
            (
                "plugin_compatibility_mode".to_owned(),
                "openclaw_modern".to_owned(),
            ),
            ("plugin_activation_contract_json".to_owned(), raw_contract),
            (
                "plugin_activation_contract_checksum".to_owned(),
                checksum.clone(),
            ),
            ("bridge_kind".to_owned(), "process_stdio".to_owned()),
        ]),
    });

    let setup_readiness_context = PluginSetupReadinessContext::default();
    let activation_plans: &[PluginActivationPlan] = &[];
    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &setup_readiness_context,
        activation_plans,
        "verified",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert_eq!(
        report.results[0]
            .activation_attestation
            .as_ref()
            .map(|attestation| attestation.integrity.as_str()),
        Some("verified")
    );
    assert_eq!(
        report.results[0]
            .activation_attestation
            .as_ref()
            .and_then(|attestation| attestation.checksum.as_deref()),
        Some(checksum.as_str())
    );
}

#[test]
fn execute_tool_search_marks_setup_ready_when_requirements_are_verified() {
    let mut catalog = IntegrationCatalog::new();
    let provider = ProviderConfig {
        provider_id: "tavily".to_owned(),
        connector_name: "tavily-http".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            (
                "plugin_setup_required_env_vars_json".to_owned(),
                "[\"TAVILY_API_KEY\"]".to_owned(),
            ),
            (
                "plugin_setup_required_config_keys_json".to_owned(),
                "[\"tools.web_search.default_provider\"]".to_owned(),
            ),
        ]),
    };
    catalog.upsert_provider(provider);

    let setup_readiness_context = PluginSetupReadinessContext {
        verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
        verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
    };

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &setup_readiness_context,
        &[],
        "tavily",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert!(report.results[0].setup_ready);
    assert!(report.results[0].missing_required_env_vars.is_empty());
    assert!(report.results[0].missing_required_config_keys.is_empty());
}

#[test]
fn execute_tool_search_prefers_higher_trust_tier_when_scores_tie() {
    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "aaa-unverified".to_owned(),
        connector_name: "search-alpha".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "aaa-unverified".to_owned()),
            ("plugin_trust_tier".to_owned(), "unverified".to_owned()),
            ("plugin_source_path".to_owned(), "/tmp/aaa.rs".to_owned()),
        ]),
    });
    catalog.upsert_provider(ProviderConfig {
        provider_id: "zzz-official".to_owned(),
        connector_name: "search-zeta".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "zzz-official".to_owned()),
            ("plugin_trust_tier".to_owned(), "official".to_owned()),
            ("plugin_source_path".to_owned(), "/tmp/zzz.rs".to_owned()),
        ]),
    });

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &PluginSetupReadinessContext::default(),
        &[],
        "",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 2);
    assert_eq!(report.results[0].trust_tier.as_deref(), Some("official"));
    assert_eq!(report.results[1].trust_tier.as_deref(), Some("unverified"));
}

#[test]
fn execute_tool_search_filters_by_trust_tier_query_prefix() {
    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "official-search".to_owned(),
        connector_name: "official-search".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "official-search".to_owned()),
            ("plugin_trust_tier".to_owned(), "official".to_owned()),
            (
                "summary".to_owned(),
                "Search across official docs".to_owned(),
            ),
        ]),
    });
    catalog.upsert_provider(ProviderConfig {
        provider_id: "verified-search".to_owned(),
        connector_name: "verified-search".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "verified-search".to_owned()),
            (
                "plugin_trust_tier".to_owned(),
                "verified-community".to_owned(),
            ),
            (
                "summary".to_owned(),
                "Search across community docs".to_owned(),
            ),
        ]),
    });
    catalog.upsert_provider(ProviderConfig {
        provider_id: "unverified-search".to_owned(),
        connector_name: "unverified-search".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "unverified-search".to_owned()),
            ("plugin_trust_tier".to_owned(), "unverified".to_owned()),
            ("summary".to_owned(), "Search across random docs".to_owned()),
        ]),
    });

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &PluginSetupReadinessContext::default(),
        &[],
        "tier:verified_community search",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert!(report.trust_filter_summary.applied);
    assert_eq!(
        report.trust_filter_summary.query_requested_tiers,
        vec!["verified-community".to_owned()]
    );
    assert_eq!(
        report.trust_filter_summary.effective_tiers,
        vec!["verified-community".to_owned()]
    );
    assert!(!report.trust_filter_summary.conflicting_requested_tiers);
    assert_eq!(report.trust_filter_summary.filtered_out_candidates, 2);
    assert_eq!(
        report
            .trust_filter_summary
            .filtered_out_tier_counts
            .get("official"),
        Some(&1)
    );
    assert_eq!(
        report
            .trust_filter_summary
            .filtered_out_tier_counts
            .get("unverified"),
        Some(&1)
    );
    assert_eq!(report.results[0].provider_id, "verified-search");
    assert_eq!(
        report.results[0].trust_tier.as_deref(),
        Some("verified-community")
    );
}

#[test]
fn execute_tool_search_filters_by_structured_trust_tiers() {
    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "official-search".to_owned(),
        connector_name: "official-search".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "official-search".to_owned()),
            ("plugin_trust_tier".to_owned(), "official".to_owned()),
            (
                "summary".to_owned(),
                "Search across official docs".to_owned(),
            ),
        ]),
    });
    catalog.upsert_provider(ProviderConfig {
        provider_id: "verified-search".to_owned(),
        connector_name: "verified-search".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "verified-search".to_owned()),
            (
                "plugin_trust_tier".to_owned(),
                "verified-community".to_owned(),
            ),
            (
                "summary".to_owned(),
                "Search across community docs".to_owned(),
            ),
        ]),
    });

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &PluginSetupReadinessContext::default(),
        &[],
        "search",
        10,
        &[PluginTrustTier::Official],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert!(report.trust_filter_summary.applied);
    assert_eq!(
        report.trust_filter_summary.structured_requested_tiers,
        vec!["official".to_owned()]
    );
    assert_eq!(
        report.trust_filter_summary.effective_tiers,
        vec!["official".to_owned()]
    );
    assert!(!report.trust_filter_summary.conflicting_requested_tiers);
    assert_eq!(report.trust_filter_summary.filtered_out_candidates, 1);
    assert_eq!(report.results[0].provider_id, "official-search");
    assert_eq!(report.results[0].trust_tier.as_deref(), Some("official"));
}

#[test]
fn execute_tool_search_conflicting_query_and_structured_trust_filters_fail_closed() {
    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "official-search".to_owned(),
        connector_name: "official-search".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "official-search".to_owned()),
            ("plugin_trust_tier".to_owned(), "official".to_owned()),
            (
                "summary".to_owned(),
                "Search across official docs".to_owned(),
            ),
        ]),
    });
    catalog.upsert_provider(ProviderConfig {
        provider_id: "verified-search".to_owned(),
        connector_name: "verified-search".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "verified-search".to_owned()),
            (
                "plugin_trust_tier".to_owned(),
                "verified-community".to_owned(),
            ),
            (
                "summary".to_owned(),
                "Search across community docs".to_owned(),
            ),
        ]),
    });

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &PluginSetupReadinessContext::default(),
        &[],
        "trust:official search",
        10,
        &[PluginTrustTier::VerifiedCommunity],
        true,
        false,
    );

    assert!(report.results.is_empty());
    assert!(report.trust_filter_summary.applied);
    assert_eq!(
        report.trust_filter_summary.query_requested_tiers,
        vec!["official".to_owned()]
    );
    assert_eq!(
        report.trust_filter_summary.structured_requested_tiers,
        vec!["verified-community".to_owned()]
    );
    assert!(report.trust_filter_summary.effective_tiers.is_empty());
    assert!(report.trust_filter_summary.conflicting_requested_tiers);
    assert_eq!(report.trust_filter_summary.filtered_out_candidates, 2);
    assert_eq!(
        report
            .trust_filter_summary
            .filtered_out_tier_counts
            .get("official"),
        Some(&1)
    );
    assert_eq!(
        report
            .trust_filter_summary
            .filtered_out_tier_counts
            .get("verified-community"),
        Some(&1)
    );
}

#[test]
fn execute_tool_search_derives_canonical_shim_from_compatibility_mode_metadata() {
    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "openclaw-weather".to_owned(),
        connector_name: "weather".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "openclaw-weather".to_owned()),
            (
                "plugin_source_path".to_owned(),
                "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            ),
            (
                "plugin_dialect".to_owned(),
                "openclaw_modern_manifest".to_owned(),
            ),
            (
                "plugin_compatibility_mode".to_owned(),
                "openclaw_modern".to_owned(),
            ),
            ("bridge_kind".to_owned(), "process_stdio".to_owned()),
        ]),
    });

    let setup_readiness_context = PluginSetupReadinessContext::default();
    let activation_plans: &[PluginActivationPlan] = &[];
    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &setup_readiness_context,
        activation_plans,
        "openclaw-modern-compat",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert_eq!(
        report.results[0].compatibility_mode.as_deref(),
        Some("openclaw_modern")
    );
    assert_eq!(
        report.results[0]
            .compatibility_shim
            .as_ref()
            .map(|shim| shim.shim_id.as_str()),
        Some("openclaw-modern-compat")
    );
    assert!(report.results[0].compatibility_shim_support.is_none());
    assert!(
        report.results[0]
            .compatibility_shim_support_mismatch_reasons
            .is_empty()
    );
}

#[test]
fn execute_tool_search_surfaces_shim_support_profile_and_mismatch_reasons() {
    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "openclaw-weather".to_owned(),
        connector_name: "weather".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "openclaw-weather".to_owned()),
            (
                "plugin_source_path".to_owned(),
                "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            ),
            (
                "plugin_dialect".to_owned(),
                "openclaw_modern_manifest".to_owned(),
            ),
            (
                "plugin_compatibility_mode".to_owned(),
                "openclaw_modern".to_owned(),
            ),
            ("bridge_kind".to_owned(), "process_stdio".to_owned()),
        ]),
    });

    let shim = PluginCompatibilityShim {
        shim_id: "openclaw-modern-compat".to_owned(),
        family: "openclaw-modern-compat".to_owned(),
    };
    let activation_plans = vec![PluginActivationPlan {
        total_plugins: 1,
        ready_plugins: 0,
        setup_incomplete_plugins: 0,
        blocked_plugins: 1,
        candidates: vec![PluginActivationCandidate {
            plugin_id: "openclaw-weather".to_owned(),
            source_path: "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            package_root: "/tmp/openclaw-weather".to_owned(),
            package_manifest_path: Some(
                "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            ),
            trust_tier: kernel::PluginTrustTier::Unverified,
            compatibility_mode: PluginCompatibilityMode::OpenClawModern,
            compatibility_shim: Some(shim.clone()),
            compatibility_shim_support: Some(kernel::PluginCompatibilityShimSupport {
                shim,
                version: Some("openclaw-modern@1".to_owned()),
                supported_dialects: BTreeSet::from([PluginContractDialect::OpenClawModernManifest]),
                supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
                supported_adapter_families: BTreeSet::new(),
                supported_source_languages: BTreeSet::from(["python".to_owned()]),
            }),
            compatibility_shim_support_mismatch_reasons: vec![
                "source language `javascript`".to_owned(),
            ],
            bridge_kind: PluginBridgeKind::ProcessStdio,
            adapter_family: "javascript-stdio-adapter".to_owned(),
            slot_claims: Vec::new(),
            diagnostic_findings: Vec::new(),
            status: PluginActivationStatus::BlockedCompatibilityMode,
            reason: "compatibility shim profile mismatch".to_owned(),
            missing_required_env_vars: Vec::new(),
            missing_required_config_keys: Vec::new(),
            bootstrap_hint: "align compatibility shim profile".to_owned(),
        }],
    }];

    let setup_readiness_context = PluginSetupReadinessContext::default();
    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &setup_readiness_context,
        &activation_plans,
        "openclaw-modern@1",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert_eq!(
        report.results[0]
            .compatibility_shim_support
            .as_ref()
            .and_then(|support| support.version.as_deref()),
        Some("openclaw-modern@1")
    );
    assert_eq!(
        report.results[0].compatibility_shim_support_mismatch_reasons,
        vec!["source language `javascript`".to_owned()]
    );
}

#[test]
fn execute_tool_search_surfaces_channel_bridge_contract_fields() {
    let mut catalog = IntegrationCatalog::new();
    let raw_canonical = serde_json::json!({
        "channel_id": "weixin",
        "setup_surface": "channel",
        "transport_family": "wechat_clawbot_ilink_bridge",
        "target_contract": "weixin:<account>:contact:<id> | weixin:<account>:room:<id>",
        "account_scope": "multi_account",
        "runtime_contract": "loong_channel_bridge_v1",
        "runtime_operations": ["send_message"],
        "runtime_metadata_issues": [],
        "readiness": {
            "ready": true,
            "missing_fields": []
        }
    })
    .to_string();
    let provider = ProviderConfig {
        provider_id: "weixin-bridge".to_owned(),
        connector_name: "weixin-clawbot-http".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "weixin-clawbot-bridge".to_owned()),
            (
                crate::spec_runtime::PLUGIN_CHANNEL_BRIDGE_CONTRACT_METADATA_KEY.to_owned(),
                raw_canonical,
            ),
            (
                "summary".to_owned(),
                "ClawBot-compatible bridge for the weixin channel surface".to_owned(),
            ),
        ]),
    };
    catalog.upsert_provider(provider);

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &PluginSetupReadinessContext::default(),
        &[],
        "weixin clawbot",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].channel_id.as_deref(), Some("weixin"));
    assert_eq!(
        report.results[0].channel_bridge.transport_family.as_deref(),
        Some("wechat_clawbot_ilink_bridge")
    );
    assert_eq!(
        report.results[0].channel_bridge.target_contract.as_deref(),
        Some("weixin:<account>:contact:<id> | weixin:<account>:room:<id>")
    );
    assert_eq!(
        report.results[0].channel_bridge.account_scope.as_deref(),
        Some("multi_account")
    );
    assert_eq!(report.results[0].channel_bridge.ready, Some(true));
    assert!(report.results[0].channel_bridge.missing_fields.is_empty());
}

#[test]
fn execute_tool_search_surfaces_incomplete_channel_bridge_contract_fields() {
    let mut catalog = IntegrationCatalog::new();
    let raw_canonical = serde_json::json!({
        "channel_id": "weixin",
        "setup_surface": "channel",
        "transport_family": null,
        "target_contract": null,
        "account_scope": null,
        "runtime_contract": "loong_channel_bridge_v1",
        "runtime_operations": ["send_message"],
        "runtime_metadata_issues": [],
        "readiness": {
            "ready": false,
            "missing_fields": ["metadata.transport_family", "metadata.target_contract"]
        }
    })
    .to_string();
    let provider = ProviderConfig {
        provider_id: "weixin-bridge".to_owned(),
        connector_name: "weixin-clawbot-http".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "weixin-clawbot-bridge".to_owned()),
            (
                crate::spec_runtime::PLUGIN_CHANNEL_BRIDGE_CONTRACT_METADATA_KEY.to_owned(),
                raw_canonical,
            ),
        ]),
    };
    catalog.upsert_provider(provider);

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &PluginSetupReadinessContext::default(),
        &[],
        "weixin",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].channel_bridge.ready, Some(false));
    assert_eq!(
        report.results[0].channel_bridge.missing_fields,
        vec![
            "metadata.transport_family".to_owned(),
            "metadata.target_contract".to_owned(),
        ]
    );
}

#[test]
fn execute_tool_search_prefers_canonical_translation_bridge_snapshot_over_canonical_metadata() {
    let mut catalog = IntegrationCatalog::new();
    let stale_canonical = serde_json::json!({
        "channel_id": "weixin",
        "setup_surface": "channel",
        "transport_family": "stale_provider_bridge",
        "target_contract": "stale:<id>",
        "account_scope": "single_account",
        "runtime_contract": "loong_channel_bridge_v1",
        "runtime_operations": ["send_message"],
        "runtime_metadata_issues": [],
        "readiness": {
            "ready": false,
            "missing_fields": ["metadata.transport_family"]
        }
    })
    .to_string();
    catalog.upsert_provider(ProviderConfig {
        provider_id: "weixin-bridge".to_owned(),
        connector_name: "weixin-clawbot-http".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "weixin-clawbot-bridge".to_owned()),
            (
                "plugin_source_path".to_owned(),
                "/tmp/weixin/loong.plugin.json".to_owned(),
            ),
            (
                crate::spec_runtime::PLUGIN_CHANNEL_BRIDGE_CONTRACT_METADATA_KEY.to_owned(),
                stale_canonical,
            ),
        ]),
    });

    let translation = test_channel_bridge_translation();
    let report = execute_tool_search(
        &catalog,
        &[],
        &[translation],
        &PluginSetupReadinessContext::default(),
        &[],
        "weixin",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert_eq!(
        report.results[0].channel_bridge.transport_family.as_deref(),
        Some("wechat_clawbot_ilink_bridge")
    );
    assert_eq!(
        report.results[0].channel_bridge.target_contract.as_deref(),
        Some("weixin:<account>:contact:<id> | weixin:<account>:room:<id>")
    );
    assert_eq!(
        report.results[0].channel_bridge.account_scope.as_deref(),
        Some("multi_account")
    );
    assert_eq!(report.results[0].channel_bridge.ready, Some(true));
    assert!(report.results[0].channel_bridge.missing_fields.is_empty());
}

#[test]
fn execute_tool_search_reads_canonical_bridge_contract_metadata() {
    let mut catalog = IntegrationCatalog::new();
    let raw_canonical = serde_json::json!({
        "channel_id": "weixin",
        "setup_surface": "channel",
        "transport_family": "wechat_clawbot_ilink_bridge",
        "target_contract": "weixin:<account>:contact:<id> | weixin:<account>:room:<id>",
        "account_scope": "multi_account",
        "runtime_contract": "loong_channel_bridge_v1",
        "runtime_operations": ["send_message"],
        "runtime_metadata_issues": [],
        "readiness": {
            "ready": true,
            "missing_fields": []
        }
    })
    .to_string();

    catalog.upsert_provider(ProviderConfig {
        provider_id: "weixin-bridge".to_owned(),
        connector_name: "weixin-clawbot-http".to_owned(),
        version: "1.0.0".to_owned(),
        metadata: BTreeMap::from([
            ("plugin_id".to_owned(), "weixin-clawbot-bridge".to_owned()),
            (
                crate::spec_runtime::PLUGIN_CHANNEL_BRIDGE_CONTRACT_METADATA_KEY.to_owned(),
                raw_canonical,
            ),
        ]),
    });

    let report = execute_tool_search(
        &catalog,
        &[],
        &[],
        &PluginSetupReadinessContext::default(),
        &[],
        "weixin",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(report.results.len(), 1);
    assert_eq!(
        report.results[0].channel_bridge.transport_family.as_deref(),
        Some("wechat_clawbot_ilink_bridge")
    );
    assert_eq!(
        report.results[0].channel_bridge.target_contract.as_deref(),
        Some("weixin:<account>:contact:<id> | weixin:<account>:room:<id>")
    );
    assert_eq!(
        report.results[0].channel_bridge.account_scope.as_deref(),
        Some("multi_account")
    );
    assert_eq!(report.results[0].channel_bridge.ready, Some(true));
    assert!(report.results[0].channel_bridge.missing_fields.is_empty());
}

#[test]
fn execute_tool_search_surfaces_channel_id_from_translated_scan_path_without_manifest_fallback() {
    let descriptor = test_channel_bridge_descriptor();
    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![descriptor],
    };
    let translation = test_channel_bridge_translation();

    let search = execute_tool_search(
        &IntegrationCatalog::new(),
        &[report],
        &[translation],
        &PluginSetupReadinessContext::default(),
        &[],
        "weixin",
        10,
        &[],
        true,
        false,
    );

    assert_eq!(search.results.len(), 1);
    assert_eq!(search.results[0].channel_id.as_deref(), Some("weixin"));
}
