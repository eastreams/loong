use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    BridgeSupportMatrix, IntegrationCatalog, PluginActivationPlan, PluginBridgeKind,
    PluginCompatibilityMode, PluginCompatibilityShim, PluginContractDialect, PluginDiagnosticCode,
    PluginDiagnosticSeverity, PluginIR, PluginRuntimeProfile, PluginScanReport, PluginSourceKind,
    PluginTranslationReport,
};
use semver::{Version, VersionReq};

use super::bridge_runtime_policy_support::bridge_support_spec_matrix;
use super::bridge_support_policy::{
    BUNDLED_BRIDGE_SUPPORT_PROFILE_IDS, bridge_support_policy_sha256, resolve_bridge_support_policy,
};
use super::plugin_inventory::collect_plugin_inventory_results;
use super::plugin_preflight_policy::resolve_plugin_preflight_policy;
use crate::spec_runtime::{
    BridgeSupportSpec, PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_PURPOSE,
    PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_SURFACE, PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION,
    PluginInventoryResult, PluginPreflightAppliedException, PluginPreflightBridgeProfileDelta,
    PluginPreflightBridgeProfileFit, PluginPreflightBridgeProfileRecommendation,
    PluginPreflightBridgeProfileRecommendationKind, PluginPreflightBridgeShimProfileDelta,
    PluginPreflightOperatorAction, PluginPreflightOperatorActionKind,
    PluginPreflightOperatorActionPlanItem, PluginPreflightOperatorActionSupport,
    PluginPreflightOperatorActionSurface, PluginPreflightPolicyException,
    PluginPreflightPolicyProfile, PluginPreflightProfile, PluginPreflightRecommendedAction,
    PluginPreflightRemediationClass, PluginPreflightResult, PluginPreflightRuleProfile,
    PluginPreflightSummary, PluginPreflightVerdict, SecurityProfileSignatureSpec,
    default_runtime_adapter_family, json_schema_descriptor, normalize_runtime_source_language,
    parse_bridge_kind_label, parse_plugin_activation_runtime_dialect,
    parse_plugin_activation_runtime_mode, plugin_preflight_operator_action_sha256,
};

pub(super) struct PluginPreflightExecutionReport {
    pub summary: PluginPreflightSummary,
    pub results: Vec<PluginPreflightResult>,
}

struct PluginPreflightEffectiveState {
    applied_exceptions: Vec<PluginPreflightAppliedException>,
    effective_policy_flags: BTreeSet<String>,
    waived_policy_flags: BTreeSet<String>,
    effective_blocking_diagnostic_codes: BTreeSet<String>,
    effective_advisory_diagnostic_codes: BTreeSet<String>,
    waived_diagnostic_codes: BTreeSet<String>,
}

struct OperatorActionPlanAccumulator {
    item: PluginPreflightOperatorActionPlanItem,
    seen_supporting_remediations: BTreeSet<(String, bool, Option<String>, Option<String>, String)>,
}

struct BridgeProfileFitAnalysis {
    active_bridge_profile: Option<String>,
    recommended_bridge_profile: Option<String>,
    recommended_bridge_profile_source: Option<String>,
    active_bridge_profile_matches_recommended: Option<bool>,
    active_bridge_support_fits_all_plugins: Option<bool>,
    bridge_profile_fits: Vec<PluginPreflightBridgeProfileFit>,
    bridge_profile_recommendation: Option<PluginPreflightBridgeProfileRecommendation>,
}

#[derive(Default)]
struct BridgeProfileDeltaAccumulator {
    supported_bridges: BTreeSet<String>,
    supported_adapter_families: BTreeSet<String>,
    supported_compatibility_modes: BTreeSet<String>,
    supported_compatibility_shims: BTreeSet<String>,
    shim_profile_additions: BTreeMap<(String, String), BridgeShimProfileDeltaAccumulator>,
    unresolved_blocking_reasons: BTreeSet<String>,
}

#[derive(Default)]
struct BridgeShimProfileDeltaAccumulator {
    supported_dialects: BTreeSet<String>,
    supported_bridges: BTreeSet<String>,
    supported_adapter_families: BTreeSet<String>,
    supported_source_languages: BTreeSet<String>,
}

struct BridgeProfilePluginFitEvaluation {
    blocking_reasons: Vec<String>,
    delta: BridgeProfileDeltaAccumulator,
}

struct BridgeProfileFitCandidate {
    fit: PluginPreflightBridgeProfileFit,
    delta: PluginPreflightBridgeProfileDelta,
}

pub(super) fn execute_plugin_preflight(
    integration_catalog: &IntegrationCatalog,
    plugin_scan_reports: &[PluginScanReport],
    plugin_translation_reports: &[PluginTranslationReport],
    plugin_activation_plans: &[PluginActivationPlan],
    active_bridge_support: Option<&BridgeSupportSpec>,
    query: &str,
    limit: usize,
    profile: PluginPreflightProfile,
    policy_path: Option<&str>,
    policy_sha256: Option<&str>,
    policy_signature: Option<&SecurityProfileSignatureSpec>,
    include_passed: bool,
    include_warned: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
) -> Result<PluginPreflightExecutionReport, String> {
    let resolved_policy =
        resolve_plugin_preflight_policy(policy_path, policy_sha256, policy_signature)?;
    let active_rules = resolved_policy.profile.rules_for(profile).clone();
    let inventory_results = collect_plugin_inventory_results(
        integration_catalog,
        plugin_scan_reports,
        plugin_translation_reports,
        plugin_activation_plans,
        query,
        true,
        true,
        include_deferred,
        include_examples,
    );

    let mut matched = inventory_results
        .into_iter()
        .enumerate()
        .filter_map(|(index, plugin)| {
            let result =
                evaluate_plugin_preflight(plugin, profile, &resolved_policy.profile, &active_rules);
            if (!include_passed && result.verdict == PluginPreflightVerdict::Pass.as_str())
                || (!include_warned && result.verdict == PluginPreflightVerdict::Warn.as_str())
                || (!include_blocked && result.verdict == PluginPreflightVerdict::Block.as_str())
            {
                None
            } else {
                Some((index, result))
            }
        })
        .collect::<Vec<_>>();

    matched.sort_by(|(left_index, left), (right_index, right)| {
        preflight_verdict_rank(right.verdict.as_str())
            .cmp(&preflight_verdict_rank(left.verdict.as_str()))
            .then_with(|| left_index.cmp(right_index))
    });

    let matched_plugins = matched.len();
    let matched_results = matched
        .iter()
        .map(|(_, result)| result.clone())
        .collect::<Vec<_>>();
    let returned_limit = limit.clamp(1, 500);
    let results = matched
        .into_iter()
        .take(returned_limit)
        .map(|(_, result)| result)
        .collect::<Vec<_>>();

    let mut summary = build_preflight_summary(
        profile,
        &resolved_policy,
        active_bridge_support,
        &matched_results,
    );
    summary.matched_plugins = matched_plugins;
    summary.returned_plugins = results.len();
    summary.truncated = matched_plugins > results.len();

    Ok(PluginPreflightExecutionReport { summary, results })
}

fn evaluate_plugin_preflight(
    plugin: PluginInventoryResult,
    profile: PluginPreflightProfile,
    policy: &PluginPreflightPolicyProfile,
    rules: &PluginPreflightRuleProfile,
) -> PluginPreflightResult {
    let activation_ready = plugin
        .activation_status
        .as_deref()
        .is_none_or(|status| status == "ready");

    let mut blocking_diagnostic_codes = BTreeSet::new();
    let mut advisory_diagnostic_codes = BTreeSet::new();
    for finding in &plugin.diagnostic_findings {
        if finding.blocking {
            blocking_diagnostic_codes.insert(finding.code.as_str().to_owned());
        } else {
            advisory_diagnostic_codes.insert(finding.code.as_str().to_owned());
        }
    }

    let mut policy_flags = BTreeSet::new();
    if !activation_ready {
        policy_flags.insert("activation_blocked".to_owned());
    }
    if plugin
        .activation_attestation
        .as_ref()
        .is_some_and(|attestation| !attestation.verified)
    {
        policy_flags.insert("runtime_attestation_invalid".to_owned());
    }
    if !blocking_diagnostic_codes.is_empty() {
        policy_flags.insert("blocking_diagnostics_present".to_owned());
    }
    if !advisory_diagnostic_codes.is_empty() {
        policy_flags.insert("non_blocking_diagnostics_present".to_owned());
    }
    if matches!(plugin.source_kind.as_str(), "embedded_source")
        || has_diagnostic_code(&plugin, PluginDiagnosticCode::EmbeddedSourceLegacyContract)
    {
        policy_flags.insert("embedded_source_contract".to_owned());
    }
    if has_diagnostic_code(&plugin, PluginDiagnosticCode::LegacyMetadataVersion) {
        policy_flags.insert("legacy_metadata_version".to_owned());
    }
    if has_diagnostic_code(&plugin, PluginDiagnosticCode::ShadowedEmbeddedSource) {
        policy_flags.insert("shadowed_embedded_source".to_owned());
    }
    if plugin.compatibility_mode != "native"
        || has_diagnostic_code(&plugin, PluginDiagnosticCode::ForeignDialectContract)
    {
        policy_flags.insert("foreign_dialect_contract".to_owned());
    }
    if plugin.compatibility_mode == "openclaw_legacy"
        || plugin.dialect == "openclaw_legacy_package"
        || has_diagnostic_code(&plugin, PluginDiagnosticCode::LegacyOpenClawContract)
    {
        policy_flags.insert("legacy_openclaw_contract".to_owned());
    }
    let compatibility_shim_profile_mismatch = has_compatibility_shim_profile_mismatch(&plugin);
    if compatibility_shim_profile_mismatch {
        policy_flags.insert("compatibility_shim_profile_mismatch".to_owned());
    }
    let activation_blocked_for_compatibility = plugin
        .activation_status
        .as_deref()
        .is_some_and(|status| status == "blocked_compatibility_mode");
    let shim_required_diagnostic =
        has_diagnostic_code(&plugin, PluginDiagnosticCode::CompatibilityShimRequired);
    let compatibility_shim_required = (activation_blocked_for_compatibility
        || shim_required_diagnostic)
        && !compatibility_shim_profile_mismatch;
    if compatibility_shim_required {
        policy_flags.insert("compatibility_shim_required".to_owned());
    }

    let recommended_actions = build_recommended_actions(&plugin, &policy_flags, profile);
    let remediation_classes = recommended_actions
        .iter()
        .map(|action| action.remediation_class)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let baseline_verdict = resolve_preflight_verdict(
        &policy_flags,
        &blocking_diagnostic_codes,
        &advisory_diagnostic_codes,
        rules,
    );
    let effective_state = apply_policy_exceptions(
        policy,
        profile,
        &plugin,
        &policy_flags,
        &blocking_diagnostic_codes,
        &advisory_diagnostic_codes,
    );
    let verdict = resolve_preflight_verdict(
        &effective_state.effective_policy_flags,
        &effective_state.effective_blocking_diagnostic_codes,
        &effective_state.effective_advisory_diagnostic_codes,
        rules,
    );
    let policy_summary = build_policy_summary(
        &effective_state.effective_policy_flags,
        &effective_state.effective_blocking_diagnostic_codes,
        &effective_state.effective_advisory_diagnostic_codes,
        &effective_state.applied_exceptions,
        profile,
        policy,
        rules,
        baseline_verdict,
        verdict,
    );

    PluginPreflightResult {
        profile: profile.as_str().to_owned(),
        baseline_verdict: baseline_verdict.as_str().to_owned(),
        verdict: verdict.as_str().to_owned(),
        exception_applied: !effective_state.applied_exceptions.is_empty(),
        activation_ready,
        policy_flags: policy_flags.iter().cloned().collect(),
        effective_policy_flags: effective_state
            .effective_policy_flags
            .iter()
            .cloned()
            .collect(),
        waived_policy_flags: effective_state
            .waived_policy_flags
            .iter()
            .cloned()
            .collect(),
        policy_summary,
        blocking_diagnostic_codes: blocking_diagnostic_codes.iter().cloned().collect(),
        advisory_diagnostic_codes: advisory_diagnostic_codes.iter().cloned().collect(),
        effective_blocking_diagnostic_codes: effective_state
            .effective_blocking_diagnostic_codes
            .iter()
            .cloned()
            .collect(),
        effective_advisory_diagnostic_codes: effective_state
            .effective_advisory_diagnostic_codes
            .iter()
            .cloned()
            .collect(),
        waived_diagnostic_codes: effective_state
            .waived_diagnostic_codes
            .iter()
            .cloned()
            .collect(),
        applied_exceptions: effective_state.applied_exceptions,
        remediation_classes,
        recommended_actions,
        plugin,
    }
}

fn apply_policy_exceptions(
    policy: &PluginPreflightPolicyProfile,
    profile: PluginPreflightProfile,
    plugin: &PluginInventoryResult,
    policy_flags: &BTreeSet<String>,
    blocking_diagnostic_codes: &BTreeSet<String>,
    advisory_diagnostic_codes: &BTreeSet<String>,
) -> PluginPreflightEffectiveState {
    let mut effective_policy_flags = policy_flags.clone();
    let mut effective_blocking_diagnostic_codes = blocking_diagnostic_codes.clone();
    let mut effective_advisory_diagnostic_codes = advisory_diagnostic_codes.clone();
    let mut waived_policy_flags = BTreeSet::new();
    let mut waived_diagnostic_codes = BTreeSet::new();
    let mut applied_exceptions = Vec::new();

    for exception in applicable_policy_exceptions(
        policy,
        profile,
        plugin.plugin_id.as_str(),
        plugin.plugin_version.as_deref(),
    ) {
        let mut waived_flags_for_exception = Vec::new();
        let mut waived_codes_for_exception = Vec::new();

        for flag in &exception.waive_policy_flags {
            if effective_policy_flags.remove(flag) {
                waived_policy_flags.insert(flag.clone());
                waived_flags_for_exception.push(flag.clone());
            }
        }

        for code in &exception.waive_diagnostic_codes {
            let removed_blocking = effective_blocking_diagnostic_codes.remove(code);
            let removed_advisory = effective_advisory_diagnostic_codes.remove(code);
            if removed_blocking || removed_advisory {
                waived_diagnostic_codes.insert(code.clone());
                waived_codes_for_exception.push(code.clone());
            }
        }

        if !waived_flags_for_exception.is_empty() || !waived_codes_for_exception.is_empty() {
            applied_exceptions.push(PluginPreflightAppliedException {
                exception_id: exception.exception_id.clone(),
                plugin_version_req: exception.plugin_version_req.clone(),
                reason: exception.reason.clone(),
                ticket_ref: exception.ticket_ref.clone(),
                approved_by: exception.approved_by.clone(),
                expires_at: exception.expires_at.clone(),
                waived_policy_flags: waived_flags_for_exception,
                waived_diagnostic_codes: waived_codes_for_exception,
            });
        }
    }

    if effective_blocking_diagnostic_codes.is_empty() {
        effective_policy_flags.remove("blocking_diagnostics_present");
    }
    if effective_advisory_diagnostic_codes.is_empty() {
        effective_policy_flags.remove("non_blocking_diagnostics_present");
    }

    PluginPreflightEffectiveState {
        applied_exceptions,
        effective_policy_flags,
        waived_policy_flags,
        effective_blocking_diagnostic_codes,
        effective_advisory_diagnostic_codes,
        waived_diagnostic_codes,
    }
}

fn applicable_policy_exceptions<'a>(
    policy: &'a PluginPreflightPolicyProfile,
    profile: PluginPreflightProfile,
    plugin_id: &str,
    plugin_version: Option<&str>,
) -> impl Iterator<Item = &'a PluginPreflightPolicyException> {
    policy.exceptions.iter().filter(move |exception| {
        exception.plugin_id == plugin_id
            && (exception.profiles.is_empty() || exception.profiles.contains(&profile))
            && plugin_preflight_exception_matches_version(exception, plugin_version)
    })
}

fn plugin_preflight_exception_matches_version(
    exception: &PluginPreflightPolicyException,
    plugin_version: Option<&str>,
) -> bool {
    let Some(version_req) = exception.plugin_version_req.as_deref() else {
        return true;
    };
    let Some(plugin_version) = plugin_version else {
        return false;
    };

    let Ok(parsed_req) = VersionReq::parse(version_req) else {
        return false;
    };
    let Ok(parsed_version) = Version::parse(plugin_version) else {
        return false;
    };
    parsed_req.matches(&parsed_version)
}

fn build_recommended_action(
    plugin: &PluginInventoryResult,
    profile: PluginPreflightProfile,
    remediation_class: PluginPreflightRemediationClass,
    diagnostic_code: Option<String>,
    field_path: Option<String>,
    blocking: bool,
    summary: String,
) -> PluginPreflightRecommendedAction {
    PluginPreflightRecommendedAction {
        remediation_class,
        diagnostic_code,
        field_path,
        blocking,
        summary,
        operator_action: build_operator_action_for_remediation(plugin, profile, remediation_class),
    }
}

fn build_operator_action_for_remediation(
    plugin: &PluginInventoryResult,
    profile: PluginPreflightProfile,
    remediation_class: PluginPreflightRemediationClass,
) -> Option<PluginPreflightOperatorAction> {
    let (surface, kind, follow_up_profile, requires_reload) = match remediation_class {
        PluginPreflightRemediationClass::QuarantineLoadedProvider => (
            PluginPreflightOperatorActionSurface::HostRuntime,
            PluginPreflightOperatorActionKind::QuarantineLoadedProvider,
            None,
            true,
        ),
        PluginPreflightRemediationClass::RepairRuntimeAttestation => (
            PluginPreflightOperatorActionSurface::HostRuntime,
            PluginPreflightOperatorActionKind::ReabsorbPlugin,
            Some(PluginPreflightProfile::RuntimeActivation),
            true,
        ),
        PluginPreflightRemediationClass::EnableCompatibilityShim
        | PluginPreflightRemediationClass::AlignCompatibilityShimProfile
        | PluginPreflightRemediationClass::SwitchSupportedBridge
        | PluginPreflightRemediationClass::SwitchSupportedAdapterFamily => (
            PluginPreflightOperatorActionSurface::BridgePolicy,
            PluginPreflightOperatorActionKind::UpdateBridgeSupportPolicy,
            Some(PluginPreflightProfile::RuntimeActivation),
            true,
        ),
        PluginPreflightRemediationClass::MigrateToPackageManifest
        | PluginPreflightRemediationClass::MigrateForeignDialect
        | PluginPreflightRemediationClass::ModernizeLegacyOpenClawContract
        | PluginPreflightRemediationClass::RemoveLegacyMetadataVersion
        | PluginPreflightRemediationClass::RemoveShadowedEmbeddedSource
        | PluginPreflightRemediationClass::ResolveHostCompatibility => (
            PluginPreflightOperatorActionSurface::PluginPackage,
            PluginPreflightOperatorActionKind::UpdatePluginPackage,
            Some(profile),
            true,
        ),
        PluginPreflightRemediationClass::ResolveSlotOwnershipConflict => (
            PluginPreflightOperatorActionSurface::PluginPackage,
            PluginPreflightOperatorActionKind::ResolveSlotOwnership,
            Some(profile),
            true,
        ),
        PluginPreflightRemediationClass::ResolveActivationBlockers
        | PluginPreflightRemediationClass::ReviewAdvisoryDiagnostics => (
            PluginPreflightOperatorActionSurface::OperatorReview,
            PluginPreflightOperatorActionKind::ReviewDiagnostics,
            Some(profile),
            false,
        ),
    };

    let mut action = PluginPreflightOperatorAction {
        action_id: String::new(),
        surface,
        kind,
        target_plugin_id: plugin.plugin_id.clone(),
        target_provider_id: Some(plugin.provider_id.clone()),
        target_source_path: plugin.source_path.clone(),
        target_manifest_path: plugin.package_manifest_path.clone(),
        follow_up_profile,
        requires_reload,
    };
    action.action_id = plugin_preflight_operator_action_sha256(&action);
    Some(action)
}

fn build_recommended_actions(
    plugin: &PluginInventoryResult,
    policy_flags: &BTreeSet<String>,
    profile: PluginPreflightProfile,
) -> Vec<PluginPreflightRecommendedAction> {
    let mut actions = Vec::new();
    let mut seen = BTreeSet::new();

    for finding in &plugin.diagnostic_findings {
        let class = remediation_class_for_diagnostic(finding.code);
        let summary = finding
            .remediation
            .clone()
            .unwrap_or_else(|| default_remediation_summary(class));
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                class,
                Some(finding.code.as_str().to_owned()),
                finding.field_path.clone(),
                finding.blocking,
                summary,
            ),
        );
    }

    if policy_flags.contains("embedded_source_contract") {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::MigrateToPackageManifest,
                Some(
                    PluginDiagnosticCode::EmbeddedSourceLegacyContract
                        .as_str()
                        .to_owned(),
                ),
                Some("loong.plugin.json".to_owned()),
                false,
                default_remediation_summary(
                    PluginPreflightRemediationClass::MigrateToPackageManifest,
                ),
            ),
        );
    }
    if policy_flags.contains("legacy_metadata_version") {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::RemoveLegacyMetadataVersion,
                Some(
                    PluginDiagnosticCode::LegacyMetadataVersion
                        .as_str()
                        .to_owned(),
                ),
                Some("metadata.version".to_owned()),
                false,
                default_remediation_summary(
                    PluginPreflightRemediationClass::RemoveLegacyMetadataVersion,
                ),
            ),
        );
    }
    if policy_flags.contains("shadowed_embedded_source") {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::RemoveShadowedEmbeddedSource,
                Some(
                    PluginDiagnosticCode::ShadowedEmbeddedSource
                        .as_str()
                        .to_owned(),
                ),
                Some("metadata.legacy_source".to_owned()),
                false,
                default_remediation_summary(
                    PluginPreflightRemediationClass::RemoveShadowedEmbeddedSource,
                ),
            ),
        );
    }
    if policy_flags.contains("foreign_dialect_contract") {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::MigrateForeignDialect,
                Some(
                    PluginDiagnosticCode::ForeignDialectContract
                        .as_str()
                        .to_owned(),
                ),
                Some("dialect".to_owned()),
                false,
                default_remediation_summary(PluginPreflightRemediationClass::MigrateForeignDialect),
            ),
        );
    }
    if policy_flags.contains("legacy_openclaw_contract") {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::ModernizeLegacyOpenClawContract,
                Some(
                    PluginDiagnosticCode::LegacyOpenClawContract
                        .as_str()
                        .to_owned(),
                ),
                Some("package.json#openclaw.extensions".to_owned()),
                false,
                default_remediation_summary(
                    PluginPreflightRemediationClass::ModernizeLegacyOpenClawContract,
                ),
            ),
        );
    }
    if policy_flags.contains("compatibility_shim_required") {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::EnableCompatibilityShim,
                Some(
                    PluginDiagnosticCode::CompatibilityShimRequired
                        .as_str()
                        .to_owned(),
                ),
                Some("compatibility_mode".to_owned()),
                true,
                plugin.activation_reason.clone().unwrap_or_else(|| {
                    default_remediation_summary(
                        PluginPreflightRemediationClass::EnableCompatibilityShim,
                    )
                }),
            ),
        );
    }
    if policy_flags.contains("compatibility_shim_profile_mismatch") {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::AlignCompatibilityShimProfile,
                Some(
                    PluginDiagnosticCode::CompatibilityShimRequired
                        .as_str()
                        .to_owned(),
                ),
                Some("bridge_support.supported_compatibility_shim_profiles".to_owned()),
                true,
                plugin.activation_reason.clone().unwrap_or_else(|| {
                    default_remediation_summary(
                        PluginPreflightRemediationClass::AlignCompatibilityShimProfile,
                    )
                }),
            ),
        );
    }
    if policy_flags.contains("runtime_attestation_invalid") {
        if plugin.loaded {
            push_recommended_action(
                &mut actions,
                &mut seen,
                build_recommended_action(
                    plugin,
                    profile,
                    PluginPreflightRemediationClass::QuarantineLoadedProvider,
                    None,
                    Some("provider_id".to_owned()),
                    true,
                    format!(
                        "quarantine loaded provider `{}` from the active catalog until activation attestation is repaired",
                        plugin.provider_id
                    ),
                ),
            );
        }
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::RepairRuntimeAttestation,
                None,
                Some("provider.metadata.plugin_activation_contract_json".to_owned()),
                true,
                plugin
                    .activation_attestation
                    .as_ref()
                    .and_then(|attestation| attestation.issue.clone())
                    .unwrap_or_else(|| {
                        default_remediation_summary(
                            PluginPreflightRemediationClass::RepairRuntimeAttestation,
                        )
                    }),
            ),
        );
    }

    let has_blocking_action = actions.iter().any(|action| action.blocking);
    if policy_flags.contains("activation_blocked") && !has_blocking_action {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::ResolveActivationBlockers,
                None,
                None,
                true,
                plugin.activation_reason.clone().unwrap_or_else(|| {
                    default_remediation_summary(
                        PluginPreflightRemediationClass::ResolveActivationBlockers,
                    )
                }),
            ),
        );
    }

    let has_advisory_action = actions.iter().any(|action| !action.blocking);
    if policy_flags.contains("non_blocking_diagnostics_present") && !has_advisory_action {
        push_recommended_action(
            &mut actions,
            &mut seen,
            build_recommended_action(
                plugin,
                profile,
                PluginPreflightRemediationClass::ReviewAdvisoryDiagnostics,
                None,
                None,
                false,
                default_remediation_summary(
                    PluginPreflightRemediationClass::ReviewAdvisoryDiagnostics,
                ),
            ),
        );
    }

    actions.sort_by(|left, right| {
        left.remediation_class
            .as_str()
            .cmp(right.remediation_class.as_str())
            .then_with(|| left.blocking.cmp(&right.blocking).reverse())
            .then_with(|| left.diagnostic_code.cmp(&right.diagnostic_code))
            .then_with(|| left.field_path.cmp(&right.field_path))
            .then_with(|| left.summary.cmp(&right.summary))
    });
    actions
}

fn push_recommended_action(
    actions: &mut Vec<PluginPreflightRecommendedAction>,
    seen: &mut BTreeSet<(
        String,
        bool,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
    )>,
    action: PluginPreflightRecommendedAction,
) {
    let operator_action_id = action
        .operator_action
        .as_ref()
        .map(|operator_action| operator_action.action_id.clone());
    let signature = (
        action.remediation_class.as_str().to_owned(),
        action.blocking,
        action.diagnostic_code.clone(),
        action.field_path.clone(),
        action.summary.clone(),
        operator_action_id,
    );
    if seen.insert(signature) {
        actions.push(action);
    }
}

fn sort_operator_action_supports(remediations: &mut [PluginPreflightOperatorActionSupport]) {
    remediations.sort_by(|left, right| {
        left.remediation_class
            .as_str()
            .cmp(right.remediation_class.as_str())
            .then_with(|| left.blocking.cmp(&right.blocking).reverse())
            .then_with(|| left.diagnostic_code.cmp(&right.diagnostic_code))
            .then_with(|| left.field_path.cmp(&right.field_path))
            .then_with(|| left.summary.cmp(&right.summary))
    });
}

fn sort_operator_action_plan(plan: &mut [PluginPreflightOperatorActionPlanItem]) {
    for item in plan.iter_mut() {
        sort_operator_action_supports(&mut item.supporting_remediations);
    }

    plan.sort_by(|left, right| {
        left.action
            .surface
            .as_str()
            .cmp(right.action.surface.as_str())
            .then_with(|| left.action.kind.as_str().cmp(right.action.kind.as_str()))
            .then_with(|| {
                left.action
                    .target_plugin_id
                    .cmp(&right.action.target_plugin_id)
            })
            .then_with(|| {
                left.action
                    .target_provider_id
                    .cmp(&right.action.target_provider_id)
            })
            .then_with(|| {
                left.action
                    .target_source_path
                    .cmp(&right.action.target_source_path)
            })
            .then_with(|| {
                left.action
                    .target_manifest_path
                    .cmp(&right.action.target_manifest_path)
            })
            .then_with(|| {
                left.action
                    .follow_up_profile
                    .map(PluginPreflightProfile::as_str)
                    .cmp(
                        &right
                            .action
                            .follow_up_profile
                            .map(PluginPreflightProfile::as_str),
                    )
            })
            .then_with(|| {
                right
                    .action
                    .requires_reload
                    .cmp(&left.action.requires_reload)
            })
            .then_with(|| left.action.action_id.cmp(&right.action.action_id))
    });
}

fn remediation_class_for_diagnostic(
    diagnostic: PluginDiagnosticCode,
) -> PluginPreflightRemediationClass {
    match diagnostic {
        PluginDiagnosticCode::EmbeddedSourceLegacyContract => {
            PluginPreflightRemediationClass::MigrateToPackageManifest
        }
        PluginDiagnosticCode::ForeignDialectContract => {
            PluginPreflightRemediationClass::MigrateForeignDialect
        }
        PluginDiagnosticCode::LegacyOpenClawContract => {
            PluginPreflightRemediationClass::ModernizeLegacyOpenClawContract
        }
        PluginDiagnosticCode::InvalidManifestContract => {
            PluginPreflightRemediationClass::ResolveActivationBlockers
        }
        PluginDiagnosticCode::CompatibilityShimRequired => {
            PluginPreflightRemediationClass::EnableCompatibilityShim
        }
        PluginDiagnosticCode::LegacyMetadataVersion => {
            PluginPreflightRemediationClass::RemoveLegacyMetadataVersion
        }
        PluginDiagnosticCode::ShadowedEmbeddedSource => {
            PluginPreflightRemediationClass::RemoveShadowedEmbeddedSource
        }
        PluginDiagnosticCode::IncompatibleHost => {
            PluginPreflightRemediationClass::ResolveHostCompatibility
        }
        PluginDiagnosticCode::UnsupportedBridge => {
            PluginPreflightRemediationClass::SwitchSupportedBridge
        }
        PluginDiagnosticCode::UnsupportedAdapterFamily => {
            PluginPreflightRemediationClass::SwitchSupportedAdapterFamily
        }
        PluginDiagnosticCode::SlotClaimConflict => {
            PluginPreflightRemediationClass::ResolveSlotOwnershipConflict
        }
    }
}

fn default_remediation_summary(remediation_class: PluginPreflightRemediationClass) -> String {
    match remediation_class {
        PluginPreflightRemediationClass::MigrateToPackageManifest => {
            "publish a `loong.plugin.json` package manifest and keep embedded source markers only as a migration bridge".to_owned()
        }
        PluginPreflightRemediationClass::MigrateForeignDialect => {
            "keep foreign plugin dialect intake at the compatibility boundary, or publish a native `loong.plugin.json` contract for first-class Loong SDK support".to_owned()
        }
        PluginPreflightRemediationClass::ModernizeLegacyOpenClawContract => {
            "replace `package.json#openclaw.extensions` with a modern `openclaw.plugin.json` contract, or migrate fully to native `loong.plugin.json` packaging".to_owned()
        }
        PluginPreflightRemediationClass::EnableCompatibilityShim => {
            "enable the runtime compatibility shim for this foreign plugin dialect explicitly, or migrate the plugin to a native contract before activation".to_owned()
        }
        PluginPreflightRemediationClass::AlignCompatibilityShimProfile => {
            "align the enabled compatibility shim support profile with this plugin's dialect, bridge, adapter family, and source-language projection before activation".to_owned()
        }
        PluginPreflightRemediationClass::QuarantineLoadedProvider => {
            "quarantine the currently loaded provider from the active catalog until activation attestation has been repaired".to_owned()
        }
        PluginPreflightRemediationClass::RepairRuntimeAttestation => {
            "re-absorb or re-register the plugin so provider metadata carries a valid activation attestation contract before runtime activation".to_owned()
        }
        PluginPreflightRemediationClass::RemoveLegacyMetadataVersion => {
            "move plugin version to top-level `version` and remove legacy `metadata.version`".to_owned()
        }
        PluginPreflightRemediationClass::RemoveShadowedEmbeddedSource => {
            "remove shadowed embedded-source markers once the package manifest is authoritative"
                .to_owned()
        }
        PluginPreflightRemediationClass::ResolveHostCompatibility => {
            "align `compatibility.host_api` / `compatibility.host_version_req` with the current host or upgrade Loong before activation".to_owned()
        }
        PluginPreflightRemediationClass::SwitchSupportedBridge => {
            "switch the plugin to a bridge kind supported by the current runtime matrix or widen bridge support intentionally before activation".to_owned()
        }
        PluginPreflightRemediationClass::SwitchSupportedAdapterFamily => {
            "switch the plugin adapter family to one supported by the current runtime matrix"
                .to_owned()
        }
        PluginPreflightRemediationClass::ResolveSlotOwnershipConflict => {
            "reassign the plugin slot claim or change the claimed key/mode so ownership stays explicit".to_owned()
        }
        PluginPreflightRemediationClass::ResolveActivationBlockers => {
            "resolve the activation blocker reported by the current host before treating this plugin as releasable".to_owned()
        }
        PluginPreflightRemediationClass::ReviewAdvisoryDiagnostics => {
            "review advisory diagnostics before publishing so the plugin contract stays migration-clean".to_owned()
        }
    }
}

fn has_diagnostic_code(plugin: &PluginInventoryResult, code: PluginDiagnosticCode) -> bool {
    plugin
        .diagnostic_findings
        .iter()
        .any(|finding| finding.code == code)
}

fn has_compatibility_shim_profile_mismatch(plugin: &PluginInventoryResult) -> bool {
    !plugin
        .compatibility_shim_support_mismatch_reasons
        .is_empty()
        || plugin
            .activation_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("support profile"))
}

fn resolve_preflight_verdict(
    policy_flags: &BTreeSet<String>,
    blocking_diagnostic_codes: &BTreeSet<String>,
    advisory_diagnostic_codes: &BTreeSet<String>,
    rules: &PluginPreflightRuleProfile,
) -> PluginPreflightVerdict {
    let blocked = (rules.block_on_activation_blocked
        && policy_flags.contains("activation_blocked"))
        || (rules.block_on_blocking_diagnostics
            && (policy_flags.contains("blocking_diagnostics_present")
                || !blocking_diagnostic_codes.is_empty()))
        || (rules.block_on_invalid_runtime_attestation
            && policy_flags.contains("runtime_attestation_invalid"))
        || (rules.block_on_foreign_dialect_contract
            && policy_flags.contains("foreign_dialect_contract"))
        || (rules.block_on_legacy_openclaw_contract
            && policy_flags.contains("legacy_openclaw_contract"))
        || (rules.block_on_compatibility_shim_required
            && policy_flags.contains("compatibility_shim_required"))
        || (rules.block_on_compatibility_shim_profile_mismatch
            && policy_flags.contains("compatibility_shim_profile_mismatch"))
        || (rules.block_on_embedded_source_contract
            && policy_flags.contains("embedded_source_contract"))
        || (rules.block_on_legacy_metadata_version
            && policy_flags.contains("legacy_metadata_version"))
        || (rules.block_on_shadowed_embedded_source
            && policy_flags.contains("shadowed_embedded_source"));

    if blocked {
        PluginPreflightVerdict::Block
    } else if rules.warn_on_advisory_diagnostics && !advisory_diagnostic_codes.is_empty() {
        PluginPreflightVerdict::Warn
    } else {
        PluginPreflightVerdict::Pass
    }
}

fn build_policy_summary(
    effective_policy_flags: &BTreeSet<String>,
    effective_blocking_diagnostic_codes: &BTreeSet<String>,
    effective_advisory_diagnostic_codes: &BTreeSet<String>,
    applied_exceptions: &[PluginPreflightAppliedException],
    profile: PluginPreflightProfile,
    policy: &PluginPreflightPolicyProfile,
    rules: &PluginPreflightRuleProfile,
    baseline_verdict: PluginPreflightVerdict,
    verdict: PluginPreflightVerdict,
) -> String {
    let mut reasons = Vec::new();
    if rules.block_on_activation_blocked && effective_policy_flags.contains("activation_blocked") {
        reasons.push("activation is currently blocked on the scanned host".to_owned());
    }
    if rules.block_on_invalid_runtime_attestation
        && effective_policy_flags.contains("runtime_attestation_invalid")
    {
        reasons.push(
            "loaded provider metadata is missing or failing activation attestation verification on the current host and should be quarantined until it is re-absorbed or re-registered"
                .to_owned(),
        );
    }
    if rules.block_on_foreign_dialect_contract
        && effective_policy_flags.contains("foreign_dialect_contract")
    {
        reasons.push(format!(
            "`{}` policy keeps foreign plugin dialects behind the compatibility boundary for profile `{}`",
            policy.policy_version.as_deref().unwrap_or("custom"),
            profile.as_str(),
        ));
    }
    if rules.block_on_legacy_openclaw_contract
        && effective_policy_flags.contains("legacy_openclaw_contract")
    {
        reasons.push(format!(
            "`{}` policy blocks legacy OpenClaw package metadata for profile `{}`",
            policy.policy_version.as_deref().unwrap_or("custom"),
            profile.as_str(),
        ));
    }
    if rules.block_on_compatibility_shim_required
        && effective_policy_flags.contains("compatibility_shim_required")
    {
        reasons.push(format!(
            "`{}` policy requires an explicit compatibility shim before profile `{}` can pass",
            policy.policy_version.as_deref().unwrap_or("custom"),
            profile.as_str(),
        ));
    }
    if rules.block_on_compatibility_shim_profile_mismatch
        && effective_policy_flags.contains("compatibility_shim_profile_mismatch")
    {
        reasons.push(format!(
            "`{}` policy blocks shim support-profile mismatches before profile `{}` can pass",
            policy.policy_version.as_deref().unwrap_or("custom"),
            profile.as_str(),
        ));
    }

    if rules.block_on_embedded_source_contract
        && effective_policy_flags.contains("embedded_source_contract")
    {
        reasons.push(format!(
            "`{}` policy blocks embedded source contracts for profile `{}`",
            policy.policy_version.as_deref().unwrap_or("custom"),
            profile.as_str(),
        ));
    }
    if rules.block_on_legacy_metadata_version
        && effective_policy_flags.contains("legacy_metadata_version")
    {
        reasons.push(format!(
            "`{}` policy blocks legacy metadata.version contract drift for profile `{}`",
            policy.policy_version.as_deref().unwrap_or("custom"),
            profile.as_str(),
        ));
    }
    if rules.block_on_shadowed_embedded_source
        && effective_policy_flags.contains("shadowed_embedded_source")
    {
        reasons.push(format!(
            "`{}` policy blocks shadowed embedded source markers for profile `{}`",
            policy.policy_version.as_deref().unwrap_or("custom"),
            profile.as_str(),
        ));
    }

    if reasons.is_empty() && !effective_blocking_diagnostic_codes.is_empty() {
        reasons.push(format!(
            "blocking diagnostics remain: {}",
            effective_blocking_diagnostic_codes
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if reasons.is_empty() && !effective_advisory_diagnostic_codes.is_empty() {
        reasons.push(format!(
            "advisory diagnostics remain: {}",
            effective_advisory_diagnostic_codes
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if reasons.is_empty() {
        let mut fallback = match verdict {
            PluginPreflightVerdict::Pass => {
                format!("plugin satisfies `{}` preflight", profile.as_str())
            }
            PluginPreflightVerdict::Warn => format!(
                "plugin satisfies `{}` preflight with advisory diagnostics",
                profile.as_str()
            ),
            PluginPreflightVerdict::Block => {
                format!("plugin does not satisfy `{}` preflight", profile.as_str())
            }
        };
        if baseline_verdict != verdict {
            fallback.push_str(" after documented policy exceptions");
        }
        reasons.push(fallback);
    }

    if !applied_exceptions.is_empty() {
        reasons.push(format!(
            "exceptions applied: {}",
            applied_exceptions
                .iter()
                .map(format_applied_exception_summary)
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    reasons.join("; ")
}

fn format_applied_exception_summary(exception: &PluginPreflightAppliedException) -> String {
    let mut waived_parts = Vec::new();
    if !exception.waived_policy_flags.is_empty() {
        waived_parts.push(format!(
            "policy flags [{}]",
            exception.waived_policy_flags.join(", ")
        ));
    }
    if !exception.waived_diagnostic_codes.is_empty() {
        waived_parts.push(format!(
            "diagnostic codes [{}]",
            exception.waived_diagnostic_codes.join(", ")
        ));
    }

    let version_scope = exception
        .plugin_version_req
        .as_deref()
        .filter(|version_req| !version_req.trim().is_empty())
        .map(|version_req| format!(" for plugin versions `{version_req}`"))
        .unwrap_or_default();
    let mut summary = format!(
        "`{}` ({}) approved by `{}` under `{}`{} waived {}",
        exception.exception_id,
        exception.reason,
        exception.approved_by,
        exception.ticket_ref,
        version_scope,
        waived_parts.join(" and ")
    );
    if let Some(expires_at) = exception.expires_at.as_deref()
        && !expires_at.trim().is_empty()
    {
        summary.push_str(&format!(" until {expires_at}"));
    }
    summary
}

fn build_preflight_summary(
    profile: PluginPreflightProfile,
    resolved_policy: &super::plugin_preflight_policy::ResolvedPluginPreflightPolicy,
    active_bridge_support: Option<&BridgeSupportSpec>,
    results: &[PluginPreflightResult],
) -> PluginPreflightSummary {
    let mut summary = PluginPreflightSummary {
        schema_version: PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION,
        schema: json_schema_descriptor(
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_SURFACE,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_PURPOSE,
        ),
        profile: profile.as_str().to_owned(),
        policy_source: resolved_policy.source.clone(),
        policy_version: resolved_policy.profile.policy_version.clone(),
        policy_checksum: resolved_policy.checksum.clone(),
        policy_sha256: resolved_policy.sha256.clone(),
        matched_plugins: 0,
        returned_plugins: 0,
        truncated: false,
        baseline_passed_plugins: 0,
        baseline_warned_plugins: 0,
        baseline_blocked_plugins: 0,
        clean_passed_plugins: 0,
        waived_passed_plugins: 0,
        passed_plugins: 0,
        warned_plugins: 0,
        blocked_plugins: 0,
        waived_plugins: 0,
        applied_exception_count: 0,
        ready_activation_plugins: 0,
        blocked_activation_plugins: 0,
        total_diagnostics: 0,
        blocking_diagnostics: 0,
        error_diagnostics: 0,
        warning_diagnostics: 0,
        info_diagnostics: 0,
        dialect_distribution: BTreeMap::new(),
        compatibility_mode_distribution: BTreeMap::new(),
        bridge_kind_distribution: BTreeMap::new(),
        source_language_distribution: BTreeMap::new(),
        findings_by_code: BTreeMap::new(),
        findings_by_phase: BTreeMap::new(),
        findings_by_severity: BTreeMap::new(),
        remediation_counts: BTreeMap::new(),
        operator_action_plan: Vec::new(),
        operator_action_counts_by_surface: BTreeMap::new(),
        operator_action_counts_by_kind: BTreeMap::new(),
        operator_actions_requiring_reload: 0,
        operator_actions_without_reload: 0,
        waived_policy_flags: BTreeMap::new(),
        waived_diagnostic_codes: BTreeMap::new(),
        exception_counts_by_ticket: BTreeMap::new(),
        exception_counts_by_approver: BTreeMap::new(),
        source_kind_distribution: BTreeMap::new(),
        active_bridge_profile: None,
        recommended_bridge_profile: None,
        recommended_bridge_profile_source: None,
        active_bridge_profile_matches_recommended: None,
        active_bridge_support_fits_all_plugins: None,
        bridge_profile_fits: Vec::new(),
        bridge_profile_recommendation: None,
    };
    let mut applied_exception_ids = BTreeSet::new();
    let mut seen_operator_actions = BTreeSet::new();
    let mut operator_action_plan = BTreeMap::new();

    for result in results {
        let mut seen_result_operator_actions = BTreeSet::new();

        match result.baseline_verdict.as_str() {
            "pass" => {
                summary.baseline_passed_plugins = summary.baseline_passed_plugins.saturating_add(1);
            }
            "warn" => {
                summary.baseline_warned_plugins = summary.baseline_warned_plugins.saturating_add(1);
            }
            "block" => {
                summary.baseline_blocked_plugins =
                    summary.baseline_blocked_plugins.saturating_add(1);
            }
            _ => {}
        }
        match result.verdict.as_str() {
            "pass" => {
                summary.passed_plugins = summary.passed_plugins.saturating_add(1);
                if result.exception_applied {
                    summary.waived_passed_plugins = summary.waived_passed_plugins.saturating_add(1);
                } else {
                    summary.clean_passed_plugins = summary.clean_passed_plugins.saturating_add(1);
                }
            }
            "warn" => summary.warned_plugins = summary.warned_plugins.saturating_add(1),
            "block" => summary.blocked_plugins = summary.blocked_plugins.saturating_add(1),
            _ => {}
        }
        if result.exception_applied {
            summary.waived_plugins = summary.waived_plugins.saturating_add(1);
        }
        if result.activation_ready {
            summary.ready_activation_plugins = summary.ready_activation_plugins.saturating_add(1);
        } else {
            summary.blocked_activation_plugins =
                summary.blocked_activation_plugins.saturating_add(1);
        }
        *summary
            .source_kind_distribution
            .entry(result.plugin.source_kind.clone())
            .or_default() += 1;
        *summary
            .dialect_distribution
            .entry(result.plugin.dialect.clone())
            .or_default() += 1;
        *summary
            .compatibility_mode_distribution
            .entry(result.plugin.compatibility_mode.clone())
            .or_default() += 1;
        *summary
            .bridge_kind_distribution
            .entry(result.plugin.bridge_kind.clone())
            .or_default() += 1;
        *summary
            .source_language_distribution
            .entry(
                result
                    .plugin
                    .source_language
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .unwrap_or("unknown")
                    .to_owned(),
            )
            .or_default() += 1;

        for remediation_class in &result.remediation_classes {
            *summary
                .remediation_counts
                .entry(remediation_class.as_str().to_owned())
                .or_default() += 1;
        }
        for action in &result.recommended_actions {
            if let Some(operator_action) = action.operator_action.as_ref() {
                let action_id = operator_action.action_id.clone();
                if seen_operator_actions.insert(action_id.clone()) {
                    *summary
                        .operator_action_counts_by_surface
                        .entry(operator_action.surface.as_str().to_owned())
                        .or_default() += 1;
                    *summary
                        .operator_action_counts_by_kind
                        .entry(operator_action.kind.as_str().to_owned())
                        .or_default() += 1;
                    if operator_action.requires_reload {
                        summary.operator_actions_requiring_reload =
                            summary.operator_actions_requiring_reload.saturating_add(1);
                    } else {
                        summary.operator_actions_without_reload =
                            summary.operator_actions_without_reload.saturating_add(1);
                    }
                }

                let support_signature = (
                    action.remediation_class.as_str().to_owned(),
                    action.blocking,
                    action.diagnostic_code.clone(),
                    action.field_path.clone(),
                    action.summary.clone(),
                );
                let support = PluginPreflightOperatorActionSupport {
                    remediation_class: action.remediation_class,
                    diagnostic_code: action.diagnostic_code.clone(),
                    field_path: action.field_path.clone(),
                    blocking: action.blocking,
                    summary: action.summary.clone(),
                };
                let accumulator = operator_action_plan
                    .entry(action_id.clone())
                    .or_insert_with(|| OperatorActionPlanAccumulator {
                        item: PluginPreflightOperatorActionPlanItem {
                            action: operator_action.clone(),
                            supporting_results: 0,
                            blocked_results: 0,
                            warned_results: 0,
                            passed_results: 0,
                            supporting_remediations: Vec::new(),
                        },
                        seen_supporting_remediations: BTreeSet::new(),
                    });
                if seen_result_operator_actions.insert(action_id) {
                    accumulator.item.supporting_results =
                        accumulator.item.supporting_results.saturating_add(1);
                    match result.verdict.as_str() {
                        "block" => {
                            accumulator.item.blocked_results =
                                accumulator.item.blocked_results.saturating_add(1);
                        }
                        "warn" => {
                            accumulator.item.warned_results =
                                accumulator.item.warned_results.saturating_add(1);
                        }
                        "pass" => {
                            accumulator.item.passed_results =
                                accumulator.item.passed_results.saturating_add(1);
                        }
                        _ => {}
                    }
                }
                if accumulator
                    .seen_supporting_remediations
                    .insert(support_signature)
                {
                    accumulator.item.supporting_remediations.push(support);
                }
            }
        }
        for exception in &result.applied_exceptions {
            applied_exception_ids.insert(exception.exception_id.clone());
            *summary
                .exception_counts_by_ticket
                .entry(exception.ticket_ref.clone())
                .or_default() += 1;
            *summary
                .exception_counts_by_approver
                .entry(exception.approved_by.clone())
                .or_default() += 1;
            for flag in &exception.waived_policy_flags {
                *summary.waived_policy_flags.entry(flag.clone()).or_default() += 1;
            }
            for code in &exception.waived_diagnostic_codes {
                *summary
                    .waived_diagnostic_codes
                    .entry(code.clone())
                    .or_default() += 1;
            }
        }

        for finding in &result.plugin.diagnostic_findings {
            summary.total_diagnostics = summary.total_diagnostics.saturating_add(1);
            if finding.blocking {
                summary.blocking_diagnostics = summary.blocking_diagnostics.saturating_add(1);
            }
            match finding.severity {
                PluginDiagnosticSeverity::Error => {
                    summary.error_diagnostics = summary.error_diagnostics.saturating_add(1);
                }
                PluginDiagnosticSeverity::Warning => {
                    summary.warning_diagnostics = summary.warning_diagnostics.saturating_add(1);
                }
                PluginDiagnosticSeverity::Info => {
                    summary.info_diagnostics = summary.info_diagnostics.saturating_add(1);
                }
            }

            *summary
                .findings_by_code
                .entry(finding.code.as_str().to_owned())
                .or_default() += 1;
            *summary
                .findings_by_phase
                .entry(finding.phase.as_str().to_owned())
                .or_default() += 1;
            *summary
                .findings_by_severity
                .entry(finding.severity.as_str().to_owned())
                .or_default() += 1;
        }
    }

    summary.applied_exception_count = applied_exception_ids.len();
    summary.operator_action_plan = operator_action_plan
        .into_values()
        .map(|accumulator| accumulator.item)
        .collect();
    sort_operator_action_plan(&mut summary.operator_action_plan);
    let fit_analysis = analyze_bridge_profile_fits(active_bridge_support, results);
    summary.active_bridge_profile = fit_analysis.active_bridge_profile;
    summary.recommended_bridge_profile = fit_analysis.recommended_bridge_profile;
    summary.recommended_bridge_profile_source = fit_analysis.recommended_bridge_profile_source;
    summary.active_bridge_profile_matches_recommended =
        fit_analysis.active_bridge_profile_matches_recommended;
    summary.active_bridge_support_fits_all_plugins =
        fit_analysis.active_bridge_support_fits_all_plugins;
    summary.bridge_profile_fits = fit_analysis.bridge_profile_fits;
    summary.bridge_profile_recommendation = fit_analysis.bridge_profile_recommendation;
    summary
}

fn analyze_bridge_profile_fits(
    active_bridge_support: Option<&BridgeSupportSpec>,
    results: &[PluginPreflightResult],
) -> BridgeProfileFitAnalysis {
    let mut fit_candidates = Vec::new();
    for profile_id in BUNDLED_BRIDGE_SUPPORT_PROFILE_IDS {
        let Ok(Some(resolved)) = resolve_bridge_support_policy(None, Some(profile_id), None) else {
            continue;
        };
        let matrix = bridge_support_spec_matrix(&resolved.profile);
        let mut fit = PluginPreflightBridgeProfileFit {
            profile_id: (*profile_id).to_owned(),
            source: resolved.source,
            policy_version: resolved.profile.policy_version.clone(),
            checksum: resolved.checksum,
            sha256: resolved.sha256,
            fits_all_plugins: false,
            supported_plugins: 0,
            blocked_plugins: 0,
            blocking_reasons: BTreeMap::new(),
            sample_blocked_plugins: Vec::new(),
        };
        let mut delta = BridgeProfileDeltaAccumulator::default();

        for result in results {
            let evaluation = bridge_profile_fit_evaluation(&matrix, &result.plugin);
            if evaluation.blocking_reasons.is_empty() {
                fit.supported_plugins = fit.supported_plugins.saturating_add(1);
                continue;
            }

            fit.blocked_plugins = fit.blocked_plugins.saturating_add(1);
            if fit.sample_blocked_plugins.len() < 8
                && !fit
                    .sample_blocked_plugins
                    .iter()
                    .any(|plugin_id| plugin_id == &result.plugin.plugin_id)
            {
                fit.sample_blocked_plugins
                    .push(result.plugin.plugin_id.clone());
            }
            for reason in evaluation.blocking_reasons {
                *fit.blocking_reasons.entry(reason).or_default() += 1;
            }
            merge_bridge_profile_delta(&mut delta, evaluation.delta);
        }

        fit.fits_all_plugins = fit.blocked_plugins == 0;
        fit_candidates.push(BridgeProfileFitCandidate {
            fit,
            delta: bridge_profile_delta_from_accumulator(delta),
        });
    }

    let active_bridge_sha256 = active_bridge_support.map(bridge_support_policy_sha256);
    let active_bridge_profile = active_bridge_sha256.as_deref().and_then(|active_sha256| {
        fit_candidates
            .iter()
            .find(|candidate| candidate.fit.sha256 == active_sha256)
            .map(|candidate| candidate.fit.profile_id.clone())
    });
    let active_bridge_support_fits_all_plugins = active_bridge_support.map(|bridge_support| {
        let matrix = bridge_support_spec_matrix(bridge_support);
        results.iter().all(|result| {
            bridge_profile_fit_evaluation(&matrix, &result.plugin)
                .blocking_reasons
                .is_empty()
        })
    });

    let recommended_fit = if results.is_empty() {
        None
    } else {
        fit_candidates
            .iter()
            .find(|candidate| candidate.fit.fits_all_plugins)
    };

    let bridge_profile_recommendation = if results.is_empty() {
        None
    } else if let Some(recommended_fit) = recommended_fit {
        if active_bridge_sha256
            .as_deref()
            .is_some_and(|active_sha256| recommended_fit.fit.sha256 == active_sha256)
        {
            None
        } else {
            Some(PluginPreflightBridgeProfileRecommendation {
                kind: PluginPreflightBridgeProfileRecommendationKind::AdoptBundledProfile,
                target_profile_id: recommended_fit.fit.profile_id.clone(),
                target_profile_source: recommended_fit.fit.source.clone(),
                target_policy_version: recommended_fit.fit.policy_version.clone(),
                summary: adopt_bridge_profile_summary(
                    active_bridge_support.is_some(),
                    active_bridge_profile.as_deref(),
                    &recommended_fit.fit,
                ),
                delta: None,
            })
        }
    } else if active_bridge_support_fits_all_plugins == Some(true) {
        None
    } else {
        fit_candidates
            .iter()
            .min_by(|left, right| compare_bridge_profile_delta_candidates(left, right))
            .map(|candidate| PluginPreflightBridgeProfileRecommendation {
                kind: PluginPreflightBridgeProfileRecommendationKind::AuthorBridgeProfileDelta,
                target_profile_id: candidate.fit.profile_id.clone(),
                target_profile_source: candidate.fit.source.clone(),
                target_policy_version: candidate.fit.policy_version.clone(),
                summary: author_bridge_profile_delta_summary(&candidate.fit, &candidate.delta),
                delta: Some(candidate.delta.clone()),
            })
    };

    BridgeProfileFitAnalysis {
        active_bridge_profile,
        recommended_bridge_profile: recommended_fit
            .map(|candidate| candidate.fit.profile_id.clone()),
        recommended_bridge_profile_source: recommended_fit
            .map(|candidate| candidate.fit.source.clone()),
        active_bridge_profile_matches_recommended: recommended_fit.map(|candidate| {
            active_bridge_sha256
                .as_deref()
                .is_some_and(|active_sha256| candidate.fit.sha256 == active_sha256)
        }),
        active_bridge_support_fits_all_plugins,
        bridge_profile_fits: fit_candidates
            .into_iter()
            .map(|candidate| candidate.fit)
            .collect(),
        bridge_profile_recommendation,
    }
}

fn bridge_profile_fit_evaluation(
    matrix: &BridgeSupportMatrix,
    plugin: &PluginInventoryResult,
) -> BridgeProfilePluginFitEvaluation {
    let mut reasons = BTreeSet::new();
    let mut delta = BridgeProfileDeltaAccumulator::default();

    let bridge_kind = parse_bridge_kind_label(&plugin.bridge_kind);
    let compatibility_mode = parse_plugin_activation_runtime_mode(&plugin.compatibility_mode);
    let dialect = parse_plugin_activation_runtime_dialect(&plugin.dialect);

    if bridge_kind.is_none() {
        reasons.insert("unrecognized_bridge_kind".to_owned());
        delta
            .unresolved_blocking_reasons
            .insert("unrecognized_bridge_kind".to_owned());
    }
    if compatibility_mode.is_none() {
        reasons.insert("unrecognized_compatibility_mode".to_owned());
        delta
            .unresolved_blocking_reasons
            .insert("unrecognized_compatibility_mode".to_owned());
    }
    if dialect.is_none() {
        reasons.insert("unrecognized_dialect".to_owned());
        delta
            .unresolved_blocking_reasons
            .insert("unrecognized_dialect".to_owned());
    }

    let source_language =
        normalize_runtime_source_language(plugin.source_language.as_deref().unwrap_or("unknown"));

    if let Some(bridge_kind) = bridge_kind {
        if !matrix.is_bridge_supported(bridge_kind) {
            reasons.insert("unsupported_bridge".to_owned());
            delta
                .supported_bridges
                .insert(bridge_kind.as_str().to_owned());
        }

        let adapter_family = normalize_profile_fit_adapter_family(
            plugin.adapter_family.as_deref(),
            source_language.as_str(),
            bridge_kind,
        );
        if !matrix.is_adapter_family_supported(&adapter_family) {
            reasons.insert("unsupported_adapter_family".to_owned());
            delta
                .supported_adapter_families
                .insert(adapter_family.clone());
        }

        if let (Some(compatibility_mode), Some(dialect)) = (compatibility_mode, dialect) {
            if !matrix.is_compatibility_mode_supported(compatibility_mode) {
                reasons.insert("unsupported_compatibility_mode".to_owned());
                delta
                    .supported_compatibility_modes
                    .insert(compatibility_mode.as_str().to_owned());
            }

            let compatibility_shim = plugin
                .compatibility_shim
                .clone()
                .or_else(|| PluginCompatibilityShim::for_mode(compatibility_mode));
            if !matrix.is_compatibility_shim_supported(compatibility_shim.as_ref()) {
                reasons.insert("unsupported_compatibility_shim".to_owned());
                if let Some(shim) = compatibility_shim.as_ref() {
                    delta
                        .supported_compatibility_shims
                        .insert(format!("{}:{}", shim.shim_id, shim.family));
                    accumulate_shim_profile_delta(
                        &mut delta,
                        shim,
                        dialect,
                        bridge_kind,
                        &adapter_family,
                        &source_language,
                    );
                }
            }

            if let Some(shim) = compatibility_shim.as_ref() {
                let ir = build_profile_fit_ir(
                    plugin,
                    dialect,
                    compatibility_mode,
                    bridge_kind,
                    adapter_family,
                    source_language.clone(),
                );
                if matrix
                    .compatibility_shim_support_issue(&ir, Some(shim))
                    .is_some()
                {
                    reasons.insert("shim_support_profile_mismatch".to_owned());
                    if let Some(profile) = matrix.compatibility_shim_support_profile(Some(shim)) {
                        accumulate_shim_profile_delta_mismatch(
                            &mut delta,
                            profile,
                            shim,
                            dialect,
                            bridge_kind,
                            ir.runtime.adapter_family.as_str(),
                            ir.runtime.source_language.as_str(),
                        );
                    } else {
                        delta
                            .unresolved_blocking_reasons
                            .insert("shim_support_profile_mismatch".to_owned());
                    }
                }
            }
        }
    } else if let Some(compatibility_mode) = compatibility_mode
        && !matrix.is_compatibility_mode_supported(compatibility_mode)
    {
        reasons.insert("unsupported_compatibility_mode".to_owned());
        delta
            .supported_compatibility_modes
            .insert(compatibility_mode.as_str().to_owned());
    }

    BridgeProfilePluginFitEvaluation {
        blocking_reasons: reasons.into_iter().collect(),
        delta,
    }
}

fn merge_bridge_profile_delta(
    target: &mut BridgeProfileDeltaAccumulator,
    incoming: BridgeProfileDeltaAccumulator,
) {
    target.supported_bridges.extend(incoming.supported_bridges);
    target
        .supported_adapter_families
        .extend(incoming.supported_adapter_families);
    target
        .supported_compatibility_modes
        .extend(incoming.supported_compatibility_modes);
    target
        .supported_compatibility_shims
        .extend(incoming.supported_compatibility_shims);
    target
        .unresolved_blocking_reasons
        .extend(incoming.unresolved_blocking_reasons);

    for (key, value) in incoming.shim_profile_additions {
        let entry = target.shim_profile_additions.entry(key).or_default();
        entry.supported_dialects.extend(value.supported_dialects);
        entry.supported_bridges.extend(value.supported_bridges);
        entry
            .supported_adapter_families
            .extend(value.supported_adapter_families);
        entry
            .supported_source_languages
            .extend(value.supported_source_languages);
    }
}

fn accumulate_shim_profile_delta(
    delta: &mut BridgeProfileDeltaAccumulator,
    shim: &PluginCompatibilityShim,
    dialect: PluginContractDialect,
    bridge_kind: PluginBridgeKind,
    adapter_family: &str,
    source_language: &str,
) {
    let entry = delta
        .shim_profile_additions
        .entry((shim.shim_id.clone(), shim.family.clone()))
        .or_default();
    entry.supported_dialects.insert(dialect.as_str().to_owned());
    entry
        .supported_bridges
        .insert(bridge_kind.as_str().to_owned());
    entry
        .supported_adapter_families
        .insert(adapter_family.to_owned());
    if source_language != "unknown" {
        entry
            .supported_source_languages
            .insert(source_language.to_owned());
    }
}

fn accumulate_shim_profile_delta_mismatch(
    delta: &mut BridgeProfileDeltaAccumulator,
    profile: &kernel::PluginCompatibilityShimSupport,
    shim: &PluginCompatibilityShim,
    dialect: PluginContractDialect,
    bridge_kind: PluginBridgeKind,
    adapter_family: &str,
    source_language: &str,
) {
    let entry = delta
        .shim_profile_additions
        .entry((shim.shim_id.clone(), shim.family.clone()))
        .or_default();

    if !profile.supported_dialects.is_empty() && !profile.supported_dialects.contains(&dialect) {
        entry.supported_dialects.insert(dialect.as_str().to_owned());
    }
    if !profile.supported_bridges.is_empty() && !profile.supported_bridges.contains(&bridge_kind) {
        entry
            .supported_bridges
            .insert(bridge_kind.as_str().to_owned());
    }
    if !profile.supported_adapter_families.is_empty()
        && !profile
            .supported_adapter_families
            .contains(&adapter_family.trim().to_ascii_lowercase())
    {
        entry
            .supported_adapter_families
            .insert(adapter_family.to_owned());
    }
    if !profile.supported_source_languages.is_empty()
        && !profile.supported_source_languages.contains(source_language)
        && source_language != "unknown"
    {
        entry
            .supported_source_languages
            .insert(source_language.to_owned());
    }
}

fn bridge_profile_delta_from_accumulator(
    accumulator: BridgeProfileDeltaAccumulator,
) -> PluginPreflightBridgeProfileDelta {
    let mut shim_profile_additions = accumulator
        .shim_profile_additions
        .into_iter()
        .map(
            |((shim_id, shim_family), delta)| PluginPreflightBridgeShimProfileDelta {
                shim_id,
                shim_family,
                supported_dialects: delta.supported_dialects.into_iter().collect(),
                supported_bridges: delta.supported_bridges.into_iter().collect(),
                supported_adapter_families: delta.supported_adapter_families.into_iter().collect(),
                supported_source_languages: delta.supported_source_languages.into_iter().collect(),
            },
        )
        .collect::<Vec<_>>();
    shim_profile_additions.sort_by(|left, right| {
        (left.shim_id.as_str(), left.shim_family.as_str())
            .cmp(&(right.shim_id.as_str(), right.shim_family.as_str()))
    });

    PluginPreflightBridgeProfileDelta {
        supported_bridges: accumulator.supported_bridges.into_iter().collect(),
        supported_adapter_families: accumulator.supported_adapter_families.into_iter().collect(),
        supported_compatibility_modes: accumulator
            .supported_compatibility_modes
            .into_iter()
            .collect(),
        supported_compatibility_shims: accumulator
            .supported_compatibility_shims
            .into_iter()
            .collect(),
        shim_profile_additions,
        unresolved_blocking_reasons: accumulator
            .unresolved_blocking_reasons
            .into_iter()
            .collect(),
    }
}

fn compare_bridge_profile_delta_candidates(
    left: &BridgeProfileFitCandidate,
    right: &BridgeProfileFitCandidate,
) -> std::cmp::Ordering {
    let left_has_unresolved = !left.delta.unresolved_blocking_reasons.is_empty();
    let right_has_unresolved = !right.delta.unresolved_blocking_reasons.is_empty();
    left_has_unresolved
        .cmp(&right_has_unresolved)
        .then_with(|| {
            bridge_profile_delta_score(&left.delta).cmp(&bridge_profile_delta_score(&right.delta))
        })
        .then_with(|| right.fit.supported_plugins.cmp(&left.fit.supported_plugins))
        .then_with(|| left.fit.blocked_plugins.cmp(&right.fit.blocked_plugins))
}

fn bridge_profile_delta_score(delta: &PluginPreflightBridgeProfileDelta) -> usize {
    delta.supported_bridges.len()
        + delta.supported_adapter_families.len()
        + delta.supported_compatibility_modes.len()
        + delta.supported_compatibility_shims.len()
        + delta
            .shim_profile_additions
            .iter()
            .map(|addition| {
                addition.supported_dialects.len()
                    + addition.supported_bridges.len()
                    + addition.supported_adapter_families.len()
                    + addition.supported_source_languages.len()
            })
            .sum::<usize>()
        + delta.unresolved_blocking_reasons.len().saturating_mul(100)
}

fn adopt_bridge_profile_summary(
    has_active_bridge_support: bool,
    active_bridge_profile: Option<&str>,
    fit: &PluginPreflightBridgeProfileFit,
) -> String {
    match active_bridge_profile {
        Some(active_bridge_profile) => format!(
            "active bundled bridge profile `{active_bridge_profile}` does not match the scanned ecosystem; adopt `{}` from `{}` to keep bridge compatibility explicit and fail-closed",
            fit.profile_id, fit.source
        ),
        None if has_active_bridge_support => format!(
            "current bridge support is custom or unpinned; adopt bundled profile `{}` from `{}` so the scanned ecosystem runs behind an explicit fail-closed contract",
            fit.profile_id, fit.source
        ),
        None => format!(
            "adopt bundled bridge profile `{}` from `{}` so the scanned ecosystem runs behind an explicit fail-closed contract",
            fit.profile_id, fit.source
        ),
    }
}

fn author_bridge_profile_delta_summary(
    fit: &PluginPreflightBridgeProfileFit,
    delta: &PluginPreflightBridgeProfileDelta,
) -> String {
    let mut parts = vec![format!(
        "no bundled bridge profile fits all scanned plugins; start from `{}` ({}) and author a custom delta profile",
        fit.profile_id, fit.source
    )];
    let delta_brief = bridge_profile_delta_brief(delta);
    if !delta_brief.is_empty() {
        parts.push(format!("required additions: {}", delta_brief.join("; ")));
    }
    parts.join("; ")
}

fn bridge_profile_delta_brief(delta: &PluginPreflightBridgeProfileDelta) -> Vec<String> {
    let mut parts = Vec::new();
    if !delta.supported_bridges.is_empty() {
        parts.push(format!("bridges={}", delta.supported_bridges.join(",")));
    }
    if !delta.supported_adapter_families.is_empty() {
        parts.push(format!(
            "adapter_families={}",
            delta.supported_adapter_families.join(",")
        ));
    }
    if !delta.supported_compatibility_modes.is_empty() {
        parts.push(format!(
            "compatibility_modes={}",
            delta.supported_compatibility_modes.join(",")
        ));
    }
    if !delta.supported_compatibility_shims.is_empty() {
        parts.push(format!(
            "compatibility_shims={}",
            delta.supported_compatibility_shims.join(",")
        ));
    }
    if !delta.shim_profile_additions.is_empty() {
        parts.push(format!(
            "shim_profiles={}",
            delta
                .shim_profile_additions
                .iter()
                .map(format_shim_profile_delta_brief)
                .collect::<Vec<_>>()
                .join(";")
        ));
    }
    if !delta.unresolved_blocking_reasons.is_empty() {
        parts.push(format!(
            "unresolved={}",
            delta.unresolved_blocking_reasons.join(",")
        ));
    }
    parts
}

fn format_shim_profile_delta_brief(delta: &PluginPreflightBridgeShimProfileDelta) -> String {
    let mut clauses = Vec::new();
    if !delta.supported_dialects.is_empty() {
        clauses.push(format!("dialects={}", delta.supported_dialects.join(",")));
    }
    if !delta.supported_bridges.is_empty() {
        clauses.push(format!("bridges={}", delta.supported_bridges.join(",")));
    }
    if !delta.supported_adapter_families.is_empty() {
        clauses.push(format!(
            "adapter_families={}",
            delta.supported_adapter_families.join(",")
        ));
    }
    if !delta.supported_source_languages.is_empty() {
        clauses.push(format!(
            "languages={}",
            delta.supported_source_languages.join(",")
        ));
    }
    if clauses.is_empty() {
        format!("{}:{}:none", delta.shim_id, delta.shim_family)
    } else {
        format!(
            "{}:{}:{}",
            delta.shim_id,
            delta.shim_family,
            clauses.join("|")
        )
    }
}

fn normalize_profile_fit_adapter_family(
    adapter_family: Option<&str>,
    source_language: &str,
    bridge_kind: PluginBridgeKind,
) -> String {
    let normalized = adapter_family
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        default_runtime_adapter_family(source_language, bridge_kind)
    } else {
        normalized
    }
}

fn build_profile_fit_ir(
    plugin: &PluginInventoryResult,
    dialect: PluginContractDialect,
    compatibility_mode: PluginCompatibilityMode,
    bridge_kind: PluginBridgeKind,
    adapter_family: String,
    source_language: String,
) -> PluginIR {
    PluginIR {
        manifest_api_version: plugin.manifest_api_version.clone(),
        plugin_version: plugin.plugin_version.clone(),
        dialect,
        dialect_version: plugin.dialect_version.clone(),
        compatibility_mode,
        plugin_id: plugin.plugin_id.clone(),
        provider_id: plugin.provider_id.clone(),
        connector_name: plugin.connector_name.clone(),
        channel_id: None,
        endpoint: plugin.entrypoint_hint.clone(),
        capabilities: BTreeSet::new(),
        trust_tier: kernel::PluginTrustTier::default(),
        metadata: BTreeMap::new(),
        source_path: plugin.source_path.clone(),
        source_kind: profile_fit_source_kind(plugin.source_kind.as_str()),
        package_root: plugin.package_root.clone(),
        package_manifest_path: plugin.package_manifest_path.clone(),
        diagnostic_findings: plugin.diagnostic_findings.clone(),
        setup: None,
        channel_bridge: None,
        slot_claims: plugin.slot_claims.clone(),
        compatibility: plugin.compatibility.clone(),
        runtime: PluginRuntimeProfile {
            source_language,
            bridge_kind,
            adapter_family,
            entrypoint_hint: plugin
                .entrypoint_hint
                .clone()
                .unwrap_or_else(|| "invoke".to_owned()),
        },
    }
}

fn profile_fit_source_kind(source_kind: &str) -> PluginSourceKind {
    match source_kind {
        "embedded_source" => PluginSourceKind::EmbeddedSource,
        _ => PluginSourceKind::PackageManifest,
    }
}

fn preflight_verdict_rank(verdict: &str) -> u8 {
    match verdict {
        "block" => 3,
        "warn" => 2,
        "pass" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
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
            capabilities: vec!["invoke_connector".to_owned()],
            extension_contract: None,
            extension_family: None,
            extension_trust_lane: None,
            extension_facets: Vec::new(),
            extension_methods: Vec::new(),
            extension_events: Vec::new(),
            extension_host_actions: Vec::new(),
            extension_metadata_issues: Vec::new(),
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
}
