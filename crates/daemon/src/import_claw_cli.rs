#![allow(dead_code)] // legacy import-claw flow remains test-covered but is not exposed by the current daemon CLI

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub(crate) enum ImportClawMode {
    Apply,
    Plan,
    Discover,
    PlanMany,
    RecommendPrimary,
    MergeProfiles,
    MapExternalSkills,
    ApplySelected,
    RollbackLastApply,
}

impl ImportClawMode {
    fn requires_input(self) -> bool {
        !matches!(self, Self::RollbackLastApply)
    }

    fn writes_output(self) -> bool {
        matches!(self, Self::Apply | Self::ApplySelected)
    }

    fn as_id(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Plan => "plan",
            Self::Discover => "discover",
            Self::PlanMany => "plan_many",
            Self::RecommendPrimary => "recommend_primary",
            Self::MergeProfiles => "merge_profiles",
            Self::MapExternalSkills => "map_external_skills",
            Self::ApplySelected => "apply_selected",
            Self::RollbackLastApply => "rollback_last_apply",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ImportClawCommandOptions {
    pub input: Option<String>,
    pub output: Option<String>,
    pub source: Option<String>,
    pub mode: ImportClawMode,
    pub json: bool,
    pub source_id: Option<String>,
    pub safe_profile_merge: bool,
    pub primary_source_id: Option<String>,
    pub apply_external_skills_plan: bool,
    pub force: bool,
}

pub(crate) fn parse_legacy_claw_source(raw: &str) -> Option<mvp::migration::LegacyClawSource> {
    mvp::migration::LegacyClawSource::from_id(raw)
}

pub(crate) fn run_import_claw_cli(options: ImportClawCommandOptions) -> CliResult<()> {
    let output_path = resolve_output_path(options.output.as_deref());
    let input_path = options.input.as_deref().map(mvp::config::expand_path);

    if options.mode.writes_output() && output_path.exists() && !options.force {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    match options.mode {
        ImportClawMode::RollbackLastApply => run_rollback_mode(&output_path, options.json),
        ImportClawMode::Discover => {
            let input = require_input_path(input_path, options.mode)?;
            let report = mvp::migration::discover_import_sources(
                &input,
                mvp::migration::DiscoveryOptions::default(),
            )?;
            if options.json {
                return print_json_payload(json!({
                    "mode": options.mode.as_id(),
                    "input_path": input.display().to_string(),
                    "sources": report
                        .sources
                        .iter()
                        .map(discovered_source_payload)
                        .collect::<Vec<_>>(),
                }));
            }

            println!("import discovery complete");
            println!("- input: {}", input.display());
            println!("- discovered sources: {}", report.sources.len());
            for source in &report.sources {
                println!(
                    "- [{}] kind={} confidence={} path={}",
                    source.source_id,
                    source.source.as_id(),
                    source.confidence_score,
                    source.path.display()
                );
            }
            Ok(())
        }
        ImportClawMode::PlanMany | ImportClawMode::RecommendPrimary => {
            let input = require_input_path(input_path, options.mode)?;
            let report = mvp::migration::discover_import_sources(
                &input,
                mvp::migration::DiscoveryOptions::default(),
            )?;
            let summary = mvp::migration::plan_import_sources(&report)?;
            let recommendation = mvp::migration::recommend_primary_source(&summary).ok();

            if matches!(options.mode, ImportClawMode::RecommendPrimary) && recommendation.is_none()
            {
                return Err(
                    "no import sources discovered; cannot recommend primary source".to_owned(),
                );
            }

            if options.json {
                return print_json_payload(json!({
                    "mode": options.mode.as_id(),
                    "input_path": input.display().to_string(),
                    "plans": summary.plans.iter().map(planned_source_payload).collect::<Vec<_>>(),
                    "recommendation": recommendation.as_ref().map(primary_recommendation_payload),
                }));
            }

            println!("import planning complete");
            println!("- mode: {}", options.mode.as_id());
            println!("- input: {}", input.display());
            println!("- planned sources: {}", summary.plans.len());
            if let Some(recommended) = recommendation.as_ref() {
                println!(
                    "- recommended source: {} ({})",
                    recommended.source_id,
                    recommended.source.as_id()
                );
            }
            for plan in &summary.plans {
                println!(
                    "- [{}] kind={} confidence={} prompt={} profile={} warnings={} path={}",
                    plan.source_id,
                    plan.source.as_id(),
                    plan.confidence_score,
                    yes_no(plan.prompt_addendum_present),
                    yes_no(plan.profile_note_present),
                    plan.warning_count,
                    plan.input_path.display()
                );
            }
            Ok(())
        }
        ImportClawMode::MergeProfiles => {
            let input = require_input_path(input_path, options.mode)?;
            let report = mvp::migration::discover_import_sources(
                &input,
                mvp::migration::DiscoveryOptions::default(),
            )?;
            let summary = mvp::migration::plan_import_sources(&report)?;
            let recommendation = mvp::migration::recommend_primary_source(&summary).ok();
            let merged = mvp::migration::merge_profile_sources(&report)?;

            if options.json {
                return print_json_payload(json!({
                    "mode": options.mode.as_id(),
                    "input_path": input.display().to_string(),
                    "plans": summary.plans.iter().map(planned_source_payload).collect::<Vec<_>>(),
                    "recommendation": recommendation.as_ref().map(primary_recommendation_payload),
                    "result": merged_profile_plan_payload(&merged),
                }));
            }

            println!("profile merge preview complete");
            println!("- input: {}", input.display());
            println!("- source count: {}", summary.plans.len());
            if let Some(recommended) = recommendation.as_ref() {
                println!("- recommended prompt owner: {}", recommended.source_id);
            }
            println!(
                "- auto apply allowed: {}",
                yes_no(merged.auto_apply_allowed)
            );
            println!(
                "- unresolved conflicts: {}",
                merged.unresolved_conflicts.len()
            );
            println!("- kept entries: {}", merged.kept_entries.len());
            println!("- dropped duplicates: {}", merged.dropped_duplicates.len());
            Ok(())
        }
        ImportClawMode::MapExternalSkills => {
            let input = require_input_path(input_path, options.mode)?;
            let mapping = mvp::migration::plan_external_skill_mapping(&input);

            if options.json {
                return print_json_payload(json!({
                    "mode": options.mode.as_id(),
                    "input_path": input.display().to_string(),
                    "result": external_skill_mapping_plan_payload(&mapping),
                }));
            }

            println!("external skills mapping plan ready");
            println!("- input: {}", input.display());
            println!("- detected artifacts: {}", mapping.artifacts.len());
            println!("- declared skills: {}", mapping.declared_skills.len());
            println!("- locked skills: {}", mapping.locked_skills.len());
            println!("- resolved skills: {}", mapping.resolved_skills.len());
            println!(
                "- profile addendum generated: {}",
                yes_no(mapping.profile_note_addendum.is_some())
            );
            for artifact in &mapping.artifacts {
                println!(
                    "- artifact: kind={} path={}",
                    artifact.kind.as_id(),
                    artifact.path.display()
                );
            }
            for warning in &mapping.warnings {
                println!("- warning: {warning}");
            }
            println!(
                "next step: loongclaw import-claw --mode apply_selected --input {} --output {} --apply-external-skills-plan --force",
                input.display(),
                output_path.display()
            );
            Ok(())
        }
        ImportClawMode::ApplySelected => {
            let input = require_input_path(input_path, options.mode)?;
            let report = mvp::migration::discover_import_sources(
                &input,
                mvp::migration::DiscoveryOptions::default(),
            )?;
            let summary = mvp::migration::plan_import_sources(&report)?;
            let selection = resolve_apply_selection_mode(&options, &summary)?;
            let result =
                mvp::migration::apply_import_selection(&mvp::migration::ApplyImportSelection {
                    discovery: report,
                    output_path,
                    mode: selection.clone(),
                    apply_external_skills_plan: options.apply_external_skills_plan,
                    external_skills_input_path: if options.apply_external_skills_plan {
                        Some(input.clone())
                    } else {
                        None
                    },
                })?;

            #[cfg(feature = "memory-sqlite")]
            let memory_path = ensure_memory_ready_from_path(&result.output_path)?;

            if options.json {
                return print_json_payload(json!({
                    "mode": options.mode.as_id(),
                    "input_path": input.display().to_string(),
                    "output_path": result.output_path.display().to_string(),
                    "selection_mode": selection_mode_id(&selection),
                    "apply_external_skills_plan": options.apply_external_skills_plan,
                    "result": apply_selection_result_payload(&result),
                }));
            }

            println!("import selection applied");
            println!("- mode: {}", options.mode.as_id());
            println!("- input: {}", input.display());
            println!("- output: {}", result.output_path.display());
            println!("- selection mode: {}", selection_mode_id(&selection));
            println!(
                "- selected primary source: {}",
                result.selected_primary_source_id
            );
            println!(
                "- merged source ids: {}",
                result.merged_source_ids.join(", ")
            );
            println!("- unresolved conflicts: {}", result.unresolved_conflicts);
            println!(
                "- external skill artifacts: {}",
                result.external_skill_artifact_count
            );
            println!(
                "- external skill entries applied: {}",
                result.external_skill_entries_applied
            );
            if let Some(path) = result.external_skills_manifest_path.as_ref() {
                println!("- external skills manifest: {}", path.display());
            }
            #[cfg(feature = "memory-sqlite")]
            println!("- sqlite memory: {}", memory_path.display());
            for warning in &result.warnings {
                println!("- warning: {warning}");
            }
            println!(
                "next step: loongclaw chat --config {}",
                result.output_path.display()
            );
            Ok(())
        }
        ImportClawMode::Plan | ImportClawMode::Apply => {
            let input = require_input_path(input_path, options.mode)?;
            let hint = if let Some(raw) = options.source.as_deref() {
                let parsed = parse_legacy_claw_source(raw).ok_or_else(|| {
                    format!(
                        "unsupported --source value \"{raw}\". supported: {}",
                        supported_legacy_source_list()
                    )
                })?;
                if parsed == mvp::migration::LegacyClawSource::Unknown {
                    None
                } else {
                    Some(parsed)
                }
            } else {
                None
            };

            let plan = mvp::migration::plan_import_from_path(&input, hint)?;
            let mut config = load_or_default_config(&output_path, output_path.exists())?;
            mvp::migration::apply_import_plan(&mut config, &plan);

            if matches!(options.mode, ImportClawMode::Plan) {
                if options.json {
                    let rendered = mvp::config::render(&config)
                        .map_err(|error| format!("render preview failed: {error}"))?;
                    return print_json_payload(json!({
                        "mode": options.mode.as_id(),
                        "source": legacy_claw_source_id(plan.source),
                        "input_path": input.display().to_string(),
                        "output_path": output_path.display().to_string(),
                        "warnings": plan.warnings,
                        "config_preview": config_preview_payload(&config),
                        "config_toml": rendered,
                    }));
                }

                println!("import plan ready");
                println!("- source: {}", legacy_claw_source_id(plan.source));
                println!("- input: {}", input.display());
                println!("- output target: {}", output_path.display());
                println!(
                    "- prompt pack: {}",
                    config
                        .cli
                        .prompt_pack_id()
                        .unwrap_or(mvp::prompt::DEFAULT_PROMPT_PACK_ID)
                );
                println!(
                    "- memory profile: {}",
                    memory_profile_id(config.memory.profile)
                );
                println!(
                    "- imported prompt addendum: {}",
                    yes_no(config.cli.system_prompt_addendum.is_some())
                );
                println!(
                    "- imported profile note: {}",
                    yes_no(config.memory.profile_note.is_some())
                );
                for warning in &plan.warnings {
                    println!("- warning: {warning}");
                }
                println!(
                    "next step: loongclaw import-claw --mode apply --input {} --output {} --force",
                    input.display(),
                    output_path.display()
                );
                return Ok(());
            }

            let output_string = output_path.display().to_string();
            let written = mvp::config::write(Some(&output_string), &config, options.force)?;

            #[cfg(feature = "memory-sqlite")]
            let memory_path = {
                let mem_config =
                    mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                        &config.memory,
                    );
                mvp::memory::ensure_memory_db_ready(
                    Some(config.memory.resolved_sqlite_path()),
                    &mem_config,
                )
                .map_err(|error| format!("failed to bootstrap sqlite memory: {error}"))?
            };

            if options.json {
                return print_json_payload(json!({
                    "mode": options.mode.as_id(),
                    "source": legacy_claw_source_id(plan.source),
                    "input_path": input.display().to_string(),
                    "output_path": written.display().to_string(),
                    "warnings": plan.warnings,
                    "config_preview": config_preview_payload(&config),
                }));
            }

            println!("import complete");
            println!("- source: {}", legacy_claw_source_id(plan.source));
            println!("- input: {}", input.display());
            println!("- config: {}", written.display());
            println!(
                "- prompt pack: {}",
                config
                    .cli
                    .prompt_pack_id()
                    .unwrap_or(mvp::prompt::DEFAULT_PROMPT_PACK_ID)
            );
            println!(
                "- memory profile: {}",
                memory_profile_id(config.memory.profile)
            );
            println!(
                "- imported prompt addendum: {}",
                yes_no(config.cli.system_prompt_addendum.is_some())
            );
            println!(
                "- imported profile note: {}",
                yes_no(config.memory.profile_note.is_some())
            );
            #[cfg(feature = "memory-sqlite")]
            println!("- sqlite memory: {}", memory_path.display());
            for warning in &plan.warnings {
                println!("- warning: {warning}");
            }
            println!("next step: loongclaw chat --config {}", written.display());
            Ok(())
        }
    }
}

fn load_or_default_config(path: &Path, exists: bool) -> CliResult<mvp::config::LoongClawConfig> {
    if !exists {
        return Ok(mvp::config::LoongClawConfig::default());
    }
    let path_string = path.display().to_string();
    let (_, config) = mvp::config::load(Some(&path_string))?;
    Ok(config)
}

fn legacy_claw_source_id(source: mvp::migration::LegacyClawSource) -> &'static str {
    source.as_id()
}

fn supported_legacy_source_list() -> &'static str {
    "auto, nanobot, openclaw, picoclaw, zeroclaw, nanoclaw"
}

fn resolve_output_path(output: Option<&str>) -> PathBuf {
    output
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path)
}

fn require_input_path(input: Option<PathBuf>, mode: ImportClawMode) -> CliResult<PathBuf> {
    if mode.requires_input() {
        return input
            .ok_or_else(|| format!("import-claw mode `{}` requires --input", mode.as_id()));
    }
    Ok(PathBuf::new())
}

fn print_json_payload(payload: Value) -> CliResult<()> {
    let encoded = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("json serialization failed: {error}"))?;
    println!("{encoded}");
    Ok(())
}

fn resolve_apply_selection_mode(
    options: &ImportClawCommandOptions,
    summary: &mvp::migration::DiscoveryPlanSummary,
) -> CliResult<mvp::migration::ImportSelectionMode> {
    let source_id = options
        .source_id
        .as_deref()
        .or(options.primary_source_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    if options.safe_profile_merge {
        let primary_source_id = source_id
            .or_else(|| {
                mvp::migration::recommend_primary_source(summary)
                    .ok()
                    .map(|recommendation| recommendation.source_id)
            })
            .ok_or_else(|| {
                "apply_selected requires --source-id or --primary-source-id when --safe-profile-merge is enabled".to_owned()
            })?;
        return Ok(mvp::migration::ImportSelectionMode::SafeProfileMerge { primary_source_id });
    }

    if let Some(source_id) = source_id {
        return Ok(mvp::migration::ImportSelectionMode::SelectedSingleSource { source_id });
    }

    let recommendation = mvp::migration::recommend_primary_source(summary)
        .map_err(|error| format!("cannot recommend primary source: {error}"))?;
    Ok(
        mvp::migration::ImportSelectionMode::RecommendedSingleSource {
            source_id: recommendation.source_id,
        },
    )
}

fn discovered_source_payload(source: &mvp::migration::DiscoveredImportSource) -> Value {
    json!({
        "source_id": source.source_id,
        "source_kind": source.source.as_id(),
        "input_path": source.path.display().to_string(),
        "confidence_score": source.confidence_score,
        "found_files": source.found_files,
    })
}

fn planned_source_payload(plan: &mvp::migration::PlannedImportSource) -> Value {
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
    recommendation: &mvp::migration::PrimarySourceRecommendation,
) -> Value {
    json!({
        "source_id": recommendation.source_id,
        "source_kind": recommendation.source.as_id(),
        "input_path": recommendation.input_path.display().to_string(),
        "reasons": recommendation.reasons,
    })
}

fn merged_profile_plan_payload(plan: &mvp::migration::MergedProfilePlan) -> Value {
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
                        mvp::migration::ProfileEntryLane::Prompt => "prompt",
                        mvp::migration::ProfileEntryLane::Profile => "profile",
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
                        mvp::migration::ProfileEntryLane::Prompt => "prompt",
                        mvp::migration::ProfileEntryLane::Profile => "profile",
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

fn external_skill_mapping_plan_payload(plan: &mvp::migration::ExternalSkillMappingPlan) -> Value {
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

fn apply_selection_result_payload(result: &mvp::migration::ApplyImportSelectionResult) -> Value {
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

fn config_preview_payload(config: &mvp::config::LoongClawConfig) -> Value {
    json!({
        "prompt_pack_id": config
            .cli
            .prompt_pack_id()
            .unwrap_or(mvp::prompt::DEFAULT_PROMPT_PACK_ID),
        "memory_profile": memory_profile_id(config.memory.profile),
        "system_prompt_addendum": config.cli.system_prompt_addendum.clone(),
        "profile_note": config.memory.profile_note.clone(),
    })
}

fn selection_mode_id(selection: &mvp::migration::ImportSelectionMode) -> &'static str {
    match selection {
        mvp::migration::ImportSelectionMode::RecommendedSingleSource { .. } => {
            "recommended_single_source"
        }
        mvp::migration::ImportSelectionMode::SelectedSingleSource { .. } => {
            "selected_single_source"
        }
        mvp::migration::ImportSelectionMode::SafeProfileMerge { .. } => "safe_profile_merge",
    }
}

fn run_rollback_mode(output_path: &Path, as_json: bool) -> CliResult<()> {
    let restored = mvp::migration::rollback_last_import(output_path)?;
    if as_json {
        return print_json_payload(json!({
            "mode": "rollback_last_apply",
            "output_path": restored.display().to_string(),
            "rolled_back": true,
        }));
    }

    println!("rollback complete");
    println!("- restored config: {}", restored.display());
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn ensure_memory_ready_from_path(path: &Path) -> CliResult<PathBuf> {
    let output = path.display().to_string();
    let (_, config) = mvp::config::load(Some(&output))?;
    let runtime =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    mvp::memory::ensure_memory_db_ready(Some(config.memory.resolved_sqlite_path()), &runtime)
        .map_err(|error| format!("failed to bootstrap sqlite memory: {error}"))
}

fn memory_profile_id(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
