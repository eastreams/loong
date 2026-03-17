use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::CliResult;

use super::{
    LegacyClawSource, MergedProfilePlan, ProfileEntryLane, ProfileMergeEntry,
    apply_external_skill_mapping, apply_import_plan, inspect_import_path, merge_profile_entries,
    plan_external_skill_mapping, plan_import_from_path,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryOptions {
    pub include_child_directories: bool,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            include_child_directories: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredImportSource {
    pub source: LegacyClawSource,
    pub source_id: String,
    pub path: PathBuf,
    pub confidence_score: u32,
    pub found_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveryReport {
    pub sources: Vec<DiscoveredImportSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedImportSource {
    pub source: LegacyClawSource,
    pub source_id: String,
    pub input_path: PathBuf,
    pub confidence_score: u32,
    pub prompt_addendum_present: bool,
    pub profile_note_present: bool,
    pub warning_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveryPlanSummary {
    pub plans: Vec<PlannedImportSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimarySourceRecommendation {
    pub source: LegacyClawSource,
    pub source_id: String,
    pub input_path: PathBuf,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSelectionMode {
    RecommendedSingleSource { source_id: String },
    SelectedSingleSource { source_id: String },
    SafeProfileMerge { primary_source_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyImportSelection {
    pub discovery: DiscoveryReport,
    pub output_path: PathBuf,
    pub mode: ImportSelectionMode,
    pub apply_external_skills_plan: bool,
    pub external_skills_input_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyImportSelectionResult {
    pub output_path: PathBuf,
    pub backup_path: PathBuf,
    pub manifest_path: PathBuf,
    pub external_skills_manifest_path: Option<PathBuf>,
    pub selected_primary_source_id: String,
    pub merged_source_ids: Vec<String>,
    pub prompt_owner_source_id: Option<String>,
    pub unresolved_conflicts: usize,
    pub warnings: Vec<String>,
    pub external_skill_artifact_count: usize,
    pub external_skill_entries_applied: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportApplyManifest {
    session_id: String,
    selected_primary_source: String,
    merged_sources: Vec<String>,
    prompt_owner_source: Option<String>,
    output_path: String,
    backup_path: String,
    output_preexisted: bool,
    warnings: Vec<String>,
    unresolved_conflicts: usize,
    #[serde(default)]
    external_skill_artifact_count: usize,
    #[serde(default)]
    external_skill_entries_applied: usize,
    #[serde(default)]
    external_skills_manifest_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalSkillsApplyManifest {
    output_path: String,
    input_path: String,
    artifact_count: usize,
    artifacts: Vec<ExternalSkillsApplyArtifact>,
    declared_skills: Vec<String>,
    locked_skills: Vec<String>,
    resolved_skills: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalSkillsApplyArtifact {
    kind: String,
    path: String,
}

pub fn discover_import_sources(
    search_root: &Path,
    options: DiscoveryOptions,
) -> CliResult<DiscoveryReport> {
    if !search_root.exists() {
        return Err(format!(
            "discovery root does not exist: {}",
            search_root.display()
        ));
    }

    let mut sources = Vec::new();
    for candidate in collect_candidate_directories(search_root, &options)? {
        let Some(inspection) = inspect_import_path(&candidate, None)? else {
            continue;
        };
        sources.push(DiscoveredImportSource {
            source: inspection.source,
            source_id: String::new(),
            confidence_score: score_discovered_source(&inspection),
            found_files: inspection.found_files,
            path: candidate,
        });
    }

    sources.sort_by(|left, right| {
        right
            .confidence_score
            .cmp(&left.confidence_score)
            .then_with(|| left.path.cmp(&right.path))
    });
    assign_discovery_source_ids(&mut sources);

    Ok(DiscoveryReport { sources })
}

pub fn plan_import_sources(report: &DiscoveryReport) -> CliResult<DiscoveryPlanSummary> {
    let mut plans = Vec::new();
    for source in &report.sources {
        let plan = plan_import_from_path(&source.path, Some(source.source))?;
        plans.push(PlannedImportSource {
            source: source.source,
            source_id: source.source_id.clone(),
            input_path: source.path.clone(),
            confidence_score: source.confidence_score,
            prompt_addendum_present: plan.system_prompt_addendum.is_some(),
            profile_note_present: plan.profile_note.is_some(),
            warning_count: plan.warnings.len(),
        });
    }
    Ok(DiscoveryPlanSummary { plans })
}

pub fn recommend_primary_source(
    summary: &DiscoveryPlanSummary,
) -> CliResult<PrimarySourceRecommendation> {
    let Some(best) = summary.plans.iter().max_by(|left, right| {
        primary_recommendation_score(left)
            .cmp(&primary_recommendation_score(right))
            .then_with(|| left.input_path.cmp(&right.input_path))
    }) else {
        return Err("cannot recommend primary source from an empty plan summary".to_owned());
    };

    let mut reasons = Vec::new();
    reasons.push(format!("confidence score {}", best.confidence_score));
    if best.prompt_addendum_present {
        reasons.push("contains imported prompt overlay".to_owned());
    }
    if best.profile_note_present {
        reasons.push("contains imported profile overlay".to_owned());
    }
    if best.warning_count == 0 {
        reasons.push("has no import warnings".to_owned());
    }

    Ok(PrimarySourceRecommendation {
        source: best.source,
        source_id: best.source_id.clone(),
        input_path: best.input_path.clone(),
        reasons,
    })
}

pub fn merge_profile_sources(report: &DiscoveryReport) -> CliResult<MergedProfilePlan> {
    if report.sources.is_empty() {
        return Err("cannot merge profiles from an empty discovery report".to_owned());
    }

    let mut entries = Vec::new();
    for source in &report.sources {
        let plan = plan_import_from_path(&source.path, Some(source.source))?;
        let source_id = source.source_id.clone();

        if let Some(prompt_addendum) = plan.system_prompt_addendum.as_deref() {
            entries.push(ProfileMergeEntry {
                lane: ProfileEntryLane::Prompt,
                canonical_text: prompt_addendum.trim().to_owned(),
                source_id: source_id.clone(),
                source_confidence: source.confidence_score,
                entry_confidence: 1,
                slot_key: None,
            });
        }

        if let Some(profile_note) = plan.profile_note.as_deref() {
            entries.extend(parse_profile_merge_entries(
                profile_note,
                &source_id,
                source.confidence_score,
            ));
        }
    }

    let mut merged = merge_profile_entries(&entries)?;
    if merged.prompt_owner_source_id.is_none() {
        let summary = plan_import_sources(report)?;
        merged.prompt_owner_source_id = Some(recommend_primary_source(&summary)?.source_id);
    }
    Ok(merged)
}

pub fn apply_import_selection(
    request: &ApplyImportSelection,
) -> CliResult<ApplyImportSelectionResult> {
    let selected_primary_source_id = match &request.mode {
        ImportSelectionMode::RecommendedSingleSource { source_id }
        | ImportSelectionMode::SelectedSingleSource { source_id } => source_id.clone(),
        ImportSelectionMode::SafeProfileMerge { primary_source_id } => primary_source_id.clone(),
    };
    let selected_primary =
        resolve_discovered_source(&request.discovery, selected_primary_source_id.as_str())?;

    let mut config = load_or_default_config(Some(&request.output_path))?;
    let mut warnings = Vec::new();
    let mut external_skill_artifact_count = 0usize;
    let mut external_skill_entries_applied = 0usize;
    let mut external_skill_mapping = None;
    let (merged_source_ids, prompt_owner_source_id, unresolved_conflicts) = match &request.mode {
        ImportSelectionMode::RecommendedSingleSource { .. }
        | ImportSelectionMode::SelectedSingleSource { .. } => {
            let plan =
                plan_import_from_path(&selected_primary.path, Some(selected_primary.source))?;
            warnings.extend(plan.warnings.clone());
            apply_import_plan(&mut config, &plan);
            (
                vec![selected_primary_source_id.clone()],
                Some(selected_primary_source_id.clone()),
                0,
            )
        }
        ImportSelectionMode::SafeProfileMerge { .. } => {
            let primary_plan =
                plan_import_from_path(&selected_primary.path, Some(selected_primary.source))?;
            warnings.extend(primary_plan.warnings);

            for source in &request.discovery.sources {
                if source.path == selected_primary.path {
                    continue;
                }
                let plan = plan_import_from_path(&source.path, Some(source.source))?;
                warnings.extend(plan.warnings);
            }

            let merged = merge_profile_sources(&request.discovery)?;
            if !merged.auto_apply_allowed {
                return Err(format!(
                    "cannot auto-apply safe profile merge with {} unresolved conflict(s)",
                    merged.unresolved_conflicts.len()
                ));
            }
            config.memory.profile = crate::config::MemoryProfile::ProfilePlusWindow;
            config.memory.profile_note = if merged.merged_profile_note.trim().is_empty() {
                None
            } else {
                Some(merged.merged_profile_note.clone())
            };
            (
                request
                    .discovery
                    .sources
                    .iter()
                    .map(|source| source.source_id.clone())
                    .collect(),
                None,
                merged.unresolved_conflicts.len(),
            )
        }
    };

    if request.apply_external_skills_plan {
        let input_path = request
            .external_skills_input_path
            .as_deref()
            .ok_or_else(|| {
                "apply_external_skills_plan requires external_skills_input_path".to_owned()
            })?;
        let mapping = plan_external_skill_mapping(input_path);
        external_skill_artifact_count = mapping.artifacts.len();
        external_skill_entries_applied = apply_external_skill_mapping(&mut config, &mapping);
        warnings.extend(mapping.warnings.clone());
        external_skill_mapping = Some(mapping);
    }
    dedup_strings_in_place(&mut warnings);

    let state_dir = migration_state_dir(&request.output_path);
    fs::create_dir_all(&state_dir).map_err(|error| {
        format!(
            "failed to create migration state directory {}: {error}",
            state_dir.display()
        )
    })?;
    let session_id = import_session_id();
    let backup_path = backup_path_for_output(&request.output_path, &state_dir, &session_id);
    let manifest_path = manifest_path_for_output(&request.output_path, &state_dir);
    let output_preexisted = request.output_path.exists();
    if output_preexisted {
        fs::copy(&request.output_path, &backup_path).map_err(|error| {
            format!(
                "failed to write import backup {}: {error}",
                backup_path.display()
            )
        })?;
    } else {
        fs::write(&backup_path, "").map_err(|error| {
            format!(
                "failed to initialize import backup {}: {error}",
                backup_path.display()
            )
        })?;
    }

    let output_string = request.output_path.display().to_string();
    let written_output_path = crate::config::write(Some(&output_string), &config, true)?;
    let external_skills_manifest_path = if let Some(mapping) = external_skill_mapping.as_ref() {
        let external_path =
            external_skills_manifest_path_for_output(&request.output_path, &state_dir);
        let external_manifest = build_external_skills_apply_manifest(&written_output_path, mapping);
        let body = serde_json::to_vec_pretty(&external_manifest)
            .map_err(|error| format!("failed to encode external skills manifest: {error}"))?;
        fs::write(&external_path, body).map_err(|error| {
            format!(
                "failed to write external skills manifest {}: {error}",
                external_path.display()
            )
        })?;
        Some(external_path)
    } else {
        None
    };

    let manifest = ImportApplyManifest {
        session_id,
        selected_primary_source: selected_primary_source_id.clone(),
        merged_sources: merged_source_ids.clone(),
        prompt_owner_source: prompt_owner_source_id.clone(),
        output_path: written_output_path.display().to_string(),
        backup_path: backup_path.display().to_string(),
        output_preexisted,
        warnings: warnings.clone(),
        unresolved_conflicts,
        external_skill_artifact_count,
        external_skill_entries_applied,
        external_skills_manifest_path: external_skills_manifest_path
            .as_ref()
            .map(|path| path.display().to_string()),
    };
    let manifest_body = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("failed to encode import manifest: {error}"))?;
    fs::write(&manifest_path, manifest_body).map_err(|error| {
        format!(
            "failed to write import manifest {}: {error}",
            manifest_path.display()
        )
    })?;

    Ok(ApplyImportSelectionResult {
        output_path: written_output_path,
        backup_path,
        manifest_path,
        external_skills_manifest_path,
        selected_primary_source_id,
        merged_source_ids,
        prompt_owner_source_id,
        unresolved_conflicts,
        warnings,
        external_skill_artifact_count,
        external_skill_entries_applied,
    })
}

fn dedup_strings_in_place(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn build_external_skills_apply_manifest(
    output_path: &Path,
    mapping: &super::ExternalSkillMappingPlan,
) -> ExternalSkillsApplyManifest {
    ExternalSkillsApplyManifest {
        output_path: output_path.display().to_string(),
        input_path: mapping.input_path.display().to_string(),
        artifact_count: mapping.artifacts.len(),
        artifacts: mapping
            .artifacts
            .iter()
            .map(|artifact| ExternalSkillsApplyArtifact {
                kind: artifact.kind.as_id().to_owned(),
                path: artifact.path.display().to_string(),
            })
            .collect(),
        declared_skills: mapping.declared_skills.clone(),
        locked_skills: mapping.locked_skills.clone(),
        resolved_skills: mapping.resolved_skills.clone(),
        warnings: mapping.warnings.clone(),
    }
}

pub fn rollback_last_migration(output_path: &Path) -> CliResult<PathBuf> {
    let manifest = load_last_migration_manifest(output_path)?;
    let backup_path = PathBuf::from(&manifest.backup_path);
    if manifest.output_preexisted {
        fs::copy(&backup_path, output_path).map_err(|error| {
            format!(
                "failed to restore config {} from backup {}: {error}",
                output_path.display(),
                backup_path.display()
            )
        })?;
    } else if output_path.exists() {
        fs::remove_file(output_path).map_err(|error| {
            format!(
                "failed to remove imported config {}: {error}",
                output_path.display()
            )
        })?;
    }
    Ok(output_path.to_path_buf())
}

fn load_last_migration_manifest(output_path: &Path) -> CliResult<ImportApplyManifest> {
    let state_dir = migration_state_dir(output_path);
    let manifest_path = manifest_path_for_output(output_path, &state_dir);
    match fs::read(&manifest_path) {
        Ok(manifest_body) => parse_import_apply_manifest(&manifest_path, &manifest_body),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let legacy_manifest_path = legacy_manifest_path_for_output(output_path, &state_dir);
            match fs::read(&legacy_manifest_path) {
                Ok(manifest_body) => {
                    parse_import_apply_manifest(&legacy_manifest_path, &manifest_body)
                }
                Err(legacy_error) if legacy_error.kind() == ErrorKind::NotFound => Err(format!(
                    "failed to read migration manifest {} or legacy import manifest {}: {error}",
                    manifest_path.display(),
                    legacy_manifest_path.display()
                )),
                Err(legacy_error) => Err(format!(
                    "failed to read legacy import manifest {}: {legacy_error}",
                    legacy_manifest_path.display()
                )),
            }
        }
        Err(error) => Err(format!(
            "failed to read migration manifest {}: {error}",
            manifest_path.display()
        )),
    }
}

fn parse_import_apply_manifest(
    path: &Path,
    manifest_body: &[u8],
) -> CliResult<ImportApplyManifest> {
    serde_json::from_slice(manifest_body).map_err(|error| {
        format!(
            "failed to parse migration manifest {}: {error}",
            path.display()
        )
    })
}

fn collect_candidate_directories(
    search_root: &Path,
    options: &DiscoveryOptions,
) -> CliResult<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    push_candidate(&mut candidates, &mut seen, search_root.to_path_buf());

    if options.include_child_directories && search_root.is_dir() {
        let entries = fs::read_dir(search_root).map_err(|error| {
            format!(
                "failed to read discovery root {}: {error}",
                search_root.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to enumerate discovery root {}: {error}",
                    search_root.display()
                )
            })?;
            let path = entry.path();
            if path.is_dir() {
                push_candidate(&mut candidates, &mut seen, path);
            }
        }
    }

    Ok(candidates)
}

fn push_candidate(candidates: &mut Vec<PathBuf>, seen: &mut BTreeSet<String>, path: PathBuf) {
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.clone())
        .display()
        .to_string();
    if seen.insert(canonical) {
        candidates.push(path);
    }
}

fn score_discovered_source(inspection: &super::ImportPathInspection) -> u32 {
    let mut score = 0u32;
    if inspection.source != LegacyClawSource::Unknown {
        score = score.saturating_add(10);
    }
    score = score.saturating_add(inspection.custom_prompt_files as u32 * 12);
    score = score.saturating_add(inspection.custom_profile_files as u32 * 12);
    score = score.saturating_add(inspection.warning_count as u32 * 3);
    score = score.saturating_add(inspection.found_files.len() as u32);
    score
}

fn assign_discovery_source_ids(sources: &mut [DiscoveredImportSource]) {
    let mut source_type_counts = BTreeMap::<String, usize>::new();
    for source in sources.iter() {
        *source_type_counts
            .entry(source.source.as_id().to_owned())
            .or_default() += 1;
    }

    let mut token_counts = BTreeMap::<String, usize>::new();
    for source in sources.iter_mut() {
        let base_id = source.source.as_id().to_owned();
        let token_base = if source_type_counts.get(&base_id).copied().unwrap_or(0) > 1 {
            format!("{base_id}-{}", selection_slug(&source.path))
        } else {
            base_id
        };
        let count = token_counts.entry(token_base.clone()).or_default();
        *count += 1;
        source.source_id = if *count == 1 {
            token_base
        } else {
            format!("{token_base}-{}", *count)
        };
    }
}

fn selection_slug(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "source".to_owned());

    let mut slug = String::new();
    let mut just_pushed_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            just_pushed_dash = false;
        } else if !just_pushed_dash {
            slug.push('-');
            just_pushed_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "source".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn primary_recommendation_score(plan: &PlannedImportSource) -> u32 {
    let mut score = plan.confidence_score;
    if plan.prompt_addendum_present {
        score = score.saturating_add(10);
    }
    if plan.profile_note_present {
        score = score.saturating_add(10);
    }
    if plan.warning_count == 0 {
        score = score.saturating_add(3);
    }
    score.saturating_sub(plan.warning_count as u32)
}

fn parse_profile_merge_entries(
    profile_note: &str,
    source_id: &str,
    source_confidence: u32,
) -> Vec<ProfileMergeEntry> {
    let mut entries = Vec::new();
    let mut block_lines = Vec::new();

    let flush_block = |entries: &mut Vec<ProfileMergeEntry>, block_lines: &mut Vec<String>| {
        let joined = block_lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        block_lines.clear();
        if joined.is_empty() {
            return;
        }
        entries.push(ProfileMergeEntry {
            lane: ProfileEntryLane::Profile,
            canonical_text: joined,
            source_id: source_id.to_owned(),
            source_confidence,
            entry_confidence: 1,
            slot_key: None,
        });
    };

    for line in profile_note.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush_block(&mut entries, &mut block_lines);
            continue;
        }
        if trimmed.starts_with("## Imported ") || trimmed.starts_with('#') {
            flush_block(&mut entries, &mut block_lines);
            continue;
        }
        if let Some((slot_key, value)) = parse_profile_slot_line(trimmed) {
            flush_block(&mut entries, &mut block_lines);
            entries.push(ProfileMergeEntry {
                lane: ProfileEntryLane::Profile,
                canonical_text: format!("{slot_key}: {value}"),
                source_id: source_id.to_owned(),
                source_confidence,
                entry_confidence: 10,
                slot_key: Some(slot_key),
            });
            continue;
        }
        block_lines.push(trimmed.to_owned());
    }
    flush_block(&mut entries, &mut block_lines);

    entries
}

fn parse_profile_slot_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start_matches(['-', '*', ' ']).trim();
    let (raw_key, raw_value) = trimmed.split_once(':')?;
    let slot_key = raw_key.trim().to_ascii_lowercase();
    let value = raw_value.trim();
    if slot_key.is_empty() || value.is_empty() {
        return None;
    }
    Some((slot_key, value.to_owned()))
}

fn resolve_discovered_source<'a>(
    report: &'a DiscoveryReport,
    source_id: &str,
) -> CliResult<&'a DiscoveredImportSource> {
    report
        .sources
        .iter()
        .find(|source| source.source_id == source_id)
        .ok_or_else(|| format!("selected import source `{source_id}` was not discovered"))
}

fn load_or_default_config(path: Option<&Path>) -> CliResult<crate::config::LoongClawConfig> {
    let Some(path) = path else {
        return Ok(crate::config::LoongClawConfig::default());
    };
    if !path.exists() {
        return Ok(crate::config::LoongClawConfig::default());
    }
    let path_string = path.display().to_string();
    let (_, config) = crate::config::load(Some(&path_string))?;
    Ok(config)
}

fn migration_state_dir(output_path: &Path) -> PathBuf {
    output_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(".loongclaw-migration")
}

fn manifest_path_for_output(output_path: &Path, state_dir: &Path) -> PathBuf {
    let file_tag = output_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "loongclaw-config".to_owned());
    state_dir.join(format!("{file_tag}.last-migration.json"))
}

fn legacy_manifest_path_for_output(output_path: &Path, state_dir: &Path) -> PathBuf {
    let file_tag = output_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "loongclaw-config".to_owned());
    state_dir.join(format!("{file_tag}.last-import.json"))
}

fn external_skills_manifest_path_for_output(output_path: &Path, state_dir: &Path) -> PathBuf {
    let file_tag = output_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "loongclaw-config".to_owned());
    state_dir.join(format!("{file_tag}.external-skills.json"))
}

fn backup_path_for_output(output_path: &Path, state_dir: &Path, session_id: &str) -> PathBuf {
    let file_tag = output_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "loongclaw-config".to_owned());
    state_dir.join(format!("{file_tag}.{session_id}.bak"))
}

fn import_session_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("import-{millis}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    #[test]
    fn discover_import_sources_returns_ranked_candidates_from_fixture_root() {
        let root = unique_temp_dir("loongclaw-import-discovery-ranked");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "SOUL.md",
            "# Soul\n\nAlways prefer brief shell output.\n",
        );

        let report = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        assert_eq!(report.sources.len(), 2);
        assert_eq!(report.sources[0].source.as_id(), "openclaw");
        assert!(
            report.sources[0].confidence_score >= report.sources[1].confidence_score,
            "expected descending confidence scores"
        );
        assert!(
            report.sources[0]
                .found_files
                .iter()
                .any(|value| value == "SOUL.md")
        );
        assert!(
            report.sources[0]
                .found_files
                .iter()
                .any(|value| value == "IDENTITY.md")
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discover_import_sources_ignores_empty_or_stock_only_noise_directories() {
        let root = unique_temp_dir("loongclaw-import-discovery-noise");
        fs::create_dir_all(&root).expect("create fixture root");

        let empty_root = root.join("empty");
        fs::create_dir_all(&empty_root).expect("create empty root");

        let stock_nanobot = root.join("stock-nanobot");
        fs::create_dir_all(&stock_nanobot).expect("create stock nanobot root");
        write_file(
            &stock_nanobot,
            "SOUL.md",
            "# Soul\n\nI am nanobot 🐈, a personal AI assistant.\n",
        );
        write_file(
            &stock_nanobot,
            "memory/MEMORY.md",
            "# Long-term Memory\n\n*This file is automatically updated by nanobot when important information should be remembered.*\n",
        );

        let report = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        assert!(
            report.sources.is_empty(),
            "noise-only roots should be ignored"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn plan_import_sources_returns_summary_for_each_candidate() {
        let root = unique_temp_dir("loongclaw-import-plan-many");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let summary = plan_import_sources(&discovery).expect("plan-many should succeed");

        assert_eq!(summary.plans.len(), 2);
        assert_eq!(summary.plans[0].source_id, "openclaw");
        assert!(summary.plans[0].prompt_addendum_present);
        assert!(summary.plans[0].profile_note_present);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recommend_primary_source_prefers_richer_custom_source() {
        let root = unique_temp_dir("loongclaw-import-recommend-primary");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "SOUL.md",
            "# Soul\n\nAlways prefer brief shell output.\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let summary = plan_import_sources(&discovery).expect("plan-many should succeed");
        let recommendation =
            recommend_primary_source(&summary).expect("primary recommendation should succeed");

        assert_eq!(recommendation.source_id, "openclaw");
        assert_eq!(recommendation.source, LegacyClawSource::OpenClaw);
        assert!(
            !recommendation.reasons.is_empty(),
            "recommendation reasons should be populated"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn apply_import_selection_writes_backup_and_manifest() {
        let root = unique_temp_dir("loongclaw-import-apply-selection");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let output_path = root.join("loongclaw.toml");
        let original_body =
            crate::config::render(&crate::config::LoongClawConfig::default()).expect("render");
        fs::write(&output_path, &original_body).expect("write original config");

        let result = apply_import_selection(&ApplyImportSelection {
            discovery,
            output_path,
            mode: ImportSelectionMode::RecommendedSingleSource {
                source_id: "openclaw".to_owned(),
            },
            apply_external_skills_plan: false,
            external_skills_input_path: None,
        })
        .expect("apply should succeed");

        assert!(result.backup_path.exists());
        assert!(result.manifest_path.exists());
        assert_eq!(result.selected_primary_source_id, "openclaw");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn safe_profile_merge_keeps_existing_prompt_and_applies_only_profile_lane() {
        let root = unique_temp_dir("loongclaw-import-safe-merge-profile-only");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- region: apac\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let summary = plan_import_sources(&discovery).expect("summary should succeed");
        let recommendation =
            recommend_primary_source(&summary).expect("recommendation should succeed");
        let output_path = root.join("loongclaw.toml");

        let mut existing = crate::config::LoongClawConfig::default();
        existing.cli.system_prompt_addendum = Some("Native LoongClaw prompt".to_owned());
        let existing_body = crate::config::render(&existing).expect("render existing config");
        fs::write(&output_path, existing_body).expect("write existing config");

        let result = apply_import_selection(&ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: ImportSelectionMode::SafeProfileMerge {
                primary_source_id: recommendation.source_id,
            },
            apply_external_skills_plan: false,
            external_skills_input_path: None,
        })
        .expect("safe profile merge should succeed");

        let output_string = output_path.display().to_string();
        let (_, merged_config) =
            crate::config::load(Some(&output_string)).expect("load merged config");
        assert_eq!(result.prompt_owner_source_id, None);
        assert_eq!(
            merged_config.cli.system_prompt_addendum.as_deref(),
            Some("Native LoongClaw prompt")
        );
        assert_eq!(
            merged_config.memory.profile,
            crate::config::MemoryProfile::ProfilePlusWindow
        );
        let profile_note = merged_config
            .memory
            .profile_note
            .as_deref()
            .expect("profile note should be present");
        assert!(profile_note.contains("role: release copilot"));
        assert!(profile_note.contains("region: apac"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn duplicate_source_types_get_distinct_ids_and_apply_selected_uses_requested_source() {
        let root = unique_temp_dir("loongclaw-import-duplicate-source-kind");
        fs::create_dir_all(&root).expect("create fixture root");

        let alpha_root = root.join("openclaw-alpha");
        fs::create_dir_all(&alpha_root).expect("create alpha root");
        write_file(&alpha_root, "SOUL.md", "# Soul\n\nAlpha prompt guidance.\n");
        write_file(&alpha_root, "IDENTITY.md", "# Identity\n\n- region: east\n");

        let beta_root = root.join("openclaw-beta");
        fs::create_dir_all(&beta_root).expect("create beta root");
        write_file(&beta_root, "SOUL.md", "# Soul\n\nBeta prompt guidance.\n");
        write_file(&beta_root, "IDENTITY.md", "# Identity\n\n- region: west\n");

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let summary = plan_import_sources(&discovery).expect("summary should succeed");
        assert_eq!(summary.plans.len(), 2);
        assert_ne!(summary.plans[0].source_id, summary.plans[1].source_id);
        assert!(summary.plans[0].source_id.starts_with("openclaw-"));
        assert!(summary.plans[1].source_id.starts_with("openclaw-"));

        let selected_source_id = summary.plans[1].source_id.clone();
        let output_path = root.join("loongclaw.toml");
        let original_body =
            crate::config::render(&crate::config::LoongClawConfig::default()).expect("render");
        fs::write(&output_path, original_body).expect("write original config");

        let result = apply_import_selection(&ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: ImportSelectionMode::SelectedSingleSource {
                source_id: selected_source_id.clone(),
            },
            apply_external_skills_plan: false,
            external_skills_input_path: None,
        })
        .expect("apply should succeed");

        let output_string = output_path.display().to_string();
        let (_, merged_config) =
            crate::config::load(Some(&output_string)).expect("load merged config");
        assert_eq!(result.selected_primary_source_id, selected_source_id);
        assert!(
            merged_config
                .cli
                .system_prompt_addendum
                .as_deref()
                .is_some_and(|value| value.contains("Beta prompt guidance")),
            "expected selected source prompt to be imported"
        );
        assert!(
            merged_config
                .memory
                .profile_note
                .as_deref()
                .is_some_and(|value| value.contains("region: west")),
            "expected selected source profile note to be imported"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn apply_import_selection_can_attach_external_skill_mapping() {
        let root = unique_temp_dir("loongclaw-import-apply-external-skills");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n",
        );
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let output_path = root.join("loongclaw.toml");

        let result = apply_import_selection(&ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: ImportSelectionMode::SelectedSingleSource {
                source_id: "openclaw".to_owned(),
            },
            apply_external_skills_plan: true,
            external_skills_input_path: Some(root.clone()),
        })
        .expect("apply should succeed");

        assert_eq!(result.external_skill_artifact_count, 1);
        assert_eq!(result.external_skill_entries_applied, 3);
        assert!(
            result.external_skills_manifest_path.is_some(),
            "expected external skills manifest path"
        );
        let output_string = output_path.display().to_string();
        let (_, merged_config) =
            crate::config::load(Some(&output_string)).expect("load merged config");
        let profile_note = merged_config
            .memory
            .profile_note
            .as_deref()
            .expect("profile note should be present");
        assert!(profile_note.contains("Imported External Skills Artifacts"));
        assert!(profile_note.contains("kind=skills_catalog"));
        let external_manifest = result
            .external_skills_manifest_path
            .as_ref()
            .expect("external skills manifest should be present");
        assert!(
            external_manifest.exists(),
            "external skills manifest should be written"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rollback_last_migration_restores_previous_config() {
        let root = unique_temp_dir("loongclaw-import-rollback");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let output_path = root.join("loongclaw.toml");
        let original_body =
            crate::config::render(&crate::config::LoongClawConfig::default()).expect("render");
        fs::write(&output_path, &original_body).expect("write original config");

        apply_import_selection(&ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: ImportSelectionMode::RecommendedSingleSource {
                source_id: "openclaw".to_owned(),
            },
            apply_external_skills_plan: false,
            external_skills_input_path: None,
        })
        .expect("apply should succeed");

        rollback_last_migration(&output_path).expect("rollback should succeed");
        assert_eq!(
            fs::read_to_string(&output_path).expect("read restored config"),
            original_body
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rollback_last_migration_falls_back_to_legacy_manifest_name() {
        let root = unique_temp_dir("loongclaw-import-rollback-legacy-manifest");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let output_path = root.join("loongclaw.toml");
        let original_body =
            crate::config::render(&crate::config::LoongClawConfig::default()).expect("render");
        fs::write(&output_path, &original_body).expect("write original config");

        let result = apply_import_selection(&ApplyImportSelection {
            discovery,
            output_path: output_path.clone(),
            mode: ImportSelectionMode::RecommendedSingleSource {
                source_id: "openclaw".to_owned(),
            },
            apply_external_skills_plan: false,
            external_skills_input_path: None,
        })
        .expect("apply should succeed");

        let legacy_manifest_path =
            migration_state_dir(&output_path).join("loongclaw.toml.last-import.json");
        fs::rename(&result.manifest_path, &legacy_manifest_path)
            .expect("rename manifest to legacy name");

        rollback_last_migration(&output_path)
            .expect("rollback should succeed from legacy manifest");
        assert_eq!(
            fs::read_to_string(&output_path).expect("read restored config"),
            original_body
        );

        fs::remove_dir_all(&root).ok();
    }
}
