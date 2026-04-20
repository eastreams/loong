use super::*;

pub(crate) fn execute_runtime_capability_rollback_managed_skill(
    options: RuntimeCapabilityRollbackCommandOptions,
    record_path: String,
    record: RuntimeCapabilityActivationRecordDocument,
) -> CliResult<RuntimeCapabilityRollbackReport> {
    let RuntimeCapabilityActivationRecordDocument {
        config_path,
        artifact_id,
        target,
        activation_surface,
        target_path,
        rollback,
        ..
    } = record;
    let rollback = match rollback {
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { previous_files } => previous_files,
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum { .. } => {
            return Err(
                "runtime capability rollback expected a managed skill activation record".to_owned(),
            );
        }
    };
    let target_path_buf = PathBuf::from(target_path.as_str());
    let current_files = collect_runtime_capability_bundle_files(target_path_buf.as_path())?;
    let already_rolled_back = current_files == rollback;
    let dry_run_verification = build_managed_skill_rollback_verification_hints(
        target_path_buf.as_path(),
        rollback.as_ref(),
    );

    if !options.apply {
        let note = if already_rolled_back {
            "managed skill already matches the recorded pre-activation state".to_owned()
        } else {
            "managed skill rollback would restore the recorded pre-activation state".to_owned()
        };
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path,
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: false,
            outcome: RuntimeCapabilityRollbackOutcome::DryRun,
            notes: vec![note],
            verification: dry_run_verification,
        });
    }

    if already_rolled_back {
        let verification = verify_managed_skill_rollback_state(
            artifact_id.as_str(),
            target_path_buf.as_path(),
            rollback.as_ref(),
        )?;
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path,
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: true,
            outcome: RuntimeCapabilityRollbackOutcome::AlreadyRolledBack,
            notes: vec![
                "managed skill already matches the recorded pre-activation state".to_owned(),
            ],
            verification,
        });
    }

    let config_override = options.config.unwrap_or(config_path);
    let (resolved_config_path, config) = mvp::config::load(Some(config_override.as_str()))?;
    let rollback_payload = RuntimeCapabilityRollbackPayload::ManagedSkillBundle {
        previous_files: rollback,
    };
    rollback_managed_skill_activation_state(
        resolved_config_path.as_path(),
        config,
        artifact_id.as_str(),
        target_path_buf.as_path(),
        rollback_payload.clone(),
    )?;
    let verification = verify_managed_skill_rollback_state(
        artifact_id.as_str(),
        target_path_buf.as_path(),
        match rollback_payload {
            RuntimeCapabilityRollbackPayload::ManagedSkillBundle { ref previous_files } => {
                previous_files.as_ref()
            }
            RuntimeCapabilityRollbackPayload::ProfileNoteAddendum { .. } => None,
        },
    )?;
    Ok(RuntimeCapabilityRollbackReport {
        generated_at: now_rfc3339()?,
        record_path,
        config_path: resolved_config_path.display().to_string(),
        artifact_id,
        target,
        activation_surface,
        target_path,
        apply_requested: true,
        outcome: RuntimeCapabilityRollbackOutcome::RolledBack,
        notes: vec!["managed skill rollback restored the recorded pre-activation state".to_owned()],
        verification,
    })
}

pub(crate) fn execute_runtime_capability_rollback_profile_note_addendum(
    options: RuntimeCapabilityRollbackCommandOptions,
    record_path: String,
    record: RuntimeCapabilityActivationRecordDocument,
) -> CliResult<RuntimeCapabilityRollbackReport> {
    let RuntimeCapabilityActivationRecordDocument {
        config_path,
        artifact_id,
        target,
        activation_surface,
        target_path,
        rollback,
        ..
    } = record;
    let rollback = match rollback {
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
            previous_profile,
            previous_profile_note,
        } => (previous_profile, previous_profile_note),
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { .. } => {
            return Err(
                "runtime capability rollback expected a profile note activation record".to_owned(),
            );
        }
    };
    let config_override = options.config.unwrap_or(config_path);
    let dry_run_verification = build_profile_note_rollback_verification_hints(
        Path::new(config_override.as_str()),
        rollback.0,
        rollback.1.as_deref(),
    );

    if !options.apply {
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path: config_override,
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: false,
            outcome: RuntimeCapabilityRollbackOutcome::DryRun,
            notes: vec![
                "profile note rollback would restore the recorded pre-activation memory state"
                    .to_owned(),
            ],
            verification: dry_run_verification,
        });
    }

    let (resolved_config_path, _) = mvp::config::load(Some(config_override.as_str()))?;
    let already_rolled_back = profile_note_state_matches(
        resolved_config_path.as_path(),
        rollback.0,
        rollback.1.as_deref(),
    )?;
    if already_rolled_back {
        let verification = verify_profile_note_rollback_state(
            resolved_config_path.as_path(),
            rollback.0,
            rollback.1.as_deref(),
        )?;
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path: resolved_config_path.display().to_string(),
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: true,
            outcome: RuntimeCapabilityRollbackOutcome::AlreadyRolledBack,
            notes: vec![
                "profile note already matches the recorded pre-activation memory state".to_owned(),
            ],
            verification,
        });
    }

    let rollback_payload = RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
        previous_profile: rollback.0,
        previous_profile_note: rollback.1.clone(),
    };
    rollback_profile_note_addendum_activation_state(
        resolved_config_path.as_path(),
        rollback.0,
        rollback_payload,
    )?;
    let verification = verify_profile_note_rollback_state(
        resolved_config_path.as_path(),
        rollback.0,
        rollback.1.as_deref(),
    )?;
    Ok(RuntimeCapabilityRollbackReport {
        generated_at: now_rfc3339()?,
        record_path,
        config_path: resolved_config_path.display().to_string(),
        artifact_id,
        target,
        activation_surface,
        target_path,
        apply_requested: true,
        outcome: RuntimeCapabilityRollbackOutcome::RolledBack,
        notes: vec![
            "profile note rollback restored the recorded pre-activation memory state".to_owned(),
        ],
        verification,
    })
}

pub(crate) fn rollback_managed_skill_activation_state(
    resolved_config_path: &Path,
    mut config: mvp::config::LoongConfig,
    artifact_id: &str,
    target_path: &Path,
    rollback: RuntimeCapabilityRollbackPayload,
) -> CliResult<()> {
    let previous_files = match rollback {
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { previous_files } => previous_files,
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum { .. } => {
            return Err(
                "runtime capability rollback expected a managed skill rollback payload".to_owned(),
            );
        }
    };
    config.external_skills.enabled = true;
    config.external_skills.install_root = target_path
        .parent()
        .map(|value| value.display().to_string());
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &config,
        Some(resolved_config_path),
    );

    match previous_files {
        Some(previous_files) => {
            let staging_base_root =
                resolve_runtime_capability_activation_staging_base_root(&tool_runtime)?;
            let staging_root = write_runtime_capability_draft_files_to_staging(
                &previous_files,
                staging_base_root.as_path(),
            )?;
            let staging_path = staging_root.display().to_string();
            let install_request = ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": staging_path,
                    "skill_id": artifact_id,
                    "replace": true,
                }),
            };
            let install_result =
                mvp::tools::execute_tool_core_with_config(install_request, &tool_runtime);
            let cleanup_result = fs::remove_dir_all(&staging_root);
            if let Err(error) = cleanup_result {
                return Err(format!(
                    "cleanup managed skill rollback staging root {} failed: {error}",
                    staging_root.display()
                ));
            }
            install_result.map_err(|error| {
                format!(
                    "restore previous managed skill `{artifact_id}` during rollback failed: {error}"
                )
            })?;
        }
        None => {
            let remove_request = ToolCoreRequest {
                tool_name: "external_skills.remove".to_owned(),
                payload: json!({
                    "skill_id": artifact_id,
                }),
            };
            mvp::tools::execute_tool_core_with_config(remove_request, &tool_runtime).map_err(
                |error| {
                    format!("remove managed skill `{artifact_id}` during rollback failed: {error}")
                },
            )?;
        }
    }
    Ok(())
}

pub(crate) fn rollback_profile_note_addendum_activation_state(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    rollback: RuntimeCapabilityRollbackPayload,
) -> CliResult<()> {
    let previous_profile_note = match rollback {
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
            previous_profile_note,
            ..
        } => previous_profile_note,
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { .. } => {
            return Err(
                "runtime capability rollback expected a profile note rollback payload".to_owned(),
            );
        }
    };
    let config_path_text = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_text.as_str()))?;
    let (_, mut config) = load_result;
    config.memory.profile = previous_profile;
    config.memory.profile_note = previous_profile_note;
    mvp::config::write(Some(config_path_text.as_str()), &config, true)?;
    Ok(())
}

fn build_managed_skill_rollback_verification_hints(
    target_path: &Path,
    previous_files: Option<&BTreeMap<String, String>>,
) -> Vec<String> {
    let target_display = target_path.display().to_string();
    match previous_files {
        Some(previous_files) => {
            let file_count = previous_files.len();
            let verification = format!(
                "verify {target_display} matches the recorded pre-activation managed skill bundle with {file_count} file(s)"
            );
            vec![verification]
        }
        None => {
            let verification = format!(
                "verify {target_display} is absent after rollback removes the managed skill"
            );
            vec![verification]
        }
    }
}

fn verify_managed_skill_rollback_state(
    artifact_id: &str,
    target_path: &Path,
    previous_files: Option<&BTreeMap<String, String>>,
) -> CliResult<Vec<String>> {
    match previous_files {
        Some(previous_files) => {
            let matches_payload =
                managed_skill_payload_matches_install_root(previous_files, target_path)?;
            if !matches_payload {
                return Err(format!(
                    "runtime capability rollback did not restore managed skill `{artifact_id}` to the recorded pre-activation bundle at {}",
                    target_path.display()
                ));
            }
            let file_count = previous_files.len();
            let verification = format!(
                "verified {} matches the recorded pre-activation managed skill bundle with {file_count} file(s)",
                target_path.display()
            );
            Ok(vec![verification])
        }
        None => {
            if target_path.exists() {
                return Err(format!(
                    "runtime capability rollback expected managed skill `{artifact_id}` to be removed from {}",
                    target_path.display()
                ));
            }
            let verification = format!(
                "verified {} is absent after rollback removed the managed skill",
                target_path.display()
            );
            Ok(vec![verification])
        }
    }
}

fn build_profile_note_rollback_verification_hints(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<&str>,
) -> Vec<String> {
    let config_display = config_path.display().to_string();
    let profile_hint = format!(
        "verify {config_display} restores memory.profile={} during rollback",
        render_memory_profile(previous_profile)
    );
    let note_hint = match previous_profile_note {
        Some(previous_profile_note) => {
            let char_count = previous_profile_note.chars().count();
            format!(
                "verify {config_display} restores the {char_count}-character pre-activation memory.profile_note"
            )
        }
        None => format!("verify {config_display} clears memory.profile_note during rollback"),
    };
    vec![profile_hint, note_hint]
}

fn profile_note_state_matches(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<&str>,
) -> CliResult<bool> {
    let config_path_text = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_text.as_str()))?;
    let (_, config) = load_result;
    if config.memory.profile != previous_profile {
        return Ok(false);
    }
    let current_profile_note = config.memory.profile_note.as_deref();
    Ok(current_profile_note == previous_profile_note)
}

fn verify_profile_note_rollback_state(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<&str>,
) -> CliResult<Vec<String>> {
    let matches = profile_note_state_matches(config_path, previous_profile, previous_profile_note)?;
    if !matches {
        return Err(format!(
            "runtime capability rollback expected {} to restore the recorded pre-activation memory state",
            config_path.display()
        ));
    }

    let config_display = config_path.display().to_string();
    let profile_verification = format!(
        "verified {config_display} restores memory.profile={}",
        render_memory_profile(previous_profile)
    );
    let note_verification = match previous_profile_note {
        Some(previous_profile_note) => {
            let char_count = previous_profile_note.chars().count();
            format!(
                "verified {config_display} restores the {char_count}-character pre-activation memory.profile_note"
            )
        }
        None => format!("verified {config_display} clears memory.profile_note during rollback"),
    };
    Ok(vec![profile_verification, note_verification])
}
