use kernel::{
    PluginDiagnosticCode, PluginDiagnosticFinding, PluginDiagnosticPhase,
    PluginDiagnosticSeverity,
};

use super::*;
use crate::spec_runtime::PluginPreflightPolicyProfile;

fn sample_inventory_result() -> PluginInventoryResult {
    PluginInventoryResult {
        manifest_api_version: Some("v1alpha1".to_owned()),
        plugin_version: Some("0.3.0".to_owned()),
        dialect: "loong_package_manifest".to_owned(),
        dialect_version: Some("v1alpha1".to_owned()),
        compatibility_mode: "native".to_owned(),
        compatibility_shim: None,
        compatibility_shim_support: None,
        compatibility_shim_support_mismatch_reasons: Vec::new(),
        plugin_id: "sample-plugin".to_owned(),
        connector_name: "sample-http".to_owned(),
        provider_id: "sample".to_owned(),
        source_path: "/tmp/sample/loong.plugin.json".to_owned(),
        source_kind: "package_manifest".to_owned(),
        package_root: "/tmp/sample".to_owned(),
        package_manifest_path: Some("/tmp/sample/loong.plugin.json".to_owned()),
        bridge_kind: "http_json".to_owned(),
        adapter_family: Some("http-adapter".to_owned()),
        entrypoint_hint: Some("https://example.com/invoke".to_owned()),
        source_language: Some("manifest".to_owned()),
        setup_mode: None,
        setup_surface: None,
        setup_required_env_vars: Vec::new(),
        setup_recommended_env_vars: Vec::new(),
        setup_required_config_keys: Vec::new(),
        setup_default_env_var: None,
        setup_docs_urls: Vec::new(),
        setup_remediation: None,
        slot_claims: Vec::new(),
        diagnostic_findings: Vec::new(),
        compatibility: None,
        activation_status: Some("ready".to_owned()),
        activation_reason: None,
        activation_attestation: None,
        runtime_health: None,
        bootstrap_hint: None,
        summary: None,
        tags: Vec::new(),
        input_examples: Vec::new(),
        output_examples: Vec::new(),
        deferred: false,
        loaded: false,
    }
}

#[test]
fn runtime_activation_profile_blocks_activation_errors() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.activation_status = Some("blocked_slot_claim_conflict".to_owned());
    plugin.activation_reason =
        Some("slot claim conflicts with an existing runtime owner".to_owned());
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::SlotClaimConflict,
        severity: PluginDiagnosticSeverity::Error,
        phase: PluginDiagnosticPhase::Activation,
        blocking: true,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: Some("slot_claims".to_owned()),
        message: "slot claim conflicts".to_owned(),
        remediation: Some("choose a different slot".to_owned()),
    }];

    let result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );

    assert_eq!(result.baseline_verdict, "block");
    assert_eq!(result.verdict, "block");
    assert!(!result.activation_ready);
    assert!(
        result
            .policy_flags
            .iter()
            .any(|flag| flag == "activation_blocked")
    );
    assert!(
        result
            .blocking_diagnostic_codes
            .iter()
            .any(|code| code == "slot_claim_conflict")
    );
    assert!(
        result
            .remediation_classes
            .contains(&PluginPreflightRemediationClass::ResolveSlotOwnershipConflict)
    );
}

#[test]
fn runtime_activation_profile_blocks_invalid_loaded_attestation() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.loaded = true;
    plugin.activation_attestation =
        Some(crate::spec_runtime::PluginActivationAttestationResult {
            attested: true,
            verified: false,
            integrity: "invalid".to_owned(),
            checksum: Some("deadbeefdeadbeef".to_owned()),
            computed_checksum: Some("beadfeedbeadfeed".to_owned()),
            issue: Some("plugin activation contract checksum mismatch".to_owned()),
        });

    let result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );

    assert_eq!(result.baseline_verdict, "block");
    assert_eq!(result.verdict, "block");
    assert!(
        result
            .policy_flags
            .iter()
            .any(|flag| flag == "runtime_attestation_invalid")
    );
    assert!(result.recommended_actions.iter().any(|action| {
        action.remediation_class == PluginPreflightRemediationClass::QuarantineLoadedProvider
            && action.blocking
            && action.field_path.as_deref() == Some("provider_id")
            && action
                .summary
                .contains("quarantine loaded provider `sample`")
            && action
                .operator_action
                .as_ref()
                .is_some_and(|operator_action| {
                    operator_action.surface == PluginPreflightOperatorActionSurface::HostRuntime
                        && operator_action.kind
                            == PluginPreflightOperatorActionKind::QuarantineLoadedProvider
                        && operator_action.follow_up_profile.is_none()
                        && operator_action.requires_reload
                })
    }));
    assert!(result.recommended_actions.iter().any(|action| {
        action.remediation_class == PluginPreflightRemediationClass::RepairRuntimeAttestation
            && action.blocking
            && action
                .operator_action
                .as_ref()
                .is_some_and(|operator_action| {
                    operator_action.surface == PluginPreflightOperatorActionSurface::HostRuntime
                        && operator_action.kind
                            == PluginPreflightOperatorActionKind::ReabsorbPlugin
                        && operator_action.follow_up_profile
                            == Some(PluginPreflightProfile::RuntimeActivation)
                        && operator_action.requires_reload
                })
    }));
    assert!(result.policy_summary.contains("should be quarantined"));

    let summary = build_preflight_summary(
        PluginPreflightProfile::RuntimeActivation,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "a".repeat(64),
        },
        None,
        &[result],
    );
    assert_eq!(
        summary
            .remediation_counts
            .get("quarantine_loaded_provider")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .operator_action_counts_by_surface
            .get("host_runtime")
            .copied(),
        Some(2)
    );
    assert_eq!(
        summary
            .operator_action_counts_by_kind
            .get("quarantine_loaded_provider")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .operator_action_counts_by_kind
            .get("reabsorb_plugin")
            .copied(),
        Some(1)
    );
    assert_eq!(summary.operator_actions_requiring_reload, 2);
    assert_eq!(summary.operator_actions_without_reload, 0);
    assert_eq!(summary.operator_action_plan.len(), 2);
    assert_eq!(
        summary
            .dialect_distribution
            .get("loong_package_manifest")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .compatibility_mode_distribution
            .get("native")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary.bridge_kind_distribution.get("http_json").copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .source_language_distribution
            .get("manifest")
            .copied(),
        Some(1)
    );
    assert!(
        summary
            .operator_action_plan
            .iter()
            .all(|item| item.action.action_id.len() == 64)
    );
    assert!(summary.operator_action_plan.iter().any(|item| {
        item.action.kind == PluginPreflightOperatorActionKind::QuarantineLoadedProvider
            && item.supporting_results == 1
            && item.blocked_results == 1
            && item.warned_results == 0
            && item.passed_results == 0
            && item.supporting_remediations.iter().any(|support| {
                support.remediation_class
                    == PluginPreflightRemediationClass::QuarantineLoadedProvider
                    && support.field_path.as_deref() == Some("provider_id")
            })
    }));
    assert!(summary.operator_action_plan.iter().any(|item| {
        item.action.kind == PluginPreflightOperatorActionKind::ReabsorbPlugin
            && item.supporting_results == 1
            && item.blocked_results == 1
            && item.warned_results == 0
            && item.passed_results == 0
            && item.supporting_remediations.iter().any(|support| {
                support.remediation_class
                    == PluginPreflightRemediationClass::RepairRuntimeAttestation
                    && support.field_path.as_deref()
                        == Some("provider.metadata.plugin_activation_contract_json")
            })
    }));
}

#[test]
fn sdk_release_profile_blocks_embedded_source_contract() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy.rules_for(PluginPreflightProfile::SdkRelease).clone();
    let mut plugin = sample_inventory_result();
    plugin.source_kind = "embedded_source".to_owned();
    plugin.source_path = "/tmp/sample/plugin.py".to_owned();
    plugin.package_manifest_path = None;
    plugin.source_language = Some("py".to_owned());
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::EmbeddedSourceLegacyContract,
        severity: PluginDiagnosticSeverity::Warning,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: None,
        message: "embedded source manifests remain migration-only".to_owned(),
        remediation: Some("add loong.plugin.json".to_owned()),
    }];

    let result =
        evaluate_plugin_preflight(plugin, PluginPreflightProfile::SdkRelease, &policy, &rules);

    assert_eq!(result.baseline_verdict, "block");
    assert_eq!(result.verdict, "block");
    assert!(
        result
            .policy_flags
            .iter()
            .any(|flag| flag == "embedded_source_contract")
    );
    assert!(
        result
            .remediation_classes
            .contains(&PluginPreflightRemediationClass::MigrateToPackageManifest)
    );
}

#[test]
fn marketplace_profile_is_stricter_than_sdk_release_for_shadowed_markers() {
    let policy = PluginPreflightPolicyProfile::default();
    let sdk_rules = policy.rules_for(PluginPreflightProfile::SdkRelease).clone();
    let marketplace_rules = policy
        .rules_for(PluginPreflightProfile::MarketplaceSubmission)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::ShadowedEmbeddedSource,
        severity: PluginDiagnosticSeverity::Warning,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: None,
        message: "shadowed source marker remains in package".to_owned(),
        remediation: Some("remove the shadowed marker".to_owned()),
    }];

    let sdk_result = evaluate_plugin_preflight(
        plugin.clone(),
        PluginPreflightProfile::SdkRelease,
        &policy,
        &sdk_rules,
    );
    let marketplace_result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::MarketplaceSubmission,
        &policy,
        &marketplace_rules,
    );

    assert_eq!(sdk_result.baseline_verdict, "warn");
    assert_eq!(sdk_result.verdict, "warn");
    assert_eq!(marketplace_result.baseline_verdict, "block");
    assert_eq!(marketplace_result.verdict, "block");

    let summary = build_preflight_summary(
        PluginPreflightProfile::MarketplaceSubmission,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "a".repeat(64),
        },
        None,
        &[marketplace_result],
    );
    assert_eq!(summary.blocked_plugins, 1);
    assert_eq!(summary.baseline_blocked_plugins, 1);
    assert_eq!(
        summary
            .findings_by_code
            .get("shadowed_embedded_source")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .remediation_counts
            .get("remove_shadowed_embedded_source")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .operator_action_counts_by_surface
            .get("plugin_package")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .operator_action_counts_by_kind
            .get("update_plugin_package")
            .copied(),
        Some(1)
    );
    assert_eq!(summary.operator_actions_requiring_reload, 1);
    assert_eq!(summary.operator_actions_without_reload, 0);
    assert_eq!(summary.operator_action_plan.len(), 1);
    assert_eq!(
        summary.operator_action_plan[0].action.kind,
        PluginPreflightOperatorActionKind::UpdatePluginPackage
    );
    assert_eq!(summary.operator_action_plan[0].supporting_results, 1);
    assert_eq!(summary.operator_action_plan[0].blocked_results, 1);
    assert_eq!(
        summary.operator_action_plan[0]
            .supporting_remediations
            .len(),
        2
    );
    assert!(
        summary.operator_action_plan[0]
            .supporting_remediations
            .iter()
            .any(|support| support.summary == "remove the shadowed marker")
    );
    assert_eq!(summary.policy_source, "bundled:test");
}

#[test]
fn sdk_release_blocks_legacy_openclaw_contract_but_keeps_modern_foreign_dialect_warn_only() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy.rules_for(PluginPreflightProfile::SdkRelease).clone();

    let mut modern = sample_inventory_result();
    modern.dialect = "openclaw_modern_manifest".to_owned();
    modern.dialect_version = Some("openclaw.plugin.json".to_owned());
    modern.compatibility_mode = "openclaw_modern".to_owned();
    modern.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::ForeignDialectContract,
        severity: PluginDiagnosticSeverity::Info,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(modern.plugin_id.clone()),
        source_path: Some(modern.source_path.clone()),
        source_kind: None,
        field_path: Some("dialect".to_owned()),
        message: "foreign dialect projected through compatibility boundary".to_owned(),
        remediation: None,
    }];

    let mut legacy = modern.clone();
    legacy.dialect = "openclaw_legacy_package".to_owned();
    legacy.compatibility_mode = "openclaw_legacy".to_owned();
    legacy.diagnostic_findings.push(PluginDiagnosticFinding {
        code: PluginDiagnosticCode::LegacyOpenClawContract,
        severity: PluginDiagnosticSeverity::Warning,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(legacy.plugin_id.clone()),
        source_path: Some(legacy.source_path.clone()),
        source_kind: None,
        field_path: Some("package.json#openclaw.extensions".to_owned()),
        message: "legacy package metadata remains compatibility-only".to_owned(),
        remediation: None,
    });

    let modern_result =
        evaluate_plugin_preflight(modern, PluginPreflightProfile::SdkRelease, &policy, &rules);
    let legacy_result =
        evaluate_plugin_preflight(legacy, PluginPreflightProfile::SdkRelease, &policy, &rules);

    assert_eq!(modern_result.baseline_verdict, "warn");
    assert_eq!(modern_result.verdict, "warn");
    assert!(
        modern_result
            .policy_flags
            .iter()
            .any(|flag| flag == "foreign_dialect_contract")
    );
    assert_eq!(legacy_result.baseline_verdict, "block");
    assert_eq!(legacy_result.verdict, "block");
    assert!(
        legacy_result
            .policy_flags
            .iter()
            .any(|flag| flag == "legacy_openclaw_contract")
    );
}

#[test]
fn runtime_activation_surfaces_missing_compatibility_shim_as_blocking_action() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.dialect = "openclaw_modern_manifest".to_owned();
    plugin.compatibility_mode = "openclaw_modern".to_owned();
    plugin.activation_status = Some("blocked_compatibility_mode".to_owned());
    plugin.activation_reason = Some(
        "runtime matrix does not enable the openclaw_modern compatibility shim".to_owned(),
    );
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::CompatibilityShimRequired,
        severity: PluginDiagnosticSeverity::Error,
        phase: PluginDiagnosticPhase::Activation,
        blocking: true,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: Some("compatibility_mode".to_owned()),
        message: "compatibility mode requires an explicit runtime shim".to_owned(),
        remediation: None,
    }];

    let result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );

    assert_eq!(result.baseline_verdict, "block");
    assert_eq!(result.verdict, "block");
    assert!(
        result
            .policy_flags
            .iter()
            .any(|flag| flag == "compatibility_shim_required")
    );
    assert!(result.recommended_actions.iter().any(|action| {
        action.remediation_class == PluginPreflightRemediationClass::EnableCompatibilityShim
            && action.blocking
    }));
}

#[test]
fn runtime_activation_surfaces_shim_profile_mismatch_as_distinct_blocking_action() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.dialect = "openclaw_modern_manifest".to_owned();
    plugin.compatibility_mode = "openclaw_modern".to_owned();
    plugin.activation_status = Some("blocked_compatibility_mode".to_owned());
    plugin.activation_reason = Some(
        "compatibility shim `openclaw-modern-compat` (openclaw-modern-compat) is enabled but its support profile version `openclaw-modern@1` does not support source language `javascript`".to_owned(),
    );
    plugin.compatibility_shim_support_mismatch_reasons =
        vec!["source language `javascript`".to_owned()];
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::CompatibilityShimRequired,
        severity: PluginDiagnosticSeverity::Error,
        phase: PluginDiagnosticPhase::Activation,
        blocking: true,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: Some("compatibility_mode".to_owned()),
        message: "compatibility shim profile does not support the selected runtime projection"
            .to_owned(),
        remediation: None,
    }];

    let result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );

    assert_eq!(result.baseline_verdict, "block");
    assert_eq!(result.verdict, "block");
    assert!(
        result
            .policy_flags
            .iter()
            .any(|flag| flag == "compatibility_shim_profile_mismatch")
    );
    assert!(
        !result
            .policy_flags
            .iter()
            .any(|flag| flag == "compatibility_shim_required")
    );
    assert!(result.recommended_actions.iter().any(|action| {
        action.remediation_class
            == PluginPreflightRemediationClass::AlignCompatibilityShimProfile
            && action.blocking
    }));
}

#[test]
fn policy_exceptions_waive_contract_drift_without_hiding_baseline_truth() {
    let policy = PluginPreflightPolicyProfile {
        exceptions: vec![PluginPreflightPolicyException {
            exception_id: "private-sdk-embedded-source".to_owned(),
            plugin_id: "sample-plugin".to_owned(),
            plugin_version_req: Some("<0.4.0".to_owned()),
            profiles: vec![PluginPreflightProfile::SdkRelease],
            waive_policy_flags: vec!["embedded_source_contract".to_owned()],
            waive_diagnostic_codes: vec!["embedded_source_legacy_contract".to_owned()],
            reason: "internal migration window".to_owned(),
            ticket_ref: "SEC-900".to_owned(),
            approved_by: "platform-security".to_owned(),
            expires_at: Some("2026-06-30".to_owned()),
        }],
        ..PluginPreflightPolicyProfile::default()
    };
    let rules = policy.rules_for(PluginPreflightProfile::SdkRelease).clone();
    let mut plugin = sample_inventory_result();
    plugin.source_kind = "embedded_source".to_owned();
    plugin.source_path = "/tmp/sample/plugin.py".to_owned();
    plugin.package_manifest_path = None;
    plugin.source_language = Some("py".to_owned());
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::EmbeddedSourceLegacyContract,
        severity: PluginDiagnosticSeverity::Warning,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: Some("loong.plugin.json".to_owned()),
        message: "embedded source manifests remain migration-only".to_owned(),
        remediation: Some("add loong.plugin.json".to_owned()),
    }];

    let result =
        evaluate_plugin_preflight(plugin, PluginPreflightProfile::SdkRelease, &policy, &rules);

    assert_eq!(result.baseline_verdict, "block");
    assert_eq!(result.verdict, "pass");
    assert!(result.exception_applied);
    assert!(
        result
            .waived_policy_flags
            .iter()
            .any(|flag| flag == "embedded_source_contract")
    );
    assert!(
        result
            .waived_diagnostic_codes
            .iter()
            .any(|code| code == "embedded_source_legacy_contract")
    );
    assert!(
        result.effective_advisory_diagnostic_codes.is_empty(),
        "waived advisory code should disappear from effective diagnostics"
    );
    assert_eq!(result.applied_exceptions.len(), 1);
    assert!(
        result.policy_summary.contains("exceptions applied"),
        "policy summary should explain the exception lane"
    );

    let summary = build_preflight_summary(
        PluginPreflightProfile::SdkRelease,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "b".repeat(64),
        },
        None,
        &[result],
    );
    assert_eq!(summary.baseline_blocked_plugins, 1);
    assert_eq!(summary.clean_passed_plugins, 0);
    assert_eq!(summary.waived_passed_plugins, 1);
    assert_eq!(summary.passed_plugins, 1);
    assert_eq!(summary.waived_plugins, 1);
    assert_eq!(summary.applied_exception_count, 1);
    assert_eq!(
        summary
            .waived_policy_flags
            .get("embedded_source_contract")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .waived_diagnostic_codes
            .get("embedded_source_legacy_contract")
            .copied(),
        Some(1)
    );
    assert_eq!(
        summary.exception_counts_by_ticket.get("SEC-900").copied(),
        Some(1)
    );
    assert_eq!(
        summary
            .exception_counts_by_approver
            .get("platform-security")
            .copied(),
        Some(1)
    );
}

#[test]
fn policy_exceptions_do_not_apply_when_plugin_version_misses_scope() {
    let policy = PluginPreflightPolicyProfile {
        exceptions: vec![PluginPreflightPolicyException {
            exception_id: "future-waiver".to_owned(),
            plugin_id: "sample-plugin".to_owned(),
            plugin_version_req: Some(">=1.0.0".to_owned()),
            profiles: vec![PluginPreflightProfile::SdkRelease],
            waive_policy_flags: vec!["embedded_source_contract".to_owned()],
            waive_diagnostic_codes: vec!["embedded_source_legacy_contract".to_owned()],
            reason: "future-only waiver".to_owned(),
            ticket_ref: "SEC-901".to_owned(),
            approved_by: "platform-security".to_owned(),
            expires_at: None,
        }],
        ..PluginPreflightPolicyProfile::default()
    };
    let rules = policy.rules_for(PluginPreflightProfile::SdkRelease).clone();
    let mut plugin = sample_inventory_result();
    plugin.source_kind = "embedded_source".to_owned();
    plugin.source_path = "/tmp/sample/plugin.py".to_owned();
    plugin.package_manifest_path = None;
    plugin.source_language = Some("py".to_owned());
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::EmbeddedSourceLegacyContract,
        severity: PluginDiagnosticSeverity::Warning,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: Some("loong.plugin.json".to_owned()),
        message: "embedded source manifests remain migration-only".to_owned(),
        remediation: Some("add loong.plugin.json".to_owned()),
    }];

    let result =
        evaluate_plugin_preflight(plugin, PluginPreflightProfile::SdkRelease, &policy, &rules);

    assert_eq!(result.baseline_verdict, "block");
    assert_eq!(result.verdict, "block");
    assert!(!result.exception_applied);
    assert!(result.applied_exceptions.is_empty());
    assert!(
        result
            .effective_policy_flags
            .iter()
            .any(|flag| flag == "embedded_source_contract")
    );
}

#[test]
fn build_recommended_actions_adds_generic_activation_action_when_reason_has_no_finding() {
    let mut plugin = sample_inventory_result();
    plugin.activation_status = Some("blocked_custom".to_owned());
    plugin.activation_reason = Some("runtime policy denied activation".to_owned());
    let policy_flags = BTreeSet::from(["activation_blocked".to_owned()]);

    let actions = build_recommended_actions(
        &plugin,
        &policy_flags,
        PluginPreflightProfile::RuntimeActivation,
    );

    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].remediation_class,
        PluginPreflightRemediationClass::ResolveActivationBlockers
    );
    assert!(actions[0].blocking);
    assert_eq!(
        actions[0]
            .operator_action
            .as_ref()
            .map(|action| action.kind),
        Some(PluginPreflightOperatorActionKind::ReviewDiagnostics)
    );
}

#[test]
fn build_recommended_actions_adds_generic_review_for_unmapped_advisory_findings() {
    let mut plugin = sample_inventory_result();
    plugin.diagnostic_findings = vec![PluginDiagnosticFinding {
        code: PluginDiagnosticCode::EmbeddedSourceLegacyContract,
        severity: PluginDiagnosticSeverity::Warning,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(plugin.plugin_id.clone()),
        source_path: Some(plugin.source_path.clone()),
        source_kind: None,
        field_path: None,
        message: "embedded source manifests remain migration-only".to_owned(),
        remediation: None,
    }];
    let policy_flags = BTreeSet::from(["non_blocking_diagnostics_present".to_owned()]);

    let actions =
        build_recommended_actions(&plugin, &policy_flags, PluginPreflightProfile::SdkRelease);

    assert!(actions.iter().any(|action| action.remediation_class
        == PluginPreflightRemediationClass::MigrateToPackageManifest));
    assert!(actions.iter().any(|action| {
        action.remediation_class == PluginPreflightRemediationClass::MigrateToPackageManifest
            && action
                .operator_action
                .as_ref()
                .is_some_and(|operator_action| {
                    operator_action.surface
                        == PluginPreflightOperatorActionSurface::PluginPackage
                        && operator_action.kind
                            == PluginPreflightOperatorActionKind::UpdatePluginPackage
                        && operator_action.follow_up_profile
                            == Some(PluginPreflightProfile::SdkRelease)
                })
    }));
}

#[test]
fn build_preflight_summary_groups_multiple_package_fixes_under_one_operator_action() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy.rules_for(PluginPreflightProfile::SdkRelease).clone();
    let mut plugin = sample_inventory_result();
    plugin.source_kind = "embedded_source".to_owned();
    plugin.source_path = "/tmp/sample/plugin.py".to_owned();
    plugin.package_manifest_path = None;
    plugin.source_language = Some("py".to_owned());
    plugin.diagnostic_findings = vec![
        PluginDiagnosticFinding {
            code: PluginDiagnosticCode::EmbeddedSourceLegacyContract,
            severity: PluginDiagnosticSeverity::Warning,
            phase: PluginDiagnosticPhase::Scan,
            blocking: false,
            plugin_id: Some(plugin.plugin_id.clone()),
            source_path: Some(plugin.source_path.clone()),
            source_kind: None,
            field_path: Some("loong.plugin.json".to_owned()),
            message: "embedded source manifests remain migration-only".to_owned(),
            remediation: Some("add loong.plugin.json".to_owned()),
        },
        PluginDiagnosticFinding {
            code: PluginDiagnosticCode::LegacyMetadataVersion,
            severity: PluginDiagnosticSeverity::Warning,
            phase: PluginDiagnosticPhase::Scan,
            blocking: false,
            plugin_id: Some(plugin.plugin_id.clone()),
            source_path: Some(plugin.source_path.clone()),
            source_kind: None,
            field_path: Some("metadata.version".to_owned()),
            message: "legacy metadata.version should be removed".to_owned(),
            remediation: Some("move version to the package top level".to_owned()),
        },
    ];

    let result =
        evaluate_plugin_preflight(plugin, PluginPreflightProfile::SdkRelease, &policy, &rules);
    let update_package_action_ids = result
        .recommended_actions
        .iter()
        .filter_map(|action| {
            action.operator_action.as_ref().and_then(|operator_action| {
                (operator_action.kind == PluginPreflightOperatorActionKind::UpdatePluginPackage)
                    .then_some(operator_action.action_id.clone())
            })
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(update_package_action_ids.len(), 1);

    let summary = build_preflight_summary(
        PluginPreflightProfile::SdkRelease,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "c".repeat(64),
        },
        None,
        &[result],
    );
    assert_eq!(
        summary
            .operator_action_counts_by_kind
            .get("update_plugin_package")
            .copied(),
        Some(1)
    );
    assert_eq!(summary.operator_action_plan.len(), 1);
    assert_eq!(summary.operator_action_plan[0].supporting_results, 1);
    assert_eq!(summary.operator_action_plan[0].blocked_results, 1);
    assert_eq!(
        summary.operator_action_plan[0]
            .supporting_remediations
            .iter()
            .filter(|support| {
                support.remediation_class
                    == PluginPreflightRemediationClass::MigrateToPackageManifest
            })
            .count(),
        2
    );
    assert!(
        summary.operator_action_plan[0]
            .supporting_remediations
            .iter()
            .any(|support| {
                support.remediation_class
                    == PluginPreflightRemediationClass::MigrateToPackageManifest
            })
    );
    assert_eq!(
        summary.operator_action_plan[0]
            .supporting_remediations
            .iter()
            .filter(|support| {
                support.remediation_class
                    == PluginPreflightRemediationClass::RemoveLegacyMetadataVersion
            })
            .count(),
        2
    );
    assert!(
        summary.operator_action_plan[0]
            .supporting_remediations
            .iter()
            .any(|support| {
                support.remediation_class
                    == PluginPreflightRemediationClass::RemoveLegacyMetadataVersion
            })
    );
}

#[test]
fn bridge_profile_fit_prefers_native_profile_for_native_plugin_sets() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let result = evaluate_plugin_preflight(
        sample_inventory_result(),
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );

    let summary = build_preflight_summary(
        PluginPreflightProfile::RuntimeActivation,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "d".repeat(64),
        },
        None,
        &[result],
    );

    assert_eq!(
        summary.recommended_bridge_profile.as_deref(),
        Some("native-balanced")
    );
    assert_eq!(summary.active_bridge_profile, None);
    assert_eq!(
        summary.active_bridge_profile_matches_recommended,
        Some(false)
    );
    assert_eq!(summary.bridge_profile_fits.len(), 2);
    assert!(summary.bridge_profile_fits.iter().any(|fit| {
        fit.profile_id == "native-balanced"
            && fit.fits_all_plugins
            && fit.supported_plugins == 1
            && fit.blocked_plugins == 0
    }));
    let recommendation = summary
        .bridge_profile_recommendation
        .as_ref()
        .expect("recommendation should be present");
    assert_eq!(
        recommendation.kind,
        PluginPreflightBridgeProfileRecommendationKind::AdoptBundledProfile
    );
    assert_eq!(recommendation.target_profile_id, "native-balanced");
    assert!(recommendation.delta.is_none());
}

#[test]
fn bridge_profile_fit_recommends_openclaw_profile_for_javascript_openclaw_plugins() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.dialect = "openclaw_modern_manifest".to_owned();
    plugin.compatibility_mode = "openclaw_modern".to_owned();
    plugin.bridge_kind = "process_stdio".to_owned();
    plugin.adapter_family = Some("openclaw-modern-compat".to_owned());
    plugin.source_language = Some("javascript".to_owned());
    plugin.compatibility_shim = Some(PluginCompatibilityShim {
        shim_id: "openclaw-modern-compat".to_owned(),
        family: "openclaw-modern-compat".to_owned(),
    });

    let result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );
    let active_bridge_support =
        resolve_bridge_support_policy(None, Some("openclaw-ecosystem-balanced"), None)
            .expect("bundled profile should resolve")
            .expect("bundled profile should be present");

    let summary = build_preflight_summary(
        PluginPreflightProfile::RuntimeActivation,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "e".repeat(64),
        },
        Some(&active_bridge_support.profile),
        &[result],
    );

    assert_eq!(
        summary.recommended_bridge_profile.as_deref(),
        Some("openclaw-ecosystem-balanced")
    );
    assert_eq!(
        summary.recommended_bridge_profile_source.as_deref(),
        Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
    );
    assert_eq!(
        summary.active_bridge_profile.as_deref(),
        Some("openclaw-ecosystem-balanced")
    );
    assert_eq!(
        summary.active_bridge_profile_matches_recommended,
        Some(true)
    );
    assert!(summary.bridge_profile_fits.iter().any(|fit| {
        fit.profile_id == "native-balanced"
            && !fit.fits_all_plugins
            && fit.blocked_plugins == 1
            && fit
                .blocking_reasons
                .get("unsupported_compatibility_mode")
                .copied()
                == Some(1)
    }));
    assert!(summary.bridge_profile_fits.iter().any(|fit| {
        fit.profile_id == "openclaw-ecosystem-balanced"
            && fit.fits_all_plugins
            && fit.supported_plugins == 1
            && fit.blocked_plugins == 0
    }));
    assert!(
        summary.bridge_profile_recommendation.is_none(),
        "active bundled profile already matches recommendation"
    );
}

#[test]
fn bridge_profile_fit_reports_when_no_bundled_profile_covers_python_openclaw_plugins() {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.dialect = "openclaw_modern_manifest".to_owned();
    plugin.compatibility_mode = "openclaw_modern".to_owned();
    plugin.bridge_kind = "process_stdio".to_owned();
    plugin.adapter_family = Some("openclaw-modern-compat".to_owned());
    plugin.source_language = Some("python".to_owned());
    plugin.compatibility_shim = Some(PluginCompatibilityShim {
        shim_id: "openclaw-modern-compat".to_owned(),
        family: "openclaw-modern-compat".to_owned(),
    });

    let result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );

    let summary = build_preflight_summary(
        PluginPreflightProfile::RuntimeActivation,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "f".repeat(64),
        },
        None,
        &[result],
    );

    assert_eq!(summary.recommended_bridge_profile, None);
    let recommendation = summary
        .bridge_profile_recommendation
        .as_ref()
        .expect("custom delta recommendation should be present");
    assert_eq!(
        recommendation.kind,
        PluginPreflightBridgeProfileRecommendationKind::AuthorBridgeProfileDelta
    );
    assert_eq!(
        recommendation.target_profile_id,
        "openclaw-ecosystem-balanced"
    );
    let delta = recommendation
        .delta
        .as_ref()
        .expect("delta recommendation should include required additions");
    assert!(
        delta.supported_compatibility_modes.is_empty(),
        "closest bundled profile should already support openclaw mode"
    );
    assert!(
        delta.supported_compatibility_shims.is_empty(),
        "closest bundled profile should already support the shim itself"
    );
    assert_eq!(delta.shim_profile_additions.len(), 1);
    assert_eq!(
        delta.shim_profile_additions[0].supported_source_languages,
        vec!["python".to_owned()]
    );
    assert!(summary.bridge_profile_fits.iter().any(|fit| {
        fit.profile_id == "openclaw-ecosystem-balanced"
            && !fit.fits_all_plugins
            && fit
                .blocking_reasons
                .get("shim_support_profile_mismatch")
                .copied()
                == Some(1)
            && fit.sample_blocked_plugins == vec!["sample-plugin".to_owned()]
    }));
}

#[test]
fn bridge_profile_fit_suppresses_repeat_delta_recommendation_when_active_custom_policy_already_fits()
 {
    let policy = PluginPreflightPolicyProfile::default();
    let rules = policy
        .rules_for(PluginPreflightProfile::RuntimeActivation)
        .clone();
    let mut plugin = sample_inventory_result();
    plugin.dialect = "openclaw_modern_manifest".to_owned();
    plugin.compatibility_mode = "openclaw_modern".to_owned();
    plugin.bridge_kind = "process_stdio".to_owned();
    plugin.adapter_family = Some("openclaw-modern-compat".to_owned());
    plugin.source_language = Some("python".to_owned());
    plugin.compatibility_shim = Some(PluginCompatibilityShim {
        shim_id: "openclaw-modern-compat".to_owned(),
        family: "openclaw-modern-compat".to_owned(),
    });

    let result = evaluate_plugin_preflight(
        plugin,
        PluginPreflightProfile::RuntimeActivation,
        &policy,
        &rules,
    );
    let active_bridge_support =
        super::super::bridge_support_policy::materialize_bridge_support_template(
            "openclaw-ecosystem-balanced",
            Some(&PluginPreflightBridgeProfileDelta {
                supported_bridges: Vec::new(),
                supported_adapter_families: Vec::new(),
                supported_compatibility_modes: Vec::new(),
                supported_compatibility_shims: Vec::new(),
                shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
                    shim_id: "openclaw-modern-compat".to_owned(),
                    shim_family: "openclaw-modern-compat".to_owned(),
                    supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                    supported_bridges: vec!["process_stdio".to_owned()],
                    supported_adapter_families: vec!["openclaw-modern-compat".to_owned()],
                    supported_source_languages: vec!["python".to_owned()],
                }],
                unresolved_blocking_reasons: Vec::new(),
            }),
        )
        .expect("active custom bridge support should materialize");

    let summary = build_preflight_summary(
        PluginPreflightProfile::RuntimeActivation,
        &super::super::plugin_preflight_policy::ResolvedPluginPreflightPolicy {
            profile: policy,
            source: "bundled:test".to_owned(),
            checksum: "checksum".to_owned(),
            sha256: "g".repeat(64),
        },
        Some(&active_bridge_support.profile),
        &[result],
    );

    assert_eq!(summary.recommended_bridge_profile, None);
    assert_eq!(summary.active_bridge_profile, None);
    assert_eq!(summary.active_bridge_support_fits_all_plugins, Some(true));
    assert!(
        summary.bridge_profile_recommendation.is_none(),
        "active custom bridge support should suppress repeat delta recommendation"
    );
}

#[test]
fn format_applied_exception_summary_mentions_expiry() {
    let summary = format_applied_exception_summary(&PluginPreflightAppliedException {
        exception_id: "legacy".to_owned(),
        plugin_version_req: Some("<0.4.0".to_owned()),
        reason: "internal rollout".to_owned(),
        ticket_ref: "SEC-902".to_owned(),
        approved_by: "platform-security".to_owned(),
        expires_at: Some("2026-06-30".to_owned()),
        waived_policy_flags: vec!["legacy_metadata_version".to_owned()],
        waived_diagnostic_codes: Vec::new(),
    });

    assert!(summary.contains("until 2026-06-30"));
    assert!(summary.contains("SEC-902"));
    assert!(summary.contains("platform-security"));
    assert!(summary.contains("<0.4.0"));
}

#[test]
fn remediation_mapping_covers_all_kernel_diagnostic_codes() {
    let cases = [
        (
            PluginDiagnosticCode::EmbeddedSourceLegacyContract,
            PluginPreflightRemediationClass::MigrateToPackageManifest,
        ),
        (
            PluginDiagnosticCode::ForeignDialectContract,
            PluginPreflightRemediationClass::MigrateForeignDialect,
        ),
        (
            PluginDiagnosticCode::LegacyOpenClawContract,
            PluginPreflightRemediationClass::ModernizeLegacyOpenClawContract,
        ),
        (
            PluginDiagnosticCode::CompatibilityShimRequired,
            PluginPreflightRemediationClass::EnableCompatibilityShim,
        ),
        (
            PluginDiagnosticCode::LegacyMetadataVersion,
            PluginPreflightRemediationClass::RemoveLegacyMetadataVersion,
        ),
        (
            PluginDiagnosticCode::ShadowedEmbeddedSource,
            PluginPreflightRemediationClass::RemoveShadowedEmbeddedSource,
        ),
        (
            PluginDiagnosticCode::IncompatibleHost,
            PluginPreflightRemediationClass::ResolveHostCompatibility,
        ),
        (
            PluginDiagnosticCode::UnsupportedBridge,
            PluginPreflightRemediationClass::SwitchSupportedBridge,
        ),
        (
            PluginDiagnosticCode::UnsupportedAdapterFamily,
            PluginPreflightRemediationClass::SwitchSupportedAdapterFamily,
        ),
        (
            PluginDiagnosticCode::SlotClaimConflict,
            PluginPreflightRemediationClass::ResolveSlotOwnershipConflict,
        ),
    ];

    for (diagnostic, expected_class) in cases {
        assert_eq!(remediation_class_for_diagnostic(diagnostic), expected_class);
    }
    assert_eq!(
        PluginPreflightRemediationClass::AlignCompatibilityShimProfile.as_str(),
        "align_compatibility_shim_profile"
    );
}
