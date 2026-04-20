use super::*;

pub(super) fn validate_proposable_run(
    run: &RuntimeExperimentArtifactDocument,
    run_path: &str,
) -> CliResult<()> {
    if run.status == RuntimeExperimentStatus::Planned {
        return Err(format!(
            "runtime capability propose requires a finished runtime experiment run; {} is still planned",
            run_path
        ));
    }
    if run.evaluation.is_none() {
        return Err(format!(
            "runtime capability propose requires evaluation data on source run {}",
            run_path
        ));
    }
    Ok(())
}

pub(super) fn build_source_run_summary(
    run: &RuntimeExperimentArtifactDocument,
    artifact_path: Option<&Path>,
) -> CliResult<RuntimeCapabilitySourceRunSummary> {
    let evaluation = run
        .evaluation
        .as_ref()
        .ok_or_else(|| "runtime capability source run is missing evaluation".to_owned())?;
    let snapshot_delta = artifact_path
        .map(|path| derive_recorded_snapshot_delta_for_run(run, &path.display().to_string()))
        .transpose()?
        .flatten();
    Ok(RuntimeCapabilitySourceRunSummary {
        run_id: run.run_id.clone(),
        experiment_id: run.experiment_id.clone(),
        label: run.label.clone(),
        status: run.status,
        decision: run.decision,
        mutation_summary: run.mutation.summary.clone(),
        baseline_snapshot_id: run.baseline_snapshot.snapshot_id.clone(),
        result_snapshot_id: run
            .result_snapshot
            .as_ref()
            .map(|snapshot| snapshot.snapshot_id.clone()),
        evaluation_summary: evaluation.summary.clone(),
        metrics: evaluation.metrics.clone(),
        warnings: evaluation.warnings.clone(),
        snapshot_delta,
        artifact_path: artifact_path.map(canonicalize_existing_path).transpose()?,
    })
}
