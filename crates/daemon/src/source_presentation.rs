use std::path::{Path, PathBuf};

use crate::migration::ImportSourceKind;

const RECOMMENDED_PLAN_SOURCE_LABEL: &str = "recommended import plan";
const ENVIRONMENT_SOURCE_LABEL: &str = "your current environment";
const WORKSPACE_SOURCE_LABEL: &str = "workspace";
const CURRENT_ONBOARDING_DRAFT_SOURCE_LABEL: &str = "current onboarding draft";
const WORKSPACE_GUIDANCE_ROLLUP_LABEL: &str = "workspace guidance";
const SUGGESTED_STARTING_POINT_LABEL: &str = "suggested starting point";
const EXISTING_CONFIG_SOURCE_PREFIX: &str = "existing config at ";
const CODEX_CONFIG_SOURCE_PREFIX: &str = "Codex config at ";

fn display_path(path: &Path) -> String {
    dunce::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

pub const fn recommended_plan_source_label() -> &'static str {
    RECOMMENDED_PLAN_SOURCE_LABEL
}

pub const fn environment_source_label() -> &'static str {
    ENVIRONMENT_SOURCE_LABEL
}

pub const fn workspace_source_label() -> &'static str {
    WORKSPACE_SOURCE_LABEL
}

pub const fn current_onboarding_draft_source_label() -> &'static str {
    CURRENT_ONBOARDING_DRAFT_SOURCE_LABEL
}

pub const fn workspace_guidance_rollup_label() -> &'static str {
    WORKSPACE_GUIDANCE_ROLLUP_LABEL
}

pub const fn suggested_starting_point_label() -> &'static str {
    SUGGESTED_STARTING_POINT_LABEL
}

pub fn existing_loong_config_source_label(path: &Path) -> String {
    let rendered_path = display_path(path);
    format!("{EXISTING_CONFIG_SOURCE_PREFIX}{rendered_path}")
}

pub fn codex_config_source_label(path: &Path) -> String {
    let rendered_path = display_path(path);
    format!("{CODEX_CONFIG_SOURCE_PREFIX}{rendered_path}")
}

pub fn source_path(source_kind: Option<ImportSourceKind>, source: &str) -> Option<PathBuf> {
    let trimmed = source.trim();
    let raw_path = match source_kind {
        Some(ImportSourceKind::ExistingLoongConfig) => {
            trimmed.strip_prefix(EXISTING_CONFIG_SOURCE_PREFIX)?
        }
        Some(ImportSourceKind::CodexConfig) => trimmed.strip_prefix(CODEX_CONFIG_SOURCE_PREFIX)?,
        Some(ImportSourceKind::ExplicitPath) => trimmed,
        Some(_) => return None,
        None => trimmed
            .strip_prefix(EXISTING_CONFIG_SOURCE_PREFIX)
            .or_else(|| trimmed.strip_prefix(CODEX_CONFIG_SOURCE_PREFIX))?,
    };
    Some(PathBuf::from(raw_path.trim()))
}

pub fn onboarding_source_label(source_kind: Option<ImportSourceKind>, source: &str) -> String {
    if matches!(source_kind, Some(ImportSourceKind::RecommendedPlan))
        || source_matches(source, recommended_plan_source_label())
    {
        suggested_starting_point_label().to_owned()
    } else {
        source.trim().to_owned()
    }
}

pub fn rollup_source_label(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty()
        || source_matches(trimmed, "multiple sources")
        || source_matches(trimmed, recommended_plan_source_label())
    {
        return None;
    }
    if source_matches(trimmed, workspace_source_label()) {
        return Some(workspace_guidance_rollup_label().to_owned());
    }
    Some(trimmed.to_owned())
}

fn source_matches(source: &str, canonical: &str) -> bool {
    source.trim().eq_ignore_ascii_case(canonical)
}
