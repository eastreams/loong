use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use kernel::{
    ArchitectureBoundaryPolicy, ArchitectureGuardReport, AuditEventKind, AutoProvisionAgent,
    AutoProvisionRequest, BootstrapPolicy, BootstrapReport, BootstrapTaskStatus,
    BridgeSupportMatrix, Clock, CodebaseAwarenessConfig, CodebaseAwarenessEngine,
    CodebaseAwarenessSnapshot, ConnectorCommand, InMemoryAuditSink, IntegrationCatalog,
    LoongClawKernel, MemoryCoreRequest, MemoryExtensionRequest, PluginAbsorbReport,
    PluginActivationPlan, PluginActivationStatus, PluginBootstrapExecutor, PluginBridgeKind,
    PluginCompatibility, PluginCompatibilityShimSupport, PluginDescriptor, PluginScanReport,
    PluginScanner, PluginSetup, PluginSetupReadinessContext, PluginSlotClaim,
    PluginTranslationReport, PluginTranslator, ProvisionPlan, RuntimeCoreRequest,
    RuntimeExtensionRequest, StaticPolicyEngine, SystemClock, TaskIntent, ToolCoreRequest,
    ToolExtensionRequest, plugin_bridge_is_high_risk_auto_apply,
    plugin_provenance_summary_for_descriptor,
};
use serde_json::{Value, json};

use crate::CliResult;
use crate::kernel_bootstrap::default_in_memory_audit_sink;
use crate::programmatic::execute_programmatic_tool_call;
use crate::spec_runtime::*;

mod approval_policy;
mod bridge_support_policy;
mod plugin_inventory;
mod plugin_preflight;
mod plugin_preflight_policy;
mod security_scan_eval;
mod security_scan_policy;
mod tool_search;
use approval_policy::evaluate_approval_guard;
use bridge_support_policy::bridge_support_policy_checksum;
use plugin_inventory::execute_plugin_inventory;
use plugin_preflight::execute_plugin_preflight;
use security_scan_eval::evaluate_plugin_security_scan;
use security_scan_policy::{
    apply_security_scan_delta, emit_security_scan_siem_record, security_scan_process_allowlist,
};
use tool_search::execute_tool_search;

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

fn blocked_outcome(reason: &str) -> Value {
    json!({
        "status": "blocked",
        "reason": reason,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_blocked_spec_run_report(
    pack_id: String,
    agent_id: String,
    reason: String,
    approval_guard: ApprovalDecisionReport,
    bridge_support_source: Option<String>,
    bridge_support_checksum: Option<String>,
    bridge_support_sha256: Option<String>,
    bridge_support_delta_source: Option<String>,
    bridge_support_delta_sha256: Option<String>,
    self_awareness: Option<CodebaseAwarenessSnapshot>,
    architecture_guard: Option<ArchitectureGuardReport>,
    plugin_scan_reports: Vec<PluginScanReport>,
    plugin_translation_reports: Vec<PluginTranslationReport>,
    plugin_activation_plans: Vec<PluginActivationPlan>,
    plugin_bootstrap_reports: Vec<BootstrapReport>,
    plugin_bootstrap_queue: Vec<String>,
    plugin_absorb_reports: Vec<PluginAbsorbReport>,
    security_scan_report: Option<SecurityScanReport>,
    auto_provision_plan: Option<ProvisionPlan>,
    integration_catalog: IntegrationCatalog,
    include_audit: bool,
    audit_sink: &Arc<InMemoryAuditSink>,
) -> SpecRunReport {
    let plugin_trust_summary = build_plugin_trust_summary(
        &plugin_scan_reports,
        &plugin_activation_plans,
        &plugin_bootstrap_reports,
    );
    SpecRunReport {
        schema_version: SPEC_RUN_REPORT_SCHEMA_VERSION,
        schema: json_schema_descriptor(
            SPEC_RUN_REPORT_SCHEMA_VERSION,
            SPEC_RUN_REPORT_SCHEMA_SURFACE,
            SPEC_RUN_REPORT_SCHEMA_PURPOSE,
        ),
        pack_id,
        agent_id,
        operation_kind: "blocked",
        blocked_reason: Some(reason.clone()),
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
        tool_search_summary: None,
        plugin_bootstrap_queue,
        plugin_absorb_reports,
        security_scan_report,
        auto_provision_plan,
        outcome: blocked_outcome(&reason),
        integration_catalog,
        audit_events: if include_audit {
            Some(audit_sink.snapshot())
        } else {
            None
        },
    }
}

fn build_plugin_trust_summary(
    plugin_scan_reports: &[PluginScanReport],
    plugin_activation_plans: &[PluginActivationPlan],
    plugin_bootstrap_reports: &[BootstrapReport],
) -> PluginTrustSummary {
    let mut summary = PluginTrustSummary::default();
    let mut provenance_by_plugin = BTreeMap::new();
    let mut bootstrap_by_plugin = BTreeMap::new();

    for report in plugin_scan_reports {
        for descriptor in &report.descriptors {
            summary.scanned_plugins = summary.scanned_plugins.saturating_add(1);
            match descriptor.manifest.trust_tier {
                kernel::PluginTrustTier::Official => {
                    summary.official_plugins = summary.official_plugins.saturating_add(1);
                }
                kernel::PluginTrustTier::VerifiedCommunity => {
                    summary.verified_community_plugins =
                        summary.verified_community_plugins.saturating_add(1);
                }
                kernel::PluginTrustTier::Unverified => {
                    summary.unverified_plugins = summary.unverified_plugins.saturating_add(1);
                }
            }
            provenance_by_plugin.insert(
                (
                    descriptor.path.clone(),
                    descriptor.manifest.plugin_id.clone(),
                ),
                plugin_provenance_summary_for_descriptor(descriptor),
            );
        }
    }

    for report in plugin_bootstrap_reports {
        for task in &report.tasks {
            bootstrap_by_plugin.insert(
                (task.source_path.clone(), task.plugin_id.clone()),
                (task.status, task.reason.clone()),
            );
        }
    }

    for plan in plugin_activation_plans {
        for candidate in &plan.candidates {
            if plugin_bridge_is_high_risk_auto_apply(candidate.bridge_kind) {
                summary.high_risk_plugins = summary.high_risk_plugins.saturating_add(1);
            }
            if !matches!(candidate.trust_tier, kernel::PluginTrustTier::Unverified)
                || !plugin_bridge_is_high_risk_auto_apply(candidate.bridge_kind)
            {
                continue;
            }

            summary.high_risk_unverified_plugins =
                summary.high_risk_unverified_plugins.saturating_add(1);

            let plugin_key = (candidate.source_path.clone(), candidate.plugin_id.clone());
            let provenance_summary = provenance_by_plugin
                .get(&plugin_key)
                .cloned()
                .unwrap_or_else(|| {
                    kernel::format_plugin_provenance_summary(
                        candidate.source_kind,
                        &candidate.source_path,
                        candidate.package_manifest_path.as_deref(),
                    )
                });
            let (bootstrap_status, bootstrap_reason) = bootstrap_by_plugin
                .get(&plugin_key)
                .map(|(status, reason)| (Some(*status), Some(reason.clone())))
                .unwrap_or((None, None));

            if matches!(
                bootstrap_status,
                Some(BootstrapTaskStatus::DeferredUnsupportedAutoApply)
            ) && bootstrap_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("bootstrap trust policy"))
            {
                summary.blocked_auto_apply_plugins =
                    summary.blocked_auto_apply_plugins.saturating_add(1);
            }

            summary
                .review_required_plugins
                .push(PluginTrustReviewEntry {
                    plugin_id: candidate.plugin_id.clone(),
                    source_path: candidate.source_path.clone(),
                    provenance_summary,
                    trust_tier: candidate.trust_tier,
                    bridge_kind: candidate.bridge_kind,
                    activation_status: candidate.status,
                    bootstrap_status,
                    reason: bootstrap_reason.unwrap_or_else(|| candidate.reason.clone()),
                });
        }
    }

    summary.review_required_plugins.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });

    summary
}

fn build_tool_search_operation_summary(outcome: &Value) -> Option<ToolSearchOperationSummary> {
    let payload = outcome.as_object()?;
    let results = payload.get("results")?.as_array()?;
    let top_results = results
        .iter()
        .take(3)
        .filter_map(build_tool_search_operation_summary_entry)
        .collect::<Vec<_>>();
    let trust_filter_summary = payload
        .get("trust_filter_summary")
        .cloned()
        .and_then(|value| serde_json::from_value::<ToolSearchTrustFilterSummary>(value).ok())
        .unwrap_or_default();
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let returned = payload
        .get("returned")
        .and_then(Value::as_u64)
        .map_or(results.len(), |value| value as usize);
    let trust_tiers = payload
        .get("trust_tiers")
        .and_then(Value::as_array)
        .map(|tiers| {
            tiers
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(ToolSearchOperationSummary {
        headline: build_tool_search_operation_headline(
            &query,
            returned,
            &trust_tiers,
            &trust_filter_summary,
            &top_results,
        ),
        query,
        returned,
        trust_tiers,
        trust_filter_summary,
        top_results,
    })
}

fn build_tool_search_operation_summary_entry(
    value: &Value,
) -> Option<ToolSearchOperationSummaryEntry> {
    let entry = value.as_object()?;
    Some(ToolSearchOperationSummaryEntry {
        tool_id: entry.get("tool_id")?.as_str()?.to_owned(),
        provider_id: entry.get("provider_id")?.as_str()?.to_owned(),
        connector_name: entry.get("connector_name")?.as_str()?.to_owned(),
        trust_tier: entry
            .get("trust_tier")
            .and_then(Value::as_str)
            .map(str::to_owned),
        bridge_kind: entry.get("bridge_kind")?.as_str()?.to_owned(),
        score: entry
            .get("score")
            .and_then(Value::as_u64)
            .map_or(0, |value| value as u32),
        setup_ready: entry
            .get("setup_ready")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        deferred: entry
            .get("deferred")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        loaded: entry
            .get("loaded")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn build_tool_search_operation_headline(
    query: &str,
    returned: usize,
    trust_tiers: &[String],
    trust_filter_summary: &ToolSearchTrustFilterSummary,
    top_results: &[ToolSearchOperationSummaryEntry],
) -> String {
    let result_noun = if returned == 1 { "result" } else { "results" };
    let mut parts = vec![format!("returned {returned} {result_noun}")];

    if trust_filter_summary.applied {
        let scope = if trust_filter_summary.effective_tiers.is_empty() {
            "none".to_owned()
        } else {
            trust_filter_summary.effective_tiers.join(",")
        };
        parts.push(format!("trust_scope={scope}"));
        if trust_filter_summary.filtered_out_candidates > 0 {
            let filtered_noun = if trust_filter_summary.filtered_out_candidates == 1 {
                "candidate"
            } else {
                "candidates"
            };
            parts.push(format!(
                "filtered_out={} {filtered_noun}",
                trust_filter_summary.filtered_out_candidates
            ));
        }
        if trust_filter_summary.conflicting_requested_tiers {
            parts.push("conflicting_trust_filters=true".to_owned());
        }
    } else if !trust_tiers.is_empty() {
        parts.push(format!("requested_tiers={}", trust_tiers.join(",")));
    }

    if let Some(first) = top_results.first() {
        parts.push(format!("top_match={}", first.provider_id));
    }

    if query.is_empty() {
        parts.join("; ")
    } else {
        format!("query=\"{query}\"; {}", parts.join("; "))
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

fn emit_security_scan_audit_event(
    kernel: &LoongClawKernel<StaticPolicyEngine>,
    pack_id: &str,
    agent_id: &str,
    report: &SecurityScanReport,
) -> Result<(), String> {
    if report.scanned_plugins == 0 && report.total_findings == 0 {
        return Ok(());
    }

    let categories: Vec<String> = report
        .findings
        .iter()
        .map(|finding| finding.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let finding_ids: Vec<String> = report
        .findings
        .iter()
        .map(|finding| finding.correlation_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    kernel
        .record_audit_event(
            Some(agent_id),
            AuditEventKind::SecurityScanEvaluated {
                pack_id: pack_id.to_owned(),
                scanned_plugins: report.scanned_plugins,
                total_findings: report.total_findings,
                high_findings: report.high_findings,
                medium_findings: report.medium_findings,
                low_findings: report.low_findings,
                blocked: report.blocked,
                block_reason: report.block_reason.clone(),
                categories,
                finding_ids,
            },
        )
        .map_err(|error| format!("failed to record security scan audit event: {error}"))
}

fn emit_plugin_trust_audit_event(
    kernel: &LoongClawKernel<StaticPolicyEngine>,
    pack_id: &str,
    agent_id: &str,
    summary: &PluginTrustSummary,
) -> Result<(), String> {
    if summary.scanned_plugins == 0 {
        return Ok(());
    }

    let review_required_plugin_ids: Vec<String> = summary
        .review_required_plugins
        .iter()
        .map(|entry| entry.plugin_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let review_required_bridges: Vec<String> = summary
        .review_required_plugins
        .iter()
        .map(|entry| entry.bridge_kind.as_str().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    kernel
        .record_audit_event(
            Some(agent_id),
            AuditEventKind::PluginTrustEvaluated {
                pack_id: pack_id.to_owned(),
                scanned_plugins: summary.scanned_plugins,
                official_plugins: summary.official_plugins,
                verified_community_plugins: summary.verified_community_plugins,
                unverified_plugins: summary.unverified_plugins,
                high_risk_plugins: summary.high_risk_plugins,
                high_risk_unverified_plugins: summary.high_risk_unverified_plugins,
                blocked_auto_apply_plugins: summary.blocked_auto_apply_plugins,
                review_required_plugin_ids,
                review_required_bridges,
            },
        )
        .map_err(|error| format!("failed to record plugin trust audit event: {error}"))
}

fn emit_tool_search_audit_event(
    kernel: &LoongClawKernel<StaticPolicyEngine>,
    pack_id: &str,
    agent_id: &str,
    summary: &ToolSearchOperationSummary,
) -> Result<(), String> {
    let top_provider_ids = summary
        .top_results
        .iter()
        .map(|entry| entry.provider_id.clone())
        .collect::<Vec<_>>();

    kernel
        .record_audit_event(
            Some(agent_id),
            AuditEventKind::ToolSearchEvaluated {
                pack_id: pack_id.to_owned(),
                query: summary.query.clone(),
                returned: summary.returned,
                trust_filter_applied: summary.trust_filter_summary.applied,
                query_requested_tiers: summary.trust_filter_summary.query_requested_tiers.clone(),
                structured_requested_tiers: summary
                    .trust_filter_summary
                    .structured_requested_tiers
                    .clone(),
                effective_tiers: summary.trust_filter_summary.effective_tiers.clone(),
                conflicting_requested_tiers: summary
                    .trust_filter_summary
                    .conflicting_requested_tiers,
                filtered_out_candidates: summary.trust_filter_summary.filtered_out_candidates,
                filtered_out_tier_counts: summary
                    .trust_filter_summary
                    .filtered_out_tier_counts
                    .clone(),
                top_provider_ids,
            },
        )
        .map_err(|error| format!("failed to record tool search audit event: {error}"))
}

pub fn resolve_plugin_relative_path(source_path: &str, artifact: &str) -> PathBuf {
    let candidate = PathBuf::from(artifact);
    if candidate.is_absolute() {
        return candidate;
    }

    let source = Path::new(source_path);
    if let Some(parent) = source.parent() {
        parent.join(candidate)
    } else {
        candidate
    }
}

fn normalize_allowed_path_prefixes(prefixes: &[String]) -> Vec<PathBuf> {
    prefixes
        .iter()
        .map(|prefix| normalize_path_for_policy(&PathBuf::from(prefix)))
        .collect()
}

fn validate_wasm_guest_readable_config_keys(keys: &BTreeSet<String>) -> CliResult<()> {
    for key in keys {
        let key_is_supported = wasm_guest_config_key_is_supported(key.as_str());
        if key_is_supported {
            continue;
        }

        return Err(format!(
            "invalid security scan runtime.guest_readable_config_keys entry `{key}`: entries must use `{WASM_GUEST_CONFIG_PROVIDER_PREFIX}<metadata_key>` or `{WASM_GUEST_CONFIG_CHANNEL_PREFIX}<metadata_key>`"
        ));
    }

    Ok(())
}

pub fn normalize_path_for_policy(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn descriptor_bridge_kind(descriptor: &PluginDescriptor) -> PluginBridgeKind {
    if let Some(raw) = descriptor.manifest.metadata.get("bridge_kind")
        && let Some(kind) = parse_bridge_kind_label(raw)
    {
        return kind;
    }

    let language = descriptor.language.trim().to_ascii_lowercase();
    match language.as_str() {
        "wasm" | "wat" => return PluginBridgeKind::WasmComponent,
        "rust" | "go" | "c" | "cpp" | "cxx" => return PluginBridgeKind::NativeFfi,
        "python" | "javascript" | "typescript" | "java" => return PluginBridgeKind::ProcessStdio,
        _ => {}
    }

    if let Some(endpoint) = descriptor.manifest.endpoint.as_deref() {
        let endpoint_lower = endpoint.to_ascii_lowercase();
        if endpoint_lower.starts_with("http://") || endpoint_lower.starts_with("https://") {
            return PluginBridgeKind::HttpJson;
        }
        if endpoint_lower.ends_with(".wasm") {
            return PluginBridgeKind::WasmComponent;
        }
    }

    PluginBridgeKind::Unknown
}

fn bridge_support_matrix(spec: &RunnerSpec) -> (BridgeSupportMatrix, bool) {
    match &spec.bridge_support {
        Some(bridge) if bridge.enabled => {
            (bridge_support_spec_matrix(bridge), bridge.enforce_supported)
        }
        _ => (BridgeSupportMatrix::default(), false),
    }
}

fn resolve_plugin_setup_readiness_context<I>(
    readiness_spec: Option<&PluginSetupReadinessSpec>,
    env_vars: I,
) -> PluginSetupReadinessContext
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let Some(readiness_spec) = readiness_spec else {
        let verified_env_vars = collect_verified_env_var_names(env_vars);

        return PluginSetupReadinessContext {
            verified_env_vars,
            verified_config_keys: BTreeSet::new(),
        };
    };

    let mut verified_env_vars = BTreeSet::new();
    if readiness_spec.inherit_process_env {
        verified_env_vars = collect_verified_env_var_names(env_vars);
    }

    let explicit_verified_env_vars = collect_verified_name_list(&readiness_spec.verified_env_vars);
    verified_env_vars.extend(explicit_verified_env_vars);

    let verified_config_keys = collect_verified_name_list(&readiness_spec.verified_config_keys);

    PluginSetupReadinessContext {
        verified_env_vars,
        verified_config_keys,
    }
}

fn collect_verified_env_var_names<I>(env_vars: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut verified_env_vars = BTreeSet::new();

    for (raw_name, raw_value) in env_vars {
        let name = raw_name.to_string_lossy().into_owned();
        let trimmed_name = name.trim();
        let name_is_blank = trimmed_name.is_empty();
        if name_is_blank {
            continue;
        }

        let value = raw_value.to_string_lossy().into_owned();
        let trimmed_value = value.trim();
        let value_is_blank = trimmed_value.is_empty();
        if value_is_blank {
            continue;
        }

        verified_env_vars.insert(name);
    }

    verified_env_vars
}

fn collect_verified_name_list(values: &[String]) -> BTreeSet<String> {
    let mut verified_names = BTreeSet::new();

    for raw_value in values {
        let value = raw_value.trim().to_owned();
        if value.is_empty() {
            continue;
        }

        verified_names.insert(value);
    }

    verified_names
}

pub(super) fn bridge_support_spec_matrix(bridge: &BridgeSupportSpec) -> BridgeSupportMatrix {
    let mut matrix = BridgeSupportMatrix::default();
    if !bridge.supported_bridges.is_empty() {
        matrix.supported_bridges = bridge.supported_bridges.iter().copied().collect();
    }
    if !bridge.supported_adapter_families.is_empty() {
        matrix.supported_adapter_families =
            bridge.supported_adapter_families.iter().cloned().collect();
    }
    if !bridge.supported_compatibility_modes.is_empty() {
        matrix.supported_compatibility_modes = bridge
            .supported_compatibility_modes
            .iter()
            .copied()
            .collect();
    }
    if !bridge.supported_compatibility_shims.is_empty() {
        matrix.supported_compatibility_shims = bridge
            .supported_compatibility_shims
            .iter()
            .cloned()
            .collect();
    }
    if !bridge.supported_compatibility_shim_profiles.is_empty() {
        matrix.supported_compatibility_shim_profiles = bridge
            .supported_compatibility_shim_profiles
            .iter()
            .cloned()
            .map(PluginCompatibilityShimSupport::normalized)
            .map(|profile| (profile.shim.clone(), profile))
            .collect();
        matrix
            .supported_compatibility_shims
            .extend(matrix.supported_compatibility_shim_profiles.keys().cloned());
    }
    matrix
}

fn raw_bridge_runtime_spec(spec: &RunnerSpec) -> SecurityRuntimeExecutionSpec {
    let raw_runtime = spec
        .bridge_support
        .as_ref()
        .filter(|bridge| bridge.enabled)
        .and_then(|bridge| bridge.security_scan.as_ref())
        .map(|scan| scan.runtime.clone());

    raw_runtime.unwrap_or_default()
}

fn bridge_runtime_policy(
    spec: &RunnerSpec,
    security_scan: Option<&SecurityScanSpec>,
) -> CliResult<BridgeRuntimePolicy> {
    let Some(bridge) = &spec.bridge_support else {
        return Ok(BridgeRuntimePolicy::default());
    };
    if !bridge.enabled {
        return Ok(BridgeRuntimePolicy::default());
    }

    let runtime = security_scan
        .map(|scan| scan.runtime.clone())
        .unwrap_or_default();
    let raw_runtime = raw_bridge_runtime_spec(spec);
    let bridge_circuit_breaker = if security_scan.is_some() {
        runtime.bridge_circuit_breaker.clone()
    } else {
        raw_runtime.bridge_circuit_breaker
    };
    validate_connector_circuit_breaker_policy(
        &bridge_circuit_breaker,
        "bridge runtime circuit breaker",
    )?;
    let (compatibility_matrix, _) = bridge_support_matrix(spec);
    let (wasm_require_hash_pin, wasm_required_sha256_by_plugin) = security_scan
        .map(|scan| {
            (
                scan.wasm.require_hash_pin,
                scan.wasm.required_sha256_by_plugin.clone(),
            )
        })
        .unwrap_or_else(|| (false, BTreeMap::new()));
    let wasm_allowed_path_prefixes = runtime
        .allowed_path_prefixes
        .iter()
        .map(PathBuf::from)
        .map(|path| normalize_path_for_policy(&path))
        .collect();
    let wasm_guest_readable_config_keys =
        collect_verified_name_list(&runtime.guest_readable_config_keys);
    validate_wasm_guest_readable_config_keys(&wasm_guest_readable_config_keys)?;

    Ok(BridgeRuntimePolicy {
        execute_process_stdio: bridge.execute_process_stdio,
        execute_http_json: bridge.execute_http_json,
        execute_wasm_component: runtime.execute_wasm_component,
        compatibility_matrix,
        allowed_process_commands: bridge
            .allowed_process_commands
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect(),
        bridge_circuit_breaker,
        wasm_allowed_path_prefixes,
        wasm_guest_readable_config_keys,
        wasm_max_component_bytes: runtime.max_component_bytes,
        wasm_max_output_bytes: runtime.max_output_bytes,
        wasm_fuel_limit: runtime.fuel_limit,
        wasm_timeout_ms: runtime.timeout_ms,
        wasm_require_hash_pin,
        wasm_required_sha256_by_plugin,
        enforce_execution_success: bridge.enforce_execution_success,
    })
}

pub fn current_epoch_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn bootstrap_policy(spec: &RunnerSpec) -> Option<BootstrapPolicy> {
    let bootstrap = spec.bootstrap.as_ref()?;
    if !bootstrap.enabled {
        return None;
    }

    let mut policy = BootstrapPolicy::default();
    if let Some(value) = bootstrap.allow_http_json_auto_apply {
        policy.allow_http_json_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_process_stdio_auto_apply {
        policy.allow_process_stdio_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_native_ffi_auto_apply {
        policy.allow_native_ffi_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_wasm_component_auto_apply {
        policy.allow_wasm_component_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_mcp_server_auto_apply {
        policy.allow_mcp_server_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_acp_bridge_auto_apply {
        policy.allow_acp_bridge_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_acp_runtime_auto_apply {
        policy.allow_acp_runtime_auto_apply = value;
    }
    if let Some(value) = bootstrap.block_unverified_high_risk_auto_apply {
        policy.block_unverified_high_risk_auto_apply = value;
    }
    if let Some(value) = bootstrap.enforce_ready_execution {
        policy.enforce_ready_execution = value;
    }
    if let Some(value) = bootstrap.max_tasks {
        policy.max_tasks = value.max(1);
    }
    Some(policy)
}

fn filter_scan_report_by_activation(
    report: &PluginScanReport,
    activation: &PluginActivationPlan,
) -> PluginScanReport {
    let ready_keys: BTreeSet<(String, String)> = activation
        .candidates
        .iter()
        .filter(|candidate| matches!(candidate.status, PluginActivationStatus::Ready))
        .map(|candidate| (candidate.source_path.clone(), candidate.plugin_id.clone()))
        .collect();

    filter_scan_report_by_keys(report, &ready_keys)
}

fn filter_scan_report_by_keys(
    report: &PluginScanReport,
    allowed_keys: &BTreeSet<(String, String)>,
) -> PluginScanReport {
    let descriptors: Vec<PluginDescriptor> = report
        .descriptors
        .iter()
        .filter(|descriptor| {
            allowed_keys.contains(&(
                descriptor.path.clone(),
                descriptor.manifest.plugin_id.clone(),
            ))
        })
        .cloned()
        .collect();
    let diagnostic_findings = report
        .diagnostic_findings
        .iter()
        .filter(|finding| {
            let (Some(source_path), Some(plugin_id)) =
                (finding.source_path.as_deref(), finding.plugin_id.as_deref())
            else {
                return false;
            };

            allowed_keys.contains(&(source_path.to_owned(), plugin_id.to_owned()))
        })
        .cloned()
        .collect();

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        diagnostic_findings,
        descriptors,
    }
}

fn enrich_scan_report_with_translation(
    report: &PluginScanReport,
    translation: &PluginTranslationReport,
    activation: Option<&PluginActivationPlan>,
) -> PluginScanReport {
    let mut runtime_by_key: BTreeMap<(String, String), (String, String, String, String)> =
        BTreeMap::new();
    let mut activation_contracts_by_key: BTreeMap<
        (String, String),
        PluginActivationRuntimeContract,
    > = BTreeMap::new();

    for entry in &translation.entries {
        runtime_by_key.insert(
            (entry.source_path.clone(), entry.plugin_id.clone()),
            (
                entry.runtime.bridge_kind.as_str().to_owned(),
                entry.runtime.adapter_family.clone(),
                entry.runtime.entrypoint_hint.clone(),
                entry.runtime.source_language.clone(),
            ),
        );
    }

    if let Some(activation) = activation {
        for entry in &translation.entries {
            let Some(candidate) = activation.candidate_for(&entry.source_path, &entry.plugin_id)
            else {
                continue;
            };
            if !matches!(candidate.status, PluginActivationStatus::Ready) {
                continue;
            }

            activation_contracts_by_key.insert(
                (entry.source_path.clone(), entry.plugin_id.clone()),
                PluginActivationRuntimeContract {
                    plugin_id: entry.plugin_id.clone(),
                    source_path: entry.source_path.clone(),
                    source_kind: entry.source_kind,
                    dialect: entry.dialect,
                    dialect_version: entry.dialect_version.clone(),
                    compatibility_mode: entry.compatibility_mode,
                    compatibility_shim: candidate.compatibility_shim.clone(),
                    bridge_kind: entry.runtime.bridge_kind,
                    adapter_family: entry.runtime.adapter_family.clone(),
                    entrypoint_hint: entry.runtime.entrypoint_hint.clone(),
                    source_language: entry.runtime.source_language.clone(),
                    compatibility: entry.compatibility.clone(),
                },
            );
        }
    }

    let descriptors: Vec<PluginDescriptor> = report
        .descriptors
        .iter()
        .cloned()
        .map(|mut descriptor| {
            stamp_plugin_provenance_metadata(&mut descriptor);
            descriptor.manifest.metadata.insert(
                "plugin_id".to_owned(),
                descriptor.manifest.plugin_id.clone(),
            );
            descriptor.manifest.metadata.insert(
                "defer_loading".to_owned(),
                descriptor.manifest.defer_loading.to_string(),
            );
            let setup = descriptor.manifest.setup.clone();
            insert_plugin_setup_metadata(&mut descriptor.manifest.metadata, setup.as_ref());
            insert_plugin_slot_claims_metadata(
                &mut descriptor.manifest.metadata,
                &descriptor.manifest.slot_claims,
            );
            let manifest_api_version = descriptor.manifest.api_version.clone();
            let plugin_version = descriptor.manifest.version.clone();
            insert_plugin_manifest_contract_metadata(
                &mut descriptor.manifest.metadata,
                manifest_api_version,
                plugin_version,
            );
            insert_plugin_compatibility_metadata(
                &mut descriptor.manifest.metadata,
                descriptor.manifest.compatibility.as_ref(),
            );
            if let Some(summary) = descriptor.manifest.summary.clone() {
                descriptor
                    .manifest
                    .metadata
                    .entry("summary".to_owned())
                    .or_insert(summary);
            }
            if !descriptor.manifest.tags.is_empty()
                && let Ok(tags_json) = serde_json::to_string(&descriptor.manifest.tags)
            {
                descriptor
                    .manifest
                    .metadata
                    .entry("tags_json".to_owned())
                    .or_insert(tags_json);
            }
            if !descriptor.manifest.input_examples.is_empty()
                && let Ok(input_examples_json) =
                    serde_json::to_string(&descriptor.manifest.input_examples)
            {
                descriptor
                    .manifest
                    .metadata
                    .entry("input_examples_json".to_owned())
                    .or_insert(input_examples_json);
            }
            if !descriptor.manifest.output_examples.is_empty()
                && let Ok(output_examples_json) =
                    serde_json::to_string(&descriptor.manifest.output_examples)
            {
                descriptor
                    .manifest
                    .metadata
                    .entry("output_examples_json".to_owned())
                    .or_insert(output_examples_json);
            }
            if let Some(component) = descriptor.manifest.metadata.get("component").cloned() {
                let resolved = resolve_plugin_relative_path(&descriptor.path, &component);
                let normalized = normalize_path_for_policy(&resolved);
                descriptor.manifest.metadata.insert(
                    "component_resolved_path".to_owned(),
                    normalized.display().to_string(),
                );
            } else {
                descriptor
                    .manifest
                    .metadata
                    .remove("component_resolved_path");
            }

            if let Some((bridge_kind, adapter_family, entrypoint_hint, source_language)) =
                runtime_by_key.get(&(
                    descriptor.path.clone(),
                    descriptor.manifest.plugin_id.clone(),
                ))
            {
                descriptor
                    .manifest
                    .metadata
                    .insert("bridge_kind".to_owned(), bridge_kind.clone());
                descriptor
                    .manifest
                    .metadata
                    .insert("adapter_family".to_owned(), adapter_family.clone());
                descriptor
                    .manifest
                    .metadata
                    .insert("entrypoint_hint".to_owned(), entrypoint_hint.clone());
                descriptor
                    .manifest
                    .metadata
                    .insert("source_language".to_owned(), source_language.clone());
            } else {
                descriptor.manifest.metadata.remove("bridge_kind");
                descriptor.manifest.metadata.remove("adapter_family");
                descriptor.manifest.metadata.remove("entrypoint_hint");
                descriptor.manifest.metadata.remove("source_language");
            }
            insert_plugin_activation_runtime_contract_metadata(
                &mut descriptor.manifest.metadata,
                activation_contracts_by_key.get(&(
                    descriptor.path.clone(),
                    descriptor.manifest.plugin_id.clone(),
                )),
            );
            descriptor
        })
        .collect();

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        diagnostic_findings: report.diagnostic_findings.clone(),
        descriptors,
    }
}

fn insert_plugin_activation_runtime_contract_metadata(
    metadata: &mut BTreeMap<String, String>,
    contract: Option<&PluginActivationRuntimeContract>,
) {
    let Some(contract) = contract else {
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY);
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY);
        return;
    };

    let Ok(serialized) = plugin_activation_runtime_contract_json(contract) else {
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY);
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY);
        return;
    };

    metadata.insert(
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY.to_owned(),
        serialized.clone(),
    );
    metadata.insert(
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY.to_owned(),
        activation_runtime_contract_checksum_hex(serialized.as_bytes()),
    );
}

fn stamp_plugin_provenance_metadata(descriptor: &mut PluginDescriptor) {
    let source_path_key = "plugin_source_path".to_owned();
    let source_path_value = descriptor.path.clone();
    let source_kind_key = "plugin_source_kind".to_owned();
    let source_kind_value = descriptor.source_kind.as_str().to_owned();
    let dialect_key = "plugin_dialect".to_owned();
    let dialect_value = descriptor.dialect.as_str().to_owned();
    let compatibility_mode_key = "plugin_compatibility_mode".to_owned();
    let compatibility_mode_value = descriptor.compatibility_mode.as_str().to_owned();
    let package_root_key = "plugin_package_root".to_owned();
    let package_root_value = descriptor.package_root.clone();
    let package_manifest_path_value = descriptor.package_manifest_path.clone();
    let provenance_summary_key = "plugin_provenance_summary".to_owned();
    let provenance_summary_value = plugin_provenance_summary_for_descriptor(descriptor);
    let trust_tier_key = "plugin_trust_tier".to_owned();
    let trust_tier_value = descriptor.manifest.trust_tier.as_str().to_owned();
    let metadata = &mut descriptor.manifest.metadata;

    metadata.insert(source_path_key, source_path_value);
    metadata.insert(source_kind_key, source_kind_value);
    metadata.insert(dialect_key, dialect_value);
    metadata.insert(compatibility_mode_key, compatibility_mode_value);
    metadata.insert(package_root_key, package_root_value);
    metadata.insert(provenance_summary_key, provenance_summary_value);
    metadata.insert(trust_tier_key, trust_tier_value);

    if let Some(shim) = kernel::PluginCompatibilityShim::for_mode(descriptor.compatibility_mode) {
        metadata.insert("plugin_compatibility_shim_id".to_owned(), shim.shim_id);
        metadata.insert("plugin_compatibility_shim_family".to_owned(), shim.family);
    } else {
        metadata.remove("plugin_compatibility_shim_id");
        metadata.remove("plugin_compatibility_shim_family");
    }

    if let Some(dialect_version) = descriptor.dialect_version.clone() {
        metadata.insert("plugin_dialect_version".to_owned(), dialect_version);
    } else {
        metadata.remove("plugin_dialect_version");
    }

    if let Some(package_manifest_path_value) = package_manifest_path_value {
        let package_manifest_path_key = "plugin_package_manifest_path".to_owned();

        metadata.insert(package_manifest_path_key, package_manifest_path_value);
    } else {
        metadata.remove("plugin_package_manifest_path");
    }
}

fn insert_plugin_setup_metadata(
    metadata: &mut BTreeMap<String, String>,
    setup: Option<&PluginSetup>,
) {
    let Some(setup) = setup else {
        metadata.remove("plugin_setup_mode");
        metadata.remove("plugin_setup_surface");
        metadata.remove("plugin_setup_required_env_vars_json");
        metadata.remove("plugin_setup_recommended_env_vars_json");
        metadata.remove("plugin_setup_required_config_keys_json");
        metadata.remove("plugin_setup_default_env_var");
        metadata.remove("plugin_setup_docs_urls_json");
        metadata.remove("plugin_setup_remediation");
        return;
    };

    let mode_key = "plugin_setup_mode".to_owned();
    let mode_value = setup.mode.as_str().to_owned();
    metadata.insert(mode_key, mode_value);

    if let Some(surface) = setup.surface.clone() {
        let surface_key = "plugin_setup_surface".to_owned();
        metadata.insert(surface_key, surface);
    } else {
        metadata.remove("plugin_setup_surface");
    }

    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_required_env_vars_json",
        &setup.required_env_vars,
    );
    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_recommended_env_vars_json",
        &setup.recommended_env_vars,
    );
    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_required_config_keys_json",
        &setup.required_config_keys,
    );

    if let Some(default_env_var) = setup.default_env_var.clone() {
        let default_env_var_key = "plugin_setup_default_env_var".to_owned();
        metadata.insert(default_env_var_key, default_env_var);
    } else {
        metadata.remove("plugin_setup_default_env_var");
    }

    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_docs_urls_json",
        &setup.docs_urls,
    );

    if let Some(remediation) = setup.remediation.clone() {
        let remediation_key = "plugin_setup_remediation".to_owned();
        metadata.insert(remediation_key, remediation);
    } else {
        metadata.remove("plugin_setup_remediation");
    }
}

fn insert_plugin_setup_string_list_metadata(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    values: &[String],
) {
    let is_empty = values.is_empty();

    if is_empty {
        metadata.remove(key);
        return;
    }

    let serialized = serde_json::to_string(values);
    let Ok(serialized) = serialized else {
        metadata.remove(key);
        return;
    };

    let metadata_key = key.to_owned();
    metadata.insert(metadata_key, serialized);
}

fn insert_plugin_slot_claims_metadata(
    metadata: &mut BTreeMap<String, String>,
    slot_claims: &[PluginSlotClaim],
) {
    if slot_claims.is_empty() {
        metadata.remove("plugin_slot_claims_json");
        return;
    }

    if let Ok(serialized) = serde_json::to_string(slot_claims) {
        metadata.insert("plugin_slot_claims_json".to_owned(), serialized);
    }
}

fn insert_plugin_manifest_contract_metadata(
    metadata: &mut BTreeMap<String, String>,
    manifest_api_version: Option<String>,
    plugin_version: Option<String>,
) {
    if let Some(api_version) = manifest_api_version {
        metadata.insert("plugin_manifest_api_version".to_owned(), api_version);
    } else {
        metadata.remove("plugin_manifest_api_version");
    }

    if let Some(version) = plugin_version {
        metadata.insert("plugin_version".to_owned(), version);
    } else {
        metadata.remove("plugin_version");
    }
}

fn insert_plugin_compatibility_metadata(
    metadata: &mut BTreeMap<String, String>,
    compatibility: Option<&PluginCompatibility>,
) {
    let Some(compatibility) = compatibility else {
        metadata.remove("plugin_compatibility_host_api");
        metadata.remove("plugin_compatibility_host_version_req");
        return;
    };

    if let Some(host_api) = compatibility.host_api.clone() {
        metadata.insert("plugin_compatibility_host_api".to_owned(), host_api);
    } else {
        metadata.remove("plugin_compatibility_host_api");
    }

    if let Some(host_version_req) = compatibility.host_version_req.clone() {
        metadata.insert(
            "plugin_compatibility_host_version_req".to_owned(),
            host_version_req,
        );
    } else {
        metadata.remove("plugin_compatibility_host_version_req");
    }
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    activation_runtime_contract_checksum_hex(bytes)
}

pub fn hex_lower(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

fn default_integration_catalog() -> IntegrationCatalog {
    let mut catalog = IntegrationCatalog::new();
    for (provider_id, connector, version, class) in [
        ("openai", "openai", "1.0.0", "llm"),
        ("anthropic", "anthropic", "1.0.0", "llm"),
        ("github", "github", "1.0.0", "devops"),
        ("slack", "slack", "1.0.0", "messaging"),
        ("notion", "notion", "1.0.0", "workspace"),
    ] {
        catalog.register_template(kernel::ProviderTemplate {
            provider_id: provider_id.to_owned(),
            default_connector_name: connector.to_owned(),
            default_version: version.to_owned(),
            metadata: BTreeMap::from([("class".to_owned(), class.to_owned())]),
        });
    }
    catalog
}

fn register_dynamic_catalog_connectors(
    kernel: &mut LoongClawKernel<StaticPolicyEngine>,
    catalog: Arc<Mutex<IntegrationCatalog>>,
    bridge_runtime_policy: BridgeRuntimePolicy,
) {
    let snapshot = {
        let guard = match catalog.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.providers()
    };

    for provider in snapshot {
        let connector = DynamicCatalogConnector::new(
            provider.connector_name,
            provider.provider_id,
            catalog.clone(),
            bridge_runtime_policy.clone(),
        );

        kernel.register_core_connector_adapter(connector);
    }
}

fn snapshot_runtime_integration_catalog(
    catalog: &Arc<Mutex<IntegrationCatalog>>,
) -> Result<IntegrationCatalog, String> {
    let guard = catalog
        .lock()
        .map_err(|_err| "integration catalog mutex poisoned".to_owned())?;
    let snapshot = guard.clone();

    Ok(snapshot)
}

fn operation_connector_name(operation: &OperationSpec) -> Option<String> {
    #[allow(clippy::wildcard_enum_match_arm)]
    match operation {
        OperationSpec::ConnectorLegacy { connector_name, .. }
        | OperationSpec::ConnectorCore { connector_name, .. }
        | OperationSpec::ConnectorExtension { connector_name, .. } => Some(connector_name.clone()),
        OperationSpec::ProgrammaticToolCall { steps, .. } => {
            steps.iter().find_map(|step| match step {
                ProgrammaticStep::ConnectorCall { connector_name, .. } => {
                    Some(connector_name.clone())
                }
                ProgrammaticStep::ConnectorBatch { calls, .. } => {
                    calls.first().map(|call| call.connector_name.clone())
                }
                ProgrammaticStep::SetLiteral { .. }
                | ProgrammaticStep::JsonPointer { .. }
                | ProgrammaticStep::Conditional { .. } => None,
            })
        }
        _ => None,
    }
}

async fn execute_spec_operation(
    kernel: &LoongClawKernel<StaticPolicyEngine>,
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
            let dispatch = kernel
                .execute_task(
                    pack_id,
                    token,
                    TaskIntent {
                        task_id: task_id.clone(),
                        objective: objective.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .map_err(|error| format!("task execution from spec failed: {error}"))?;
            Ok((
                "task",
                json!({
                    "route": dispatch.adapter_route,
                    "outcome": dispatch.outcome,
                }),
            ))
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

fn apply_default_selection(
    kernel: &mut LoongClawKernel<StaticPolicyEngine>,
    defaults: Option<&DefaultCoreSelection>,
) -> CliResult<()> {
    if let Some(defaults) = defaults {
        if let Some(connector) = defaults.connector.as_deref() {
            kernel
                .set_default_core_connector_adapter(connector)
                .map_err(|error| {
                    format!("invalid default connector core adapter ({connector}): {error}")
                })?;
        }
        if let Some(runtime) = defaults.runtime.as_deref() {
            kernel
                .set_default_core_runtime_adapter(runtime)
                .map_err(|error| {
                    format!("invalid default runtime core adapter ({runtime}): {error}")
                })?;
        }
        if let Some(tool) = defaults.tool.as_deref() {
            kernel
                .set_default_core_tool_adapter(tool)
                .map_err(|error| format!("invalid default tool core adapter ({tool}): {error}"))?;
        }
        if let Some(memory) = defaults.memory.as_deref() {
            kernel
                .set_default_core_memory_adapter(memory)
                .map_err(|error| {
                    format!("invalid default memory core adapter ({memory}): {error}")
                })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod bootstrap_policy_tests {
    use super::*;
    use crate::spec_runtime::BootstrapSpec;

    #[test]
    fn bootstrap_policy_maps_distinct_acp_bridge_and_runtime_flags() {
        let mut spec = RunnerSpec::template();
        spec.bootstrap = Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: None,
            allow_process_stdio_auto_apply: None,
            allow_native_ffi_auto_apply: None,
            allow_wasm_component_auto_apply: None,
            allow_mcp_server_auto_apply: None,
            allow_acp_bridge_auto_apply: Some(true),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: Some(true),
            enforce_ready_execution: None,
            max_tasks: None,
        });

        let policy = bootstrap_policy(&spec).expect("bootstrap policy should resolve");
        assert!(policy.allow_acp_bridge_auto_apply);
        assert!(!policy.allow_acp_runtime_auto_apply);
        assert!(policy.block_unverified_high_risk_auto_apply);
    }
}

#[cfg(test)]
mod setup_readiness_context_tests {
    use super::*;
    use crate::PluginSetupReadinessSpec;

    #[test]
    fn collect_verified_env_var_names_ignores_blank_names_and_values() {
        let env_vars = vec![
            (OsString::from("TAVILY_API_KEY"), OsString::from("secret")),
            (OsString::from("EMPTY_VALUE"), OsString::from("   ")),
            (OsString::from("   "), OsString::from("ignored")),
        ];

        let verified_env_vars = collect_verified_env_var_names(env_vars);

        assert_eq!(
            verified_env_vars,
            BTreeSet::from(["TAVILY_API_KEY".to_owned()])
        );
    }

    #[test]
    fn collect_verified_env_var_names_preserves_non_blank_name_spelling() {
        let env_vars = vec![
            (OsString::from(" TAVILY_API_KEY"), OsString::from("secret")),
            (OsString::from("TAVILY_API_KEY "), OsString::from("secret")),
        ];

        let verified_env_vars = collect_verified_env_var_names(env_vars);

        assert_eq!(
            verified_env_vars,
            BTreeSet::from([" TAVILY_API_KEY".to_owned(), "TAVILY_API_KEY ".to_owned(),])
        );
    }

    #[test]
    fn resolve_plugin_setup_readiness_context_falls_back_to_process_env_when_unspecified() {
        let env_vars = vec![
            (OsString::from("TAVILY_API_KEY"), OsString::from("secret")),
            (OsString::from("EMPTY_VALUE"), OsString::from("   ")),
        ];

        let context = resolve_plugin_setup_readiness_context(None, env_vars);

        assert_eq!(
            context.verified_env_vars,
            BTreeSet::from(["TAVILY_API_KEY".to_owned()])
        );
        assert!(context.verified_config_keys.is_empty());
    }

    #[test]
    fn resolve_plugin_setup_readiness_context_uses_explicit_values_without_env_inheritance() {
        let readiness_spec = PluginSetupReadinessSpec {
            inherit_process_env: false,
            verified_env_vars: vec![" TAVILY_API_KEY ".to_owned(), "".to_owned()],
            verified_config_keys: vec![
                " tools.web_search.default_provider ".to_owned(),
                "   ".to_owned(),
            ],
        };
        let env_vars = vec![(OsString::from("SHOULD_NOT_BE_USED"), OsString::from("set"))];

        let context = resolve_plugin_setup_readiness_context(Some(&readiness_spec), env_vars);

        assert_eq!(
            context.verified_env_vars,
            BTreeSet::from(["TAVILY_API_KEY".to_owned()])
        );
        assert_eq!(
            context.verified_config_keys,
            BTreeSet::from(["tools.web_search.default_provider".to_owned()])
        );
    }

    #[test]
    fn resolve_plugin_setup_readiness_context_merges_process_env_when_requested() {
        let readiness_spec = PluginSetupReadinessSpec {
            inherit_process_env: true,
            verified_env_vars: vec!["TAVILY_API_KEY".to_owned()],
            verified_config_keys: vec!["tools.web_search.default_provider".to_owned()],
        };
        let env_vars = vec![(OsString::from("OPENAI_API_KEY"), OsString::from("set"))];

        let context = resolve_plugin_setup_readiness_context(Some(&readiness_spec), env_vars);

        assert_eq!(
            context.verified_env_vars,
            BTreeSet::from(["OPENAI_API_KEY".to_owned(), "TAVILY_API_KEY".to_owned(),])
        );
        assert_eq!(
            context.verified_config_keys,
            BTreeSet::from(["tools.web_search.default_provider".to_owned()])
        );
    }
}

#[cfg(test)]
mod bridge_runtime_policy_tests {
    use super::*;

    #[test]
    fn bridge_runtime_policy_honors_raw_circuit_breaker_override_when_scan_is_disabled() {
        let mut spec = RunnerSpec::template();
        let (mut bridge, _source) =
            load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
                .expect("bundled bridge support should resolve");
        let mut security_scan = SecurityScanSpec {
            enabled: false,
            ..SecurityScanSpec::default()
        };
        security_scan.runtime.bridge_circuit_breaker.enabled = false;
        bridge.security_scan = Some(security_scan);
        spec.bridge_support = Some(bridge);

        let policy =
            bridge_runtime_policy(&spec, None).expect("bridge runtime policy should build");

        assert!(!policy.bridge_circuit_breaker.enabled);
    }

    #[test]
    fn bridge_runtime_policy_rejects_invalid_circuit_breaker_override() {
        let mut spec = RunnerSpec::template();
        let (mut bridge, _source) =
            load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
                .expect("bundled bridge support should resolve");
        let mut security_scan = SecurityScanSpec {
            enabled: false,
            ..SecurityScanSpec::default()
        };
        security_scan
            .runtime
            .bridge_circuit_breaker
            .failure_threshold = 0;
        bridge.security_scan = Some(security_scan);
        spec.bridge_support = Some(bridge);

        let error = bridge_runtime_policy(&spec, None)
            .expect_err("invalid bridge circuit breaker policy should fail");

        assert!(error.contains("failure_threshold"));
    }

    #[test]
    fn bridge_runtime_policy_normalizes_guest_readable_config_keys() {
        let mut spec = RunnerSpec::template();
        let (mut bridge, _source) =
            load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
                .expect("bundled bridge support should resolve");
        let mut security_scan = SecurityScanSpec {
            enabled: true,
            ..SecurityScanSpec::default()
        };
        security_scan.runtime.guest_readable_config_keys = vec![
            " provider.region ".to_owned(),
            "channel.mode".to_owned(),
            "provider.region".to_owned(),
        ];
        bridge.security_scan = Some(security_scan);
        spec.bridge_support = Some(bridge);
        let security_scan = spec
            .bridge_support
            .as_ref()
            .and_then(|bridge| bridge.security_scan.as_ref());

        let policy = bridge_runtime_policy(&spec, security_scan)
            .expect("bridge runtime policy should build");

        assert_eq!(
            policy.wasm_guest_readable_config_keys,
            BTreeSet::from(["channel.mode".to_owned(), "provider.region".to_owned(),])
        );
    }

    #[test]
    fn bridge_runtime_policy_rejects_invalid_guest_readable_config_key_namespace() {
        let mut spec = RunnerSpec::template();
        let (mut bridge, _source) =
            load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
                .expect("bundled bridge support should resolve");
        let mut security_scan = SecurityScanSpec {
            enabled: true,
            ..SecurityScanSpec::default()
        };
        security_scan.runtime.guest_readable_config_keys = vec!["region".to_owned()];
        bridge.security_scan = Some(security_scan);
        spec.bridge_support = Some(bridge);
        let security_scan = spec
            .bridge_support
            .as_ref()
            .and_then(|bridge| bridge.security_scan.as_ref());

        let error = bridge_runtime_policy(&spec, security_scan)
            .expect_err("invalid guest-readable config key should fail");

        assert!(error.contains("guest_readable_config_keys"));
        assert!(error.contains("region"));
        assert!(error.contains(WASM_GUEST_CONFIG_PROVIDER_PREFIX));
        assert!(error.contains(WASM_GUEST_CONFIG_CHANNEL_PREFIX));
    }

    #[test]
    fn bridge_runtime_policy_rejects_guest_readable_config_key_with_inner_whitespace() {
        let mut spec = RunnerSpec::template();
        let (mut bridge, _source) =
            load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
                .expect("bundled bridge support should resolve");
        let mut security_scan = SecurityScanSpec {
            enabled: true,
            ..SecurityScanSpec::default()
        };
        security_scan.runtime.guest_readable_config_keys = vec!["provider.region name".to_owned()];
        bridge.security_scan = Some(security_scan);
        spec.bridge_support = Some(bridge);
        let security_scan = spec
            .bridge_support
            .as_ref()
            .and_then(|bridge| bridge.security_scan.as_ref());

        let error = bridge_runtime_policy(&spec, security_scan)
            .expect_err("guest-readable config key with inner whitespace should fail");

        assert!(error.contains("provider.region name"));
        assert!(error.contains("guest_readable_config_keys"));
    }

    #[tokio::test]
    async fn execute_spec_rejects_invalid_guest_readable_config_keys_before_plugin_scan() {
        let mut spec = RunnerSpec::template();
        let (mut bridge, _source) =
            load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
                .expect("bundled bridge support should resolve");
        let mut security_scan = SecurityScanSpec {
            enabled: true,
            ..SecurityScanSpec::default()
        };
        security_scan.runtime.guest_readable_config_keys = vec!["region".to_owned()];
        bridge.security_scan = Some(security_scan);
        spec.bridge_support = Some(bridge);
        spec.plugin_scan = Some(PluginScanSpec {
            enabled: true,
            roots: vec!["/definitely/missing/loongclaw-plugin-root".to_owned()],
        });

        let report = execute_spec(&spec, false).await;
        let blocked_reason = report
            .blocked_reason
            .expect("invalid bridge runtime policy should block the spec");

        assert_eq!(report.operation_kind, "blocked");
        assert!(blocked_reason.contains("bridge runtime policy is invalid"));
        assert!(blocked_reason.contains("guest_readable_config_keys"));
        assert!(!blocked_reason.contains("plugin scan failed"));
    }
}

#[cfg(test)]
mod plugin_metadata_tests {
    use super::*;
    use kernel::{
        Capability, PluginActivationCandidate, PluginActivationPlan, PluginActivationStatus,
        PluginBridgeKind, PluginCompatibility, PluginCompatibilityMode, PluginContractDialect,
        PluginDescriptor, PluginIR, PluginManifest, PluginRuntimeProfile, PluginScanReport,
        PluginSetup, PluginSetupMode, PluginSlotClaim, PluginSlotMode, PluginSourceKind,
        PluginTranslationReport, PluginTrustTier,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn test_descriptor(source_kind: PluginSourceKind) -> PluginDescriptor {
        let path = match source_kind {
            PluginSourceKind::PackageManifest => "/tmp/pkg/loongclaw.plugin.json".to_owned(),
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
                PluginSourceKind::PackageManifest => {
                    PluginContractDialect::LoongClawPackageManifest
                }
                PluginSourceKind::EmbeddedSource => PluginContractDialect::LoongClawEmbeddedSource,
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
                    host_api: Some("loongclaw-plugin/v1".to_owned()),
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

    fn ready_activation_plan(descriptor: &PluginDescriptor) -> PluginActivationPlan {
        PluginActivationPlan {
            total_plugins: 1,
            ready_plugins: 1,
            setup_incomplete_plugins: 0,
            blocked_plugins: 0,
            candidates: vec![PluginActivationCandidate {
                plugin_id: descriptor.manifest.plugin_id.clone(),
                source_path: descriptor.path.clone(),
                source_kind: descriptor.source_kind,
                package_root: descriptor.package_root.clone(),
                package_manifest_path: descriptor.package_manifest_path.clone(),
                trust_tier: descriptor.manifest.trust_tier,
                compatibility_mode: descriptor.compatibility_mode,
                compatibility_shim: None,
                compatibility_shim_support: None,
                compatibility_shim_support_mismatch_reasons: Vec::new(),
                bridge_kind: PluginBridgeKind::HttpJson,
                adapter_family: "http-adapter".to_owned(),
                slot_claims: descriptor.manifest.slot_claims.clone(),
                diagnostic_findings: Vec::new(),
                status: PluginActivationStatus::Ready,
                reason: "plugin runtime profile is supported by current runtime matrix".to_owned(),
                missing_required_env_vars: Vec::new(),
                missing_required_config_keys: Vec::new(),
                bootstrap_hint: "spawn python worker and then wire http adapter".to_owned(),
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
            Some("package_manifest:/tmp/pkg/loongclaw.plugin.json")
        );
        assert_eq!(
            metadata
                .get("plugin_package_manifest_path")
                .map(String::as_str),
            Some("/tmp/pkg/loongclaw.plugin.json")
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
            Some("loongclaw-plugin/v1")
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
            Some("/tmp/pkg/loongclaw.plugin.json")
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
            Some("package_manifest:/tmp/pkg/loongclaw.plugin.json")
        );
        assert_eq!(
            metadata.get("plugin_trust_tier").map(String::as_str),
            Some("verified-community")
        );
        assert_eq!(
            metadata
                .get("plugin_package_manifest_path")
                .map(String::as_str),
            Some("/tmp/pkg/loongclaw.plugin.json")
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
    fn enrich_scan_report_removes_stale_host_projected_metadata_when_authoritative_values_absent() {
        let mut descriptor = test_descriptor(PluginSourceKind::PackageManifest);
        descriptor.manifest.setup = None;
        descriptor.manifest.defer_loading = true;
        descriptor
            .manifest
            .metadata
            .insert("plugin_id".to_owned(), "forged-plugin-id".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("defer_loading".to_owned(), "false".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("plugin_setup_mode".to_owned(), "legacy".to_owned());
        descriptor.manifest.metadata.insert(
            "plugin_setup_required_env_vars_json".to_owned(),
            "[\"LEGACY_KEY\"]".to_owned(),
        );
        descriptor.manifest.metadata.insert(
            "component_resolved_path".to_owned(),
            "/tmp/forged-component".to_owned(),
        );
        descriptor
            .manifest
            .metadata
            .insert("bridge_kind".to_owned(), "process_stdio".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("adapter_family".to_owned(), "legacy-adapter".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("entrypoint_hint".to_owned(), "stdio://legacy".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("source_language".to_owned(), "legacy".to_owned());

        let report = PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        };

        let enriched =
            enrich_scan_report_with_translation(&report, &PluginTranslationReport::default(), None);
        let metadata = &enriched.descriptors[0].manifest.metadata;

        assert_eq!(
            metadata.get("plugin_id").map(String::as_str),
            Some("search-plugin")
        );
        assert_eq!(
            metadata.get("defer_loading").map(String::as_str),
            Some("true")
        );
        assert!(!metadata.contains_key("plugin_setup_mode"));
        assert!(!metadata.contains_key("plugin_setup_required_env_vars_json"));
        assert!(!metadata.contains_key("component_resolved_path"));
        assert!(!metadata.contains_key("bridge_kind"));
        assert!(!metadata.contains_key("adapter_family"));
        assert!(!metadata.contains_key("entrypoint_hint"));
        assert!(!metadata.contains_key("source_language"));
    }

    #[test]
    fn enrich_scan_report_overwrites_forged_runtime_projection_metadata() {
        let mut descriptor = test_descriptor(PluginSourceKind::PackageManifest);
        descriptor
            .manifest
            .metadata
            .insert("bridge_kind".to_owned(), "process_stdio".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("adapter_family".to_owned(), "legacy-adapter".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("entrypoint_hint".to_owned(), "stdio://legacy".to_owned());
        descriptor
            .manifest
            .metadata
            .insert("source_language".to_owned(), "legacy".to_owned());
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
            metadata.get("bridge_kind").map(String::as_str),
            Some("http_json")
        );
        assert_eq!(
            metadata.get("adapter_family").map(String::as_str),
            Some("http-adapter")
        );
        assert_eq!(
            metadata.get("entrypoint_hint").map(String::as_str),
            Some("https://example.com/search")
        );
        assert_eq!(
            metadata.get("source_language").map(String::as_str),
            Some("manifest")
        );
    }

    #[test]
    fn enrich_scan_report_attaches_activation_runtime_contract_metadata() {
        let descriptor = test_descriptor(PluginSourceKind::EmbeddedSource);
        let report = PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor.clone()],
        };
        let translation = test_translation(&descriptor);
        let activation = ready_activation_plan(&descriptor);

        let enriched =
            enrich_scan_report_with_translation(&report, &translation, Some(&activation));
        let metadata = &enriched.descriptors[0].manifest.metadata;
        let raw_contract = metadata
            .get(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY)
            .expect("activation contract metadata should be stamped");
        let contract = parse_plugin_activation_runtime_contract(raw_contract)
            .expect("activation contract should decode");
        let checksum = activation_runtime_contract_checksum_hex(raw_contract.as_bytes());

        assert_eq!(contract.plugin_id, "search-plugin");
        assert_eq!(contract.source_path, "/tmp/pkg/plugin.py");
        assert_eq!(contract.source_kind, PluginSourceKind::EmbeddedSource);
        assert_eq!(
            contract.dialect,
            PluginContractDialect::LoongClawEmbeddedSource
        );
        assert_eq!(contract.bridge_kind, PluginBridgeKind::HttpJson);
        assert_eq!(contract.adapter_family, "http-adapter");
        assert_eq!(contract.entrypoint_hint, "https://example.com/search");
        assert_eq!(contract.source_language, "py");
        assert_eq!(
            metadata
                .get(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY)
                .map(String::as_str),
            Some(checksum.as_str())
        );
    }
}

#[cfg(test)]
mod plugin_trust_summary_tests {
    use super::build_plugin_trust_summary;
    use kernel::{
        BootstrapReport, BootstrapTask, BootstrapTaskStatus, Capability, PluginActivationCandidate,
        PluginActivationPlan, PluginActivationStatus, PluginBridgeKind, PluginDescriptor,
        PluginManifest, PluginScanReport, PluginSourceKind, PluginTrustTier,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn descriptor(
        plugin_id: &str,
        source_path: &str,
        trust_tier: PluginTrustTier,
    ) -> PluginDescriptor {
        PluginDescriptor {
            path: source_path.to_owned(),
            source_kind: PluginSourceKind::EmbeddedSource,
            dialect: kernel::PluginContractDialect::LoongClawEmbeddedSource,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: kernel::PluginCompatibilityMode::Native,
            package_root: "/tmp/plugins".to_owned(),
            package_manifest_path: None,
            language: "python".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("0.1.0".to_owned()),
                plugin_id: plugin_id.to_owned(),
                provider_id: plugin_id.to_owned(),
                connector_name: plugin_id.to_owned(),
                channel_id: Some("primary".to_owned()),
                endpoint: Some(format!("local://{plugin_id}")),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier,
                metadata: BTreeMap::new(),
                summary: None,
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        }
    }

    #[test]
    fn build_plugin_trust_summary_counts_tiers_and_review_required_plugins() {
        let official = descriptor(
            "official-http",
            "/tmp/plugins/official.rs",
            PluginTrustTier::Official,
        );
        let unverified = descriptor(
            "stdio-review",
            "/tmp/plugins/stdio.py",
            PluginTrustTier::Unverified,
        );
        let scan_report = PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![official, unverified],
        };
        let activation_plan = PluginActivationPlan {
            total_plugins: 2,
            ready_plugins: 2,
            setup_incomplete_plugins: 0,
            blocked_plugins: 0,
            candidates: vec![
                PluginActivationCandidate {
                    plugin_id: "official-http".to_owned(),
                    source_path: "/tmp/plugins/official.rs".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    package_root: "/tmp/plugins".to_owned(),
                    package_manifest_path: None,
                    trust_tier: PluginTrustTier::Official,
                    compatibility_mode: kernel::PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::Ready,
                    reason: "ready".to_owned(),
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "register provider".to_owned(),
                },
                PluginActivationCandidate {
                    plugin_id: "stdio-review".to_owned(),
                    source_path: "/tmp/plugins/stdio.py".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    package_root: "/tmp/plugins".to_owned(),
                    package_manifest_path: None,
                    trust_tier: PluginTrustTier::Unverified,
                    compatibility_mode: kernel::PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::ProcessStdio,
                    adapter_family: "python-stdio-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::Ready,
                    reason: "ready".to_owned(),
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "spawn stdio worker".to_owned(),
                },
            ],
        };
        let bootstrap_report = BootstrapReport {
            total_tasks: 2,
            applied_tasks: 1,
            deferred_tasks: 1,
            skipped_tasks: 0,
            blocked: true,
            block_reason: Some(
                "bootstrap policy blocked 1 ready plugin(s) that were not auto-applied"
                    .to_owned(),
            ),
            applied_plugin_keys: BTreeSet::from([(
                "/tmp/plugins/official.rs".to_owned(),
                "official-http".to_owned(),
            )]),
            tasks: vec![
                BootstrapTask {
                    plugin_id: "official-http".to_owned(),
                    source_path: "/tmp/plugins/official.rs".to_owned(),
                    trust_tier: PluginTrustTier::Official,
                    compatibility_mode: kernel::PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    bootstrap_hint: "register provider".to_owned(),
                    status: BootstrapTaskStatus::Applied,
                    reason: "bridge is allowed for automatic bootstrap apply".to_owned(),
                },
                BootstrapTask {
                    plugin_id: "stdio-review".to_owned(),
                    source_path: "/tmp/plugins/stdio.py".to_owned(),
                    trust_tier: PluginTrustTier::Unverified,
                    compatibility_mode: kernel::PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    bridge_kind: PluginBridgeKind::ProcessStdio,
                    adapter_family: "python-stdio-adapter".to_owned(),
                    bootstrap_hint: "spawn stdio worker".to_owned(),
                    status: BootstrapTaskStatus::DeferredUnsupportedAutoApply,
                    reason:
                        "bridge is ready but auto-apply is blocked by bootstrap trust policy for unverified high-risk plugins"
                            .to_owned(),
                },
            ],
        };

        let summary =
            build_plugin_trust_summary(&[scan_report], &[activation_plan], &[bootstrap_report]);

        assert_eq!(summary.scanned_plugins, 2);
        assert_eq!(summary.official_plugins, 1);
        assert_eq!(summary.unverified_plugins, 1);
        assert_eq!(summary.high_risk_plugins, 1);
        assert_eq!(summary.high_risk_unverified_plugins, 1);
        assert_eq!(summary.blocked_auto_apply_plugins, 1);
        assert_eq!(summary.review_required_plugins.len(), 1);
        assert_eq!(summary.review_required_plugins[0].plugin_id, "stdio-review");
        assert_eq!(
            summary.review_required_plugins[0].provenance_summary,
            "embedded_source:/tmp/plugins/stdio.py"
        );
        assert_eq!(
            summary.review_required_plugins[0].bootstrap_status,
            Some(BootstrapTaskStatus::DeferredUnsupportedAutoApply)
        );
    }
}
