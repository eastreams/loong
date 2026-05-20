use super::*;
pub fn read_spec_file(path: &str) -> CliResult<RunnerSpec> {
    read_spec_file_with_bridge_support_resolution(path, None).map(|resolved| resolved.spec)
}

pub fn read_spec_file_with_bridge_support_selection(
    path: &str,
    bridge_support_selection_override: Option<&BridgeSupportSelectionInput>,
) -> CliResult<RunnerSpec> {
    read_spec_file_with_bridge_support_resolution(path, bridge_support_selection_override)
        .map(|resolved| resolved.spec)
}

pub fn read_spec_file_with_bridge_support_resolution(
    path: &str,
    bridge_support_selection_override: Option<&BridgeSupportSelectionInput>,
) -> CliResult<ResolvedRunnerSpecFile> {
    let mut input = read_spec_file_input(path)?;
    let spec_has_bridge_support_config =
        input.spec.bridge_support.is_some() || input.bridge_support_selection.is_some();

    if let Some(selection) = bridge_support_selection_override {
        if spec_has_bridge_support_config {
            return Err(format!(
                "spec file {path} accepts either file-local bridge support configuration or CLI bridge support selection overrides, not both"
            ));
        }
        let override_selection = resolve_process_relative_bridge_support_selection(selection)?;
        input.bridge_support_selection = Some(override_selection);
    }

    resolve_spec_file_input(path, input)
}

fn resolve_process_relative_bridge_support_selection(
    selection: &BridgeSupportSelectionInput,
) -> CliResult<BridgeSupportSelectionInput> {
    let path = selection
        .path
        .as_deref()
        .map(resolve_process_relative_path)
        .transpose()?;
    let delta_artifact = selection
        .delta_artifact
        .as_deref()
        .map(resolve_process_relative_path)
        .transpose()?;

    Ok(BridgeSupportSelectionInput {
        path,
        bundled_profile: selection.bundled_profile.clone(),
        delta_artifact,
        expected_sha256: selection.expected_sha256.clone(),
        expected_delta_sha256: selection.expected_delta_sha256.clone(),
    })
}

fn read_spec_file_input(path: &str) -> CliResult<RunnerSpecFileInput> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read spec file {path}: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("failed to parse spec file {path}: {error}"))
}

fn resolve_spec_file_input(
    path: &str,
    mut input: RunnerSpecFileInput,
) -> CliResult<ResolvedRunnerSpecFile> {
    if let Some(selection) = input.bridge_support_selection.take() {
        if input.spec.bridge_support.is_some() {
            return Err(format!(
                "spec file {path} accepts either inline `bridge_support` or `bridge_support_selection`, not both"
            ));
        }

        let policy_path = selection
            .path
            .as_deref()
            .map(|value| resolve_spec_relative_path(path, value));
        let delta_artifact_path = selection
            .delta_artifact
            .as_deref()
            .map(|value| resolve_spec_relative_path(path, value));
        let resolved = resolve_bridge_support_selection(
            policy_path.as_deref(),
            selection.bundled_profile.as_deref(),
            delta_artifact_path.as_deref(),
            selection.expected_sha256.as_deref(),
            selection.expected_delta_sha256.as_deref(),
        )
        .map_err(|error| {
            format!("failed to resolve bridge support selection in {path}: {error}")
        })?;
        let bridge_support_source = resolved
            .as_ref()
            .map(|selection| selection.policy.source.clone());
        let bridge_support_delta_source = resolved
            .as_ref()
            .and_then(|selection| selection.delta_source.clone());
        let bridge_support_delta_sha256 = resolved.as_ref().and_then(|selection| {
            selection
                .delta_artifact
                .as_ref()
                .map(|artifact| artifact.sha256.clone())
        });
        input.spec.bridge_support = resolved.map(|selection| selection.policy.profile);
        return Ok(ResolvedRunnerSpecFile {
            spec: input.spec,
            bridge_support_source,
            bridge_support_delta_source,
            bridge_support_delta_sha256,
        });
    }

    let bridge_support_source = input
        .spec
        .bridge_support
        .as_ref()
        .map(|_| format!("inline:{path}"));

    Ok(ResolvedRunnerSpecFile {
        spec: input.spec,
        bridge_support_source,
        bridge_support_delta_source: None,
        bridge_support_delta_sha256: None,
    })
}

fn resolve_process_relative_path(value: &str) -> CliResult<String> {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return Ok(value.to_owned());
    }

    let current_dir = std::env::current_dir()
        .map_err(|error| format!("resolve current directory failed: {error}"))?;
    let resolved = current_dir.join(candidate);

    Ok(resolved.display().to_string())
}

fn resolve_spec_relative_path(spec_path: &str, value: &str) -> String {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return value.to_owned();
    }

    Path::new(spec_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(candidate)
        .display()
        .to_string()
}

pub fn write_json_file<T: Serialize>(path: &str, value: &T) -> CliResult<()> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize JSON value for output file failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create output directory failed: {error}"))?;
    }
    fs::write(path, serialized)
        .map_err(|error| format!("write JSON output file failed: {error}"))?;
    Ok(())
}
