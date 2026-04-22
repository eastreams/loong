use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loong_contracts::ToolCoreRequest;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tools;

/// Workspace-scoped guidance files that Loong recognizes as first-class
/// runtime guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceGuidanceKind {
    Agents,
}

impl WorkspaceGuidanceKind {
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Agents => "AGENTS.md",
        }
    }
}

/// Search policy for workspace guidance discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceGuidanceSearchScope {
    SingleRoot,
    WorkspaceAndNestedWorkspace,
}

/// Resolved path for one detected workspace-guidance file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGuidancePath {
    pub kind: WorkspaceGuidanceKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceGuidanceModel {
    pub entries: Vec<String>,
}

impl WorkspaceGuidanceModel {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuidanceTruncationCause {
    SourceBudget,
    TotalBudget,
}

struct TruncatedWorkspaceGuidanceContent {
    rendered_content: String,
    budgeted_chars: usize,
}

const RUNTIME_PROMPT_WORKSPACE_GUIDANCE_KINDS: &[WorkspaceGuidanceKind] =
    &[WorkspaceGuidanceKind::Agents];

const IMPORT_DISCOVERY_WORKSPACE_GUIDANCE_KINDS: &[WorkspaceGuidanceKind] =
    &[WorkspaceGuidanceKind::Agents];

/// Guidance kinds that may feed the runtime prompt.
pub const fn runtime_prompt_workspace_guidance_kinds() -> &'static [WorkspaceGuidanceKind] {
    RUNTIME_PROMPT_WORKSPACE_GUIDANCE_KINDS
}

/// Guidance kinds that onboarding/import flows may surface to operators.
pub const fn import_discovery_workspace_guidance_kinds() -> &'static [WorkspaceGuidanceKind] {
    IMPORT_DISCOVERY_WORKSPACE_GUIDANCE_KINDS
}

/// Candidate workspace roots searched for guidance files.
pub fn candidate_workspace_roots(
    workspace_root: &Path,
    search_scope: WorkspaceGuidanceSearchScope,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    roots.push(workspace_root.to_path_buf());

    let include_nested_workspace = matches!(
        search_scope,
        WorkspaceGuidanceSearchScope::WorkspaceAndNestedWorkspace
    );
    if !include_nested_workspace {
        return roots;
    }

    let nested_workspace_root = workspace_root.join("workspace");
    let nested_workspace_exists = nested_workspace_root.is_dir();
    if nested_workspace_exists {
        roots.push(nested_workspace_root);
    }

    roots
}

/// Detect workspace-guidance files under the requested search scope.
pub fn detect_workspace_guidance_paths(
    workspace_root: &Path,
    search_scope: WorkspaceGuidanceSearchScope,
    kinds: &[WorkspaceGuidanceKind],
) -> Vec<WorkspaceGuidancePath> {
    let mut detected_paths = Vec::new();
    let search_roots = candidate_workspace_roots(workspace_root, search_scope);

    for search_root in search_roots {
        for kind in kinds {
            let candidate_path = search_root.join(kind.file_name());
            let candidate_exists = candidate_path.is_file();
            if !candidate_exists {
                continue;
            }

            let detected_path = WorkspaceGuidancePath {
                kind: *kind,
                path: candidate_path,
            };
            detected_paths.push(detected_path);
        }
    }

    detected_paths
}

pub fn workspace_guidance_source_candidates(workspace_root: &Path) -> Vec<PathBuf> {
    let search_scope = WorkspaceGuidanceSearchScope::WorkspaceAndNestedWorkspace;
    let detected_paths = detect_workspace_guidance_paths(
        workspace_root,
        search_scope,
        runtime_prompt_workspace_guidance_kinds(),
    );
    let mut source_candidates = Vec::new();

    for detected_path in detected_paths {
        source_candidates.push(detected_path.path);
    }

    source_candidates
}

pub fn load_workspace_guidance_model(workspace_root: &Path) -> WorkspaceGuidanceModel {
    let tool_runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
        file_root: Some(workspace_root.to_path_buf()),
        ..crate::tools::runtime_config::ToolRuntimeConfig::default()
    };

    load_workspace_guidance_model_with_config(workspace_root, &tool_runtime_config)
}

pub fn load_workspace_guidance_model_with_config(
    workspace_root: &Path,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> WorkspaceGuidanceModel {
    let mut remaining_total_chars = tool_runtime_config.runtime_self.max_total_chars;
    load_workspace_guidance_model_with_budget(
        workspace_root,
        tool_runtime_config,
        &mut remaining_total_chars,
    )
}

pub(crate) fn load_workspace_guidance_model_with_budget(
    workspace_root: &Path,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
    remaining_total_chars: &mut usize,
) -> WorkspaceGuidanceModel {
    let source_candidates = workspace_guidance_source_candidates(workspace_root);
    let mut loaded_paths = BTreeSet::new();
    let mut model = WorkspaceGuidanceModel::default();

    for source_path in source_candidates {
        let maybe_content =
            read_workspace_guidance_source(workspace_root, &source_path, tool_runtime_config);
        let Some(content) = maybe_content else {
            continue;
        };

        let budget_was_exhausted = *remaining_total_chars == 0;
        let appended_content = ingest_workspace_guidance_source(
            &mut model,
            &mut loaded_paths,
            remaining_total_chars,
            &source_path,
            content.as_str(),
            tool_runtime_config,
        );

        if budget_was_exhausted && appended_content {
            break;
        }
    }

    model
}

pub fn render_workspace_guidance_section(model: &WorkspaceGuidanceModel) -> Option<String> {
    if model.is_empty() {
        return None;
    }

    let mut sections = Vec::new();
    let guidance_entries = model.entries.join("\n\n");

    sections.push("## Workspace Guidance".to_owned());
    sections.push(guidance_entries);

    Some(sections.join("\n\n"))
}

pub fn normalized_workspace_source_path_key(path: &Path) -> String {
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical_path.display().to_string()
}

pub fn workspace_source_request_path(workspace_root: &Path, path: &Path) -> Option<String> {
    let path_is_file = path.is_file();
    if !path_is_file {
        return None;
    }

    let canonical_workspace_root = workspace_root.canonicalize().ok()?;
    let canonical_path = path.canonicalize().ok()?;
    let path_within_workspace = canonical_path.starts_with(canonical_workspace_root);
    if !path_within_workspace {
        return None;
    }

    let relative_path = path.strip_prefix(workspace_root).ok()?;
    let request_path = relative_path.to_string_lossy().to_string();
    Some(request_path)
}

fn read_workspace_guidance_source(
    workspace_root: &Path,
    path: &Path,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Option<String> {
    let request_path = workspace_source_request_path(workspace_root, path)?;
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": request_path,
        }),
    };

    let outcome = tools::execute_tool_core_with_config(request, tool_runtime_config).ok()?;
    let payload_content = outcome.payload.get("content")?;
    let content = payload_content.as_str()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_owned())
}

pub(crate) fn ingest_workspace_guidance_source(
    model: &mut WorkspaceGuidanceModel,
    loaded_paths: &mut BTreeSet<String>,
    remaining_total_chars: &mut usize,
    path: &Path,
    content: &str,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> bool {
    let path_key = normalized_workspace_source_path_key(path);
    let inserted = loaded_paths.insert(path_key);
    if !inserted {
        return false;
    }

    let maybe_truncated = truncate_workspace_guidance_source_content(
        path,
        content,
        *remaining_total_chars,
        tool_runtime_config,
    );
    let Some(truncated) = maybe_truncated else {
        return false;
    };

    let budgeted_chars = truncated.budgeted_chars;
    let rendered_content = truncated.rendered_content;

    *remaining_total_chars = remaining_total_chars.saturating_sub(budgeted_chars);
    model.entries.push(rendered_content);
    true
}

fn truncate_workspace_guidance_source_content(
    path: &Path,
    content: &str,
    remaining_total_chars: usize,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Option<TruncatedWorkspaceGuidanceContent> {
    if remaining_total_chars == 0 {
        let source_label = workspace_source_label(path);
        let rendered_content = workspace_guidance_truncation_notice_text(
            source_label.as_str(),
            GuidanceTruncationCause::TotalBudget,
        );
        return Some(TruncatedWorkspaceGuidanceContent {
            rendered_content,
            budgeted_chars: 0,
        });
    }

    let max_source_chars = tool_runtime_config.runtime_self.max_source_chars;
    let effective_limit = max_source_chars.min(remaining_total_chars);
    let content_char_count = content.chars().count();
    if content_char_count <= effective_limit {
        return Some(TruncatedWorkspaceGuidanceContent {
            rendered_content: content.to_owned(),
            budgeted_chars: content_char_count,
        });
    }

    let total_budget_is_tighter = remaining_total_chars < max_source_chars;
    let truncation_cause = if total_budget_is_tighter {
        GuidanceTruncationCause::TotalBudget
    } else {
        GuidanceTruncationCause::SourceBudget
    };
    let source_label = workspace_source_label(path);
    let truncation_notice =
        workspace_guidance_truncation_notice_text(source_label.as_str(), truncation_cause);
    let notice_char_count = truncation_notice.chars().count();
    let separator = "\n\n";
    let separator_char_count = separator.chars().count();
    let minimum_notice_limit = notice_char_count + separator_char_count + 1;

    if effective_limit < minimum_notice_limit {
        let rendered_content = compact_workspace_guidance_truncation_notice(
            source_label.as_str(),
            truncation_cause,
            effective_limit,
        );
        return Some(TruncatedWorkspaceGuidanceContent {
            rendered_content,
            budgeted_chars: effective_limit,
        });
    }

    let prefix_limit = effective_limit - notice_char_count - separator_char_count;
    let content_prefix = take_workspace_guidance_prefix(content, prefix_limit);
    let rendered_content = format!("{content_prefix}{separator}{truncation_notice}");

    Some(TruncatedWorkspaceGuidanceContent {
        rendered_content,
        budgeted_chars: effective_limit,
    })
}

fn workspace_source_label(path: &Path) -> String {
    let file_name = path.file_name().and_then(|value| value.to_str());
    let file_name = file_name.unwrap_or("workspace guidance source");
    file_name.to_owned()
}

fn workspace_guidance_truncation_notice_text(
    source_label: &str,
    truncation_cause: GuidanceTruncationCause,
) -> String {
    let budget_label = match truncation_cause {
        GuidanceTruncationCause::SourceBudget => "per-source budget",
        GuidanceTruncationCause::TotalBudget => "remaining total budget",
    };

    format!("[workspace guidance source truncated: {source_label} exceeded the {budget_label}]")
}

fn compact_workspace_guidance_truncation_notice(
    source_label: &str,
    truncation_cause: GuidanceTruncationCause,
    max_chars: usize,
) -> String {
    let detailed_notice = workspace_guidance_truncation_notice_text(source_label, truncation_cause);
    if detailed_notice.chars().count() <= max_chars {
        return detailed_notice;
    }

    let source_notice = format!("[workspace guidance truncated: {source_label}]");
    if source_notice.chars().count() <= max_chars {
        return source_notice;
    }

    let generic_notice = "[workspace guidance truncated]".to_owned();
    if generic_notice.chars().count() <= max_chars {
        return generic_notice;
    }

    let compact_notice = "[truncated]".to_owned();
    if compact_notice.chars().count() <= max_chars {
        return compact_notice;
    }

    let ellipsis = "...".to_owned();
    if ellipsis.chars().count() <= max_chars {
        return ellipsis;
    }

    ".".repeat(max_chars)
}

fn take_workspace_guidance_prefix(content: &str, max_chars: usize) -> String {
    content.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn candidate_workspace_roots_respects_single_root_scope() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace");

        let roots =
            candidate_workspace_roots(workspace_root, WorkspaceGuidanceSearchScope::SingleRoot);

        assert_eq!(roots, vec![workspace_root.to_path_buf()]);
    }

    #[test]
    fn candidate_workspace_roots_includes_nested_workspace_when_requested() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace");

        let roots = candidate_workspace_roots(
            workspace_root,
            WorkspaceGuidanceSearchScope::WorkspaceAndNestedWorkspace,
        );

        assert_eq!(
            roots,
            vec![workspace_root.to_path_buf(), nested_workspace_root]
        );
    }

    #[test]
    fn detect_workspace_guidance_paths_detects_only_agents() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let agents_path = workspace_root.join("AGENTS.md");
        let claude_path = workspace_root.join("CLAUDE.md");

        std::fs::write(&agents_path, "agents").expect("write AGENTS");
        std::fs::write(&claude_path, "claude").expect("write CLAUDE");

        let detected_paths = detect_workspace_guidance_paths(
            workspace_root,
            WorkspaceGuidanceSearchScope::SingleRoot,
            runtime_prompt_workspace_guidance_kinds(),
        );

        assert_eq!(detected_paths.len(), 1);
        assert_eq!(detected_paths[0].kind, WorkspaceGuidanceKind::Agents);
        assert_eq!(detected_paths[0].path, agents_path);
        assert_eq!(import_discovery_workspace_guidance_kinds().len(), 1);
    }

    #[test]
    fn detect_workspace_guidance_paths_preserves_root_then_nested_order() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");
        let root_agents_path = workspace_root.join("AGENTS.md");
        let nested_agents_path = nested_workspace_root.join("AGENTS.md");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace");
        std::fs::write(&root_agents_path, "root").expect("write root AGENTS");
        std::fs::write(&nested_agents_path, "nested").expect("write nested AGENTS");

        let detected_paths = detect_workspace_guidance_paths(
            workspace_root,
            WorkspaceGuidanceSearchScope::WorkspaceAndNestedWorkspace,
            runtime_prompt_workspace_guidance_kinds(),
        );

        assert_eq!(detected_paths.len(), 2);
        assert_eq!(detected_paths[0].path, root_agents_path);
        assert_eq!(detected_paths[1].path, nested_agents_path);
    }

    #[test]
    fn render_workspace_guidance_section_renders_agents_entries() {
        let model = WorkspaceGuidanceModel {
            entries: vec!["Always keep repo guidance explicit.".to_owned()],
        };

        let rendered =
            render_workspace_guidance_section(&model).expect("workspace guidance should render");

        assert!(rendered.contains("## Workspace Guidance"));
        assert!(rendered.contains("Always keep repo guidance explicit."));
    }

    #[test]
    fn load_workspace_guidance_model_ignores_claude_file() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let agents_path = workspace_root.join("AGENTS.md");
        let claude_path = workspace_root.join("CLAUDE.md");

        std::fs::write(&agents_path, "agents").expect("write AGENTS");
        std::fs::write(&claude_path, "claude").expect("write CLAUDE");

        let model = load_workspace_guidance_model(workspace_root);

        assert_eq!(model.entries, vec!["agents".to_owned()]);
    }
}
