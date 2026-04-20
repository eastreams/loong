use super::*;

pub fn render_runtime_capability_text(artifact: &RuntimeCapabilityArtifactDocument) -> String {
    let body = [
        format!("candidate_id={}", artifact.candidate_id),
        format!("status={}", render_capability_status(artifact.status)),
        format!("decision={}", render_capability_decision(artifact.decision)),
        format!("target={}", render_target(artifact.proposal.target)),
        format!("target_summary={}", artifact.proposal.summary),
        format!("bounded_scope={}", artifact.proposal.bounded_scope),
        format!(
            "required_capabilities={}",
            render_string_values(&artifact.proposal.required_capabilities)
        ),
        format!("tags={}", render_string_values(&artifact.proposal.tags)),
        format!("source_run_id={}", artifact.source_run.run_id),
        format!("source_experiment_id={}", artifact.source_run.experiment_id),
        format!(
            "source_run_status={}",
            render_experiment_status(artifact.source_run.status)
        ),
        format!(
            "source_run_decision={}",
            render_experiment_decision(artifact.source_run.decision)
        ),
        format!(
            "source_metrics={}",
            render_metrics(&artifact.source_run.metrics)
        ),
        format!(
            "source_warnings={}",
            render_string_values_with_separator(&artifact.source_run.warnings, " | ")
        ),
        format!(
            "source_snapshot_delta_changed_surface_count={}",
            artifact
                .source_run
                .snapshot_delta
                .as_ref()
                .map(|delta| delta.changed_surface_count.to_string())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "source_snapshot_delta_changed_surfaces={}",
            artifact
                .source_run
                .snapshot_delta
                .as_ref()
                .map(|delta| render_string_values(&delta.changed_surfaces()))
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "review_summary={}",
            artifact
                .review
                .as_ref()
                .map(|review| review.summary.as_str())
                .unwrap_or("-")
        ),
        format!(
            "review_warnings={}",
            artifact
                .review
                .as_ref()
                .map(|review| render_string_values_with_separator(&review.warnings, " | "))
                .unwrap_or_else(|| "-".to_owned())
        ),
    ]
    .join("\n");

    wrap_runtime_capability_surface("capability candidate", body)
}

pub fn render_runtime_capability_index_text(report: &RuntimeCapabilityIndexReport) -> String {
    let mut lines = vec![
        format!("root={}", report.root),
        format!("family_count={}", report.family_count),
        format!("total_candidate_count={}", report.total_candidate_count),
    ];

    for family in &report.families {
        lines.push(String::new());
        lines.push(format!("family_id={}", family.family_id));
        lines.push(format!(
            "readiness={}",
            render_family_readiness_status(family.readiness.status)
        ));
        lines.push(format!("target={}", render_target(family.proposal.target)));
        lines.push(format!("target_summary={}", family.proposal.summary));
        lines.push(format!("bounded_scope={}", family.proposal.bounded_scope));
        lines.push(format!(
            "candidate_ids={}",
            render_string_values(&family.candidate_ids)
        ));
        lines.push(format!(
            "evidence_counts=total:{} reviewed:{} accepted:{} rejected:{} undecided:{}",
            family.evidence.total_candidates,
            family.evidence.reviewed_candidates,
            family.evidence.accepted_candidates,
            family.evidence.rejected_candidates,
            family.evidence.undecided_candidates
        ));
        lines.push(format!(
            "distinct_source_runs={}",
            family.evidence.distinct_source_run_count
        ));
        lines.push(format!(
            "distinct_experiments={}",
            family.evidence.distinct_experiment_count
        ));
        lines.push(format!(
            "metric_ranges={}",
            render_metric_ranges(&family.evidence.metric_ranges)
        ));
        lines.push(format!(
            "warnings={}",
            render_string_values_with_separator(&family.evidence.unique_warnings, " | ")
        ));
        lines.push(format!(
            "delta_evidence_candidates={}",
            family.evidence.delta_candidate_count
        ));
        lines.push(format!(
            "delta_changed_surfaces={}",
            render_string_values(&family.evidence.changed_surfaces)
        ));
        lines.push(format!(
            "checks={}",
            family
                .readiness
                .checks
                .iter()
                .map(render_family_readiness_check)
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    let body = lines.join("\n");

    wrap_runtime_capability_surface("capability index", body)
}

pub fn render_runtime_capability_apply_text(report: &RuntimeCapabilityApplyReport) -> String {
    let artifact = &report.applied_artifact;
    let body = [
        format!("family_id={}", report.family_id),
        format!(
            "outcome={}",
            render_runtime_capability_apply_outcome(report.outcome)
        ),
        format!("artifact_kind={}", artifact.artifact_kind),
        format!("artifact_id={}", artifact.artifact_id),
        format!("delivery_surface={}", artifact.delivery_surface),
        format!("output_path={}", report.output_path),
        format!("target={}", render_target(artifact.target)),
        format!("target_summary={}", artifact.summary),
        format!("bounded_scope={}", artifact.bounded_scope),
        format!(
            "required_capabilities={}",
            render_string_values(&artifact.required_capabilities)
        ),
        format!("tags={}", render_string_values(&artifact.tags)),
        format!(
            "approval_checklist={}",
            render_string_values_with_separator(&artifact.approval_checklist, " | ")
        ),
        format!(
            "rollback_hints={}",
            render_string_values_with_separator(&artifact.rollback_hints, " | ")
        ),
        format!("delta_candidate_count={}", artifact.delta_candidate_count),
        format!(
            "changed_surfaces={}",
            render_string_values(&artifact.changed_surfaces)
        ),
        format!(
            "candidate_ids={}",
            render_string_values(&artifact.candidate_ids)
        ),
        format!(
            "source_run_ids={}",
            render_string_values(&artifact.source_run_ids)
        ),
        format!(
            "experiment_ids={}",
            render_string_values(&artifact.experiment_ids)
        ),
        format!(
            "payload={}",
            render_runtime_capability_draft_payload(&artifact.payload)
        ),
    ]
    .join("\n");

    wrap_runtime_capability_surface("capability apply", body)
}

pub fn render_runtime_capability_activate_text(report: &RuntimeCapabilityActivateReport) -> String {
    let body = [
        format!("artifact_path={}", report.artifact_path),
        format!("config_path={}", report.config_path),
        format!("artifact_id={}", report.artifact_id),
        format!("target={}", render_target(report.target)),
        format!("delivery_surface={}", report.delivery_surface),
        format!("activation_surface={}", report.activation_surface),
        format!("target_path={}", report.target_path),
        format!("apply_requested={}", report.apply_requested),
        format!("replace_requested={}", report.replace_requested),
        format!(
            "outcome={}",
            render_runtime_capability_activate_outcome(report.outcome)
        ),
        format!(
            "notes={}",
            render_string_values_with_separator(&report.notes, " | ")
        ),
        format!(
            "verification={}",
            render_string_values_with_separator(&report.verification, " | ")
        ),
        format!(
            "rollback_hints={}",
            render_string_values_with_separator(&report.rollback_hints, " | ")
        ),
        format!(
            "activation_record_path={}",
            report.activation_record_path.as_deref().unwrap_or("-")
        ),
    ]
    .join("\n");

    wrap_runtime_capability_surface("capability activate", body)
}

pub fn render_runtime_capability_rollback_text(report: &RuntimeCapabilityRollbackReport) -> String {
    let body = [
        format!("record_path={}", report.record_path),
        format!("config_path={}", report.config_path),
        format!("artifact_id={}", report.artifact_id),
        format!("target={}", render_target(report.target)),
        format!("activation_surface={}", report.activation_surface),
        format!("target_path={}", report.target_path),
        format!("apply_requested={}", report.apply_requested),
        format!(
            "outcome={}",
            render_runtime_capability_rollback_outcome(report.outcome)
        ),
        format!(
            "notes={}",
            render_string_values_with_separator(&report.notes, " | ")
        ),
        format!(
            "verification={}",
            render_string_values_with_separator(&report.verification, " | ")
        ),
    ]
    .join("\n");

    wrap_runtime_capability_surface("capability rollback", body)
}

fn render_metrics(metrics: &std::collections::BTreeMap<String, f64>) -> String {
    if metrics.is_empty() {
        "-".to_owned()
    } else {
        metrics
            .iter()
            .map(|(key, value)| format!("{key}:{value}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

pub(super) fn render_string_values(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn render_string_values_with_separator(values: &[String], separator: &str) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(separator)
    }
}

pub(super) fn normalized_path_text(value: &str) -> String {
    value.replace('\\', "/")
}

fn render_metric_ranges(ranges: &BTreeMap<String, RuntimeCapabilityMetricRange>) -> String {
    if ranges.is_empty() {
        "-".to_owned()
    } else {
        ranges
            .iter()
            .map(|(key, range)| format!("{key}:{}..{}", range.min, range.max))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn render_family_readiness_check(check: &RuntimeCapabilityFamilyReadinessCheck) -> String {
    format!(
        "{}:{}:{}",
        check.dimension,
        render_family_readiness_check_status(check.status),
        check.summary
    )
}

pub(super) fn render_target(target: RuntimeCapabilityTarget) -> &'static str {
    match target {
        RuntimeCapabilityTarget::ManagedSkill => "managed_skill",
        RuntimeCapabilityTarget::ProgrammaticFlow => "programmatic_flow",
        RuntimeCapabilityTarget::ProfileNoteAddendum => "profile_note_addendum",
    }
}

pub(super) fn render_memory_profile(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

fn render_capability_status(status: RuntimeCapabilityStatus) -> &'static str {
    match status {
        RuntimeCapabilityStatus::Proposed => "proposed",
        RuntimeCapabilityStatus::Reviewed => "reviewed",
    }
}

fn render_capability_decision(decision: RuntimeCapabilityDecision) -> &'static str {
    match decision {
        RuntimeCapabilityDecision::Undecided => "undecided",
        RuntimeCapabilityDecision::Accepted => "accepted",
        RuntimeCapabilityDecision::Rejected => "rejected",
    }
}

fn render_runtime_capability_apply_outcome(outcome: RuntimeCapabilityApplyOutcome) -> &'static str {
    match outcome {
        RuntimeCapabilityApplyOutcome::Applied => "applied",
        RuntimeCapabilityApplyOutcome::AlreadyApplied => "already_applied",
    }
}

fn render_runtime_capability_activate_outcome(
    outcome: RuntimeCapabilityActivateOutcome,
) -> &'static str {
    match outcome {
        RuntimeCapabilityActivateOutcome::DryRun => "dry_run",
        RuntimeCapabilityActivateOutcome::Activated => "activated",
        RuntimeCapabilityActivateOutcome::AlreadyActivated => "already_activated",
    }
}

fn render_runtime_capability_rollback_outcome(
    outcome: RuntimeCapabilityRollbackOutcome,
) -> &'static str {
    match outcome {
        RuntimeCapabilityRollbackOutcome::DryRun => "dry_run",
        RuntimeCapabilityRollbackOutcome::RolledBack => "rolled_back",
        RuntimeCapabilityRollbackOutcome::AlreadyRolledBack => "already_rolled_back",
    }
}

fn render_runtime_capability_planned_payload(
    payload: &RuntimeCapabilityPromotionPlannedPayload,
) -> String {
    let accepted_candidate_ids = render_string_values(&payload.provenance.accepted_candidate_ids);
    let changed_surfaces = render_string_values(&payload.provenance.changed_surfaces);
    let draft_payload = render_runtime_capability_draft_payload(&payload.payload);
    format!(
        "target={} draft_id={} review_scope={} accepted_candidate_ids={} changed_surfaces={} payload={}",
        render_target(payload.target),
        payload.draft_id,
        payload.review_scope,
        accepted_candidate_ids,
        changed_surfaces,
        draft_payload
    )
}

fn render_runtime_capability_draft_payload(payload: &RuntimeCapabilityDraftPayload) -> String {
    match payload {
        RuntimeCapabilityDraftPayload::ManagedSkillBundle { files } => {
            let file_names = files.keys().cloned().collect::<Vec<_>>().join(",");
            format!("managed_skill_bundle files={file_names}")
        }
        RuntimeCapabilityDraftPayload::ProgrammaticFlowSpec { files } => {
            let file_names = files.keys().cloned().collect::<Vec<_>>().join(",");
            format!("programmatic_flow_spec files={file_names}")
        }
        RuntimeCapabilityDraftPayload::ProfileNoteAddendum { content } => {
            let content_chars = content.chars().count();
            format!("profile_note_addendum chars={content_chars}")
        }
    }
}

fn render_experiment_status(status: RuntimeExperimentStatus) -> &'static str {
    match status {
        RuntimeExperimentStatus::Planned => "planned",
        RuntimeExperimentStatus::Completed => "completed",
        RuntimeExperimentStatus::Aborted => "aborted",
    }
}

fn render_experiment_decision(decision: RuntimeExperimentDecision) -> &'static str {
    match decision {
        RuntimeExperimentDecision::Undecided => "undecided",
        RuntimeExperimentDecision::Promoted => "promoted",
        RuntimeExperimentDecision::Rejected => "rejected",
    }
}

pub(super) fn render_family_readiness_status(
    status: RuntimeCapabilityFamilyReadinessStatus,
) -> &'static str {
    match status {
        RuntimeCapabilityFamilyReadinessStatus::Ready => "ready",
        RuntimeCapabilityFamilyReadinessStatus::NotReady => "not_ready",
        RuntimeCapabilityFamilyReadinessStatus::Blocked => "blocked",
    }
}

fn render_family_readiness_check_status(
    status: RuntimeCapabilityFamilyReadinessCheckStatus,
) -> &'static str {
    match status {
        RuntimeCapabilityFamilyReadinessCheckStatus::Pass => "pass",
        RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence => "needs_evidence",
        RuntimeCapabilityFamilyReadinessCheckStatus::Blocked => "blocked",
    }
}

pub fn render_runtime_capability_promotion_plan_text(
    report: &RuntimeCapabilityPromotionPlanReport,
) -> String {
    let body = [
        format!("family_id={}", report.family_id),
        format!("promotable={}", report.promotable),
        format!(
            "readiness={}",
            render_family_readiness_status(report.readiness.status)
        ),
        format!(
            "target={}",
            render_target(report.planned_artifact.target_kind)
        ),
        format!("artifact_kind={}", report.planned_artifact.artifact_kind),
        format!("artifact_id={}", report.planned_artifact.artifact_id),
        format!(
            "delivery_surface={}",
            report.planned_artifact.delivery_surface
        ),
        format!("target_summary={}", report.planned_artifact.summary),
        format!("bounded_scope={}", report.planned_artifact.bounded_scope),
        format!(
            "required_capabilities={}",
            render_string_values(&report.planned_artifact.required_capabilities)
        ),
        format!(
            "tags={}",
            render_string_values(&report.planned_artifact.tags)
        ),
        format!(
            "delta_evidence_candidates={}",
            report.evidence.delta_candidate_count
        ),
        format!(
            "delta_changed_surfaces={}",
            render_string_values(&report.evidence.changed_surfaces)
        ),
        format!(
            "blockers={}",
            render_family_readiness_checks(&report.blockers)
        ),
        format!(
            "checks={}",
            render_family_readiness_checks(&report.readiness.checks)
        ),
        format!(
            "approval_checklist={}",
            render_string_values_with_separator(&report.approval_checklist, " | ")
        ),
        format!(
            "rollback_hints={}",
            render_string_values_with_separator(&report.rollback_hints, " | ")
        ),
        format!(
            "provenance_candidate_ids={}",
            render_string_values(&report.provenance.candidate_ids)
        ),
        format!(
            "provenance_source_run_ids={}",
            render_string_values(&report.provenance.source_run_ids)
        ),
        format!(
            "provenance_experiment_ids={}",
            render_string_values(&report.provenance.experiment_ids)
        ),
        format!(
            "provenance_source_run_artifact_paths={}",
            render_string_values_with_separator(
                &report.provenance.source_run_artifact_paths,
                " | "
            )
        ),
        format!(
            "planned_payload={}",
            render_runtime_capability_planned_payload(&report.planned_payload)
        ),
    ]
    .join("\n");

    wrap_runtime_capability_surface("promotion plan", body)
}

fn wrap_runtime_capability_surface(title: &str, body: String) -> String {
    crate::render_operator_shell_surface_from_body(title, "runtime capability", body)
}

pub(super) fn render_family_readiness_checks(
    checks: &[RuntimeCapabilityFamilyReadinessCheck],
) -> String {
    if checks.is_empty() {
        "-".to_owned()
    } else {
        checks
            .iter()
            .map(render_family_readiness_check)
            .collect::<Vec<_>>()
            .join(" | ")
    }
}
