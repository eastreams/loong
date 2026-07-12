use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use loong_kernel::access::fs::FsWriteOptions;
use serde_json::{Value, json};

use crate::{
    config::{self, LoongConfig, MemoryProfile},
    migration::{self, LegacyClawSource},
};

const DEFAULT_MODE: &str = "plan";
pub(super) const CONFIG_IMPORT_TOOL_NAME: &str = "config.import";
const SUPPORTED_SOURCES: &str = "auto, nanobot, openclaw, picoclaw, zeroclaw, nanoclaw";
const MAP_SKILLS_MODE_KEY: &str = "map_skills";
const APPLY_SKILLS_PLAN_KEY: &str = "apply_skills_plan";
const SKILLS_MANIFEST_PATH_KEY: &str = "skills_manifest_path";

pub(super) fn config_import_mode(payload: &serde_json::Map<String, Value>) -> &str {
    let raw_mode = payload.get("mode");
    let raw_mode = raw_mode.and_then(Value::as_str);
    let trimmed_mode = raw_mode.map(str::trim);
    let mode = trimmed_mode.filter(|value| !value.is_empty());

    mode.unwrap_or(DEFAULT_MODE)
}

pub(super) fn config_import_mode_requires_write_object(
    payload: &serde_json::Map<String, Value>,
) -> bool {
    let mode = config_import_mode(payload);

    matches!(mode, "apply" | "apply_selected" | "rollback_last_apply")
}

pub(super) fn config_import_mode_requires_write_value(payload: &Value) -> bool {
    let payload = payload.as_object();
    let Some(payload) = payload else {
        return false;
    };

    config_import_mode_requires_write_object(payload)
}

// Only these modes are safe to route through the context-aware access path
// today. `apply` writes the output config through fs access; apply_selected
// still needs governed backup/manifest/skills-bridge work before joining.
pub(super) fn config_import_mode_is_context_access_backed(mode: &str) -> bool {
    matches!(
        mode,
        "plan"
            | "discover"
            | "plan_many"
            | "recommend_primary"
            | "merge_profiles"
            | MAP_SKILLS_MODE_KEY
            | "apply"
            | "rollback_last_apply"
    )
}

pub(super) fn config_import_payload_is_context_access_backed(
    payload: &serde_json::Map<String, Value>,
) -> bool {
    let mode = config_import_mode(payload);
    if mode == "apply_selected" {
        return !payload
            .get(APPLY_SKILLS_PLAN_KEY)
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    config_import_mode_is_context_access_backed(mode)
}

pub(super) fn execute_config_import_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    // TODO(config-import-access): keep this legacy path out of the typed
    // ToolPlane until migration filesystem I/O is supplied by ctx.access().
    // Registering this whole function as a typed tool would only hide the
    // remaining direct apply_selected I/O behind a new name.
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| format!("{CONFIG_IMPORT_TOOL_NAME} payload must be an object"))?;
    let mode = config_import_mode(payload);
    if !matches!(
        mode,
        "plan"
            | "apply"
            | "apply_selected"
            | "discover"
            | "plan_many"
            | "recommend_primary"
            | "merge_profiles"
            | MAP_SKILLS_MODE_KEY
            | "rollback_last_apply"
    ) {
        return Err(format!(
            "{CONFIG_IMPORT_TOOL_NAME} payload.mode must be `plan`, `apply`, `apply_selected`, `discover`, `plan_many`, `recommend_primary`, `merge_profiles`, `map_skills`, or `rollback_last_apply`, got `{mode}`"
        ));
    }

    let output_path = payload
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolve_safe_path_with_config(value, config))
        .transpose()?;

    let mode_requires_write = config_import_mode_requires_write_object(payload);
    if mode_requires_write && output_path.is_none() {
        return Err(format!(
            "{CONFIG_IMPORT_TOOL_NAME} {mode} mode requires payload.output_path"
        ));
    }

    let input_path = if mode == "rollback_last_apply" {
        None
    } else {
        Some(
            payload
                .get("input_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{CONFIG_IMPORT_TOOL_NAME} requires payload.input_path"))
                .and_then(|value| resolve_safe_path_with_config(value, config))?,
        )
    };
    let input_path = input_path.as_ref();

    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let hint = payload
        .get("source")
        .and_then(Value::as_str)
        .map(parse_source_hint)
        .transpose()?
        .flatten();

    if mode == "rollback_last_apply" {
        let output_path = output_path.ok_or_else(|| {
            format!(
                "{CONFIG_IMPORT_TOOL_NAME} rollback_last_apply mode requires payload.output_path"
            )
        })?;
        let restored_path = migration::rollback_last_migration(&output_path)?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "rollback_last_apply",
                "output_path": restored_path.display().to_string(),
                "rolled_back": true,
            }),
        });
    }

    let input_path = input_path
        .ok_or_else(|| format!("{CONFIG_IMPORT_TOOL_NAME} requires payload.input_path"))?;

    if mode == "discover" {
        let report = migration::discover_import_sources(
            input_path.as_path(),
            migration::DiscoveryOptions::default(),
        )?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "discover",
                "input_path": input_path.display().to_string(),
                "sources": report
                    .sources
                    .iter()
                    .map(discovered_source_payload)
                    .collect::<Vec<_>>(),
            }),
        });
    }

    if matches!(mode, "plan_many" | "recommend_primary") {
        let report = migration::discover_import_sources(
            input_path.as_path(),
            migration::DiscoveryOptions::default(),
        )?;
        let summary = migration::plan_import_sources(&report)?;
        let recommendation = migration::recommend_primary_source(&summary).ok();
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": mode,
                "input_path": input_path.display().to_string(),
                "plans": summary.plans.iter().map(planned_source_payload).collect::<Vec<_>>(),
                "recommendation": recommendation.as_ref().map(primary_recommendation_payload),
            }),
        });
    }

    if mode == "merge_profiles" {
        let report = migration::discover_import_sources(
            input_path.as_path(),
            migration::DiscoveryOptions::default(),
        )?;
        let summary = migration::plan_import_sources(&report)?;
        let recommendation = migration::recommend_primary_source(&summary).ok();
        let merged = migration::merge_profile_sources(&report)?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "merge_profiles",
                "input_path": input_path.display().to_string(),
                "plans": summary.plans.iter().map(planned_source_payload).collect::<Vec<_>>(),
                "recommendation": recommendation.as_ref().map(primary_recommendation_payload),
                "result": merged_profile_plan_payload(&merged),
            }),
        });
    }

    if mode == MAP_SKILLS_MODE_KEY {
        let mapping = migration::plan_external_skill_mapping(input_path.as_path());
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": MAP_SKILLS_MODE_KEY,
                "input_path": input_path.display().to_string(),
                "result": external_skill_mapping_plan_payload(&mapping),
            }),
        });
    }

    if mode == "apply_selected" {
        let report =
            migration::discover_import_sources(input_path, migration::DiscoveryOptions::default())?;
        let summary = migration::plan_import_sources(&report)?;
        let selection = parse_apply_selection_mode(payload, &summary)?;
        let apply_skills_plan = payload
            .get(APPLY_SKILLS_PLAN_KEY)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let selected_output_path = output_path.ok_or_else(|| {
            format!("{CONFIG_IMPORT_TOOL_NAME} apply_selected mode requires payload.output_path")
        })?;
        let result = migration::apply_import_selection(&migration::ApplyImportSelection {
            discovery: report,
            output_path: selected_output_path,
            mode: selection,
            apply_skills_plan,
            skills_input_path: if apply_skills_plan {
                Some(input_path.to_path_buf())
            } else {
                None
            },
        })?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "apply_selected",
                "input_path": input_path.display().to_string(),
                "output_path": result.output_path.display().to_string(),
                APPLY_SKILLS_PLAN_KEY: apply_skills_plan,
                "result": apply_selection_result_payload(&result),
            }),
        });
    }

    let plan = migration::plan_import_from_path(input_path.as_path(), hint)?;

    let mut merged_config = load_or_default_config(output_path.as_deref())?;
    migration::apply_import_plan(&mut merged_config, &plan);
    let config_toml = config::render(&merged_config)?;

    let written_output_path = if mode == "apply" {
        let output_path = output_path.clone().ok_or_else(|| {
            format!("{CONFIG_IMPORT_TOOL_NAME} apply mode requires payload.output_path")
        })?;
        let output_string = output_path.display().to_string();
        Some(config::write(Some(&output_string), &merged_config, force)?)
    } else {
        None
    };
    let response_output_path = written_output_path
        .as_ref()
        .or(output_path.as_ref())
        .map(|path| path.display().to_string());

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "mode": mode,
            "source": plan.source.as_id(),
            "input_path": input_path.display().to_string(),
            "output_path": response_output_path,
            "config_written": mode == "apply",
            "warnings": plan.warnings,
            "config_preview": config_preview_payload(&merged_config),
            "config_toml": config_toml,
            "next_step": written_output_path
                .as_ref()
                .map(|path| {
                    format!(
                        "{} chat --config {}",
                        config::active_cli_command_name(),
                        path.display()
                    )
                }),
        }),
    })
}

pub(super) async fn execute_config_import_tool_with_context(
    request: ToolCoreRequest,
    ctx: &crate::context::AppExecutionContext<'_>,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| format!("{CONFIG_IMPORT_TOOL_NAME} payload must be an object"))?;
    let mode = config_import_mode(payload);
    if !config_import_payload_is_context_access_backed(payload) {
        return Err(format!(
            "{CONFIG_IMPORT_TOOL_NAME} context-aware access path does not support `{mode}` yet"
        ));
    }

    let output_path = payload
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let output_path = match output_path {
        Some(path) => Some(resolve_path_with_access(ctx, path.as_path()).await?),
        None => None,
    };

    if mode == "rollback_last_apply" {
        let output_path = output_path.ok_or_else(|| {
            format!(
                "{CONFIG_IMPORT_TOOL_NAME} rollback_last_apply mode requires payload.output_path"
            )
        })?;
        let restored_path =
            migration::rollback_last_migration_with_access(ctx, output_path.as_path()).await?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "rollback_last_apply",
                "output_path": restored_path.display().to_string(),
                "rolled_back": true,
            }),
        });
    }

    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{CONFIG_IMPORT_TOOL_NAME} requires payload.input_path"))?;
    let input_path = resolve_path_with_access(ctx, Path::new(input_path)).await?;
    let hint = payload
        .get("source")
        .and_then(Value::as_str)
        .map(parse_source_hint)
        .transpose()?
        .flatten();

    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if mode == "discover" {
        let report = migration::discover_import_sources_with_access(
            ctx,
            input_path.as_path(),
            migration::DiscoveryOptions::default(),
        )
        .await?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "discover",
                "input_path": input_path.display().to_string(),
                "sources": report
                    .sources
                    .iter()
                    .map(discovered_source_payload)
                    .collect::<Vec<_>>(),
            }),
        });
    }

    if matches!(mode, "plan_many" | "recommend_primary") {
        let report = migration::discover_import_sources_with_access(
            ctx,
            input_path.as_path(),
            migration::DiscoveryOptions::default(),
        )
        .await?;
        let summary = migration::plan_import_sources_with_access(ctx, &report).await?;
        let recommendation = migration::recommend_primary_source(&summary).ok();
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": mode,
                "input_path": input_path.display().to_string(),
                "plans": summary.plans.iter().map(planned_source_payload).collect::<Vec<_>>(),
                "recommendation": recommendation.as_ref().map(primary_recommendation_payload),
            }),
        });
    }

    if mode == "merge_profiles" {
        let report = migration::discover_import_sources_with_access(
            ctx,
            input_path.as_path(),
            migration::DiscoveryOptions::default(),
        )
        .await?;
        let summary = migration::plan_import_sources_with_access(ctx, &report).await?;
        let recommendation = migration::recommend_primary_source(&summary).ok();
        let merged = migration::merge_profile_sources_with_access(ctx, &report).await?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "merge_profiles",
                "input_path": input_path.display().to_string(),
                "plans": summary.plans.iter().map(planned_source_payload).collect::<Vec<_>>(),
                "recommendation": recommendation.as_ref().map(primary_recommendation_payload),
                "result": merged_profile_plan_payload(&merged),
            }),
        });
    }

    if mode == MAP_SKILLS_MODE_KEY {
        let mapping =
            migration::plan_external_skill_mapping_with_access(ctx, input_path.as_path()).await?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": MAP_SKILLS_MODE_KEY,
                "input_path": input_path.display().to_string(),
                "result": external_skill_mapping_plan_payload(&mapping),
            }),
        });
    }

    if mode == "apply_selected" {
        let report = migration::discover_import_sources_with_access(
            ctx,
            input_path.as_path(),
            migration::DiscoveryOptions::default(),
        )
        .await?;
        let summary = migration::plan_import_sources_with_access(ctx, &report).await?;
        let selection = parse_apply_selection_mode(payload, &summary)?;
        let output_path = output_path.ok_or_else(|| {
            format!("{CONFIG_IMPORT_TOOL_NAME} apply_selected mode requires payload.output_path")
        })?;
        let result = migration::apply_import_selection_with_access(
            ctx,
            &migration::ApplyImportSelection {
                discovery: report,
                output_path,
                mode: selection,
                apply_skills_plan: false,
                skills_input_path: None,
            },
        )
        .await?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "apply_selected",
                "input_path": input_path.display().to_string(),
                "output_path": result.output_path.display().to_string(),
                APPLY_SKILLS_PLAN_KEY: false,
                "result": apply_selection_result_payload(&result),
            }),
        });
    }

    if mode == "apply" {
        let output_path = output_path.ok_or_else(|| {
            format!("{CONFIG_IMPORT_TOOL_NAME} apply mode requires payload.output_path")
        })?;
        let plan =
            migration::plan_import_from_path_with_access(ctx, input_path.as_path(), hint).await?;
        let mut merged_config =
            load_or_default_config_with_access(ctx, Some(output_path.as_path())).await?;
        migration::apply_import_plan(&mut merged_config, &plan);
        let config_toml = config::render(&merged_config)?;
        let written = ctx
            .access()
            .fs()
            .write_file(
                output_path.as_path(),
                config_toml.clone().into_bytes(),
                FsWriteOptions {
                    create_dirs: true,
                    overwrite: force,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "apply",
                "source": plan.source.as_id(),
                "input_path": input_path.display().to_string(),
                "output_path": written.path.display().to_string(),
                "config_written": true,
                "warnings": plan.warnings,
                "config_preview": config_preview_payload(&merged_config),
                "config_toml": config_toml,
                "next_step": format!(
                    "{} chat --config {}",
                    config::active_cli_command_name(),
                    written.path.display()
                ),
            }),
        });
    }

    let plan =
        migration::plan_import_from_path_with_access(ctx, input_path.as_path(), hint).await?;
    let mut merged_config = load_or_default_config_with_access(ctx, output_path.as_deref()).await?;
    migration::apply_import_plan(&mut merged_config, &plan);
    let config_toml = config::render(&merged_config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "mode": mode,
            "source": plan.source.as_id(),
            "input_path": input_path.display().to_string(),
            "output_path": output_path.as_ref().map(|path| path.display().to_string()),
            "config_written": false,
            "warnings": plan.warnings,
            "config_preview": config_preview_payload(&merged_config),
            "config_toml": config_toml,
            "next_step": null,
        }),
    })
}

fn discovered_source_payload(source: &migration::DiscoveredImportSource) -> Value {
    json!({
        "source_id": source.source_id,
        "source_kind": source.source.as_id(),
        "input_path": source.path.display().to_string(),
        "confidence_score": source.confidence_score,
        "found_files": source.found_files,
    })
}

fn planned_source_payload(plan: &migration::PlannedImportSource) -> Value {
    json!({
        "source_id": plan.source_id,
        "source_kind": plan.source.as_id(),
        "input_path": plan.input_path.display().to_string(),
        "confidence_score": plan.confidence_score,
        "prompt_addendum_present": plan.prompt_addendum_present,
        "profile_note_present": plan.profile_note_present,
        "warning_count": plan.warning_count,
    })
}

fn primary_recommendation_payload(
    recommendation: &migration::PrimarySourceRecommendation,
) -> Value {
    json!({
        "source_id": recommendation.source_id,
        "source_kind": recommendation.source.as_id(),
        "input_path": recommendation.input_path.display().to_string(),
        "reasons": recommendation.reasons,
    })
}

fn merged_profile_plan_payload(plan: &migration::MergedProfilePlan) -> Value {
    json!({
        "prompt_owner_source_id": plan.prompt_owner_source_id,
        "merged_profile_note": plan.merged_profile_note,
        "auto_apply_allowed": plan.auto_apply_allowed,
        "kept_entries": plan
            .kept_entries
            .iter()
            .map(|entry| {
                json!({
                    "lane": match entry.lane {
                        migration::ProfileEntryLane::Prompt => "prompt",
                        migration::ProfileEntryLane::Profile => "profile",
                    },
                    "canonical_text": entry.canonical_text,
                    "source_id": entry.source_id,
                    "slot_key": entry.slot_key,
                })
            })
            .collect::<Vec<_>>(),
        "dropped_duplicates": plan
            .dropped_duplicates
            .iter()
            .map(|entry| {
                json!({
                    "lane": match entry.lane {
                        migration::ProfileEntryLane::Prompt => "prompt",
                        migration::ProfileEntryLane::Profile => "profile",
                    },
                    "canonical_text": entry.canonical_text,
                    "source_id": entry.source_id,
                    "slot_key": entry.slot_key,
                })
            })
            .collect::<Vec<_>>(),
        "unresolved_conflicts": plan
            .unresolved_conflicts
            .iter()
            .map(|conflict| {
                json!({
                    "slot_key": conflict.slot_key,
                    "preferred_source_id": conflict.preferred_source_id,
                    "discarded_source_id": conflict.discarded_source_id,
                    "preferred_text": conflict.preferred_text,
                    "discarded_text": conflict.discarded_text,
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn external_skill_mapping_plan_payload(plan: &migration::ExternalSkillMappingPlan) -> Value {
    json!({
        "input_path": plan.input_path.display().to_string(),
        "artifact_count": plan.artifacts.len(),
        "artifacts": plan
            .artifacts
            .iter()
            .map(|artifact| {
                json!({
                    "kind": artifact.kind.as_id(),
                    "path": artifact.path.display().to_string(),
                })
            })
            .collect::<Vec<_>>(),
        "declared_skills": plan.declared_skills,
        "locked_skills": plan.locked_skills,
        "resolved_skills": plan.resolved_skills,
        "profile_note_addendum": plan.profile_note_addendum,
        "warnings": plan.warnings,
    })
}

fn apply_selection_result_payload(result: &migration::ApplyImportSelectionResult) -> Value {
    json!({
        "output_path": result.output_path.display().to_string(),
        "backup_path": result.backup_path.display().to_string(),
        "manifest_path": result.manifest_path.display().to_string(),
        SKILLS_MANIFEST_PATH_KEY: result
            .skills_manifest_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "selected_primary_source_id": result.selected_primary_source_id,
        "merged_source_ids": result.merged_source_ids,
        "prompt_owner_source_id": result.prompt_owner_source_id,
        "unresolved_conflicts": result.unresolved_conflicts,
        "external_skill_artifact_count": result.external_skill_artifact_count,
        "external_skill_entries_applied": result.external_skill_entries_applied,
        "external_skill_managed_install_count": result.external_skill_managed_install_count,
        "external_skill_managed_skill_ids": result.external_skill_managed_skill_ids,
        "warnings": result.warnings,
    })
}

fn parse_source_hint(raw: &str) -> Result<Option<LegacyClawSource>, String> {
    let parsed = LegacyClawSource::from_id(raw).ok_or_else(|| {
        format!(
            "unsupported {CONFIG_IMPORT_TOOL_NAME} payload.source `{raw}`. supported: {SUPPORTED_SOURCES}"
        )
    })?;
    if matches!(parsed, LegacyClawSource::Unknown) {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

fn parse_apply_selection_mode(
    payload: &serde_json::Map<String, Value>,
    summary: &migration::DiscoveryPlanSummary,
) -> Result<migration::ImportSelectionMode, String> {
    if payload
        .get("safe_profile_merge")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let primary_source_id = payload
            .get("primary_selection_id")
            .or_else(|| payload.get("selection_id"))
            .or_else(|| payload.get("primary_source_id"))
            .or_else(|| payload.get("source_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                migration::recommend_primary_source(summary)
                    .ok()
                    .map(|recommendation| recommendation.source_id)
            })
            .ok_or_else(|| {
                "apply_selected requires a primary source for safe profile merge".to_owned()
            })?;
        return Ok(migration::ImportSelectionMode::SafeProfileMerge { primary_source_id });
    }

    if let Some(source_id) = payload
        .get("selection_id")
        .or_else(|| payload.get("source_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(migration::ImportSelectionMode::SelectedSingleSource {
            source_id: source_id.to_owned(),
        });
    }

    let recommendation = migration::recommend_primary_source(summary)
        .map_err(|error| format!("apply_selected could not recommend a primary source: {error}"))?;
    Ok(migration::ImportSelectionMode::RecommendedSingleSource {
        source_id: recommendation.source_id,
    })
}

fn load_or_default_config(path: Option<&Path>) -> Result<LoongConfig, String> {
    let Some(path) = path else {
        return Ok(LoongConfig::default());
    };
    if !path.exists() {
        return Ok(LoongConfig::default());
    }
    let path_string = path.display().to_string();
    let (_, config) = config::load(Some(&path_string))?;
    Ok(config)
}

async fn load_or_default_config_with_access(
    ctx: &crate::context::AppExecutionContext<'_>,
    path: Option<&Path>,
) -> Result<LoongConfig, String> {
    let Some(path) = path else {
        return Ok(LoongConfig::default());
    };
    let inspection = ctx
        .access()
        .fs()
        .inspect_path(path)
        .await
        .map_err(|error| error.to_string())?;
    if inspection.kind.is_none() {
        return Ok(LoongConfig::default());
    }

    let output = ctx
        .access()
        .fs()
        .read_file(path)
        .await
        .map_err(|error| error.to_string())?;
    let raw = String::from_utf8(output.bytes).map_err(|error| {
        format!(
            "failed to decode config {} as UTF-8: {error}",
            output.path.display()
        )
    })?;
    config::parse(raw.as_str())
}

// Use governed inspect for response path normalization too; otherwise the
// context-aware plan path would report raw payload paths while reads use access.
async fn resolve_path_with_access(
    ctx: &crate::context::AppExecutionContext<'_>,
    path: &Path,
) -> Result<PathBuf, String> {
    ctx.access()
        .fs()
        .inspect_path(path)
        .await
        .map(|output| output.path)
        .map_err(|error| error.to_string())
}

fn config_preview_payload(config: &LoongConfig) -> Value {
    json!({
        "prompt_pack_id": config
            .cli
            .prompt_pack_id()
            .unwrap_or(crate::prompt::DEFAULT_PROMPT_PACK_ID),
        "memory_profile": memory_profile_id(config.memory.profile),
        "system_prompt_addendum": config.cli.system_prompt_addendum.clone(),
        "profile_note": config.memory.profile_note.clone(),
    })
}

fn memory_profile_id(profile: MemoryProfile) -> &'static str {
    match profile {
        MemoryProfile::WindowOnly => "window_only",
        MemoryProfile::WindowPlusSummary => "window_plus_summary",
        MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

fn resolve_safe_path_with_config(
    raw: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    if config.file_root.is_none() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let candidate = Path::new(raw);
        let combined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        return canonicalize_or_fallback(combined);
    }

    let Some(root) = config.file_root.clone() else {
        return Err("configured file root was missing during safe path resolution".to_owned());
    };
    let root = canonicalize_or_fallback(root)?;

    let candidate = Path::new(raw);
    let combined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let normalized = super::normalize_without_fs(&combined);
    resolve_path_within_root(&root, &normalized)
}

fn canonicalize_or_fallback(path: PathBuf) -> Result<PathBuf, String> {
    if path.exists() {
        let canonical = dunce::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()));
        let canonical = canonical.map(|resolved| dunce::simplified(&resolved).to_path_buf())?;
        return Ok(canonical);
    }
    Ok(super::normalize_without_fs(&path))
}

fn resolve_path_within_root(root: &Path, normalized: &Path) -> Result<PathBuf, String> {
    ensure_path_within_root(root, normalized)?;

    if normalized.exists() {
        let canonical = dunce::canonicalize(normalized).map_err(|error| {
            format!(
                "failed to canonicalize target path {}: {error}",
                normalized.display()
            )
        })?;
        let canonical = dunce::simplified(&canonical).to_path_buf();
        ensure_path_within_root(root, &canonical)?;
        return Ok(canonical);
    }

    let (ancestor, suffix) = split_existing_ancestor(normalized)?;
    let canonical_ancestor = dunce::canonicalize(&ancestor).map_err(|error| {
        format!(
            "failed to canonicalize ancestor {}: {error}",
            ancestor.display()
        )
    })?;
    let canonical_ancestor = dunce::simplified(&canonical_ancestor).to_path_buf();
    ensure_path_within_root(root, &canonical_ancestor)?;

    let mut reconstructed = canonical_ancestor;
    for component in suffix {
        reconstructed.push(component);
    }
    ensure_path_within_root(root, &reconstructed)?;
    Ok(reconstructed)
}

fn ensure_path_within_root(root: &Path, path: &Path) -> Result<(), String> {
    let normalized_root = dunce::simplified(root);
    let normalized_path = dunce::simplified(path);
    if normalized_path.starts_with(normalized_root) {
        return Ok(());
    }
    Err(format!(
        "policy_denied: migration path {} escapes configured file root {}",
        path.display(),
        root.display()
    ))
}

fn split_existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<OsString>), String> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();

    loop {
        if cursor.exists() {
            suffix.reverse();
            return Ok((cursor, suffix));
        }

        let Some(name) = cursor.file_name().map(|value| value.to_owned()) else {
            return Err(format!(
                "cannot resolve existing ancestor for {}",
                path.display()
            ));
        };
        suffix.push(name);
        let Some(parent) = cursor.parent() else {
            return Err(format!(
                "cannot resolve existing ancestor for {}",
                path.display()
            ));
        };
        cursor = parent.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use loong_contracts::{Capability, ToolCoreRequest};
    use serde_json::json;

    use super::*;
    use crate::test_support::TurnTestHarness;
    use crate::tools::runtime_config::ToolRuntimeConfig;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    #[test]
    fn resolve_safe_path_rejects_root_escape_with_policy_prefix() {
        let base = unique_temp_dir("loong-config-import");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let error = resolve_safe_path_with_config("../outside.toml", &config)
            .expect_err("escape should be denied");

        assert!(error.starts_with("policy_denied: "));
        assert!(error.contains("escapes configured file root"));
        let _ = fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn kernel_routed_config_import_plan_uses_access_backed_reader() {
        let harness = TurnTestHarness::new();
        fs::write(
            harness.temp_dir.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        )
        .expect("write prompt fixture");
        fs::write(
            harness.temp_dir.join("IDENTITY.md"),
            "# Identity\n\n- role: release copilot\n",
        )
        .expect("write profile fixture");

        let outcome = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "plan",
                    "source": "openclaw",
                    "input_path": "."
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect("config.import plan should execute through kernel context");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "plan");
        assert_eq!(outcome.payload["source"], "openclaw");
        assert_eq!(outcome.payload["config_written"], false);
        assert!(
            outcome.payload["config_preview"]["profile_note"]
                .as_str()
                .is_some_and(|note| note.contains("release copilot"))
        );
    }

    #[tokio::test]
    async fn kernel_routed_config_import_plan_requires_filesystem_read_capability() {
        let harness = TurnTestHarness::with_capabilities(BTreeSet::from([Capability::InvokeTool]));
        fs::write(
            harness.temp_dir.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers.\n",
        )
        .expect("write prompt fixture");

        let error = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "plan",
                    "source": "openclaw",
                    "input_path": "."
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect_err("missing read capability should deny config.import plan");

        assert!(
            error.contains("FilesystemRead") || error.contains("filesystem_read"),
            "unexpected denial: {error}"
        );
    }

    #[tokio::test]
    async fn kernel_routed_config_import_map_skills_uses_access_backed_reader() {
        let harness = TurnTestHarness::new();
        fs::write(
            harness.temp_dir.join("SKILLS.md"),
            "# Skills\n\n- custom/skill-a\n",
        )
        .expect("write skills catalog");
        fs::create_dir_all(harness.temp_dir.join(".codex/skills/release-guard"))
            .expect("create skill dir");

        let outcome = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "map_skills",
                    "input_path": "."
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect("config.import map_skills should execute through kernel context");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], MAP_SKILLS_MODE_KEY);
        assert_eq!(outcome.payload["result"]["artifact_count"], 2);
        assert_eq!(
            outcome.payload["result"]["resolved_skills"],
            json!(["custom/skill-a", "release-guard"])
        );
    }

    #[tokio::test]
    async fn kernel_routed_config_import_apply_writes_through_access() {
        let harness = TurnTestHarness::new();
        fs::write(
            harness.temp_dir.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        )
        .expect("write prompt fixture");
        let output_path = harness.temp_dir.join("generated/loong.toml");

        let outcome = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "apply",
                    "source": "openclaw",
                    "input_path": ".",
                    "output_path": "generated/loong.toml",
                    "force": true
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect("config.import apply should execute through kernel context");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply");
        assert_eq!(outcome.payload["config_written"], true);
        let expected_output_path =
            dunce::canonicalize(&output_path).expect("canonicalize generated config path");
        assert_eq!(
            outcome.payload["output_path"],
            expected_output_path.display().to_string()
        );
        let raw = fs::read_to_string(output_path).expect("read generated config");
        assert!(raw.contains("prompt_pack_id = \"loong-core-v1\""));
        let loaded = config::parse(raw.as_str()).expect("parse generated config");
        assert_eq!(
            loaded.cli.prompt_pack_id(),
            Some(crate::prompt::DEFAULT_PROMPT_PACK_ID)
        );
        assert!(
            loaded
                .cli
                .system_prompt_addendum
                .as_deref()
                .is_some_and(|addendum| addendum.contains("Loong style concise")),
            "unexpected generated config: {raw}"
        );
    }

    #[tokio::test]
    async fn kernel_routed_config_import_apply_requires_filesystem_write_capability() {
        let harness = TurnTestHarness::with_capabilities(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        fs::write(
            harness.temp_dir.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers.\n",
        )
        .expect("write prompt fixture");

        let error = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "apply",
                    "source": "openclaw",
                    "input_path": ".",
                    "output_path": "generated/loong.toml",
                    "force": true
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect_err("missing write capability should deny config.import apply");

        assert!(
            error.contains("FilesystemWrite") || error.contains("filesystem_write"),
            "unexpected denial: {error}"
        );
        assert!(!harness.temp_dir.join("generated/loong.toml").exists());
    }

    #[tokio::test]
    async fn kernel_routed_config_import_rollback_last_apply_restores_through_access() {
        let harness = TurnTestHarness::new();
        let openclaw_root = harness.temp_dir.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        fs::write(
            openclaw_root.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        )
        .expect("write prompt fixture");

        let output_path = harness.temp_dir.join("loong.toml");
        let original_body = config::render(&LoongConfig::default()).expect("render default config");
        fs::write(&output_path, &original_body).expect("write original config");
        let discovery = migration::discover_import_sources(
            &harness.temp_dir,
            migration::DiscoveryOptions::default(),
        )
        .expect("discovery should succeed");
        migration::apply_import_selection(&migration::ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: migration::ImportSelectionMode::RecommendedSingleSource {
                source_id: "openclaw".to_owned(),
            },
            apply_skills_plan: false,
            skills_input_path: None,
        })
        .expect("apply selection should create rollback manifest");

        let outcome = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "rollback_last_apply",
                    "output_path": "loong.toml"
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect("rollback should execute through kernel context");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "rollback_last_apply");
        assert_eq!(
            fs::read_to_string(&output_path).expect("read restored config"),
            original_body
        );
    }

    #[tokio::test]
    async fn kernel_routed_config_import_apply_selected_without_skills_writes_through_access() {
        let harness = TurnTestHarness::new();
        let openclaw_root = harness.temp_dir.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        fs::write(
            openclaw_root.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        )
        .expect("write prompt fixture");
        fs::write(
            openclaw_root.join("IDENTITY.md"),
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        )
        .expect("write identity fixture");
        let output_path = harness.temp_dir.join("generated/loong.toml");

        let outcome = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "apply_selected",
                    "input_path": ".",
                    "output_path": "generated/loong.toml",
                    "selection_id": "openclaw",
                    APPLY_SKILLS_PLAN_KEY: false
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect("apply_selected should execute through kernel context");

        let expected_output_path =
            dunce::canonicalize(&output_path).expect("canonicalize generated config path");
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply_selected");
        assert_eq!(
            outcome.payload["result"]["output_path"],
            expected_output_path.display().to_string()
        );
        assert_eq!(
            config::parse(
                fs::read_to_string(&output_path)
                    .expect("read generated config")
                    .as_str()
            )
            .expect("parse generated config")
            .memory
            .profile,
            MemoryProfile::ProfilePlusWindow
        );
        assert!(
            outcome.payload["result"]["manifest_path"]
                .as_str()
                .is_some_and(|path| path.contains(".loong-migration"))
        );
    }

    #[tokio::test]
    async fn kernel_routed_config_import_rollback_last_apply_requires_filesystem_write_capability()
    {
        let harness = TurnTestHarness::with_capabilities(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let openclaw_root = harness.temp_dir.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        fs::write(
            openclaw_root.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers.\n",
        )
        .expect("write prompt fixture");

        let output_path = harness.temp_dir.join("loong.toml");
        let original_body = config::render(&LoongConfig::default()).expect("render default config");
        fs::write(&output_path, &original_body).expect("write original config");
        let discovery = migration::discover_import_sources(
            &harness.temp_dir,
            migration::DiscoveryOptions::default(),
        )
        .expect("discovery should succeed");
        migration::apply_import_selection(&migration::ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: migration::ImportSelectionMode::RecommendedSingleSource {
                source_id: "openclaw".to_owned(),
            },
            apply_skills_plan: false,
            skills_input_path: None,
        })
        .expect("apply selection should create rollback manifest");
        let applied_body = fs::read_to_string(&output_path).expect("read applied config");
        assert_ne!(applied_body, original_body);

        let error = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "rollback_last_apply",
                    "output_path": "loong.toml"
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect_err("missing write capability should deny rollback");

        assert!(
            error.contains("FilesystemWrite") || error.contains("filesystem_write"),
            "unexpected denial: {error}"
        );
        assert_eq!(
            fs::read_to_string(&output_path).expect("read preserved applied config"),
            applied_body
        );
    }

    #[tokio::test]
    async fn kernel_routed_config_import_rollback_last_apply_requires_filesystem_read_capability() {
        let harness = TurnTestHarness::with_capabilities(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemWrite,
        ]));
        let openclaw_root = harness.temp_dir.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        fs::write(
            openclaw_root.join("SOUL.md"),
            "# Soul\n\nPrefer direct answers.\n",
        )
        .expect("write prompt fixture");

        let output_path = harness.temp_dir.join("loong.toml");
        let original_body = config::render(&LoongConfig::default()).expect("render default config");
        fs::write(&output_path, &original_body).expect("write original config");
        let discovery = migration::discover_import_sources(
            &harness.temp_dir,
            migration::DiscoveryOptions::default(),
        )
        .expect("discovery should succeed");
        migration::apply_import_selection(&migration::ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: migration::ImportSelectionMode::RecommendedSingleSource {
                source_id: "openclaw".to_owned(),
            },
            apply_skills_plan: false,
            skills_input_path: None,
        })
        .expect("apply selection should create rollback manifest");
        let applied_body = fs::read_to_string(&output_path).expect("read applied config");
        assert_ne!(applied_body, original_body);

        let error = crate::tools::execute_tool(
            ToolCoreRequest {
                tool_name: CONFIG_IMPORT_TOOL_NAME.to_owned(),
                payload: json!({
                    "mode": "rollback_last_apply",
                    "output_path": "loong.toml"
                }),
            },
            &harness.kernel_ctx,
        )
        .await
        .expect_err("missing read capability should deny rollback manifest read");

        assert!(
            error.contains("FilesystemRead") || error.contains("filesystem_read"),
            "unexpected denial: {error}"
        );
        assert_eq!(
            fs::read_to_string(&output_path).expect("read preserved applied config"),
            applied_body
        );
    }
}
