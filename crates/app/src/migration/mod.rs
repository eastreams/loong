mod orchestrator;

use std::{collections::BTreeSet, fs, path::Path};

use crate::{
    config::{LoongClawConfig, MemoryProfile},
    prompt::DEFAULT_PROMPT_PACK_ID,
    CliResult,
};
use serde_json::Value;

pub use orchestrator::{
    discover_import_sources, DiscoveredImportSource, DiscoveryOptions, DiscoveryReport,
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
                        "{} contains active periodic tasks; LoongClaw does not auto-wire heartbeat jobs yet",
                        file.label
                    ));
                }
            }
        }
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

pub fn apply_import_plan(config: &mut LoongClawConfig, plan: &ImportPlan) {
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
    let files = collect_import_files(input_path)?;
    if files.is_empty() {
        return Ok(None);
    }

    let source = hint.unwrap_or_else(|| detect_source(input_path, &files));
    let mut found_files = Vec::new();
    let mut custom_prompt_files = 0usize;
    let mut custom_profile_files = 0usize;
    let mut warning_count = 0usize;

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
        normalized = normalized.replace(needle, "LoongClaw");
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LoongClawConfig;
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
    fn nanobot_stock_templates_nativeize_to_loongclaw_defaults() {
        let root = unique_temp_dir("loongclaw-import-nanobot-stock");
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
        let mut config = LoongClawConfig::default();
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
        let root = unique_temp_dir("loongclaw-import-customized");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. If you mention nanobot, say LoongClaw instead.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Name: My build copilot\n- Motto: updated by nanobot after every release\n",
        );

        let plan = plan_import_from_path(&root, Some(LegacyClawSource::Nanobot))
            .expect("plan should succeed");
        let mut config = LoongClawConfig::default();
        apply_import_plan(&mut config, &plan);

        assert_eq!(config.cli.prompt_pack_id(), Some(DEFAULT_PROMPT_PACK_ID));
        assert_eq!(config.memory.profile, MemoryProfile::ProfilePlusWindow);
        assert_eq!(
            config.cli.system_prompt_addendum.as_deref(),
            Some(
                "## Imported SOUL.md\n# Soul\n\nAlways prefer concise shell output. If you mention LoongClaw, say LoongClaw instead."
            )
        );
        assert_eq!(
            config.memory.profile_note.as_deref(),
            Some(
                "## Imported IDENTITY.md\n# Identity\n\n- Name: My build copilot\n- Motto: updated by LoongClaw after every release"
            )
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn zeroclaw_aieos_identity_is_promoted_into_profile_note() {
        let root = unique_temp_dir("loongclaw-import-zeroclaw-aieos");
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
        let mut config = LoongClawConfig::default();
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
        assert!(note.contains("LoongClaw Labs"));
        assert!(note.contains("privacy first"));

        fs::remove_dir_all(&root).ok();
    }
}
