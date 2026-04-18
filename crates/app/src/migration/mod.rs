mod merge;
mod orchestrator;

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    CliResult,
    config::{LoongConfig, MemoryProfile, active_cli_command_name},
    prompt::DEFAULT_PROMPT_PACK_ID,
};
use serde_json::Value;

pub use merge::{
    MergedProfilePlan, ProfileEntryLane, ProfileMergeConflict, ProfileMergeEntry,
    merge_profile_entries,
};
pub use orchestrator::{
    ApplyImportSelection, ApplyImportSelectionResult, DiscoveredImportSource, DiscoveryOptions,
    DiscoveryPlanSummary, DiscoveryReport, ImportSelectionMode, PlannedImportSource,
    PrimarySourceRecommendation, apply_import_selection, discover_import_sources,
    merge_profile_sources, plan_import_sources, recommend_primary_source, rollback_last_migration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyClawSource {
    Nanobot,
    OpenClaw,
    PicoClaw,
    ZeroClaw,
    NanoClaw,
    Unknown,
}

impl LegacyClawSource {
    pub fn as_id(self) -> &'static str {
        match self {
            Self::Nanobot => "nanobot",
            Self::OpenClaw => "openclaw",
            Self::PicoClaw => "picoclaw",
            Self::ZeroClaw => "zeroclaw",
            Self::NanoClaw => "nanoclaw",
            Self::Unknown => "auto",
        }
    }

    pub fn from_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Unknown),
            "nanobot" => Some(Self::Nanobot),
            "openclaw" => Some(Self::OpenClaw),
            "picoclaw" => Some(Self::PicoClaw),
            "zeroclaw" => Some(Self::ZeroClaw),
            "nanoclaw" => Some(Self::NanoClaw),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    pub source: LegacyClawSource,
    pub system_prompt_addendum: Option<String>,
    pub profile_note: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalSkillArtifactKind {
    SkillsCatalog,
    SkillsLock,
    CodexSkillsDir,
    ClaudeSkillsDir,
    SkillsDir,
}

impl ExternalSkillArtifactKind {
    pub fn as_id(self) -> &'static str {
        match self {
            Self::SkillsCatalog => "skills_catalog",
            Self::SkillsLock => "skills_lock",
            Self::CodexSkillsDir => "codex_skills_dir",
            Self::ClaudeSkillsDir => "claude_skills_dir",
            Self::SkillsDir => "skills_dir",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSkillArtifact {
    pub kind: ExternalSkillArtifactKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExternalSkillMappingPlan {
    pub input_path: PathBuf,
    pub artifacts: Vec<ExternalSkillArtifact>,
    pub declared_skills: Vec<String>,
    pub locked_skills: Vec<String>,
    pub resolved_skills: Vec<String>,
    pub profile_note_addendum: Option<String>,
    pub warnings: Vec<String>,
}

pub fn plan_external_skill_mapping(input_path: &Path) -> ExternalSkillMappingPlan {
    let artifacts = detect_external_skill_artifacts(input_path);
    let mut warnings = artifacts
        .iter()
        .map(external_skill_warning)
        .collect::<Vec<_>>();
    let declared_skills = collect_declared_skills(&artifacts, &mut warnings);
    let locked_skills = collect_locked_skills(&artifacts, &mut warnings);
    let resolved_skills = merge_resolved_skills(&declared_skills, &locked_skills);
    ExternalSkillMappingPlan {
        input_path: input_path.to_path_buf(),
        profile_note_addendum: render_external_skill_profile_note_addendum(
            &artifacts,
            &declared_skills,
            &locked_skills,
            &resolved_skills,
        ),
        artifacts,
        declared_skills,
        locked_skills,
        resolved_skills,
        warnings,
    }
}

pub fn apply_external_skill_mapping(
    config: &mut LoongConfig,
    plan: &ExternalSkillMappingPlan,
) -> usize {
    let Some(addendum) = plan.profile_note_addendum.as_deref() else {
        return 0;
    };
    let Some(merged) = merge_profile_note_addendum(config.memory.profile_note.as_deref(), addendum)
    else {
        return 0;
    };

    config.memory.profile = MemoryProfile::ProfilePlusWindow;
    config.memory.profile_note = Some(merged);
    trimmed_bullet_line_count(addendum)
}

pub fn plan_import_from_path(
    input_path: &Path,
    hint: Option<LegacyClawSource>,
) -> CliResult<ImportPlan> {
    let files = collect_import_files(input_path)?;
    let source = hint.unwrap_or_else(|| detect_source(input_path, &files));
    let mut prompt_blocks = Vec::new();
    let mut profile_blocks = Vec::new();
    let mut warnings = Vec::new();

    for file in files {
        match file.kind {
            ImportFileKind::Prompt => {
                if is_stock_template(source, &file.label, &file.content) {
                    continue;
                }
                prompt_blocks.push(format!(
                    "## Imported {}\n{}",
                    file.label,
                    normalize_brand_references(&file.content)
                ));
            }
            ImportFileKind::Profile => {
                if is_stock_template(source, &file.label, &file.content) {
                    continue;
                }
                profile_blocks.push(format!(
                    "## Imported {}\n{}",
                    file.label,
                    normalize_brand_references(&file.content)
                ));
            }
            ImportFileKind::AieosJson => {
                if let Some(rendered) = render_aieos_profile_note(&file.content)? {
                    profile_blocks.push(format!("## Imported {}\n{}", file.label, rendered));
                }
            }
            ImportFileKind::Heartbeat => {
                if heartbeat_has_active_tasks(&file.content) {
                    warnings.push(format!(
                        "{} contains active periodic tasks; Loong does not auto-wire heartbeat jobs yet",
                        file.label
                    ));
                }
            }
        }
    }

    for warning in build_external_skill_warnings(input_path) {
        warnings.push(warning);
    }

    if prompt_blocks.is_empty()
        && profile_blocks.is_empty()
        && warnings.is_empty()
        && matches!(source, LegacyClawSource::Unknown)
    {
        return Err(format!(
            "no supported migration content found under {}",
            input_path.display()
        ));
    }

    Ok(ImportPlan {
        source,
        system_prompt_addendum: join_blocks(prompt_blocks),
        profile_note: join_blocks(profile_blocks),
        warnings,
    })
}

pub fn apply_import_plan(config: &mut LoongConfig, plan: &ImportPlan) {
    config.cli.prompt_pack_id = Some(DEFAULT_PROMPT_PACK_ID.to_owned());
    config.cli.system_prompt_addendum = plan.system_prompt_addendum.clone();
    config.cli.refresh_native_system_prompt();
    config.memory.profile = MemoryProfile::ProfilePlusWindow;
    config.memory.profile_note = plan.profile_note.clone();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportPathInspection {
    pub source: LegacyClawSource,
    pub found_files: Vec<String>,
    pub custom_prompt_files: usize,
    pub custom_profile_files: usize,
    pub warning_count: usize,
}

pub(crate) fn inspect_import_path(
    input_path: &Path,
    hint: Option<LegacyClawSource>,
) -> CliResult<Option<ImportPathInspection>> {
    let external_skill_artifacts = detect_external_skill_artifacts(input_path);
    let files = collect_import_files(input_path)?;
    if files.is_empty() && external_skill_artifacts.is_empty() {
        return Ok(None);
    }

    let source = hint.unwrap_or_else(|| detect_source(input_path, &files));
    let mut found_files = Vec::new();
    let mut custom_prompt_files = 0usize;
    let mut custom_profile_files = 0usize;
    let mut warning_count = 0usize;

    if !external_skill_artifacts.is_empty() {
        found_files.extend(external_skill_artifacts.iter().map(|artifact| {
            format!(
                "external_skill:{}:{}",
                artifact.kind.as_id(),
                artifact.path.display()
            )
        }));
        warning_count = warning_count.saturating_add(external_skill_artifacts.len());
    }

    for file in files {
        found_files.push(file.label.clone());
        match file.kind {
            ImportFileKind::Prompt => {
                if !is_stock_template(source, &file.label, &file.content) {
                    custom_prompt_files = custom_prompt_files.saturating_add(1);
                }
            }
            ImportFileKind::Profile => {
                if !is_stock_template(source, &file.label, &file.content) {
                    custom_profile_files = custom_profile_files.saturating_add(1);
                }
            }
            ImportFileKind::AieosJson => {
                if render_aieos_profile_note(&file.content)?.is_some() {
                    custom_profile_files = custom_profile_files.saturating_add(1);
                }
            }
            ImportFileKind::Heartbeat => {
                if heartbeat_has_active_tasks(&file.content) {
                    warning_count = warning_count.saturating_add(1);
                }
            }
        }
    }

    if custom_prompt_files == 0 && custom_profile_files == 0 && warning_count == 0 {
        return Ok(None);
    }

    found_files.sort();
    found_files.dedup();

    Ok(Some(ImportPathInspection {
        source,
        found_files,
        custom_prompt_files,
        custom_profile_files,
        warning_count,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFileKind {
    Prompt,
    Profile,
    AieosJson,
    Heartbeat,
}

#[derive(Debug, Clone)]
struct ImportFile {
    label: String,
    kind: ImportFileKind,
    content: String,
}

fn collect_import_files(input_path: &Path) -> CliResult<Vec<ImportFile>> {
    if input_path.is_file() {
        return read_single_import_file(input_path)
            .map(|file| file.into_iter().collect())
            .map_err(|error| {
                format!(
                    "failed to read migration input {}: {error}",
                    input_path.display()
                )
            });
    }

    if !input_path.exists() {
        return Err(format!(
            "migration input does not exist: {}",
            input_path.display()
        ));
    }

    let mut roots = vec![input_path.to_path_buf()];
    let workspace_root = input_path.join("workspace");
    if workspace_root.is_dir() {
        roots.push(workspace_root);
    }

    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for root in roots {
        for relative in [
            "AGENTS.md",
            "SOUL.md",
            "TOOLS.md",
            "IDENTITY.md",
            "USER.md",
            "BOOTSTRAP.md",
            "HEARTBEAT.md",
            "MEMORY.md",
            "memory/MEMORY.md",
            "identity.json",
            "CLAUDE.md",
            "groups/main/CLAUDE.md",
            "groups/global/CLAUDE.md",
        ] {
            let path = root.join(relative);
            if !path.is_file() {
                continue;
            }
            let canonical = path
                .canonicalize()
                .unwrap_or_else(|_| path.clone())
                .display()
                .to_string();
            if !seen.insert(canonical) {
                continue;
            }
            if let Some(file) = read_single_import_file(&path).map_err(|error| {
                format!("failed to read migration file {}: {error}", path.display())
            })? {
                files.push(file);
            }
        }
    }
    Ok(files)
}

fn read_single_import_file(path: &Path) -> Result<Option<ImportFile>, std::io::Error> {
    let content = fs::read_to_string(path)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let kind = classify_file_kind(path, file_name)?;
    Ok(Some(ImportFile {
        label: relative_label(path),
        kind,
        content: trimmed.to_owned(),
    }))
}

fn classify_file_kind(path: &Path, file_name: &str) -> Result<ImportFileKind, std::io::Error> {
    if file_name.eq_ignore_ascii_case("identity.json") {
        return Ok(ImportFileKind::AieosJson);
    }
    if file_name.eq_ignore_ascii_case("HEARTBEAT.md") {
        return Ok(ImportFileKind::Heartbeat);
    }
    if file_name.eq_ignore_ascii_case("CLAUDE.md") {
        return Ok(ImportFileKind::Prompt);
    }
    if matches!(
        file_name,
        "AGENTS.md" | "SOUL.md" | "TOOLS.md" | "BOOTSTRAP.md"
    ) {
        return Ok(ImportFileKind::Prompt);
    }
    if matches!(file_name, "IDENTITY.md" | "USER.md" | "MEMORY.md") {
        return Ok(ImportFileKind::Profile);
    }

    let _ = path;
    Ok(ImportFileKind::Profile)
}

fn relative_label(path: &Path) -> String {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let parent_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|value| value.to_string_lossy().to_string());
    if matches!(
        parent_name.as_deref(),
        Some("memory") | Some("main") | Some("global")
    ) {
        return format!("{}/{}", parent_name.unwrap_or_default(), file_name);
    }
    file_name
}

fn detect_source(input_path: &Path, files: &[ImportFile]) -> LegacyClawSource {
    let path_text = input_path.display().to_string().to_ascii_lowercase();
    if path_text.contains("nanobot") {
        return LegacyClawSource::Nanobot;
    }
    if path_text.contains("openclaw") {
        return LegacyClawSource::OpenClaw;
    }
    if path_text.contains("picoclaw") {
        return LegacyClawSource::PicoClaw;
    }
    if path_text.contains("zeroclaw") {
        return LegacyClawSource::ZeroClaw;
    }
    if path_text.contains("nanoclaw") {
        return LegacyClawSource::NanoClaw;
    }

    for file in files {
        let lower = file.content.to_ascii_lowercase();
        if lower.contains("i am nanobot")
            || lower.contains("your nanobot agent")
            || lower.contains("updated by nanobot")
        {
            return LegacyClawSource::Nanobot;
        }
        if lower.contains("i am picoclaw")
            || lower.contains("github.com/sipeed/picoclaw")
            || lower.contains("picoclaw 🦞")
        {
            return LegacyClawSource::PicoClaw;
        }
        if lower.contains("openclaw workspace")
            || lower.contains("folder is the assistant's working directory")
            || lower.contains("first run ritual")
        {
            return LegacyClawSource::OpenClaw;
        }
        if lower.contains("3mb binary. zero bloat.")
            || lower.contains("you wake up fresh each session")
            || lower.contains("\"format\": \"aieos\"")
        {
            return LegacyClawSource::ZeroClaw;
        }
        if lower.contains("you are andy, a personal assistant") {
            return LegacyClawSource::NanoClaw;
        }
    }

    LegacyClawSource::Unknown
}

fn is_stock_template(source: LegacyClawSource, label: &str, content: &str) -> bool {
    let normalized = content.trim();

    match source {
        LegacyClawSource::Nanobot => {
            (label.ends_with("SOUL.md")
                && normalized.contains("I am nanobot 🐈, a personal AI assistant."))
                || (label.ends_with("HEARTBEAT.md")
                    && normalized
                        .contains("This file is checked every 30 minutes by your nanobot agent."))
                || (label.ends_with("MEMORY.md")
                    && normalized.contains("automatically updated by nanobot"))
        }
        LegacyClawSource::OpenClaw => {
            (label.ends_with("AGENTS.md")
                && normalized.contains("# AGENTS.md - OpenClaw Workspace"))
                || (label.ends_with("SOUL.md")
                    && normalized.contains("# SOUL.md - Persona & Boundaries"))
                || (label.ends_with("IDENTITY.md")
                    && normalized.contains("# IDENTITY.md - Agent Identity"))
                || (label.ends_with("USER.md") && normalized.contains("# USER.md - User Profile"))
                || (label.ends_with("BOOTSTRAP.md")
                    && normalized.contains("# BOOTSTRAP.md - First Run Ritual"))
        }
        LegacyClawSource::PicoClaw => (label.ends_with("SOUL.md")
            && normalized.contains("I am picoclaw, a lightweight AI assistant powered by AI."))
            || (label.ends_with("AGENTS.md") && normalized.contains("# Agent Instructions"))
            || (label.ends_with("IDENTITY.md")
                && normalized.contains(
                    "Ultra-lightweight personal AI assistant written in Go, inspired by nanobot.",
                ))
            || (label.ends_with("USER.md")
                && normalized.contains("Information about user goes here."))
            || (label.ends_with("MEMORY.md")
                && normalized.contains(
                    "This file stores important information that should persist across sessions.",
                )),
        LegacyClawSource::ZeroClaw => {
            (label.ends_with("SOUL.md")
                && normalized.contains("*You're not a chatbot. You're becoming someone.*")
                && normalized.contains("Built in Rust. 3MB binary. Zero bloat."))
                || (label.ends_with("AGENTS.md")
                    && normalized.contains("## Every Session (required)"))
                || (label.ends_with("IDENTITY.md")
                    && normalized.contains(
                        "Update this file as you evolve. Your identity is yours to shape.",
                    ))
                || (label.ends_with("HEARTBEAT.md")
                    && normalized.contains(
                        "Keep this file empty (or with only comments) to skip heartbeat work.",
                    ))
                || (label.ends_with("MEMORY.md")
                    && normalized
                        .contains("*Your curated memories. The distilled essence, not raw logs.*"))
        }
        LegacyClawSource::NanoClaw => {
            label.ends_with("CLAUDE.md")
                && normalized.contains("You are Andy, a personal assistant.")
        }
        LegacyClawSource::Unknown => false,
    }
}

fn heartbeat_has_active_tasks(content: &str) -> bool {
    content
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("- ") && line.len() > 2)
}

fn build_external_skill_warnings(input_path: &Path) -> Vec<String> {
    plan_external_skill_mapping(input_path).warnings
}

fn detect_external_skill_artifacts(input_path: &Path) -> Vec<ExternalSkillArtifact> {
    let mut artifacts = Vec::new();
    let mut seen = BTreeSet::new();
    for root in external_skill_probe_roots(input_path) {
        for (relative, kind) in [
            ("SKILLS.md", ExternalSkillArtifactKind::SkillsCatalog),
            ("skills-lock.json", ExternalSkillArtifactKind::SkillsLock),
            (".codex/skills", ExternalSkillArtifactKind::CodexSkillsDir),
            (".claude/skills", ExternalSkillArtifactKind::ClaudeSkillsDir),
            ("skills", ExternalSkillArtifactKind::SkillsDir),
        ] {
            let path = root.join(relative);
            if path.is_file() || path.is_dir() {
                let canonical = path.canonicalize().unwrap_or(path);
                let key = canonical.display().to_string();
                if seen.insert(key) {
                    artifacts.push(ExternalSkillArtifact {
                        kind,
                        path: canonical,
                    });
                }
            }
        }
    }
    artifacts.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.kind.as_id().cmp(right.kind.as_id()))
    });
    artifacts
}

fn external_skill_probe_roots(input_path: &Path) -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();
    if input_path.is_file() {
        if let Some(parent) = input_path.parent() {
            roots.insert(parent.to_path_buf());
        }
    } else {
        roots.insert(input_path.to_path_buf());
        let workspace_root = input_path.join("workspace");
        if workspace_root.is_dir() {
            roots.insert(workspace_root);
        }
    }
    roots.into_iter().collect()
}

fn external_skill_warning(artifact: &ExternalSkillArtifact) -> String {
    format!(
        "detected external skills artifact `{}` ({}); Loong imports prompt/profile content by default, and installable local skills can be bridged into the managed runtime with `{} migrate --mode apply_selected --apply-external-skills-plan` or the explicit external skills lifecycle (`fetch` -> `install` -> `list` -> `invoke`)",
        artifact.path.display(),
        artifact.kind.as_id(),
        active_cli_command_name()
    )
}

fn collect_declared_skills(
    artifacts: &[ExternalSkillArtifact],
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut collected = BTreeSet::new();
    for artifact in artifacts {
        match artifact.kind {
            ExternalSkillArtifactKind::SkillsCatalog => {
                let content = match fs::read_to_string(&artifact.path) {
                    Ok(content) => content,
                    Err(error) => {
                        warnings.push(format!(
                            "failed to read declared skills catalog {}: {error}",
                            artifact.path.display()
                        ));
                        continue;
                    }
                };
                for skill in parse_skills_markdown_entries(&content) {
                    collected.insert(skill);
                }
            }
            ExternalSkillArtifactKind::CodexSkillsDir
            | ExternalSkillArtifactKind::ClaudeSkillsDir
            | ExternalSkillArtifactKind::SkillsDir => {
                for skill in list_directory_skill_entries(&artifact.path, warnings) {
                    collected.insert(skill);
                }
            }
            ExternalSkillArtifactKind::SkillsLock => {}
        }
    }
    collected.into_iter().collect()
}

fn collect_locked_skills(
    artifacts: &[ExternalSkillArtifact],
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut collected = BTreeSet::new();
    for artifact in artifacts {
        if artifact.kind != ExternalSkillArtifactKind::SkillsLock {
            continue;
        }

        let content = match fs::read_to_string(&artifact.path) {
            Ok(content) => content,
            Err(error) => {
                warnings.push(format!(
                    "failed to read skills lock {}: {error}",
                    artifact.path.display()
                ));
                continue;
            }
        };
        let value = match serde_json::from_str::<Value>(&content) {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!(
                    "failed to parse skills lock {}: {error}",
                    artifact.path.display()
                ));
                continue;
            }
        };
        for skill in parse_skills_lock_entries(&value) {
            collected.insert(skill);
        }
    }
    collected.into_iter().collect()
}

fn merge_resolved_skills(declared: &[String], locked: &[String]) -> Vec<String> {
    let mut resolved = BTreeSet::new();
    for skill in declared.iter().chain(locked.iter()) {
        resolved.insert(skill.clone());
    }
    resolved.into_iter().collect()
}

fn parse_skills_markdown_entries(content: &str) -> Vec<String> {
    let mut skills = BTreeSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let candidate = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .map(str::trim);
        let Some(raw_entry) = candidate else {
            continue;
        };
        if let Some(skill) = normalize_skill_reference(raw_entry) {
            skills.insert(skill);
        }
    }
    skills.into_iter().collect()
}

fn list_directory_skill_entries(path: &Path, warnings: &mut Vec<String>) -> Vec<String> {
    if !path.is_dir() {
        return Vec::new();
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!(
                "failed to enumerate skills directory {}: {error}",
                path.display()
            ));
            return Vec::new();
        }
    };

    let mut skills = BTreeSet::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!(
                    "failed to read skills directory entry under {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        if let Some(skill) = entry_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(normalize_skill_reference)
        {
            skills.insert(skill);
        }
    }
    skills.into_iter().collect()
}

fn parse_skills_lock_entries(value: &Value) -> Vec<String> {
    let mut skills = BTreeSet::new();
    extract_skill_refs_from_lock_value(value, &mut skills);
    skills.into_iter().collect()
}

fn extract_skill_refs_from_lock_value(value: &Value, skills: &mut BTreeSet<String>) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                match key.as_str() {
                    "skills" | "skill" | "skill_id" | "skillId" | "id" | "name" | "slug" => {
                        collect_skill_refs_from_value(nested, skills);
                    }
                    _ => extract_skill_refs_from_lock_value(nested, skills),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                extract_skill_refs_from_lock_value(item, skills);
            }
        }
        Value::String(raw) => {
            if let Some(skill) = normalize_skill_reference(raw) {
                skills.insert(skill);
            }
        }
        _ => {}
    }
}

fn collect_skill_refs_from_value(value: &Value, skills: &mut BTreeSet<String>) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match value {
        Value::String(raw) => {
            if let Some(skill) = normalize_skill_reference(raw) {
                skills.insert(skill);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_skill_refs_from_value(item, skills);
            }
        }
        Value::Object(object) => {
            for nested in object.values() {
                collect_skill_refs_from_value(nested, skills);
            }
        }
        _ => {}
    }
}

fn normalize_skill_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let unwrapped = trimmed.trim_matches('`').trim();
    let mut candidate = if unwrapped.starts_with('[') {
        unwrapped
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']'))
            .map(|(label, _)| label)
            .unwrap_or(unwrapped)
    } else {
        unwrapped
    };

    if let Some((head, _)) = candidate.split_once("](") {
        candidate = head;
    }
    if let Some((head, _)) = candidate.split_once(char::is_whitespace) {
        candidate = head;
    }
    let normalized = candidate
        .trim_matches(|ch: char| matches!(ch, ',' | ';' | '.' | '"' | '\'' | ')' | '('))
        .trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized.starts_with('#') {
        return None;
    }
    if normalized.len() > 120 {
        return None;
    }

    let canonical = normalized.to_ascii_lowercase();
    if canonical
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.'))
    {
        return Some(canonical);
    }
    None
}

fn render_external_skill_profile_note_addendum(
    artifacts: &[ExternalSkillArtifact],
    declared_skills: &[String],
    locked_skills: &[String],
    resolved_skills: &[String],
) -> Option<String> {
    if artifacts.is_empty() {
        return None;
    }

    let mut lines = vec!["## Imported External Skills Artifacts".to_owned()];
    for artifact in artifacts {
        lines.push(format!(
            "- kind={} label={}",
            artifact.kind.as_id(),
            external_skill_artifact_label(artifact)
        ));
    }
    if !declared_skills.is_empty() {
        lines.push("## Imported External Skills Declared".to_owned());
        for skill in declared_skills {
            lines.push(format!("- skill={skill} source=declared"));
        }
    }
    if !locked_skills.is_empty() {
        lines.push("## Imported External Skills Locked".to_owned());
        for skill in locked_skills {
            lines.push(format!("- skill={skill} source=lock"));
        }
    }
    if !resolved_skills.is_empty() {
        lines.push("## Imported External Skills Resolved".to_owned());
        for skill in resolved_skills {
            lines.push(format!("- skill={skill} source=resolved"));
        }
    }
    Some(lines.join("\n"))
}

fn external_skill_artifact_label(artifact: &ExternalSkillArtifact) -> String {
    artifact
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(artifact.kind.as_id())
        .to_owned()
}

pub fn merge_profile_note_addendum(existing: Option<&str>, addendum: &str) -> Option<String> {
    let trimmed_addendum = addendum.trim();
    if trimmed_addendum.is_empty() {
        return None;
    }

    match existing {
        None => Some(trimmed_addendum.to_owned()),
        Some(current) => {
            if current.contains(trimmed_addendum) {
                return None;
            }
            if current.trim().is_empty() {
                return Some(trimmed_addendum.to_owned());
            }
            Some(format!("{}\n\n{trimmed_addendum}", current.trim_end()))
        }
    }
}

fn trimmed_bullet_line_count(content: &str) -> usize {
    content
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- "))
        .count()
}

fn render_aieos_profile_note(content: &str) -> CliResult<Option<String>> {
    let value = serde_json::from_str::<Value>(content)
        .map_err(|error| format!("failed to parse imported identity.json: {error}"))?;
    let identity = value.get("identity").unwrap_or(&value);
    let mut lines = Vec::new();

    if let Some(name) = json_string(identity, &["names", "first"]) {
        lines.push(format!(
            "- names.first: {}",
            normalize_brand_references(name)
        ));
    }
    if let Some(bio) = json_string(identity, &["bio"]) {
        lines.push(format!("- bio: {}", normalize_brand_references(bio)));
    }
    if let Some(values) = json_string_array(identity, &["values"]) {
        lines.push(format!(
            "- values: {}",
            values
                .into_iter()
                .map(|value| normalize_brand_references(&value))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if lines.is_empty() {
        return Ok(None);
    }

    Ok(Some(lines.join("\n")))
}

fn json_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str()
}

fn json_string_array(value: &Value, path: &[&str]) -> Option<Vec<String>> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    let items = current.as_array()?;
    let values = items
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    Some(values)
}

fn join_blocks(blocks: Vec<String>) -> Option<String> {
    let joined = blocks
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if joined.is_empty() {
        return None;
    }
    Some(joined)
}

fn normalize_brand_references(content: &str) -> String {
    let mut normalized = content.to_owned();
    for needle in [
        "nanobot", "Nanobot", "NanoBot", "openclaw", "OpenClaw", "picoclaw", "PicoClaw",
        "zeroclaw", "ZeroClaw", "nanoclaw", "NanoClaw",
    ] {
        normalized = replace_identity_token(&normalized, needle, "Loong");
    }
    normalized
}

fn replace_identity_token(content: &str, needle: &str, replacement: &str) -> String {
    let mut normalized = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(relative) = content[cursor..].find(needle) {
        let start = cursor + relative;
        let end = start + needle.len();
        normalized.push_str(&content[cursor..start]);
        if should_replace_identity_match(content.as_bytes(), start, end) {
            normalized.push_str(replacement);
        } else {
            normalized.push_str(&content[start..end]);
        }
        cursor = end;
    }

    normalized.push_str(&content[cursor..]);
    normalized
}

fn should_replace_identity_match(content: &[u8], start: usize, end: usize) -> bool {
    is_identity_boundary(content, start, true) && is_identity_boundary(content, end, false)
}

fn is_identity_boundary(content: &[u8], index: usize, leading: bool) -> bool {
    let adjacent = if leading {
        index
            .checked_sub(1)
            .and_then(|offset| content.get(offset).copied())
    } else {
        content.get(index).copied()
    };
    let beyond = if leading {
        index
            .checked_sub(2)
            .and_then(|offset| content.get(offset).copied())
    } else {
        content.get(index + 1).copied()
    };

    match adjacent {
        None => true,
        Some(byte)
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'/' | b'\\' | b'-') =>
        {
            false
        }
        Some(b'.') => {
            beyond.is_none_or(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        }
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LoongConfig;
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
    fn nanobot_stock_templates_nativeize_to_loong_defaults() {
        let root = unique_temp_dir("loong-import-nanobot-stock");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nI am nanobot 🐈, a personal AI assistant.\n",
        );
        write_file(
            &root,
            "HEARTBEAT.md",
            "# Heartbeat Tasks\n\nThis file is checked every 30 minutes by your nanobot agent.\n",
        );
        write_file(
            &root,
            "memory/MEMORY.md",
            "# Long-term Memory\n\n*This file is automatically updated by nanobot when important information should be remembered.*\n",
        );

        let plan = plan_import_from_path(&root, None).expect("plan should succeed");
        let mut config = LoongConfig::default();
        apply_import_plan(&mut config, &plan);

        assert_eq!(plan.source, LegacyClawSource::Nanobot);
        assert_eq!(config.cli.prompt_pack_id(), Some(DEFAULT_PROMPT_PACK_ID));
        assert_eq!(config.memory.profile, MemoryProfile::ProfilePlusWindow);
        assert!(config.cli.system_prompt_addendum.is_none());
        assert!(config.memory.profile_note.is_none());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn custom_prompt_and_memory_content_are_preserved_with_brand_remap() {
        let root = unique_temp_dir("loong-import-customized");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. If you mention nanobot, say Loong instead.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Name: My build copilot\n- Motto: updated by nanobot after every release\n",
        );

        let plan = plan_import_from_path(&root, Some(LegacyClawSource::Nanobot))
            .expect("plan should succeed");
        let mut config = LoongConfig::default();
        apply_import_plan(&mut config, &plan);

        assert_eq!(config.cli.prompt_pack_id(), Some(DEFAULT_PROMPT_PACK_ID));
        assert_eq!(config.memory.profile, MemoryProfile::ProfilePlusWindow);
        assert_eq!(
            config.cli.system_prompt_addendum.as_deref(),
            Some(
                "## Imported SOUL.md\n# Soul\n\nAlways prefer concise shell output. If you mention Loong, say Loong instead."
            )
        );
        assert_eq!(
            config.memory.profile_note.as_deref(),
            Some(
                "## Imported IDENTITY.md\n# Identity\n\n- Name: My build copilot\n- Motto: updated by Loong after every release"
            )
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn brand_remap_preserves_paths_urls_and_identifiers() {
        let normalized = normalize_brand_references(
            "I am nanobot. Use your nanobot agent for deploys.\n\
             Repo: github.com/openclaw-ai/openclaw\n\
             Path: ~/.config/openclaw/IDENTITY.md\n\
             File: openclaw.toml\n\
             Key: openclaw_agent_id",
        );

        assert!(normalized.contains("I am Loong."));
        assert!(normalized.contains("your Loong agent"));
        assert!(normalized.contains("github.com/openclaw-ai/openclaw"));
        assert!(normalized.contains("~/.config/openclaw/IDENTITY.md"));
        assert!(normalized.contains("openclaw.toml"));
        assert!(normalized.contains("openclaw_agent_id"));
    }

    #[test]
    fn zeroclaw_aieos_identity_is_promoted_into_profile_note() {
        let root = unique_temp_dir("loong-import-zeroclaw-aieos");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "identity.json",
            r#"{
  "identity": {
    "names": { "first": "Nova" },
    "bio": "Built by ZeroClaw Labs for safe, direct help",
    "values": ["privacy first", "fast execution"]
  }
}"#,
        );

        let plan = plan_import_from_path(&root, Some(LegacyClawSource::ZeroClaw))
            .expect("plan should succeed");
        let mut config = LoongConfig::default();
        apply_import_plan(&mut config, &plan);

        assert_eq!(plan.source, LegacyClawSource::ZeroClaw);
        assert_eq!(config.cli.prompt_pack_id(), Some(DEFAULT_PROMPT_PACK_ID));
        assert_eq!(config.memory.profile, MemoryProfile::ProfilePlusWindow);
        let note = config
            .memory
            .profile_note
            .as_deref()
            .expect("profile note should be present");
        assert!(note.contains("Nova"));
        assert!(note.contains("Loong Labs"));
        assert!(note.contains("privacy first"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn external_skill_artifacts_emit_warnings_in_import_plan() {
        let root = unique_temp_dir("loong-import-external-skill-warning");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nPrefer concise shell output with clear reasoning.\n",
        );
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");
        fs::create_dir_all(root.join(".codex/skills")).expect("create skills dir");

        let plan = plan_import_from_path(&root, Some(LegacyClawSource::Unknown))
            .expect("plan should succeed");
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("external skills artifact")),
            "expected at least one external skills warning"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn plan_external_skill_mapping_builds_profile_note_addendum() {
        let root = unique_temp_dir("loong-import-external-skill-map");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");
        fs::create_dir_all(root.join(".codex/skills")).expect("create codex skills dir");
        write_file(
            &root,
            "skills-lock.json",
            "{ \"version\": 1, \"skills\": [\"custom/skill-a\"] }\n",
        );

        let mapping = plan_external_skill_mapping(&root);
        assert_eq!(mapping.input_path, root);
        assert_eq!(mapping.artifacts.len(), 3);
        assert_eq!(mapping.warnings.len(), 3);
        assert_eq!(mapping.declared_skills, vec!["custom/skill-a".to_owned()]);
        assert_eq!(mapping.locked_skills, vec!["custom/skill-a".to_owned()]);
        assert_eq!(mapping.resolved_skills, vec!["custom/skill-a".to_owned()]);
        let addendum = mapping
            .profile_note_addendum
            .as_deref()
            .expect("profile note addendum should exist");
        assert!(addendum.contains("Imported External Skills Artifacts"));
        assert!(addendum.contains("kind=skills_catalog"));
        assert!(addendum.contains("kind=skills_lock"));
        assert!(addendum.contains("kind=codex_skills_dir"));
        assert!(
            !addendum.contains(root.display().to_string().as_str()),
            "profile note addendum should not leak absolute local paths"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn external_skill_warning_points_to_explicit_runtime_lifecycle() {
        let warning = external_skill_warning(&ExternalSkillArtifact {
            kind: ExternalSkillArtifactKind::SkillsDir,
            path: PathBuf::from("/tmp/demo/skills"),
        });
        assert!(warning.contains("apply_selected"));
        assert!(warning.contains("apply-external-skills-plan"));
        assert!(warning.contains("fetch"));
        assert!(warning.contains("install"));
        assert!(warning.contains("invoke"));
    }

    #[test]
    fn apply_external_skill_mapping_appends_profile_note_once() {
        let root = unique_temp_dir("loong-import-external-skill-apply");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");

        let plan = plan_external_skill_mapping(&root);
        let mut config = LoongConfig::default();
        config.memory.profile_note = Some("## Imported IDENTITY.md\n- tone steady".to_owned());

        let first_applied = apply_external_skill_mapping(&mut config, &plan);
        assert_eq!(first_applied, 3);
        let first_note = config
            .memory
            .profile_note
            .clone()
            .expect("profile note should exist");
        assert!(first_note.contains("Imported External Skills Artifacts"));

        let second_applied = apply_external_skill_mapping(&mut config, &plan);
        assert_eq!(second_applied, 0, "duplicate addendum should not re-append");
        assert_eq!(
            config.memory.profile_note.as_deref(),
            Some(first_note.as_str()),
            "profile note should remain stable after duplicate apply"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn external_skill_profile_note_addendum_does_not_embed_absolute_paths() {
        let root = unique_temp_dir("loong-import-external-skill-redacted-paths");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");
        fs::create_dir_all(root.join(".codex/skills")).expect("create codex skills dir");

        let mapping = plan_external_skill_mapping(&root);
        let addendum = mapping
            .profile_note_addendum
            .as_deref()
            .expect("profile note addendum should exist");

        assert!(
            !addendum.contains(&root.display().to_string()),
            "profile note addendum must not leak absolute import roots: {addendum}"
        );
        assert!(
            !addendum.contains("/private/")
                && !addendum.contains("/Users/")
                && !addendum.contains("\\\\")
                && !addendum.contains(":\\"),
            "profile note addendum must not contain absolute local filesystem paths: {addendum}"
        );

        fs::remove_dir_all(&root).ok();
    }
}
