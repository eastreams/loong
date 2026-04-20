use super::rollback::{
    rollback_managed_skill_activation_state, rollback_profile_note_addendum_activation_state,
};
use super::*;

pub(crate) fn execute_runtime_capability_activate_managed_skill(
    options: RuntimeCapabilityActivateCommandOptions,
    artifact_path: String,
    applied_artifact: RuntimeCapabilityAppliedArtifactDocument,
) -> CliResult<RuntimeCapabilityActivateReport> {
    if options.replace && !options.apply {
        return Err("runtime capability activate --replace requires --apply".to_owned());
    }

    let RuntimeCapabilityAppliedArtifactDocument {
        artifact_id,
        target,
        delivery_surface,
        payload,
        rollback_hints,
        ..
    } = applied_artifact;
    let payload = match payload {
        RuntimeCapabilityDraftPayload::ManagedSkillBundle { files } => files,
        RuntimeCapabilityDraftPayload::ProgrammaticFlowSpec { .. }
        | RuntimeCapabilityDraftPayload::ProfileNoteAddendum { .. } => {
            return Err(
                "runtime capability activate expected a managed skill bundle payload".to_owned(),
            );
        }
    };

    let (resolved_config_path, config) = mvp::config::load(options.config.as_deref())?;
    let tool_runtime =
        build_runtime_capability_activation_tool_runtime(&resolved_config_path, &config, true);
    let install_root = resolve_runtime_capability_activation_install_root(&tool_runtime)?;
    let target_path = install_root.join(artifact_id.as_str());
    let previous_files = collect_runtime_capability_bundle_files(target_path.as_path())?;
    let already_matches =
        managed_skill_payload_matches_install_root(&payload, target_path.as_path())?;
    let dry_run_target_path = canonicalize_optional_path(target_path.as_path())?;
    let dry_run_verification =
        build_managed_skill_activation_verification_hints(target_path.as_path(), payload.len());

    if !options.apply {
        let notes = vec![
            "activation is dry-run by default".to_owned(),
            "managed skill activation reuses external_skills.install under a governed runtime config"
                .to_owned(),
        ];
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: resolved_config_path.display().to_string(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "external_skills.install".to_owned(),
            target_path: dry_run_target_path,
            apply_requested: false,
            replace_requested: options.replace,
            outcome: RuntimeCapabilityActivateOutcome::DryRun,
            notes,
            verification: dry_run_verification,
            rollback_hints,
            activation_record_path: None,
        });
    }

    if already_matches {
        let notes = vec!["managed skill already matches the applied draft payload".to_owned()];
        let verified_target_path = canonicalize_existing_path(target_path.as_path())?;
        let verification =
            verify_managed_skill_activation_state(&artifact_id, target_path.as_path(), &payload)?;
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: resolved_config_path.display().to_string(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "external_skills.install".to_owned(),
            target_path: verified_target_path,
            apply_requested: true,
            replace_requested: options.replace,
            outcome: RuntimeCapabilityActivateOutcome::AlreadyActivated,
            notes,
            verification,
            rollback_hints,
            activation_record_path: None,
        });
    }

    let staging_base_root = resolve_runtime_capability_activation_staging_base_root(&tool_runtime)?;
    let staging_root =
        write_runtime_capability_draft_files_to_staging(&payload, staging_base_root.as_path())?;
    let staging_path = staging_root.display().to_string();
    let install_payload = json!({
        "path": staging_path,
        "skill_id": artifact_id,
        "replace": options.replace,
    });
    let install_request = ToolCoreRequest {
        tool_name: "external_skills.install".to_owned(),
        payload: install_payload,
    };
    let install_result = mvp::tools::execute_tool_core_with_config(install_request, &tool_runtime);
    let cleanup_result = fs::remove_dir_all(&staging_root);
    if let Err(error) = cleanup_result {
        let cleanup_error = format!(
            "cleanup managed skill staging root {} failed: {error}",
            staging_root.display()
        );
        return Err(cleanup_error);
    }
    install_result
        .map_err(|error| format!("activate managed skill `{}` failed: {error}", artifact_id))?;
    let verification =
        verify_managed_skill_activation_state(&artifact_id, target_path.as_path(), &payload)?;
    let activated_target_path = canonicalize_existing_path(target_path.as_path())?;
    let activation_record = build_runtime_capability_managed_skill_activation_record(
        artifact_path.as_str(),
        resolved_config_path.as_path(),
        artifact_id.as_str(),
        target,
        delivery_surface.as_str(),
        "external_skills.install",
        activated_target_path.as_str(),
        &verification,
        &rollback_hints,
        previous_files,
    )?;
    let activation_record_path = build_runtime_capability_activation_record_path(
        Path::new(artifact_path.as_str()),
        artifact_id.as_str(),
    )?;
    if let Err(error) = persist_runtime_capability_activation_record(
        activation_record_path.as_path(),
        &activation_record,
    ) {
        let rollback_result = rollback_managed_skill_activation_state(
            resolved_config_path.as_path(),
            config,
            artifact_id.as_str(),
            target_path.as_path(),
            activation_record.rollback.clone(),
        );
        if let Err(rollback_error) = rollback_result {
            return Err(format!(
                "persist runtime capability activation record {} failed: {error}; managed skill rollback also failed: {rollback_error}",
                activation_record_path.display()
            ));
        }
        return Err(format!(
            "persist runtime capability activation record {} failed after reverting managed skill activation: {error}",
            activation_record_path.display()
        ));
    }
    let canonical_activation_record_path =
        canonicalize_existing_path(activation_record_path.as_path())?;

    let notes =
        vec!["managed skill installed into the governed external skills runtime".to_owned()];
    Ok(RuntimeCapabilityActivateReport {
        generated_at: now_rfc3339()?,
        artifact_path,
        config_path: resolved_config_path.display().to_string(),
        artifact_id,
        target,
        delivery_surface,
        activation_surface: "external_skills.install".to_owned(),
        target_path: activated_target_path,
        apply_requested: true,
        replace_requested: options.replace,
        outcome: RuntimeCapabilityActivateOutcome::Activated,
        notes,
        verification,
        rollback_hints,
        activation_record_path: Some(canonical_activation_record_path),
    })
}

pub(crate) fn execute_runtime_capability_activate_profile_note_addendum(
    options: RuntimeCapabilityActivateCommandOptions,
    artifact_path: String,
    applied_artifact: RuntimeCapabilityAppliedArtifactDocument,
) -> CliResult<RuntimeCapabilityActivateReport> {
    if options.replace {
        return Err(
            "runtime capability activate --replace is not supported for profile_note_addendum artifacts"
                .to_owned(),
        );
    }

    let RuntimeCapabilityAppliedArtifactDocument {
        artifact_id,
        target,
        delivery_surface,
        payload,
        rollback_hints,
        ..
    } = applied_artifact;
    let addendum = match payload {
        RuntimeCapabilityDraftPayload::ProfileNoteAddendum { content } => content,
        RuntimeCapabilityDraftPayload::ManagedSkillBundle { .. }
        | RuntimeCapabilityDraftPayload::ProgrammaticFlowSpec { .. } => {
            return Err(
                "runtime capability activate expected a profile note addendum payload".to_owned(),
            );
        }
    };

    let (resolved_config_path, mut config) = mvp::config::load(options.config.as_deref())?;
    let previous_profile = config.memory.profile;
    let previous_profile_note = config.memory.profile_note.clone();
    let merged_profile_note = mvp::migration::merge_profile_note_addendum(
        config.memory.profile_note.as_deref(),
        addendum.as_str(),
    );
    let canonical_config_path = canonicalize_optional_path(resolved_config_path.as_path())?;
    let dry_run_verification = build_profile_note_activation_verification_hints(
        resolved_config_path.as_path(),
        addendum.as_str(),
    );

    if !options.apply {
        let note = if merged_profile_note.is_some() {
            "profile note activation would append the advisory addendum".to_owned()
        } else {
            "profile note already contains the advisory addendum".to_owned()
        };
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: canonical_config_path.clone(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "config.memory.profile_note".to_owned(),
            target_path: canonical_config_path,
            apply_requested: false,
            replace_requested: false,
            outcome: RuntimeCapabilityActivateOutcome::DryRun,
            notes: vec![note],
            verification: dry_run_verification,
            rollback_hints,
            activation_record_path: None,
        });
    }

    let Some(merged_profile_note) = merged_profile_note else {
        let verification = verify_profile_note_addendum_activation_state(
            resolved_config_path.as_path(),
            addendum.as_str(),
        )?;
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: canonical_config_path.clone(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "config.memory.profile_note".to_owned(),
            target_path: canonical_config_path,
            apply_requested: true,
            replace_requested: false,
            outcome: RuntimeCapabilityActivateOutcome::AlreadyActivated,
            notes: vec!["profile note already contains the advisory addendum".to_owned()],
            verification,
            rollback_hints,
            activation_record_path: None,
        });
    };

    config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
    config.memory.profile_note = Some(merged_profile_note);
    let resolved_config_path_string = resolved_config_path.display().to_string();
    mvp::config::write(Some(resolved_config_path_string.as_str()), &config, true)?;
    let verification = verify_profile_note_addendum_activation_state(
        resolved_config_path.as_path(),
        addendum.as_str(),
    )?;
    let canonical_record_target_path = canonical_config_path.clone();
    let activation_record = build_runtime_capability_profile_note_activation_record(
        artifact_path.as_str(),
        resolved_config_path.as_path(),
        artifact_id.as_str(),
        target,
        delivery_surface.as_str(),
        "config.memory.profile_note",
        canonical_record_target_path.as_str(),
        &verification,
        &rollback_hints,
        previous_profile,
        previous_profile_note,
    )?;
    let activation_record_path = build_runtime_capability_activation_record_path(
        Path::new(artifact_path.as_str()),
        artifact_id.as_str(),
    )?;
    if let Err(error) = persist_runtime_capability_activation_record(
        activation_record_path.as_path(),
        &activation_record,
    ) {
        let rollback_result = rollback_profile_note_addendum_activation_state(
            resolved_config_path.as_path(),
            previous_profile,
            activation_record.rollback.clone(),
        );
        if let Err(rollback_error) = rollback_result {
            return Err(format!(
                "persist runtime capability activation record {} failed: {error}; profile note rollback also failed: {rollback_error}",
                activation_record_path.display()
            ));
        }
        return Err(format!(
            "persist runtime capability activation record {} failed after reverting profile note activation: {error}",
            activation_record_path.display()
        ));
    }
    let canonical_activation_record_path =
        canonicalize_existing_path(activation_record_path.as_path())?;

    Ok(RuntimeCapabilityActivateReport {
        generated_at: now_rfc3339()?,
        artifact_path,
        config_path: canonical_config_path.clone(),
        artifact_id,
        target,
        delivery_surface,
        activation_surface: "config.memory.profile_note".to_owned(),
        target_path: canonical_config_path,
        apply_requested: true,
        replace_requested: false,
        outcome: RuntimeCapabilityActivateOutcome::Activated,
        notes: vec![
            "profile_note_addendum activation also enforces profile_plus_window memory mode"
                .to_owned(),
        ],
        verification,
        rollback_hints,
        activation_record_path: Some(canonical_activation_record_path),
    })
}

fn build_runtime_capability_activation_tool_runtime(
    resolved_config_path: &Path,
    config: &mvp::config::LoongConfig,
    external_skills_enabled: bool,
) -> mvp::tools::runtime_config::ToolRuntimeConfig {
    let mut adjusted_config = config.clone();
    adjusted_config.external_skills.enabled = external_skills_enabled;
    mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &adjusted_config,
        Some(resolved_config_path),
    )
}

fn resolve_runtime_capability_activation_install_root(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> CliResult<PathBuf> {
    if let Some(path) = tool_runtime.external_skills.install_root.clone() {
        return Ok(path);
    }

    let file_root = match tool_runtime.file_root.clone() {
        Some(path) => path,
        None => std::env::current_dir().map_err(|error| {
            format!("read current dir for managed skill activation failed: {error}")
        })?,
    };
    Ok(file_root.join("external-skills-installed"))
}
