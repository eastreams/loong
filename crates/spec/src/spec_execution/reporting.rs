use std::collections::BTreeSet;

use super::*;

fn blocked_outcome(reason: &str) -> Value {
    json!({
        "status": "blocked",
        "reason": reason,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_blocked_spec_run_report(
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

pub(super) fn build_plugin_trust_summary(
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

pub(super) fn emit_security_scan_audit_event(
    kernel: &LoongKernel<StaticPolicyEngine>,
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

pub(super) fn emit_plugin_trust_audit_event(
    kernel: &LoongKernel<StaticPolicyEngine>,
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
