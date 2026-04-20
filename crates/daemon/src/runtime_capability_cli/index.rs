use super::*;

fn collect_runtime_capability_artifacts(
    root: &Path,
    artifacts: &mut Vec<RuntimeCapabilityArtifactDocument>,
) -> CliResult<()> {
    let mut entries = fs::read_dir(root)
        .map_err(|error| {
            format!(
                "read runtime capability index root {} failed: {error}",
                root.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "enumerate runtime capability index root {} failed: {error}",
                root.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let entry_type = entry.file_type().map_err(|error| {
            format!(
                "inspect runtime capability index entry {} failed: {error}",
                path.display()
            )
        })?;
        if entry_type.is_symlink() {
            continue;
        }
        if entry_type.is_dir() {
            collect_runtime_capability_artifacts(&path, artifacts)?;
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(artifact) = load_supported_runtime_capability_artifact(&path)? else {
            continue;
        };
        artifacts.push(artifact);
    }
    Ok(())
}

pub(super) fn collect_runtime_capability_family_artifacts(
    root: &Path,
) -> CliResult<BTreeMap<String, Vec<RuntimeCapabilityArtifactDocument>>> {
    let mut artifacts = Vec::new();
    collect_runtime_capability_artifacts(root, &mut artifacts)?;

    let mut families_by_id = BTreeMap::<String, Vec<RuntimeCapabilityArtifactDocument>>::new();
    for artifact in artifacts {
        let family_id = compute_family_id(&artifact.proposal)?;
        families_by_id.entry(family_id).or_default().push(artifact);
    }
    Ok(families_by_id)
}

fn load_supported_runtime_capability_artifact(
    path: &Path,
) -> CliResult<Option<RuntimeCapabilityArtifactDocument>> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability index entry {} failed: {error}",
            path.display()
        )
    })?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
        format!(
            "decode runtime capability index entry {} failed: {error}",
            path.display()
        )
    })?;
    let Some(surface) = value
        .get("schema")
        .and_then(|schema| schema.get("surface"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    if surface != RUNTIME_CAPABILITY_ARTIFACT_SURFACE {
        return Ok(None);
    }
    load_runtime_capability_artifact(path).map(Some)
}

pub(super) fn sort_runtime_capability_artifacts(
    artifacts: &mut [RuntimeCapabilityArtifactDocument],
) {
    artifacts.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
}
