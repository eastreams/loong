use std::{
    collections::{BTreeMap, BTreeSet},
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
    PluginDescriptor, PluginScanReport, PluginScanner, PluginTranslationReport, PluginTranslator,
    ProvisionPlan, RuntimeCoreRequest, RuntimeExtensionRequest, StaticPolicyEngine, SystemClock,
    TaskIntent, ToolCoreRequest, ToolExtensionRequest,
};
use serde_json::{Value, json};

use crate::CliResult;
use crate::programmatic::execute_programmatic_tool_call;
use crate::spec_runtime::*;

mod approval_policy;
mod bridge_support_policy;
mod security_scan_eval;
mod security_scan_policy;
mod tool_search;
use approval_policy::evaluate_approval_guard;
use bridge_support_policy::bridge_support_policy_checksum;
use security_scan_eval::evaluate_plugin_security_scan;
use security_scan_policy::{
    apply_security_scan_delta, emit_security_scan_siem_record, security_scan_process_allowlist,
};
use tool_search::execute_tool_search;

pub use approval_policy::operation_risk_profile;
pub use bridge_support_policy::{
    bridge_support_policy_sha256, security_scan_profile_message, security_scan_profile_sha256,
};
pub use security_scan_policy::{load_security_scan_profile_from_path, security_scan_policy};

pub async fn execute_spec(spec: RunnerSpec, include_audit: bool) -> SpecRunReport {
    let mut spec = spec;
    let audit_sink = Arc::new(InMemoryAuditSink::default());
    let mut kernel = crate::kernel_bootstrap::KernelBuilder::default()
        .clock(Arc::new(SystemClock) as Arc<dyn Clock>)
        .audit(audit_sink.clone())
        .build();

    let mut integration_catalog = default_integration_catalog();
    let mut blocked_reason = None;
    let mut bridge_support_checksum = None;
    let mut bridge_support_sha256 = None;
    let approval_guard = evaluate_approval_guard(&spec);
    let mut self_awareness = None;
    let mut architecture_guard = None;
    let mut plugin_scan_reports = Vec::new();
    let mut plugin_translation_reports = Vec::new();
    let mut plugin_activation_plans = Vec::new();
    let mut plugin_bootstrap_reports = Vec::new();
    let mut plugin_bootstrap_queue = Vec::new();
    let mut plugin_absorb_reports = Vec::new();
    let security_scan_policy = match security_scan_policy(&spec) {
        Ok(policy) => policy,
        Err(error) => {
            blocked_reason = Some(match blocked_reason {
                Some(existing) => format!("{existing}; {error}"),
                None => error,
            });
            None
        }
    };
    let security_process_allowlist = security_scan_process_allowlist(&spec);
    let mut security_scan_report = security_scan_policy
        .as_ref()
        .map(|_| SecurityScanReport::default());
    let mut auto_provision_plan = None;

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
            spec.pack.pack_id,
            spec.agent_id,
            reason,
            approval_guard,
            bridge_support_checksum,
            bridge_support_sha256,
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

    if let Some(plugin_scan) = &spec.plugin_scan
        && plugin_scan.enabled
    {
        let scanner = PluginScanner::new();
        let translator = PluginTranslator::new();
        let bootstrap_executor = PluginBootstrapExecutor::new();
        let bootstrap_policy = bootstrap_policy(&spec);
        let (bridge_matrix, enforce_bridge_support) = bridge_support_matrix(&spec);
        let mut pending_absorb_inputs = Vec::new();
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
            let activation = translator.plan_activation(&translation, &bridge_matrix);

            if enforce_bridge_support && activation.has_blockers() {
                blocked_reason = Some(format!(
                    "bridge support enforcement blocked {} plugin(s)",
                    activation.blocked_plugins
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
                enrich_scan_report_with_translation(&ready_report, &translation);
            let enriched_filtered_report =
                enrich_scan_report_with_translation(&filtered_report, &translation);

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
            pending_absorb_inputs.push(enriched_filtered_report);

            if blocked_reason.is_some() {
                break;
            }
        }

        if blocked_reason.is_none() {
            for pending in pending_absorb_inputs {
                let absorb = scanner.absorb(&mut integration_catalog, &mut spec.pack, &pending);
                plugin_absorb_reports.push(absorb);
            }
        }
    }

    if let (Some(policy), Some(report)) =
        (security_scan_policy.as_ref(), security_scan_report.as_mut())
        && let Some(export_spec) = policy.siem_export.as_ref().filter(|value| value.enabled)
    {
        match emit_security_scan_siem_record(
            &spec.pack.pack_id,
            &spec.agent_id,
            report,
            export_spec,
        ) {
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
            emit_security_scan_audit_event(&kernel, &spec.pack.pack_id, &spec.agent_id, report)
        && blocked_reason.is_none()
    {
        blocked_reason = Some(error);
    }

    if let Some(reason) = blocked_reason.clone() {
        return build_blocked_spec_run_report(
            spec.pack.pack_id,
            spec.agent_id,
            reason,
            approval_guard,
            bridge_support_checksum,
            bridge_support_sha256,
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

        match agent.plan(&integration_catalog, &spec.pack, &request) {
            Ok(plan) => {
                if !plan.is_noop() {
                    if let Err(error) = integration_catalog.apply_plan(&mut spec.pack, &plan) {
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
            spec.pack.pack_id,
            spec.agent_id,
            reason,
            approval_guard,
            bridge_support_checksum,
            bridge_support_sha256,
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
    let bridge_runtime_policy = bridge_runtime_policy(&spec, security_scan_policy.as_ref());
    register_dynamic_catalog_connectors(&mut kernel, shared_catalog, bridge_runtime_policy);

    if let Err(error) = kernel.register_pack(spec.pack.clone()) {
        let reason = format!("spec pack registration failed: {error}");
        return build_blocked_spec_run_report(
            spec.pack.pack_id,
            spec.agent_id,
            reason,
            approval_guard,
            bridge_support_checksum,
            bridge_support_sha256,
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
        return build_blocked_spec_run_report(
            spec.pack.pack_id,
            spec.agent_id,
            error,
            approval_guard,
            bridge_support_checksum,
            bridge_support_sha256,
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

    let token = match kernel.issue_token(&spec.pack.pack_id, &spec.agent_id, spec.ttl_s) {
        Ok(token) => token,
        Err(error) => {
            let reason = format!("token issue for spec failed: {error}");
            return build_blocked_spec_run_report(
                spec.pack.pack_id,
                spec.agent_id,
                reason,
                approval_guard,
                bridge_support_checksum,
                bridge_support_sha256,
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
        &spec.pack.pack_id,
        &token,
        &integration_catalog,
        &plugin_scan_reports,
        &plugin_translation_reports,
        &spec.operation,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return build_blocked_spec_run_report(
                spec.pack.pack_id,
                spec.agent_id,
                error,
                approval_guard,
                bridge_support_checksum,
                bridge_support_sha256,
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

    SpecRunReport {
        pack_id: spec.pack.pack_id,
        agent_id: spec.agent_id,
        operation_kind,
        blocked_reason,
        approval_guard,
        bridge_support_checksum,
        bridge_support_sha256,
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
    bridge_support_checksum: Option<String>,
    bridge_support_sha256: Option<String>,
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
    SpecRunReport {
        pack_id,
        agent_id,
        operation_kind: "blocked",
        blocked_reason: Some(reason.clone()),
        approval_guard,
        bridge_support_checksum,
        bridge_support_sha256,
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
        outcome: blocked_outcome(&reason),
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
            let mut matrix = BridgeSupportMatrix::default();
            if !bridge.supported_bridges.is_empty() {
                matrix.supported_bridges = bridge.supported_bridges.iter().copied().collect();
            }
            if !bridge.supported_adapter_families.is_empty() {
                matrix.supported_adapter_families =
                    bridge.supported_adapter_families.iter().cloned().collect();
            }
            (matrix, bridge.enforce_supported)
        }
        _ => (BridgeSupportMatrix::default(), false),
    }
}

fn bridge_runtime_policy(
    spec: &RunnerSpec,
    security_scan: Option<&SecurityScanSpec>,
) -> BridgeRuntimePolicy {
    let Some(bridge) = &spec.bridge_support else {
        return BridgeRuntimePolicy::default();
    };
    if !bridge.enabled {
        return BridgeRuntimePolicy::default();
    }

    let runtime = security_scan
        .map(|scan| scan.runtime.clone())
        .unwrap_or_default();
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

    BridgeRuntimePolicy {
        execute_process_stdio: bridge.execute_process_stdio,
        execute_http_json: bridge.execute_http_json,
        execute_wasm_component: runtime.execute_wasm_component,
        allowed_process_commands: bridge
            .allowed_process_commands
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect(),
        wasm_allowed_path_prefixes,
        wasm_max_component_bytes: runtime.max_component_bytes,
        wasm_fuel_limit: runtime.fuel_limit,
        wasm_require_hash_pin,
        wasm_required_sha256_by_plugin,
        enforce_execution_success: bridge.enforce_execution_success,
    }
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

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        descriptors,
    }
}

fn enrich_scan_report_with_translation(
    report: &PluginScanReport,
    translation: &PluginTranslationReport,
) -> PluginScanReport {
    let mut runtime_by_key: BTreeMap<(String, String), (String, String, String, String)> =
        BTreeMap::new();

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

    let descriptors: Vec<PluginDescriptor> = report
        .descriptors
        .iter()
        .cloned()
        .map(|mut descriptor| {
            descriptor
                .manifest
                .metadata
                .entry("plugin_source_path".to_owned())
                .or_insert_with(|| descriptor.path.clone());
            descriptor
                .manifest
                .metadata
                .entry("plugin_id".to_owned())
                .or_insert_with(|| descriptor.manifest.plugin_id.clone());
            descriptor
                .manifest
                .metadata
                .entry("defer_loading".to_owned())
                .or_insert_with(|| descriptor.manifest.defer_loading.to_string());
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
                descriptor
                    .manifest
                    .metadata
                    .entry("component_resolved_path".to_owned())
                    .or_insert_with(|| normalized.display().to_string());
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
                    .entry("bridge_kind".to_owned())
                    .or_insert_with(|| bridge_kind.clone());
                descriptor
                    .manifest
                    .metadata
                    .entry("adapter_family".to_owned())
                    .or_insert_with(|| adapter_family.clone());
                descriptor
                    .manifest
                    .metadata
                    .entry("entrypoint_hint".to_owned())
                    .or_insert_with(|| entrypoint_hint.clone());
                descriptor
                    .manifest
                    .metadata
                    .entry("source_language".to_owned())
                    .or_insert_with(|| source_language.clone());
            }
            descriptor
        })
        .collect();

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        descriptors,
    }
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
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
        kernel.register_connector(DynamicCatalogConnector {
            connector_name: provider.connector_name,
            provider_id: provider.provider_id,
            catalog: catalog.clone(),
            bridge_runtime_policy: bridge_runtime_policy.clone(),
        });
    }
}

fn operation_connector_name(operation: &OperationSpec) -> Option<String> {
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
                .invoke_connector(
                    pack_id,
                    token,
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
            include_deferred,
            include_examples,
        } => {
            let matches = execute_tool_search(
                integration_catalog,
                plugin_scan_reports,
                plugin_translation_reports,
                query,
                *limit,
                *include_deferred,
                *include_examples,
            );
            Ok((
                "tool_search",
                json!({
                    "query": query,
                    "limit": limit,
                    "include_deferred": include_deferred,
                    "include_examples": include_examples,
                    "returned": matches.len(),
                    "results": matches,
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
