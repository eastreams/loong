use super::*;

pub(super) fn now_rfc3339() -> CliResult<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("format runtime capability timestamp failed: {error}"))
}

pub(super) fn persist_runtime_capability_artifact(
    output: &str,
    artifact: &RuntimeCapabilityArtifactDocument,
) -> CliResult<()> {
    let output_path = PathBuf::from(output);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime capability artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("serialize runtime capability artifact failed: {error}"))?;
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "write runtime capability artifact {} failed: {error}",
            output_path.display()
        )
    })?;
    Ok(())
}
