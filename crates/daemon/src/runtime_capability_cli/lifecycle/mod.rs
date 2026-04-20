use super::*;

mod activate;
mod record;
mod rollback;

pub(super) use activate::{
    execute_runtime_capability_activate_managed_skill,
    execute_runtime_capability_activate_profile_note_addendum,
};
pub(super) use record::{
    build_runtime_capability_activation_record_path,
    build_runtime_capability_managed_skill_activation_record,
    build_runtime_capability_profile_note_activation_record,
    load_runtime_capability_activation_record, persist_runtime_capability_activation_record,
};
pub(super) use rollback::{
    execute_runtime_capability_rollback_managed_skill,
    execute_runtime_capability_rollback_profile_note_addendum,
};

pub(super) fn validate_runtime_capability_apply_plan(
    plan: &RuntimeCapabilityPromotionPlanReport,
) -> CliResult<()> {
    if plan.promotable {
        return Ok(());
    }

    let readiness = render_family_readiness_status(plan.readiness.status);
    let blockers = render_family_readiness_checks(&plan.blockers);
    let error = format!(
        "runtime capability family `{}` is not promotable for apply; readiness={} blockers={}",
        plan.family_id, readiness, blockers
    );
    Err(error)
}

pub(super) fn resolve_runtime_capability_apply_output_path(
    root: &Path,
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
) -> PathBuf {
    let delivery_surface = planned_artifact.delivery_surface.as_str();
    let artifact_id = planned_artifact.artifact_id.as_str();
    let artifact_file_name = format!("{artifact_id}.json");
    root.join(delivery_surface).join(artifact_file_name)
}

pub(super) fn build_runtime_capability_apply_artifact(
    plan: &RuntimeCapabilityPromotionPlanReport,
) -> RuntimeCapabilityAppliedArtifactDocument {
    let planned_artifact = &plan.planned_artifact;
    let provenance = &plan.provenance;
    let evidence = &plan.evidence;
    let planned_payload = &plan.planned_payload;

    RuntimeCapabilityAppliedArtifactDocument {
        schema: RuntimeCapabilityArtifactSchema {
            version: RUNTIME_CAPABILITY_APPLY_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: RUNTIME_CAPABILITY_APPLY_ARTIFACT_SURFACE.to_owned(),
            purpose: RUNTIME_CAPABILITY_APPLY_ARTIFACT_PURPOSE.to_owned(),
        },
        family_id: plan.family_id.clone(),
        artifact_kind: planned_payload.artifact_kind.clone(),
        artifact_id: planned_payload.draft_id.clone(),
        delivery_surface: planned_artifact.delivery_surface.clone(),
        target: planned_payload.target,
        summary: planned_payload.summary.clone(),
        bounded_scope: planned_payload.review_scope.clone(),
        required_capabilities: planned_payload.required_capabilities.clone(),
        tags: planned_payload.tags.clone(),
        payload: planned_payload.payload.clone(),
        approval_checklist: plan.approval_checklist.clone(),
        rollback_hints: plan.rollback_hints.clone(),
        delta_candidate_count: evidence.delta_candidate_count,
        changed_surfaces: evidence.changed_surfaces.clone(),
        candidate_ids: planned_payload.provenance.accepted_candidate_ids.clone(),
        source_run_ids: provenance.source_run_ids.clone(),
        experiment_ids: provenance.experiment_ids.clone(),
        source_run_artifact_paths: provenance.source_run_artifact_paths.clone(),
        latest_candidate_at: provenance.latest_candidate_at.clone(),
        latest_reviewed_at: provenance.latest_reviewed_at.clone(),
    }
}

pub(super) fn load_runtime_capability_apply_artifact(
    path: &Path,
) -> CliResult<RuntimeCapabilityAppliedArtifactDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability apply artifact {} failed: {error}",
            path.display()
        )
    })?;
    let artifact = serde_json::from_str::<RuntimeCapabilityAppliedArtifactDocument>(&raw).map_err(
        |error| {
            format!(
                "decode runtime capability apply artifact {} failed: {error}",
                path.display()
            )
        },
    )?;
    validate_runtime_capability_apply_artifact_schema(&artifact, path)?;
    Ok(artifact)
}

fn validate_runtime_capability_apply_artifact_schema(
    artifact: &RuntimeCapabilityAppliedArtifactDocument,
    path: &Path,
) -> CliResult<()> {
    let schema = &artifact.schema;
    if schema.version != RUNTIME_CAPABILITY_APPLY_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime capability apply artifact {} uses unsupported schema version {}; expected {}",
            path.display(),
            schema.version,
            RUNTIME_CAPABILITY_APPLY_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }
    if schema.surface != RUNTIME_CAPABILITY_APPLY_ARTIFACT_SURFACE {
        return Err(format!(
            "runtime capability apply artifact {} uses unsupported schema surface {}; expected {}",
            path.display(),
            schema.surface,
            RUNTIME_CAPABILITY_APPLY_ARTIFACT_SURFACE
        ));
    }
    if schema.purpose != RUNTIME_CAPABILITY_APPLY_ARTIFACT_PURPOSE {
        return Err(format!(
            "runtime capability apply artifact {} uses unsupported schema purpose {}; expected {}",
            path.display(),
            schema.purpose,
            RUNTIME_CAPABILITY_APPLY_ARTIFACT_PURPOSE
        ));
    }
    Ok(())
}

pub(super) fn persist_runtime_capability_apply_artifact(
    output_path: &Path,
    artifact: &RuntimeCapabilityAppliedArtifactDocument,
) -> CliResult<RuntimeCapabilityApplyOutcome> {
    let write_result = write_pretty_json_file_create_new(output_path, artifact);
    match write_result {
        Ok(()) => Ok(RuntimeCapabilityApplyOutcome::Applied),
        Err(error) if error.contains("already exists") => {
            let existing_artifact = load_runtime_capability_apply_artifact(output_path)?;
            if existing_artifact == *artifact {
                return Ok(RuntimeCapabilityApplyOutcome::AlreadyApplied);
            }

            let message = format!(
                "runtime capability apply output {} already exists with different content",
                output_path.display()
            );
            Err(message)
        }
        Err(error) => Err(error),
    }
}

fn write_pretty_json_file_create_new(path: &Path, value: &impl Serialize) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime capability apply artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }

    let encoded = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize runtime capability apply artifact failed: {error}"))?;
    let temp_path = runtime_capability_apply_temp_path(path);
    let mut temp_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| {
            format!(
                "write runtime capability apply artifact {} failed: {error}",
                path.display()
            )
        })?;
    let write_result = temp_file.write_all(encoded.as_slice());
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "write runtime capability apply artifact {} failed: {error}",
            path.display()
        ));
    }

    let sync_result = temp_file.sync_all();
    if let Err(error) = sync_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "write runtime capability apply artifact {} failed: {error}",
            path.display()
        ));
    }

    drop(temp_file);

    let publish_result = fs::hard_link(&temp_path, path);
    let _ = fs::remove_file(&temp_path);
    match publish_result {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Err(format!(
                "runtime capability apply artifact {} already exists",
                path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "write runtime capability apply artifact {} failed: {error}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn resolve_runtime_capability_activation_staging_base_root(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> CliResult<PathBuf> {
    let file_root = match tool_runtime.file_root.clone() {
        Some(path) => path,
        None => std::env::current_dir()
            .map_err(|error| format!("read current dir for activation staging failed: {error}"))?,
    };
    let staging_base_root = file_root.join(".runtime-capability-staging");
    Ok(staging_base_root)
}

fn managed_skill_payload_matches_install_root(
    expected_files: &BTreeMap<String, String>,
    install_root: &Path,
) -> CliResult<bool> {
    let current_files = collect_runtime_capability_bundle_files(install_root)?;
    let Some(current_files) = current_files else {
        return Ok(false);
    };
    Ok(&current_files == expected_files)
}

fn write_runtime_capability_draft_files_to_staging(
    files: &BTreeMap<String, String>,
    staging_base_root: &Path,
) -> CliResult<PathBuf> {
    fs::create_dir_all(staging_base_root).map_err(|error| {
        format!(
            "create runtime capability staging root {} failed: {error}",
            staging_base_root.display()
        )
    })?;
    let staging_root = build_runtime_capability_temp_dir(staging_base_root, "draft");
    fs::create_dir_all(&staging_root).map_err(|error| {
        format!(
            "create runtime capability staging directory {} failed: {error}",
            staging_root.display()
        )
    })?;
    for (relative_path, contents) in files {
        let relative_path = normalize_runtime_capability_relative_path(relative_path)?;
        let output_path = staging_root.join(relative_path);
        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "create runtime capability staging directory {} failed: {error}",
                    parent.display()
                )
            })?;
        }
        fs::write(&output_path, contents).map_err(|error| {
            format!(
                "write runtime capability staging file {} failed: {error}",
                output_path.display()
            )
        })?;
    }
    Ok(staging_root)
}

fn normalize_runtime_capability_relative_path(raw: &str) -> CliResult<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("runtime capability draft file path must not be empty".to_owned());
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        return Err(format!(
            "runtime capability draft file path `{trimmed}` must be relative"
        ));
    }
    let escapes_root = path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir));
    if escapes_root {
        return Err(format!(
            "runtime capability draft file path `{trimmed}` must not traverse parent directories"
        ));
    }
    Ok(path)
}

fn build_runtime_capability_temp_dir(staging_base_root: &Path, label: &str) -> PathBuf {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let pid = std::process::id();
    let directory_name = format!("{label}-{pid}-{now_ms}");
    staging_base_root.join(directory_name)
}

fn canonicalize_optional_path(path: &Path) -> CliResult<String> {
    match fs::canonicalize(path) {
        Ok(canonicalized) => Ok(canonicalized.display().to_string()),
        Err(_) => Ok(path.display().to_string()),
    }
}

fn runtime_capability_apply_temp_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "runtime-capability.json".to_owned());
    let pid = std::process::id();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    parent.join(format!(".{file_name}.{pid}.{now_ms}.tmp"))
}

pub(super) fn canonicalize_existing_path(path: &Path) -> CliResult<String> {
    fs::canonicalize(path)
        .map(|canonicalized| canonicalized.display().to_string())
        .map_err(|error| format!("canonicalize {} failed: {error}", path.display()))
}

fn build_managed_skill_activation_verification_hints(
    target_path: &Path,
    file_count: usize,
) -> Vec<String> {
    let target_display = target_path.display().to_string();
    let verification = format!(
        "verify {target_display} exists and matches the applied managed skill bundle with {file_count} file(s)"
    );
    vec![verification]
}

fn verify_managed_skill_activation_state(
    artifact_id: &str,
    target_path: &Path,
    expected_files: &BTreeMap<String, String>,
) -> CliResult<Vec<String>> {
    let matches_payload = managed_skill_payload_matches_install_root(expected_files, target_path)?;
    if !matches_payload {
        return Err(format!(
            "runtime capability activation did not materialize managed skill `{artifact_id}` at {}",
            target_path.display()
        ));
    }

    let file_count = expected_files.len();
    let verification = format!(
        "verified {} matches the applied managed skill bundle with {file_count} file(s)",
        target_path.display()
    );
    Ok(vec![verification])
}

fn build_profile_note_activation_verification_hints(
    config_path: &Path,
    addendum: &str,
) -> Vec<String> {
    let config_display = config_path.display().to_string();
    let trimmed_addendum = addendum.trim();
    let addendum_chars = trimmed_addendum.chars().count();
    vec![
        format!(
            "verify {config_display} uses memory.profile={}",
            render_memory_profile(mvp::config::MemoryProfile::ProfilePlusWindow)
        ),
        format!(
            "verify {config_display} appends the {addendum_chars}-character advisory addendum to memory.profile_note"
        ),
    ]
}

fn verify_profile_note_addendum_activation_state(
    config_path: &Path,
    addendum: &str,
) -> CliResult<Vec<String>> {
    let trimmed_addendum = addendum.trim();
    let config_path_text = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_text.as_str()))?;
    let (_, config) = load_result;
    if config.memory.profile != mvp::config::MemoryProfile::ProfilePlusWindow {
        return Err(format!(
            "runtime capability activation expected {} to use memory.profile={}",
            config_path.display(),
            render_memory_profile(mvp::config::MemoryProfile::ProfilePlusWindow)
        ));
    }
    let profile_note = config.memory.profile_note.as_deref().unwrap_or("");
    if !profile_note.contains(trimmed_addendum) {
        return Err(format!(
            "runtime capability activation expected {} to contain the advisory profile_note addendum",
            config_path.display()
        ));
    }
    let addendum_chars = trimmed_addendum.chars().count();
    Ok(vec![
        format!(
            "verified {} uses memory.profile={}",
            config_path.display(),
            render_memory_profile(mvp::config::MemoryProfile::ProfilePlusWindow)
        ),
        format!(
            "verified {} contains the {addendum_chars}-character advisory profile_note addendum",
            config_path.display()
        ),
    ])
}

fn collect_runtime_capability_bundle_files(
    root: &Path,
) -> CliResult<Option<BTreeMap<String, String>>> {
    if !root.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(root).map_err(|error| {
        format!(
            "read runtime capability bundle root metadata {} failed: {error}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "runtime capability bundle root {} must be a directory",
            root.display()
        ));
    }

    let mut files = BTreeMap::new();
    collect_runtime_capability_bundle_files_recursive(root, root, &mut files)?;
    Ok(Some(files))
}

fn collect_runtime_capability_bundle_files_recursive(
    bundle_root: &Path,
    current_root: &Path,
    files: &mut BTreeMap<String, String>,
) -> CliResult<()> {
    let read_dir = fs::read_dir(current_root).map_err(|error| {
        format!(
            "read runtime capability bundle directory {} failed: {error}",
            current_root.display()
        )
    })?;
    let mut entries = Vec::new();
    for entry_result in read_dir {
        let entry = entry_result.map_err(|error| {
            format!(
                "read runtime capability bundle directory entry under {} failed: {error}",
                current_root.display()
            )
        })?;
        entries.push(entry.path());
    }
    entries.sort();

    for entry_path in entries {
        let entry_metadata = fs::metadata(&entry_path).map_err(|error| {
            format!(
                "read runtime capability bundle entry metadata {} failed: {error}",
                entry_path.display()
            )
        })?;
        if entry_metadata.is_dir() {
            collect_runtime_capability_bundle_files_recursive(
                bundle_root,
                entry_path.as_path(),
                files,
            )?;
            continue;
        }
        if !entry_metadata.is_file() {
            continue;
        }
        let relative_path = entry_path.strip_prefix(bundle_root).map_err(|error| {
            format!(
                "derive runtime capability bundle relative path for {} failed: {error}",
                entry_path.display()
            )
        })?;
        let relative_path_text = normalized_path_text(&relative_path.display().to_string());
        let contents = fs::read_to_string(&entry_path).map_err(|error| {
            format!(
                "read runtime capability bundle file {} failed: {error}",
                entry_path.display()
            )
        })?;
        files.insert(relative_path_text, contents);
    }
    Ok(())
}
