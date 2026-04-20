use super::*;

pub(crate) fn build_runtime_capability_activation_record_path(
    artifact_path: &Path,
    artifact_id: &str,
) -> CliResult<PathBuf> {
    let artifact_root = artifact_path.parent().ok_or_else(|| {
        format!(
            "runtime capability artifact {} has no parent directory",
            artifact_path.display()
        )
    })?;
    Ok(artifact_root
        .join("activation-records")
        .join(format!("{artifact_id}.json")))
}

pub(crate) fn build_runtime_capability_managed_skill_activation_record(
    artifact_path: &str,
    config_path: &Path,
    artifact_id: &str,
    target: RuntimeCapabilityTarget,
    delivery_surface: &str,
    activation_surface: &str,
    target_path: &str,
    verification: &[String],
    rollback_hints: &[String],
    previous_files: Option<BTreeMap<String, String>>,
) -> CliResult<RuntimeCapabilityActivationRecordDocument> {
    let activation_id =
        build_runtime_capability_activation_id(artifact_id, target, target_path, verification)?;
    let rollback = RuntimeCapabilityRollbackPayload::ManagedSkillBundle { previous_files };
    let record = RuntimeCapabilityActivationRecordDocument {
        schema: RuntimeCapabilityArtifactSchema {
            version: RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION,
            surface: RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE.to_owned(),
            purpose: RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE.to_owned(),
        },
        activation_id,
        activated_at: now_rfc3339()?,
        artifact_path: artifact_path.to_owned(),
        config_path: config_path.display().to_string(),
        artifact_id: artifact_id.to_owned(),
        target,
        delivery_surface: delivery_surface.to_owned(),
        activation_surface: activation_surface.to_owned(),
        target_path: target_path.to_owned(),
        verification: verification.to_vec(),
        rollback_hints: rollback_hints.to_vec(),
        rollback,
    };
    Ok(record)
}

pub(crate) fn build_runtime_capability_profile_note_activation_record(
    artifact_path: &str,
    config_path: &Path,
    artifact_id: &str,
    target: RuntimeCapabilityTarget,
    delivery_surface: &str,
    activation_surface: &str,
    target_path: &str,
    verification: &[String],
    rollback_hints: &[String],
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<String>,
) -> CliResult<RuntimeCapabilityActivationRecordDocument> {
    let activation_id =
        build_runtime_capability_activation_id(artifact_id, target, target_path, verification)?;
    let rollback = RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
        previous_profile,
        previous_profile_note,
    };
    let record = RuntimeCapabilityActivationRecordDocument {
        schema: RuntimeCapabilityArtifactSchema {
            version: RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION,
            surface: RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE.to_owned(),
            purpose: RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE.to_owned(),
        },
        activation_id,
        activated_at: now_rfc3339()?,
        artifact_path: artifact_path.to_owned(),
        config_path: config_path.display().to_string(),
        artifact_id: artifact_id.to_owned(),
        target,
        delivery_surface: delivery_surface.to_owned(),
        activation_surface: activation_surface.to_owned(),
        target_path: target_path.to_owned(),
        verification: verification.to_vec(),
        rollback_hints: rollback_hints.to_vec(),
        rollback,
    };
    Ok(record)
}

fn build_runtime_capability_activation_id(
    artifact_id: &str,
    target: RuntimeCapabilityTarget,
    target_path: &str,
    verification: &[String],
) -> CliResult<String> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(artifact_id.as_bytes());
    hasher.update(render_target(target).as_bytes());
    hasher.update(target_path.as_bytes());
    for item in verification {
        hasher.update(item.as_bytes());
    }
    let digest = hasher.finalize();
    let activation_digest = hex::encode(digest);
    let activation_id = format!("runtime-capability-activation-{activation_digest}");
    Ok(activation_id)
}

pub(crate) fn persist_runtime_capability_activation_record(
    path: &Path,
    record: &RuntimeCapabilityActivationRecordDocument,
) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime capability activation record directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_vec_pretty(record).map_err(|error| {
        format!("serialize runtime capability activation record failed: {error}")
    })?;
    fs::write(path, encoded).map_err(|error| {
        format!(
            "write runtime capability activation record {} failed: {error}",
            path.display()
        )
    })?;
    Ok(())
}

pub(crate) fn load_runtime_capability_activation_record(
    path: &Path,
) -> CliResult<RuntimeCapabilityActivationRecordDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability activation record {} failed: {error}",
            path.display()
        )
    })?;
    let record = serde_json::from_str::<RuntimeCapabilityActivationRecordDocument>(&raw).map_err(
        |error| {
            format!(
                "decode runtime capability activation record {} failed: {error}",
                path.display()
            )
        },
    )?;
    validate_runtime_capability_activation_record_schema(&record, path)?;
    Ok(record)
}

fn validate_runtime_capability_activation_record_schema(
    record: &RuntimeCapabilityActivationRecordDocument,
    path: &Path,
) -> CliResult<()> {
    let schema = &record.schema;
    if schema.version != RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime capability activation record {} uses unsupported schema version {}; expected {}",
            path.display(),
            schema.version,
            RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION
        ));
    }
    if schema.surface != RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE {
        return Err(format!(
            "runtime capability activation record {} uses unsupported schema surface {}; expected {}",
            path.display(),
            schema.surface,
            RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE
        ));
    }
    if schema.purpose != RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE {
        return Err(format!(
            "runtime capability activation record {} uses unsupported schema purpose {}; expected {}",
            path.display(),
            schema.purpose,
            RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE
        ));
    }
    Ok(())
}
