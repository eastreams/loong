use std::{
    collections::BTreeMap,
    ffi::OsString,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use kernel::{
    ArchitectureBoundaryPolicy, ArchitectureGuardReport, AuditEventKind, AutoProvisionAgent,
    AutoProvisionRequest, BootstrapPolicy, BootstrapReport, BootstrapTaskStatus, Clock,
    CodebaseAwarenessConfig, CodebaseAwarenessEngine, CodebaseAwarenessSnapshot, ConnectorCommand,
    InMemoryAuditSink, IntegrationCatalog, LoongKernel, MemoryCoreRequest, MemoryExtensionRequest,
    PluginAbsorbReport, PluginActivationPlan, PluginActivationStatus, PluginBootstrapExecutor,
    PluginCompatibility, PluginCompatibilityShimSupport, PluginScanReport, PluginScanner,
    PluginSetup, PluginSetupReadinessContext, PluginSlotClaim, PluginTranslationReport,
    PluginTranslator, ProvisionPlan, RuntimeCoreRequest, RuntimeExtensionRequest,
    StaticPolicyEngine, SystemClock, TaskIntent, TaskSupervisor, ToolCoreRequest,
    ToolExtensionRequest, plugin_bridge_is_high_risk_auto_apply,
    plugin_provenance_summary_for_descriptor,
};
use serde_json::{Value, json};

use crate::CliResult;
use crate::kernel_bootstrap::default_in_memory_audit_sink;
use crate::programmatic::execute_programmatic_tool_call;
use crate::spec_runtime::*;

mod approval_policy;
mod bootstrap_support;
mod bridge_runtime_policy_support;
mod bridge_selection_support;
mod bridge_support_policy;
mod integration_catalog_support;
mod path_policy_support;
mod plugin_inventory;
mod plugin_kind_support;
mod plugin_metadata;
mod plugin_preflight;
mod plugin_preflight_policy;
mod reporting;
mod security_scan_eval;
mod security_scan_policy;
mod tool_search;
use approval_policy::evaluate_approval_guard;
pub(crate) use bootstrap_support::current_epoch_s;
use bootstrap_support::{apply_default_selection, bootstrap_policy};
#[cfg(test)]
pub(crate) use bridge_runtime_policy_support::collect_verified_env_var_names;
use bridge_runtime_policy_support::{
    bridge_runtime_policy, filter_scan_report_by_activation, filter_scan_report_by_keys,
    resolve_plugin_setup_readiness_context,
};
use bridge_selection_support::bridge_support_matrix;
use bridge_support_policy::bridge_support_policy_checksum;
pub(crate) use integration_catalog_support::fnv1a64_hex;
pub use integration_catalog_support::hex_lower;
use integration_catalog_support::{
    default_integration_catalog, operation_connector_name, register_dynamic_catalog_connectors,
    snapshot_runtime_integration_catalog,
};
pub(crate) use path_policy_support::{
    normalize_allowed_path_prefixes, validate_wasm_guest_readable_config_keys,
};
pub use path_policy_support::{normalize_path_for_policy, resolve_plugin_relative_path};
use plugin_inventory::execute_plugin_inventory;
pub(crate) use plugin_kind_support::descriptor_bridge_kind;
use plugin_metadata::enrich_scan_report_with_translation;
use plugin_preflight::execute_plugin_preflight;
use reporting::{
    build_blocked_spec_run_report, build_plugin_trust_summary, emit_plugin_trust_audit_event,
    emit_security_scan_audit_event,
};
use security_scan_eval::evaluate_plugin_security_scan;
use security_scan_policy::{
    apply_security_scan_delta, emit_security_scan_siem_record, security_scan_process_allowlist,
};
use tool_search::{
    build_tool_search_operation_summary, emit_tool_search_audit_event, execute_tool_search,
};

pub use approval_policy::operation_risk_profile;
pub use bridge_support_policy::{
    MaterializedBridgeSupportDeltaArtifact, ResolvedBridgeSupportSelection,
    bridge_support_policy_sha256, load_bridge_support_delta_artifact_from_path,
    load_bridge_support_policy_from_path, load_bundled_bridge_support_policy,
    materialize_bridge_support_delta_artifact, materialize_bridge_support_template,
    resolve_bridge_support_policy, resolve_bridge_support_selection, security_scan_profile_message,
    security_scan_profile_sha256,
};
pub use plugin_preflight_policy::{
    load_plugin_preflight_policy_from_path, plugin_preflight_policy_checksum,
    plugin_preflight_policy_message, plugin_preflight_policy_sha256,
};
pub use security_scan_policy::{load_security_scan_profile_from_path, security_scan_policy};

pub async fn execute_spec(spec: &RunnerSpec, include_audit: bool) -> SpecRunReport {
    execute_spec_internal(spec, include_audit, None, None, None, None).await
}

pub async fn execute_spec_with_native_tool_executor(
    spec: &RunnerSpec,
    include_audit: bool,
    native_tool_executor: Option<crate::NativeToolExecutor>,
) -> SpecRunReport {
    execute_spec_internal(spec, include_audit, native_tool_executor, None, None, None).await
}

pub async fn execute_spec_with_native_tool_executor_and_bridge_support_provenance(
    spec: &RunnerSpec,
    include_audit: bool,
    native_tool_executor: Option<crate::NativeToolExecutor>,
    bridge_support_source: Option<String>,
    bridge_support_delta_source: Option<String>,
    bridge_support_delta_sha256: Option<String>,
) -> SpecRunReport {
    execute_spec_internal(
        spec,
        include_audit,
        native_tool_executor,
        bridge_support_source,
        bridge_support_delta_source,
        bridge_support_delta_sha256,
    )
    .await
}

async fn execute_spec_internal(
    spec: &RunnerSpec,
    include_audit: bool,
    native_tool_executor: Option<crate::NativeToolExecutor>,
    bridge_support_source: Option<String>,
    bridge_support_delta_source: Option<String>,
    bridge_support_delta_sha256: Option<String>,
) -> SpecRunReport {
    let mut pack = spec.pack.clone();
    let audit_sink = default_in_memory_audit_sink();
    let mut builder = crate::kernel_bootstrap::KernelBuilder::default()
        .clock(Arc::new(SystemClock) as Arc<dyn Clock>)
        .audit(audit_sink.clone());
    if let Some(executor) = native_tool_executor {
        builder = builder.native_tool_executor(executor);
    }
    let mut kernel = builder.build();

    let mut integration_catalog = default_integration_catalog();
    let mut blocked_reason = None;
    let mut bridge_support_checksum = None;
    let mut bridge_support_sha256 = None;
    let approval_guard = evaluate_approval_guard(spec);
    let mut self_awareness = None;
    let mut architecture_guard = None;
    let mut plugin_scan_reports = Vec::new();
    let mut plugin_translation_reports = Vec::new();
    let mut plugin_activation_plans = Vec::new();
    let mut plugin_bootstrap_reports = Vec::new();
    let mut plugin_bootstrap_queue = Vec::new();
    let mut plugin_absorb_reports = Vec::new();
    let security_scan_policy = match security_scan_policy(spec) {
        Ok(policy) => policy,
        Err(error) => {
            blocked_reason = Some(match blocked_reason {
                Some(existing) => format!("{existing}; {error}"),
                None => error,
            });
            None
        }
    };
    let security_process_allowlist = security_scan_process_allowlist(spec);
    let mut security_scan_report = security_scan_policy
        .as_ref()
        .map(|_| SecurityScanReport::default());
    let mut auto_provision_plan = None;
    let plugin_setup_readiness = spec.plugin_setup_readiness.as_ref();
    let setup_readiness_context =
        resolve_plugin_setup_readiness_context(plugin_setup_readiness, std::env::vars_os());

    if !approval_guard.approved {
        blocked_reason = Some(approval_guard.reason.clone());
    }

    if let Some(bridge) = &spec.bridge_support
        && bridge.enabled
    {
        let checksum = bridge_support_policy_checksum(bridge);
        let sha256 = bridge_support_policy_sha256(bridge);
        bridge_support_checksum = Some(checksum.clone());
        bridge_support_sha256 = Some(sha256.clone());

        let version = bridge.policy_version.as_deref().unwrap_or("unknown");
        let mut mismatch_reasons = Vec::new();
        if let Some(expected) = &bridge.expected_checksum
            && !expected.eq_ignore_ascii_case(&checksum)
        {
            mismatch_reasons.push(format!(
                "bridge support policy checksum mismatch (version {version})"
            ));
        }
        if let Some(expected_sha256) = &bridge.expected_sha256
            && !expected_sha256.eq_ignore_ascii_case(&sha256)
        {
            mismatch_reasons.push(format!(
                "bridge support policy sha256 mismatch (version {version})"
            ));
        }
        if !mismatch_reasons.is_empty() {
            blocked_reason = Some(mismatch_reasons.join("; "));
        }
    }

    if let Some(self_awareness_spec) = &spec.self_awareness
        && self_awareness_spec.enabled
    {
        let mut architecture_policy = ArchitectureBoundaryPolicy::default();
        if !self_awareness_spec.immutable_core_paths.is_empty() {
            architecture_policy.immutable_prefixes = self_awareness_spec
                .immutable_core_paths
                .iter()
                .cloned()
                .collect();
        }
        if !self_awareness_spec.mutable_extension_paths.is_empty() {
            architecture_policy.mutable_prefixes = self_awareness_spec
                .mutable_extension_paths
                .iter()
                .cloned()
                .collect();
        }

        let engine = CodebaseAwarenessEngine::new();
        match engine.snapshot(&CodebaseAwarenessConfig {
            roots: self_awareness_spec.roots.clone(),
            plugin_roots: self_awareness_spec.plugin_roots.clone(),
            proposed_mutations: self_awareness_spec.proposed_mutations.clone(),
            architecture_policy,
        }) {
            Ok(snapshot) => {
                architecture_guard = Some(snapshot.architecture_guard.clone());
                if self_awareness_spec.enforce_guard && snapshot.architecture_guard.has_denials() {
                    blocked_reason = Some(
                        "architecture guard blocked proposed mutations outside mutable boundaries"
                            .to_owned(),
                    );
                }
                self_awareness = Some(snapshot);
            }
            Err(error) => {
                blocked_reason = Some(format!("self-awareness snapshot failed: {error}"));
            }
        }
    }

    if let Some(reason) = blocked_reason.clone() {
        return build_blocked_spec_run_report(
            pack.pack_id.clone(),
            spec.agent_id.clone(),
            reason,
            approval_guard,
            bridge_support_source.clone(),
            bridge_support_checksum,
            bridge_support_sha256,
            bridge_support_delta_source.clone(),
            bridge_support_delta_sha256.clone(),
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report,
            auto_provision_plan,
            integration_catalog,
            include_audit,
            &audit_sink,
        );
    }

    let bridge_runtime_policy = match bridge_runtime_policy(spec, security_scan_policy.as_ref()) {
        Ok(policy) => policy,
        Err(error) => {
            let reason = format!("bridge runtime policy is invalid: {error}");
            return build_blocked_spec_run_report(
                pack.pack_id.clone(),
                spec.agent_id.clone(),
                reason,
                approval_guard,
                bridge_support_source.clone(),
                bridge_support_checksum,
                bridge_support_sha256,
                bridge_support_delta_source.clone(),
                bridge_support_delta_sha256.clone(),
                self_awareness,
                architecture_guard,
                plugin_scan_reports,
                plugin_translation_reports,
                plugin_activation_plans,
                plugin_bootstrap_reports,
                plugin_bootstrap_queue,
                plugin_absorb_reports,
                security_scan_report,
                auto_provision_plan,
                integration_catalog,
                include_audit,
                &audit_sink,
            );
        }
    };

    if let Some(plugin_scan) = &spec.plugin_scan
        && plugin_scan.enabled
    {
        let scanner = PluginScanner::new();
        let translator = PluginTranslator::new();
        let bootstrap_executor = PluginBootstrapExecutor::new();
        let bootstrap_policy = bootstrap_policy(spec);
        let (bridge_matrix, enforce_bridge_support) = bridge_support_matrix(spec);
        let mut pending_absorb_inputs = Vec::new();
        let mut planning_catalog = integration_catalog.clone();
        let mut planning_pack = pack.clone();
        let mut remaining_bootstrap_budget =
            bootstrap_policy.as_ref().map(|policy| policy.max_tasks);
        for root in &plugin_scan.roots {
            let report = match scanner.scan_path(root) {
                Ok(report) => report,
                Err(error) => {
                    blocked_reason = Some(format!("plugin scan failed for root {root}: {error}"));
                    break;
                }
            };
            let translation = translator.translate_scan_report(&report);
            let activation = translator.plan_activation_with_catalog(
                &translation,
                &bridge_matrix,
                &setup_readiness_context,
                Some(&planning_catalog),
            );

            if enforce_bridge_support && activation.has_blockers() {
                blocked_reason = Some(format!(
                    "bridge support enforcement blocked {} plugin(s): {}",
                    activation.blocked_plugins,
                    activation.blocker_summary(3)
                ));
            }

            let ready_report = filter_scan_report_by_activation(&report, &activation);
            let mut filtered_report = ready_report.clone();
            if let Some(policy) = bootstrap_policy.as_ref() {
                let mut effective_policy = policy.clone();
                if let Some(remaining) = remaining_bootstrap_budget {
                    effective_policy.max_tasks = remaining;
                }
                let bootstrap_report = bootstrap_executor.execute(&activation, &effective_policy);
                if blocked_reason.is_none() && bootstrap_report.blocked {
                    blocked_reason =
                        Some(bootstrap_report.block_reason.clone().unwrap_or_else(|| {
                            "bootstrap policy blocked ready plugins".to_owned()
                        }));
                }

                if let Some(remaining) = remaining_bootstrap_budget.as_mut() {
                    *remaining = remaining.saturating_sub(bootstrap_report.applied_tasks);
                }

                plugin_bootstrap_queue.extend(
                    bootstrap_report
                        .tasks
                        .iter()
                        .filter(|task| matches!(task.status, BootstrapTaskStatus::Applied))
                        .map(|task| task.bootstrap_hint.clone()),
                );
                filtered_report =
                    filter_scan_report_by_keys(&report, &bootstrap_report.applied_plugin_keys);
                plugin_bootstrap_reports.push(bootstrap_report);
            } else {
                plugin_bootstrap_queue.extend(
                    activation
                        .candidates
                        .iter()
                        .filter(|candidate| {
                            matches!(candidate.status, PluginActivationStatus::Ready)
                        })
                        .map(|candidate| candidate.bootstrap_hint.clone()),
                );
            }

            let enriched_ready_report =
                enrich_scan_report_with_translation(&ready_report, &translation, Some(&activation));
            let enriched_filtered_report = enrich_scan_report_with_translation(
                &filtered_report,
                &translation,
                Some(&activation),
            );

            if let (Some(policy), Some(report)) =
                (security_scan_policy.as_ref(), security_scan_report.as_mut())
            {
                let delta = evaluate_plugin_security_scan(
                    &enriched_ready_report,
                    policy,
                    &security_process_allowlist,
                );
                apply_security_scan_delta(report, delta);

                if blocked_reason.is_none() && report.blocked {
                    blocked_reason = report.block_reason.clone();
                }
            }

            plugin_translation_reports.push(translation);
            plugin_activation_plans.push(activation);
            plugin_scan_reports.push(report);
            if blocked_reason.is_none() {
                match scanner.absorb(
                    &mut planning_catalog,
                    &mut planning_pack,
                    &enriched_filtered_report,
                ) {
                    Ok(_) => pending_absorb_inputs.push(enriched_filtered_report),
                    Err(error) => {
                        blocked_reason = Some(format!("plugin absorb failed: {error}"));
                    }
                }
            }

            if blocked_reason.is_some() {
                break;
            }
        }

        if blocked_reason.is_none() {
            for pending in pending_absorb_inputs {
                match scanner.absorb(&mut integration_catalog, &mut pack, &pending) {
                    Ok(absorb) => plugin_absorb_reports.push(absorb),
                    Err(error) => {
                        blocked_reason = Some(format!("plugin absorb failed: {error}"));
                        break;
                    }
                }
            }
        }
    }

    if let (Some(policy), Some(report)) =
        (security_scan_policy.as_ref(), security_scan_report.as_mut())
        && let Some(export_spec) = policy.siem_export.as_ref().filter(|value| value.enabled)
    {
        match emit_security_scan_siem_record(&pack.pack_id, &spec.agent_id, report, export_spec) {
            Ok(export_report) => report.siem_export = Some(export_report),
            Err(error) => {
                report.siem_export = Some(SecuritySiemExportReport {
                    enabled: true,
                    path: export_spec.path.clone(),
                    success: false,
                    exported_records: 0,
                    exported_findings: 0,
                    truncated_findings: 0,
                    error: Some(error.clone()),
                });
                if export_spec.fail_on_error && blocked_reason.is_none() {
                    blocked_reason = Some(format!("security scan siem export failed: {error}"));
                }
            }
        }
    }

    if let Some(report) = security_scan_report.as_ref()
        && let Err(error) =
            emit_security_scan_audit_event(&kernel, &pack.pack_id, &spec.agent_id, report)
        && blocked_reason.is_none()
    {
        blocked_reason = Some(error);
    }

    let plugin_trust_summary = build_plugin_trust_summary(
        &plugin_scan_reports,
        &plugin_activation_plans,
        &plugin_bootstrap_reports,
    );
    if let Err(error) = emit_plugin_trust_audit_event(
        &kernel,
        &pack.pack_id,
        &spec.agent_id,
        &plugin_trust_summary,
    ) && blocked_reason.is_none()
    {
        blocked_reason = Some(error);
    }

    if let Some(reason) = blocked_reason.clone() {
        return build_blocked_spec_run_report(
            pack.pack_id.clone(),
            spec.agent_id.clone(),
            reason,
            approval_guard,
            bridge_support_source.clone(),
            bridge_support_checksum,
            bridge_support_sha256,
            bridge_support_delta_source.clone(),
            bridge_support_delta_sha256.clone(),
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report,
            auto_provision_plan,
            integration_catalog,
            include_audit,
            &audit_sink,
        );
    }

    if let Some(auto) = &spec.auto_provision
        && auto.enabled
    {
        let agent = AutoProvisionAgent::new();
        let connector_name = auto
            .connector_name
            .clone()
            .or_else(|| operation_connector_name(&spec.operation));
        let request = AutoProvisionRequest {
            provider_id: auto.provider_id.clone(),
            channel_id: auto.channel_id.clone(),
            connector_name,
            endpoint: auto.endpoint.clone(),
            required_capabilities: auto.required_capabilities.clone(),
        };

        match agent.plan(&integration_catalog, &pack, &request) {
            Ok(plan) => {
                if !plan.is_noop() {
                    if let Err(error) = integration_catalog.apply_plan(&mut pack, &plan) {
                        blocked_reason =
                            Some(format!("applying auto-provision plan failed: {error}"));
                    } else {
                        auto_provision_plan = Some(plan);
                    }
                }
            }
            Err(error) => {
                blocked_reason = Some(format!("auto-provision planning failed: {error}"));
            }
        }
    }

    if blocked_reason.is_none() {
        for hotfix in &spec.hotfixes {
            if let Err(error) = integration_catalog.apply_hotfix(&hotfix.to_kernel_hotfix()) {
                blocked_reason = Some(format!("hotfix application failed: {error}"));
                break;
            }
        }
    }

    if let Some(reason) = blocked_reason.clone() {
        return build_blocked_spec_run_report(
            pack.pack_id.clone(),
            spec.agent_id.clone(),
            reason,
            approval_guard,
            bridge_support_source.clone(),
            bridge_support_checksum,
            bridge_support_sha256,
            bridge_support_delta_source.clone(),
            bridge_support_delta_sha256.clone(),
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report,
            auto_provision_plan,
            integration_catalog,
            include_audit,
            &audit_sink,
        );
    }

    let shared_catalog = Arc::new(Mutex::new(integration_catalog.clone()));
    register_dynamic_catalog_connectors(&mut kernel, shared_catalog.clone(), bridge_runtime_policy);

    if let Err(error) = kernel.register_pack(pack.clone()) {
        let base_reason = format!("spec pack registration failed: {error}");
        let snapshot_result = snapshot_runtime_integration_catalog(&shared_catalog);
        let (integration_catalog, reason) = match snapshot_result {
            Ok(catalog) => (catalog, base_reason),
            Err(error) => {
                let fallback_catalog = integration_catalog.clone();
                let reason = format!(
                    "{base_reason}; failed to snapshot runtime integration catalog: {error}"
                );
                (fallback_catalog, reason)
            }
        };
        return build_blocked_spec_run_report(
            pack.pack_id.clone(),
            spec.agent_id.clone(),
            reason,
            approval_guard,
            bridge_support_source.clone(),
            bridge_support_checksum,
            bridge_support_sha256,
            bridge_support_delta_source.clone(),
            bridge_support_delta_sha256.clone(),
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report,
            auto_provision_plan,
            integration_catalog,
            include_audit,
            &audit_sink,
        );
    }
    if let Err(error) = apply_default_selection(&mut kernel, spec.defaults.as_ref()) {
        let snapshot_result = snapshot_runtime_integration_catalog(&shared_catalog);
        let (integration_catalog, reason) = match snapshot_result {
            Ok(catalog) => (catalog, error),
            Err(snapshot_error) => {
                let fallback_catalog = integration_catalog.clone();
                let reason = format!(
                    "{error}; failed to snapshot runtime integration catalog: {snapshot_error}"
                );
                (fallback_catalog, reason)
            }
        };
        return build_blocked_spec_run_report(
            pack.pack_id.clone(),
            spec.agent_id.clone(),
            reason,
            approval_guard,
            bridge_support_source.clone(),
            bridge_support_checksum,
            bridge_support_sha256,
            bridge_support_delta_source.clone(),
            bridge_support_delta_sha256.clone(),
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report,
            auto_provision_plan,
            integration_catalog,
            include_audit,
            &audit_sink,
        );
    }

    let token = match kernel.issue_token(&pack.pack_id, &spec.agent_id, spec.ttl_s) {
        Ok(token) => token,
        Err(error) => {
            let base_reason = format!("token issue for spec failed: {error}");
            let snapshot_result = snapshot_runtime_integration_catalog(&shared_catalog);
            let (integration_catalog, reason) = match snapshot_result {
                Ok(catalog) => (catalog, base_reason),
                Err(error) => {
                    let fallback_catalog = integration_catalog.clone();
                    let reason = format!(
                        "{base_reason}; failed to snapshot runtime integration catalog: {error}"
                    );
                    (fallback_catalog, reason)
                }
            };
            return build_blocked_spec_run_report(
                pack.pack_id.clone(),
                spec.agent_id.clone(),
                reason,
                approval_guard,
                bridge_support_source.clone(),
                bridge_support_checksum,
                bridge_support_sha256,
                bridge_support_delta_source.clone(),
                bridge_support_delta_sha256.clone(),
                self_awareness,
                architecture_guard,
                plugin_scan_reports,
                plugin_translation_reports,
                plugin_activation_plans,
                plugin_bootstrap_reports,
                plugin_bootstrap_queue,
                plugin_absorb_reports,
                security_scan_report,
                auto_provision_plan,
                integration_catalog,
                include_audit,
                &audit_sink,
            );
        }
    };

    let (operation_kind, outcome) = match execute_spec_operation(
        &kernel,
        &pack.pack_id,
        &token,
        &integration_catalog,
        &plugin_scan_reports,
        &plugin_translation_reports,
        &setup_readiness_context,
        &plugin_activation_plans,
        spec.bridge_support.as_ref().filter(|bridge| bridge.enabled),
        &spec.operation,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let snapshot_result = snapshot_runtime_integration_catalog(&shared_catalog);
            let (integration_catalog, reason) = match snapshot_result {
                Ok(catalog) => (catalog, error),
                Err(snapshot_error) => {
                    let fallback_catalog = integration_catalog.clone();
                    let reason = format!(
                        "{error}; failed to snapshot runtime integration catalog: {snapshot_error}"
                    );
                    (fallback_catalog, reason)
                }
            };
            return build_blocked_spec_run_report(
                pack.pack_id.clone(),
                spec.agent_id.clone(),
                reason,
                approval_guard,
                bridge_support_source.clone(),
                bridge_support_checksum,
                bridge_support_sha256,
                bridge_support_delta_source.clone(),
                bridge_support_delta_sha256.clone(),
                self_awareness,
                architecture_guard,
                plugin_scan_reports,
                plugin_translation_reports,
                plugin_activation_plans,
                plugin_bootstrap_reports,
                plugin_bootstrap_queue,
                plugin_absorb_reports,
                security_scan_report,
                auto_provision_plan,
                integration_catalog,
                include_audit,
                &audit_sink,
            );
        }
    };

    let integration_catalog = match snapshot_runtime_integration_catalog(&shared_catalog) {
        Ok(catalog) => catalog,
        Err(error) => {
            let reason = format!("snapshotting runtime integration catalog failed: {error}");
            return build_blocked_spec_run_report(
                pack.pack_id.clone(),
                spec.agent_id.clone(),
                reason,
                approval_guard,
                bridge_support_source.clone(),
                bridge_support_checksum,
                bridge_support_sha256,
                bridge_support_delta_source.clone(),
                bridge_support_delta_sha256.clone(),
                self_awareness,
                architecture_guard,
                plugin_scan_reports,
                plugin_translation_reports,
                plugin_activation_plans,
                plugin_bootstrap_reports,
                plugin_bootstrap_queue,
                plugin_absorb_reports,
                security_scan_report,
                auto_provision_plan,
                integration_catalog,
                include_audit,
                &audit_sink,
            );
        }
    };
    let tool_search_summary = (operation_kind == "tool_search")
        .then(|| build_tool_search_operation_summary(&outcome))
        .flatten();
    if let Some(summary) = tool_search_summary.as_ref()
        && let Err(error) =
            emit_tool_search_audit_event(&kernel, &pack.pack_id, &spec.agent_id, summary)
    {
        blocked_reason = Some(error);
    }
    if let Some(reason) = blocked_reason.clone() {
        return build_blocked_spec_run_report(
            pack.pack_id.clone(),
            spec.agent_id.clone(),
            reason,
            approval_guard,
            bridge_support_source,
            bridge_support_checksum,
            bridge_support_sha256,
            bridge_support_delta_source.clone(),
            bridge_support_delta_sha256.clone(),
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report,
            auto_provision_plan,
            integration_catalog,
            include_audit,
            &audit_sink,
        );
    }

    SpecRunReport {
        schema_version: SPEC_RUN_REPORT_SCHEMA_VERSION,
        schema: json_schema_descriptor(
            SPEC_RUN_REPORT_SCHEMA_VERSION,
            SPEC_RUN_REPORT_SCHEMA_SURFACE,
            SPEC_RUN_REPORT_SCHEMA_PURPOSE,
        ),
        pack_id: pack.pack_id.clone(),
        agent_id: spec.agent_id.clone(),
        operation_kind,
        blocked_reason,
        approval_guard,
        bridge_support_source,
        bridge_support_checksum,
        bridge_support_sha256,
        bridge_support_delta_source,
        bridge_support_delta_sha256,
        self_awareness,
        architecture_guard,
        plugin_scan_reports,
        plugin_translation_reports,
        plugin_activation_plans,
        plugin_bootstrap_reports,
        plugin_trust_summary,
        tool_search_summary,
        plugin_bootstrap_queue,
        plugin_absorb_reports,
        security_scan_report,
        auto_provision_plan,
        outcome,
        integration_catalog,
        audit_events: if include_audit {
            Some(audit_sink.snapshot())
        } else {
            None
        },
    }
}

#[derive(Debug, Default)]
struct SecurityScanDelta {
    scanned_plugins: usize,
    high_findings: usize,
    medium_findings: usize,
    low_findings: usize,
    findings: Vec<SecurityFinding>,
    block_reason: Option<String>,
}

async fn execute_spec_operation(
    kernel: &LoongKernel<StaticPolicyEngine>,
    pack_id: &str,
    token: &kernel::CapabilityToken,
    integration_catalog: &IntegrationCatalog,
    plugin_scan_reports: &[PluginScanReport],
    plugin_translation_reports: &[PluginTranslationReport],
    setup_readiness_context: &PluginSetupReadinessContext,
    plugin_activation_plans: &[PluginActivationPlan],
    active_bridge_support: Option<&BridgeSupportSpec>,
    operation: &OperationSpec,
) -> CliResult<(&'static str, Value)> {
    match operation {
        OperationSpec::Task {
            task_id,
            objective,
            required_capabilities,
            payload,
        } => {
            let mut supervisor = TaskSupervisor::new(TaskIntent {
                task_id: task_id.clone(),
                objective: objective.clone(),
                required_capabilities: required_capabilities.clone(),
                payload: payload.clone(),
            });
            let dispatch_result = supervisor.execute(kernel, pack_id, token).await;
            let outcome = match dispatch_result {
                Ok(dispatch) => json!({
                    "route": dispatch.adapter_route,
                    "outcome": dispatch.outcome,
                    "supervisor_state": supervisor.state(),
                    "error": Value::Null,
                }),
                Err(error) => {
                    let error_message = format!("task execution from spec failed: {error}");
                    json!({
                        "route": Value::Null,
                        "outcome": Value::Null,
                        "supervisor_state": supervisor.state(),
                        "error": error_message,
                    })
                }
            };
            Ok(("task", outcome))
        }
        OperationSpec::ConnectorLegacy {
            connector_name,
            operation,
            required_capabilities,
            payload,
        } => {
            let dispatch = kernel
                .execute_connector_core(
                    pack_id,
                    token,
                    Some(connector_name.as_str()),
                    ConnectorCommand {
                        connector_name: connector_name.clone(),
                        operation: operation.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("legacy connector execution from spec failed: {error}"))?;
            Ok((
                "connector_legacy",
                json!({
                    "connector_name": dispatch.connector_name,
                    "outcome": dispatch.outcome,
                }),
            ))
        }
        OperationSpec::ConnectorCore {
            connector_name,
            operation,
            required_capabilities,
            payload,
            core,
        } => {
            let dispatch = kernel
                .execute_connector_core(
                    pack_id,
                    token,
                    core.as_deref(),
                    ConnectorCommand {
                        connector_name: connector_name.clone(),
                        operation: operation.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("core connector execution from spec failed: {error}"))?;
            Ok((
                "connector_core",
                json!({
                    "connector_name": dispatch.connector_name,
                    "outcome": dispatch.outcome,
                }),
            ))
        }
        OperationSpec::ConnectorExtension {
            connector_name,
            operation,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let dispatch = kernel
                .execute_connector_extension(
                    pack_id,
                    token,
                    extension,
                    core.as_deref(),
                    ConnectorCommand {
                        connector_name: connector_name.clone(),
                        operation: operation.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| {
                    format!("extension connector execution from spec failed: {error}")
                })?;
            Ok((
                "connector_extension",
                json!({
                    "connector_name": dispatch.connector_name,
                    "outcome": dispatch.outcome,
                }),
            ))
        }
        OperationSpec::RuntimeCore {
            action,
            required_capabilities,
            payload,
            core,
        } => {
            let outcome = kernel
                .execute_runtime_core(
                    pack_id,
                    token,
                    required_capabilities,
                    core.as_deref(),
                    RuntimeCoreRequest {
                        action: action.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("runtime core execution from spec failed: {error}"))?;
            Ok(("runtime_core", json!({ "outcome": outcome })))
        }
        OperationSpec::RuntimeExtension {
            action,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let outcome = kernel
                .execute_runtime_extension(
                    pack_id,
                    token,
                    required_capabilities,
                    extension,
                    core.as_deref(),
                    RuntimeExtensionRequest {
                        action: action.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| {
                    format!("runtime extension execution from spec failed: {error}")
                })?;
            Ok(("runtime_extension", json!({ "outcome": outcome })))
        }
        OperationSpec::ToolCore {
            tool_name,
            required_capabilities,
            payload,
            core,
        } => {
            let outcome = kernel
                .execute_tool_core(
                    pack_id,
                    token,
                    required_capabilities,
                    core.as_deref(),
                    ToolCoreRequest {
                        tool_name: tool_name.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("tool core execution from spec failed: {error}"))?;
            Ok(("tool_core", json!({ "outcome": outcome })))
        }
        OperationSpec::ToolExtension {
            extension_action,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let outcome = kernel
                .execute_tool_extension(
                    pack_id,
                    token,
                    required_capabilities,
                    extension,
                    core.as_deref(),
                    ToolExtensionRequest {
                        extension_action: extension_action.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("tool extension execution from spec failed: {error}"))?;
            Ok(("tool_extension", json!({ "outcome": outcome })))
        }
        OperationSpec::MemoryCore {
            operation,
            required_capabilities,
            payload,
            core,
        } => {
            let outcome = kernel
                .execute_memory_core(
                    pack_id,
                    token,
                    required_capabilities,
                    core.as_deref(),
                    MemoryCoreRequest {
                        operation: operation.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("memory core execution from spec failed: {error}"))?;
            Ok(("memory_core", json!({ "outcome": outcome })))
        }
        OperationSpec::MemoryExtension {
            operation,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let outcome = kernel
                .execute_memory_extension(
                    pack_id,
                    token,
                    required_capabilities,
                    extension,
                    core.as_deref(),
                    MemoryExtensionRequest {
                        operation: operation.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("memory extension execution from spec failed: {error}"))?;
            Ok(("memory_extension", json!({ "outcome": outcome })))
        }
        OperationSpec::ToolSearch {
            query,
            limit,
            trust_tiers,
            include_deferred,
            include_examples,
        } => {
            let search_report = execute_tool_search(
                integration_catalog,
                plugin_scan_reports,
                plugin_translation_reports,
                setup_readiness_context,
                plugin_activation_plans,
                query,
                *limit,
                trust_tiers,
                *include_deferred,
                *include_examples,
            );
            Ok((
                "tool_search",
                json!({
                    "query": query,
                    "limit": limit,
                    "trust_tiers": trust_tiers.iter().map(|tier| tier.as_str()).collect::<Vec<_>>(),
                    "include_deferred": include_deferred,
                    "include_examples": include_examples,
                    "returned": search_report.results.len(),
                    "trust_filter_summary": search_report.trust_filter_summary,
                    "results": search_report.results,
                }),
            ))
        }
        OperationSpec::PluginInventory {
            query,
            limit,
            include_ready,
            include_blocked,
            include_deferred,
            include_examples,
        } => {
            let results = execute_plugin_inventory(
                integration_catalog,
                plugin_scan_reports,
                plugin_translation_reports,
                plugin_activation_plans,
                query,
                *limit,
                *include_ready,
                *include_blocked,
                *include_deferred,
                *include_examples,
            );
            Ok((
                "plugin_inventory",
                json!({
                    "query": query,
                    "limit": limit,
                    "include_ready": include_ready,
                    "include_blocked": include_blocked,
                    "include_deferred": include_deferred,
                    "include_examples": include_examples,
                    "returned": results.len(),
                    "results": results,
                }),
            ))
        }
        OperationSpec::PluginPreflight {
            query,
            limit,
            profile,
            policy_path,
            policy_sha256,
            policy_signature,
            include_passed,
            include_warned,
            include_blocked,
            include_deferred,
            include_examples,
        } => {
            let report = execute_plugin_preflight(
                integration_catalog,
                plugin_scan_reports,
                plugin_translation_reports,
                plugin_activation_plans,
                active_bridge_support,
                query,
                *limit,
                *profile,
                policy_path.as_deref(),
                policy_sha256.as_deref(),
                policy_signature.as_ref(),
                *include_passed,
                *include_warned,
                *include_blocked,
                *include_deferred,
                *include_examples,
            )?;
            Ok((
                "plugin_preflight",
                json!({
                    "query": query,
                    "limit": limit,
                    "profile": profile.as_str(),
                    "policy_path": policy_path,
                    "policy_sha256": policy_sha256,
                    "include_passed": include_passed,
                    "include_warned": include_warned,
                    "include_blocked": include_blocked,
                    "include_deferred": include_deferred,
                    "include_examples": include_examples,
                    "summary": report.summary,
                    "returned": report.results.len(),
                    "results": report.results,
                }),
            ))
        }
        OperationSpec::ProgrammaticToolCall {
            caller,
            max_calls,
            include_intermediate,
            allowed_connectors,
            connector_rate_limits,
            connector_circuit_breakers,
            concurrency,
            return_step,
            steps,
        } => {
            let outcome = execute_programmatic_tool_call(
                kernel,
                pack_id,
                token,
                caller,
                *max_calls,
                *include_intermediate,
                allowed_connectors,
                connector_rate_limits,
                connector_circuit_breakers,
                concurrency,
                return_step.as_deref(),
                steps,
            )
            .await?;
            Ok(("programmatic_tool_call", outcome))
        }
    }
}

#[cfg(test)]
#[path = "spec_execution/bootstrap_policy_tests.rs"]
mod bootstrap_policy_tests;

#[cfg(test)]
#[path = "spec_execution/setup_readiness_context_tests.rs"]
mod setup_readiness_context_tests;

#[cfg(test)]
#[path = "spec_execution/bridge_runtime_policy_tests.rs"]
mod bridge_runtime_policy_tests;

#[cfg(test)]
#[path = "spec_execution/plugin_metadata_tests.rs"]
mod plugin_metadata_tests;
