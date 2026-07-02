use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::kernel::{Capability, ExecutionRoute, HarnessKind, VerticalPackManifest};
use crate::{
    BridgeSupportSpec, CliResult, HumanApprovalMode, HumanApprovalSpec,
    MaterializedBridgeSupportDeltaArtifact, OperationSpec, PluginInventoryResult,
    PluginPreflightBridgeProfileRecommendation, PluginPreflightProfile, PluginPreflightResult,
    PluginScanSpec, ResolvedBridgeSupportSelection, RunnerSpec, SecurityProfileSignatureSpec,
    SpecRunReport, default_plugin_inventory_limit, default_plugin_preflight_limit,
    resolve_bridge_support_policy, resolve_bridge_support_selection,
};

use super::{
    PluginBridgeProfileArg, PluginDoctorSourceArgs, PluginGovernanceSourceArgs,
    PluginScanSourceArgs, PluginsBridgeProfileExecutionView, PluginsBridgeShimSupportProfileView,
    PluginsBridgeSupportProvenanceView, PluginsPreflightSummaryView,
};

#[derive(Debug, Clone)]
struct ResolvedPluginScanSource {
    scan_roots: Vec<String>,
    query: String,
    limit: usize,
    bridge_support: Option<ResolvedBridgeSupportSelection>,
}

impl ResolvedPluginScanSource {
    fn bridge_support_source(&self) -> Option<String> {
        self.bridge_support
            .as_ref()
            .map(|selection| selection.policy.source.clone())
    }

    fn bridge_support_sha256(&self) -> Option<String> {
        self.bridge_support
            .as_ref()
            .map(|selection| selection.policy.sha256.clone())
    }

    fn bridge_support_delta_source(&self) -> Option<String> {
        self.bridge_support
            .as_ref()
            .and_then(|selection| selection.delta_source.clone())
    }

    fn bridge_support_delta_sha256(&self) -> Option<String> {
        self.bridge_support.as_ref().and_then(|selection| {
            selection
                .delta_artifact
                .as_ref()
                .map(|artifact| artifact.sha256.clone())
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct PluginInventoryContext {
    pub(super) scan_roots: Vec<String>,
    pub(super) query: String,
    pub(super) limit: usize,
    pub(super) bridge_support_source: Option<String>,
    pub(super) bridge_support_sha256: Option<String>,
    pub(super) bridge_support_delta_source: Option<String>,
    pub(super) bridge_support_delta_sha256: Option<String>,
    pub(super) spec: RunnerSpec,
}

impl PluginInventoryContext {
    pub(super) fn bridge_support_provenance(&self) -> Option<PluginsBridgeSupportProvenanceView> {
        PluginsBridgeSupportProvenanceView::from_fields(
            self.bridge_support_source.as_deref(),
            self.bridge_support_sha256.as_deref(),
            self.bridge_support_delta_source.as_deref(),
            self.bridge_support_delta_sha256.as_deref(),
        )
    }
}

#[derive(Debug, Clone)]
pub(super) struct PluginPreflightContext {
    pub(super) scan_roots: Vec<String>,
    pub(super) query: String,
    pub(super) limit: usize,
    pub(super) profile: String,
    pub(super) bridge_support_source: Option<String>,
    pub(super) bridge_support_sha256: Option<String>,
    pub(super) bridge_support_delta_source: Option<String>,
    pub(super) bridge_support_delta_sha256: Option<String>,
    pub(super) spec: RunnerSpec,
}

impl PluginPreflightContext {
    pub(super) fn bridge_support_provenance(&self) -> Option<PluginsBridgeSupportProvenanceView> {
        PluginsBridgeSupportProvenanceView::from_fields(
            self.bridge_support_source.as_deref(),
            self.bridge_support_sha256.as_deref(),
            self.bridge_support_delta_source.as_deref(),
            self.bridge_support_delta_sha256.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct PluginGovernanceSurfaceContextSpec {
    pack_id: &'static str,
    agent_id: &'static str,
    operator_surface: &'static str,
    surface_label: &'static str,
}

pub(super) fn build_plugin_inventory_context(
    source: &PluginScanSourceArgs,
    include_ready: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
) -> CliResult<PluginInventoryContext> {
    let default_limit = default_plugin_inventory_limit();
    let resolved = resolve_plugin_scan_source(source, default_limit, 100, "plugins inventory")?;
    let mut spec = RunnerSpec::template();
    spec.pack = VerticalPackManifest {
        pack_id: "plugin-inventory".to_owned(),
        domain: "ops".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: Some("pi-local".to_owned()),
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
        metadata: BTreeMap::from([("operator_surface".to_owned(), "plugin_inventory".to_owned())]),
    };
    spec.agent_id = "agent-plugin-inventory".to_owned();
    spec.ttl_s = 120;
    spec.approval = Some(HumanApprovalSpec {
        mode: HumanApprovalMode::Disabled,
        ..HumanApprovalSpec::default()
    });
    spec.defaults = None;
    spec.self_awareness = None;
    spec.plugin_scan = Some(PluginScanSpec {
        enabled: true,
        roots: resolved.scan_roots.clone(),
    });
    spec.bridge_support = resolved
        .bridge_support
        .as_ref()
        .map(|selection| selection.policy.profile.clone());
    spec.bootstrap = None;
    spec.auto_provision = None;
    spec.hotfixes = Vec::new();
    spec.operation = OperationSpec::PluginInventory {
        query: resolved.query.clone(),
        limit: resolved.limit,
        include_ready,
        include_blocked,
        include_deferred,
        include_examples,
    };
    Ok(PluginInventoryContext {
        scan_roots: resolved.scan_roots.clone(),
        query: resolved.query.clone(),
        limit: resolved.limit,
        bridge_support_source: resolved.bridge_support_source(),
        bridge_support_sha256: resolved.bridge_support_sha256(),
        bridge_support_delta_source: resolved.bridge_support_delta_source(),
        bridge_support_delta_sha256: resolved.bridge_support_delta_sha256(),
        spec,
    })
}

pub(super) fn build_plugin_doctor_context(
    source: &PluginDoctorSourceArgs,
    include_passed: bool,
    include_warned: bool,
    include_blocked: bool,
    include_deferred: bool,
) -> CliResult<PluginPreflightContext> {
    let policy_signature = build_policy_signature_spec(
        source.policy_signature_algorithm.as_str(),
        source.policy_signature_public_key_base64.as_deref(),
        source.policy_signature_base64.as_deref(),
    )?;
    build_plugin_preflight_context_from_parts(
        &source.scan,
        source.profile.as_profile(),
        source.policy_path.clone(),
        source.policy_sha256.clone(),
        policy_signature,
        include_passed,
        include_warned,
        include_blocked,
        include_deferred,
        false,
        PluginGovernanceSurfaceContextSpec {
            pack_id: "plugin-doctor",
            agent_id: "agent-plugin-doctor",
            operator_surface: "plugin_doctor",
            surface_label: "plugins doctor",
        },
    )
}

pub(super) fn build_plugin_preflight_context(
    source: &PluginGovernanceSourceArgs,
    include_passed: bool,
    include_warned: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
) -> CliResult<PluginPreflightContext> {
    let policy_signature = build_policy_signature_spec(
        source.policy_signature_algorithm.as_str(),
        source.policy_signature_public_key_base64.as_deref(),
        source.policy_signature_base64.as_deref(),
    )?;
    build_plugin_preflight_context_from_parts(
        &source.scan,
        source.profile.as_profile(),
        source.policy_path.clone(),
        source.policy_sha256.clone(),
        policy_signature,
        include_passed,
        include_warned,
        include_blocked,
        include_deferred,
        include_examples,
        PluginGovernanceSurfaceContextSpec {
            pack_id: "plugin-governance",
            agent_id: "agent-plugin-governance",
            operator_surface: "plugin_governance",
            surface_label: "plugins governance",
        },
    )
}

fn build_plugin_preflight_context_from_parts(
    scan: &PluginScanSourceArgs,
    profile: PluginPreflightProfile,
    policy_path: Option<String>,
    policy_sha256: Option<String>,
    policy_signature: Option<SecurityProfileSignatureSpec>,
    include_passed: bool,
    include_warned: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
    surface_spec: PluginGovernanceSurfaceContextSpec,
) -> CliResult<PluginPreflightContext> {
    let default_limit = default_plugin_preflight_limit();
    let resolved =
        resolve_plugin_scan_source(scan, default_limit, 500, surface_spec.surface_label)?;
    let mut spec = RunnerSpec::template();
    spec.pack = VerticalPackManifest {
        pack_id: surface_spec.pack_id.to_owned(),
        domain: "ops".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: Some("pi-local".to_owned()),
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
        metadata: BTreeMap::from([(
            "operator_surface".to_owned(),
            surface_spec.operator_surface.to_owned(),
        )]),
    };
    spec.agent_id = surface_spec.agent_id.to_owned();
    spec.ttl_s = 120;
    spec.approval = Some(HumanApprovalSpec {
        mode: HumanApprovalMode::Disabled,
        ..HumanApprovalSpec::default()
    });
    spec.defaults = None;
    spec.self_awareness = None;
    spec.plugin_scan = Some(PluginScanSpec {
        enabled: true,
        roots: resolved.scan_roots.clone(),
    });
    spec.bridge_support = resolved
        .bridge_support
        .as_ref()
        .map(|selection| selection.policy.profile.clone());
    spec.bootstrap = None;
    spec.auto_provision = None;
    spec.hotfixes = Vec::new();
    spec.operation = OperationSpec::PluginPreflight {
        query: resolved.query.clone(),
        limit: resolved.limit,
        profile,
        policy_path,
        policy_sha256,
        policy_signature,
        include_passed,
        include_warned,
        include_blocked,
        include_deferred,
        include_examples,
    };
    Ok(PluginPreflightContext {
        scan_roots: resolved.scan_roots.clone(),
        query: resolved.query.clone(),
        limit: resolved.limit,
        profile: profile.as_str().to_owned(),
        bridge_support_source: resolved.bridge_support_source(),
        bridge_support_sha256: resolved.bridge_support_sha256(),
        bridge_support_delta_source: resolved.bridge_support_delta_source(),
        bridge_support_delta_sha256: resolved.bridge_support_delta_sha256(),
        spec,
    })
}

fn resolve_plugin_scan_source(
    source: &PluginScanSourceArgs,
    default_limit: usize,
    max_limit: usize,
    surface_label: &str,
) -> CliResult<ResolvedPluginScanSource> {
    let roots = normalize_scan_roots(&source.roots, surface_label)?;
    let requested_limit = source.limit.unwrap_or(default_limit);
    let limit = validate_plugin_limit(requested_limit, max_limit, surface_label)?;
    let bridge_support = resolve_bridge_support_selection(
        source.bridge_support.as_deref(),
        source.bridge_profile.map(PluginBridgeProfileArg::as_str),
        source.bridge_support_delta.as_deref(),
        source.bridge_support_sha256.as_deref(),
        source.bridge_support_delta_sha256.as_deref(),
    )?;
    Ok(ResolvedPluginScanSource {
        scan_roots: roots,
        query: source.query.clone(),
        limit,
        bridge_support,
    })
}

pub(super) fn load_bridge_profile_views(
    requested: &[PluginBridgeProfileArg],
) -> CliResult<Vec<PluginsBridgeProfileExecutionView>> {
    let requested = if requested.is_empty() {
        vec![
            PluginBridgeProfileArg::NativeBalanced,
            PluginBridgeProfileArg::OpenclawEcosystemBalanced,
        ]
    } else {
        requested.to_vec()
    };

    let mut views = Vec::new();
    let mut seen = BTreeSet::new();
    for profile in requested {
        let profile_id = profile.as_str();
        if !seen.insert(profile_id.to_owned()) {
            continue;
        }
        let resolved =
            resolve_bridge_support_policy(None, Some(profile_id), None)?.ok_or_else(|| {
                format!("bundled bridge support profile `{profile_id}` was not resolved")
            })?;
        let mut supported_bridges = resolved
            .profile
            .supported_bridges
            .iter()
            .map(|bridge| bridge.as_str().to_owned())
            .collect::<Vec<_>>();
        supported_bridges.sort();
        let mut supported_compatibility_modes = resolved
            .profile
            .supported_compatibility_modes
            .iter()
            .map(|mode| mode.as_str().to_owned())
            .collect::<Vec<_>>();
        supported_compatibility_modes.sort();
        let mut supported_compatibility_shims = resolved
            .profile
            .supported_compatibility_shims
            .iter()
            .map(|shim| format!("{}:{}", shim.shim_id, shim.family))
            .collect::<Vec<_>>();
        supported_compatibility_shims.sort();

        let mut shim_support_profiles = resolved
            .profile
            .supported_compatibility_shim_profiles
            .iter()
            .map(|profile| {
                let mut supported_dialects = profile
                    .supported_dialects
                    .iter()
                    .map(|dialect| dialect.as_str().to_owned())
                    .collect::<Vec<_>>();
                supported_dialects.sort();
                let mut supported_bridges = profile
                    .supported_bridges
                    .iter()
                    .map(|bridge| bridge.as_str().to_owned())
                    .collect::<Vec<_>>();
                supported_bridges.sort();
                let mut supported_adapter_families = profile
                    .supported_adapter_families
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                supported_adapter_families.sort();
                let mut supported_source_languages = profile
                    .supported_source_languages
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                supported_source_languages.sort();

                PluginsBridgeShimSupportProfileView {
                    shim_id: profile.shim.shim_id.clone(),
                    shim_family: profile.shim.family.clone(),
                    version: profile.version.clone(),
                    supported_dialects,
                    supported_bridges,
                    supported_adapter_families,
                    supported_source_languages,
                }
            })
            .collect::<Vec<_>>();
        shim_support_profiles.sort_by(|left, right| {
            (
                left.shim_id.as_str(),
                left.shim_family.as_str(),
                left.version.as_deref().unwrap_or_default(),
            )
                .cmp(&(
                    right.shim_id.as_str(),
                    right.shim_family.as_str(),
                    right.version.as_deref().unwrap_or_default(),
                ))
        });

        views.push(PluginsBridgeProfileExecutionView {
            profile_id: profile_id.to_owned(),
            source: resolved.source,
            policy_version: resolved.profile.policy_version.clone(),
            checksum: resolved.checksum,
            sha256: resolved.sha256,
            supported_bridges,
            supported_compatibility_modes,
            supported_compatibility_shims,
            shim_support_profiles,
            execute_process_stdio: resolved.profile.execute_process_stdio,
            execute_http_json: resolved.profile.execute_http_json,
            enforce_supported: resolved.profile.enforce_supported,
            enforce_execution_success: resolved.profile.enforce_execution_success,
        });
    }

    Ok(views)
}

pub(super) fn normalize_scan_roots(
    roots: &[String],
    surface_label: &str,
) -> CliResult<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        let trimmed = root.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_owned()) {
            normalized.push(trimmed.to_owned());
        }
    }
    if normalized.is_empty() {
        return Err(format!(
            "{surface_label} requires at least one non-empty --root"
        ));
    }
    Ok(normalized)
}

fn validate_plugin_limit(limit: usize, max_limit: usize, surface_label: &str) -> CliResult<usize> {
    if !(1..=max_limit).contains(&limit) {
        return Err(format!(
            "{surface_label} limit must be between 1 and {max_limit}"
        ));
    }
    Ok(limit)
}

pub(super) fn build_policy_signature_spec(
    algorithm: &str,
    public_key_base64: Option<&str>,
    signature_base64: Option<&str>,
) -> CliResult<Option<SecurityProfileSignatureSpec>> {
    match (public_key_base64, signature_base64) {
        (None, None) => Ok(None),
        (Some(_), None) => {
            Err("plugins governance policy signature requires --policy-signature-base64".to_owned())
        }
        (None, Some(_)) => Err(
            "plugins governance policy signature requires --policy-signature-public-key-base64"
                .to_owned(),
        ),
        (Some(public_key_base64), Some(signature_base64)) => {
            Ok(Some(SecurityProfileSignatureSpec {
                algorithm: algorithm.to_owned(),
                public_key_base64: public_key_base64.to_owned(),
                signature_base64: signature_base64.to_owned(),
            }))
        }
    }
}

pub(super) fn decode_preflight_bridge_profile_recommendation(
    report: &SpecRunReport,
) -> CliResult<Option<PluginPreflightBridgeProfileRecommendation>> {
    let recommendation_value = report
        .outcome
        .get("summary")
        .and_then(|summary| summary.get("bridge_profile_recommendation"))
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::from_value(recommendation_value).map_err(|error| {
        format!("decode plugin preflight bridge profile recommendation failed: {error}")
    })
}

pub(super) fn decode_plugin_inventory_results(
    report: &SpecRunReport,
) -> CliResult<Vec<PluginInventoryResult>> {
    let results_value = report
        .outcome
        .get("results")
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::from_value(results_value)
        .map_err(|error| format!("decode plugin inventory results failed: {error}"))
}

pub(super) fn decode_preflight_summary(
    report: &SpecRunReport,
    bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
) -> CliResult<PluginsPreflightSummaryView> {
    let summary_value = report
        .outcome
        .get("summary")
        .cloned()
        .ok_or_else(|| "decode plugin preflight summary failed: missing summary".to_owned())?;
    let mut summary: PluginsPreflightSummaryView = serde_json::from_value(summary_value)
        .map_err(|error| format!("decode plugin preflight summary failed: {error}"))?;
    summary.bridge_support_provenance = bridge_support_provenance;
    Ok(summary)
}

pub(super) fn decode_preflight_results(
    report: &SpecRunReport,
) -> CliResult<Vec<PluginPreflightResult>> {
    let results_value = report
        .outcome
        .get("results")
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::from_value(results_value)
        .map_err(|error| format!("decode plugin preflight results failed: {error}"))
}

pub(super) fn write_bridge_support_template(
    path: &str,
    template: &BridgeSupportSpec,
) -> CliResult<()> {
    let rendered = serde_json::to_string_pretty(template)
        .map_err(|error| format!("serialize bridge support template failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create bridge template parent directory `{}` failed: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, rendered)
        .map_err(|error| format!("write bridge support template `{path}` failed: {error}"))
}

pub(super) fn write_bridge_support_delta_artifact(
    path: &str,
    artifact: &MaterializedBridgeSupportDeltaArtifact,
) -> CliResult<()> {
    let rendered = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("serialize bridge support delta artifact failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create bridge delta parent directory `{}` failed: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, rendered)
        .map_err(|error| format!("write bridge support delta artifact `{path}` failed: {error}"))
}
