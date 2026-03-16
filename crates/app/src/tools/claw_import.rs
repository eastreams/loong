use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};

use crate::{
    config::{self, LoongClawConfig, MemoryProfile},
    migration::{self, LegacyClawSource},
};

const DEFAULT_MODE: &str = "plan";
const SUPPORTED_SOURCES: &str = "auto, nanobot, openclaw, picoclaw, zeroclaw, nanoclaw";

pub(super) fn execute_claw_import_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "claw.import payload must be an object".to_owned())?;
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_MODE);
    if !matches!(
        mode,
        "plan"
            | "apply"
            | "apply_selected"
            | "discover"
            | "plan_many"
            | "recommend_primary"
            | "merge_profiles"
            | "map_external_skills"
            | "rollback_last_apply"
    ) {
        return Err(format!(
            "claw.import payload.mode must be `plan`, `apply`, `apply_selected`, `discover`, `plan_many`, `recommend_primary`, `merge_profiles`, `map_external_skills`, or `rollback_last_apply`, got `{mode}`"
        ));
    }

    let output_path = payload
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolve_safe_path_with_config(value, config))
        .transpose()?;

    if matches!(mode, "apply" | "apply_selected" | "rollback_last_apply") && output_path.is_none() {
        return Err(format!(
            "claw.import {mode} mode requires payload.output_path"
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
                .ok_or_else(|| "claw.import requires payload.input_path".to_owned())
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
            "claw.import rollback_last_apply mode requires payload.output_path".to_owned()
        })?;
        let restored_path = migration::rollback_last_import(&output_path)?;
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

    let input_path =
        input_path.ok_or_else(|| "claw.import requires payload.input_path".to_owned())?;

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

    if mode == "map_external_skills" {
        let mapping = migration::plan_external_skill_mapping(input_path.as_path());
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "mode": "map_external_skills",
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
        let apply_external_skills_plan = payload
            .get("apply_external_skills_plan")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let selected_output_path = output_path.ok_or_else(|| {
            "claw.import apply_selected mode requires payload.output_path".to_owned()
        })?;
        let result = migration::apply_import_selection(&migration::ApplyImportSelection {
            discovery: report,
            output_path: selected_output_path,
            mode: selection,
            apply_external_skills_plan,
            external_skills_input_path: if apply_external_skills_plan {
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
                "apply_external_skills_plan": apply_external_skills_plan,
                "result": apply_selection_result_payload(&result),
            }),
        });
    }

    let plan = migration::plan_import_from_path(input_path.as_path(), hint)?;

    let mut merged_config = load_or_default_config(output_path.as_deref())?;
    migration::apply_import_plan(&mut merged_config, &plan);
    let config_toml = config::render(&merged_config)?;

    let written_output_path = if mode == "apply" {
        let output_path = output_path
            .clone()
            .ok_or_else(|| "claw.import apply mode requires payload.output_path".to_owned())?;
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
                .map(|path| format!("loongclaw chat --config {}", path.display())),
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
        "external_skills_manifest_path": result
            .external_skills_manifest_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "selected_primary_source_id": result.selected_primary_source_id,
        "merged_source_ids": result.merged_source_ids,
        "prompt_owner_source_id": result.prompt_owner_source_id,
        "unresolved_conflicts": result.unresolved_conflicts,
        "external_skill_artifact_count": result.external_skill_artifact_count,
        "external_skill_entries_applied": result.external_skill_entries_applied,
        "warnings": result.warnings,
    })
}

fn parse_source_hint(raw: &str) -> Result<Option<LegacyClawSource>, String> {
    let parsed = LegacyClawSource::from_id(raw).ok_or_else(|| {
        format!("unsupported claw.import payload.source `{raw}`. supported: {SUPPORTED_SOURCES}")
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

fn load_or_default_config(path: Option<&Path>) -> Result<LoongClawConfig, String> {
    let Some(path) = path else {
        return Ok(LoongClawConfig::default());
    };
    if !path.exists() {
        return Ok(LoongClawConfig::default());
    }
    let path_string = path.display().to_string();
    let (_, config) = config::load(Some(&path_string))?;
    Ok(config)
}

fn config_preview_payload(config: &LoongClawConfig) -> Value {
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
        return fs::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()));
    }
    Ok(super::normalize_without_fs(&path))
}

fn resolve_path_within_root(root: &Path, normalized: &Path) -> Result<PathBuf, String> {
    ensure_path_within_root(root, normalized)?;

    if normalized.exists() {
        let canonical = fs::canonicalize(normalized).map_err(|error| {
            format!(
                "failed to canonicalize target path {}: {error}",
                normalized.display()
            )
        })?;
        ensure_path_within_root(root, &canonical)?;
        return Ok(canonical);
    }

    let (ancestor, suffix) = split_existing_ancestor(normalized)?;
    let canonical_ancestor = fs::canonicalize(&ancestor).map_err(|error| {
        format!(
            "failed to canonicalize ancestor {}: {error}",
            ancestor.display()
        )
    })?;
    ensure_path_within_root(root, &canonical_ancestor)?;

    let mut reconstructed = canonical_ancestor;
    for component in suffix {
        reconstructed.push(component);
    }
    ensure_path_within_root(root, &reconstructed)?;
    Ok(reconstructed)
}

fn ensure_path_within_root(root: &Path, path: &Path) -> Result<(), String> {
    if path.starts_with(root) {
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
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
        let base = unique_temp_dir("loongclaw-claw-import");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");

        let config = ToolRuntimeConfig {
            shell_allow: Default::default(),
            shell_deny: Default::default(),
            shell_default_mode: crate::tools::shell_policy_ext::ShellPolicyDefault::Deny,
            file_root: Some(root),
            config_path: None,
            sessions_enabled: true,
            messages_enabled: true,
            delegate_enabled: true,
            browser: Default::default(),
            web_fetch: Default::default(),
            external_skills: Default::default(),
            #[cfg(feature = "feishu-integration")]
            feishu: None,
        };
        let error = resolve_safe_path_with_config("../outside.toml", &config)
            .expect_err("escape should be denied");

        assert!(error.starts_with("policy_denied: "));
        assert!(error.contains("escapes configured file root"));
        let _ = fs::remove_dir_all(base);
    }
}
