use super::*;

pub(super) fn build_runtime_capability_family_summary(
    family_id: String,
    mut artifacts: Vec<RuntimeCapabilityArtifactDocument>,
) -> CliResult<RuntimeCapabilityFamilySummary> {
    sort_runtime_capability_artifacts(&mut artifacts);
    let proposal = artifacts
        .first()
        .map(|artifact| artifact.proposal.clone())
        .ok_or_else(|| "runtime capability family cannot be empty".to_owned())?;
    let candidate_ids = artifacts
        .iter()
        .map(|artifact| artifact.candidate_id.clone())
        .collect::<Vec<_>>();
    let evidence = build_family_evidence_digest(&artifacts);
    let readiness = evaluate_family_readiness(&artifacts, &evidence);

    Ok(RuntimeCapabilityFamilySummary {
        family_id,
        proposal,
        candidate_ids,
        evidence,
        readiness,
    })
}

pub(super) fn compute_family_id(proposal: &RuntimeCapabilityProposal) -> CliResult<String> {
    let tags = normalize_repeated_values(&proposal.tags);
    let required_capabilities = parse_required_capabilities(&proposal.required_capabilities)?;
    let encoded = serde_json::to_vec(&json!({
        "target": render_target(proposal.target),
        "summary": proposal.summary.trim(),
        "bounded_scope": proposal.bounded_scope.trim(),
        "tags": tags,
        "required_capabilities": required_capabilities,
    }))
    .map_err(|error| format!("serialize runtime capability family_id input failed: {error}"))?;
    Ok(hex::encode(sha2::Sha256::digest(encoded)))
}

fn build_family_evidence_digest(
    artifacts: &[RuntimeCapabilityArtifactDocument],
) -> RuntimeCapabilityEvidenceDigest {
    let reviewed_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.status == RuntimeCapabilityStatus::Reviewed)
        .count();
    let undecided_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Undecided)
        .count();
    let accepted_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Accepted)
        .count();
    let rejected_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Rejected)
        .count();
    let distinct_source_run_count = artifacts
        .iter()
        .map(|artifact| artifact.source_run.run_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let distinct_experiment_count = artifacts
        .iter()
        .map(|artifact| artifact.source_run.experiment_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let latest_candidate_at = artifacts
        .iter()
        .map(|artifact| artifact.created_at.as_str())
        .max()
        .map(str::to_owned);
    let latest_reviewed_at = artifacts
        .iter()
        .filter_map(|artifact| artifact.reviewed_at.as_deref())
        .max()
        .map(str::to_owned);

    let mut promoted = 0;
    let mut rejected = 0;
    let mut undecided = 0;
    let mut unique_warnings = BTreeSet::new();
    let mut changed_surfaces = BTreeSet::new();
    let mut delta_candidate_count = 0;
    let mut metric_bounds = BTreeMap::<String, RuntimeCapabilityMetricRange>::new();

    for artifact in artifacts {
        match artifact.source_run.decision {
            RuntimeExperimentDecision::Promoted => promoted += 1,
            RuntimeExperimentDecision::Rejected => rejected += 1,
            RuntimeExperimentDecision::Undecided => undecided += 1,
        }

        if let Some(snapshot_delta) = artifact.source_run.snapshot_delta.as_ref() {
            delta_candidate_count += 1;
            changed_surfaces.extend(snapshot_delta.changed_surfaces());
        }

        if artifact.decision == RuntimeCapabilityDecision::Accepted {
            for warning in &artifact.source_run.warnings {
                unique_warnings.insert(warning.clone());
            }
        }

        for (metric, value) in &artifact.source_run.metrics {
            let entry = metric_bounds.entry(metric.clone()).or_insert_with(|| {
                RuntimeCapabilityMetricRange {
                    min: *value,
                    max: *value,
                }
            });
            entry.min = entry.min.min(*value);
            entry.max = entry.max.max(*value);
        }
    }

    RuntimeCapabilityEvidenceDigest {
        total_candidates: artifacts.len(),
        reviewed_candidates,
        undecided_candidates,
        accepted_candidates,
        rejected_candidates,
        distinct_source_run_count,
        distinct_experiment_count,
        latest_candidate_at,
        latest_reviewed_at,
        source_decisions: RuntimeCapabilitySourceDecisionRollup {
            promoted,
            rejected,
            undecided,
        },
        unique_warnings: unique_warnings.into_iter().collect(),
        delta_candidate_count,
        changed_surfaces: changed_surfaces.into_iter().collect(),
        metric_ranges: metric_bounds,
    }
}

fn evaluate_family_readiness(
    artifacts: &[RuntimeCapabilityArtifactDocument],
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadiness {
    let review_consensus = evaluate_review_consensus(evidence);
    let stability = evaluate_stability(evidence);
    let accepted_source_integrity = evaluate_accepted_source_integrity(artifacts, evidence);
    let warning_pressure = evaluate_warning_pressure(evidence);
    let checks = vec![
        review_consensus,
        stability,
        accepted_source_integrity,
        warning_pressure,
    ];
    let status = if checks
        .iter()
        .any(|check| check.status == RuntimeCapabilityFamilyReadinessCheckStatus::Blocked)
    {
        RuntimeCapabilityFamilyReadinessStatus::Blocked
    } else if checks
        .iter()
        .all(|check| check.status == RuntimeCapabilityFamilyReadinessCheckStatus::Pass)
    {
        RuntimeCapabilityFamilyReadinessStatus::Ready
    } else {
        RuntimeCapabilityFamilyReadinessStatus::NotReady
    };
    RuntimeCapabilityFamilyReadiness { status, checks }
}

fn evaluate_review_consensus(
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    let (status, summary) = if evidence.rejected_candidates > 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Blocked,
            format!(
                "{} candidate(s) in this family were explicitly rejected",
                evidence.rejected_candidates
            ),
        )
    } else if evidence.undecided_candidates > 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            format!(
                "{} candidate(s) still require operator review",
                evidence.undecided_candidates
            ),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            "all candidate evidence is reviewed and accepted".to_owned(),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "review_consensus".to_owned(),
        status,
        summary,
    }
}

fn evaluate_stability(
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    let (status, summary) = if evidence.distinct_source_run_count >= 2 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            format!(
                "family is supported by {} distinct source runs",
                evidence.distinct_source_run_count
            ),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            "family needs repeated evidence from at least two distinct source runs".to_owned(),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "stability".to_owned(),
        status,
        summary,
    }
}

fn evaluate_accepted_source_integrity(
    artifacts: &[RuntimeCapabilityArtifactDocument],
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    if evidence.accepted_candidates == 0 {
        return RuntimeCapabilityFamilyReadinessCheck {
            dimension: "accepted_source_integrity".to_owned(),
            status: RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            summary: "family has no accepted candidates yet".to_owned(),
        };
    }

    let invalid_sources = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Accepted)
        .filter(|artifact| {
            artifact.source_run.status != RuntimeExperimentStatus::Completed
                || artifact.source_run.decision != RuntimeExperimentDecision::Promoted
                || artifact.source_run.result_snapshot_id.is_none()
        })
        .count();

    let (status, summary) = if invalid_sources > 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Blocked,
            format!(
                "{} accepted candidate(s) came from incomplete or non-promoted source runs",
                invalid_sources
            ),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            "accepted candidates all trace back to completed promoted runs".to_owned(),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "accepted_source_integrity".to_owned(),
        status,
        summary,
    }
}

fn evaluate_warning_pressure(
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    let (status, summary) = if evidence.accepted_candidates == 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            "warning pressure cannot be evaluated before the family has accepted evidence"
                .to_owned(),
        )
    } else if evidence.unique_warnings.is_empty() {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            "accepted candidates carry no source warnings".to_owned(),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            format!(
                "accepted evidence still carries warnings: {}",
                evidence.unique_warnings.join(" | ")
            ),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "warning_pressure".to_owned(),
        status,
        summary,
    }
}
