use super::*;

pub(super) fn emit_runtime_capability_artifact(
    artifact: &RuntimeCapabilityArtifactDocument,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(artifact)
            .map_err(|error| format!("serialize runtime capability artifact failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_text(artifact));
    Ok(())
}

pub(super) fn emit_runtime_capability_index_report(
    report: &RuntimeCapabilityIndexReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability index report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_index_text(report));
    Ok(())
}

pub(super) fn emit_runtime_capability_promotion_plan(
    report: &RuntimeCapabilityPromotionPlanReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability promotion plan failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_promotion_plan_text(report));
    Ok(())
}

pub(super) fn emit_runtime_capability_apply_report(
    report: &RuntimeCapabilityApplyReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability apply report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_apply_text(report));
    Ok(())
}

pub(super) fn emit_runtime_capability_activate_report(
    report: &RuntimeCapabilityActivateReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability activate report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_activate_text(report));
    Ok(())
}

pub(super) fn emit_runtime_capability_rollback_report(
    report: &RuntimeCapabilityRollbackReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability rollback report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_rollback_text(report));
    Ok(())
}
