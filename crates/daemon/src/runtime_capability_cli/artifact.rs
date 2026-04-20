use super::*;

pub(super) fn load_runtime_capability_artifact(
    path: &Path,
) -> CliResult<RuntimeCapabilityArtifactDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability artifact {} failed: {error}",
            path.display()
        )
    })?;
    let artifact =
        serde_json::from_str::<RuntimeCapabilityArtifactDocument>(&raw).map_err(|error| {
            format!(
                "decode runtime capability artifact {} failed: {error}",
                path.display()
            )
        })?;
    if artifact.schema.version != RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime capability artifact {} uses unsupported schema version {}; expected {}",
            path.display(),
            artifact.schema.version,
            RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }
    validate_runtime_capability_artifact_schema(&artifact, path)?;
    validate_runtime_capability_artifact_state(&artifact, path)?;
    Ok(artifact)
}

fn validate_runtime_capability_artifact_schema(
    artifact: &RuntimeCapabilityArtifactDocument,
    path: &Path,
) -> CliResult<()> {
    if artifact.schema.surface != RUNTIME_CAPABILITY_ARTIFACT_SURFACE {
        return Err(format!(
            "runtime capability artifact {} uses unsupported schema surface {}; expected {}",
            path.display(),
            artifact.schema.surface,
            RUNTIME_CAPABILITY_ARTIFACT_SURFACE
        ));
    }
    if artifact.schema.purpose != RUNTIME_CAPABILITY_ARTIFACT_PURPOSE {
        return Err(format!(
            "runtime capability artifact {} uses unsupported schema purpose {}; expected {}",
            path.display(),
            artifact.schema.purpose,
            RUNTIME_CAPABILITY_ARTIFACT_PURPOSE
        ));
    }
    Ok(())
}

fn validate_runtime_capability_artifact_state(
    artifact: &RuntimeCapabilityArtifactDocument,
    path: &Path,
) -> CliResult<()> {
    match artifact.status {
        RuntimeCapabilityStatus::Proposed => {
            if artifact.reviewed_at.is_some()
                || artifact.review.is_some()
                || artifact.decision != RuntimeCapabilityDecision::Undecided
            {
                return Err(format!(
                    "runtime capability artifact {} has inconsistent proposed state",
                    path.display()
                ));
            }
        }
        RuntimeCapabilityStatus::Reviewed => {
            if artifact.reviewed_at.is_none()
                || artifact.review.is_none()
                || artifact.decision == RuntimeCapabilityDecision::Undecided
            {
                return Err(format!(
                    "runtime capability artifact {} has inconsistent reviewed state",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn compute_candidate_id(
    created_at: &str,
    label: Option<&str>,
    source_run: &RuntimeCapabilitySourceRunSummary,
    target: RuntimeCapabilityTarget,
    summary: &str,
    bounded_scope: &str,
    tags: &[String],
    required_capabilities: &[String],
) -> CliResult<String> {
    let encoded = serde_json::to_vec(&json!({
        "created_at": created_at,
        "label": label,
        "source_run_id": source_run.run_id,
        "target": render_target(target),
        "summary": summary,
        "bounded_scope": bounded_scope,
        "tags": tags,
        "required_capabilities": required_capabilities,
    }))
    .map_err(|error| format!("serialize runtime capability candidate_id input failed: {error}"))?;
    Ok(hex::encode(sha2::Sha256::digest(encoded)))
}
