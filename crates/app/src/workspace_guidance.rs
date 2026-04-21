use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Workspace-scoped guidance files that Loong recognizes across runtime and
/// onboarding/import flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceGuidanceKind {
    Agents,
    Claude,
    Gemini,
    Opencode,
}

impl WorkspaceGuidanceKind {
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Agents => "AGENTS.md",
            Self::Claude => "CLAUDE.md",
            Self::Gemini => "GEMINI.md",
            Self::Opencode => "OPENCODE.md",
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

const RUNTIME_PROMPT_WORKSPACE_GUIDANCE_KINDS: &[WorkspaceGuidanceKind] =
    &[WorkspaceGuidanceKind::Agents, WorkspaceGuidanceKind::Claude];

const IMPORT_DISCOVERY_WORKSPACE_GUIDANCE_KINDS: &[WorkspaceGuidanceKind] = &[
    WorkspaceGuidanceKind::Agents,
    WorkspaceGuidanceKind::Claude,
    WorkspaceGuidanceKind::Gemini,
    WorkspaceGuidanceKind::Opencode,
];

/// Guidance kinds that may feed the runtime prompt in the current phase.
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
    fn detect_workspace_guidance_paths_filters_to_requested_kinds() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let agents_path = workspace_root.join("AGENTS.md");
        let claude_path = workspace_root.join("CLAUDE.md");
        let gemini_path = workspace_root.join("GEMINI.md");

        std::fs::write(&agents_path, "agents").expect("write AGENTS");
        std::fs::write(&claude_path, "claude").expect("write CLAUDE");
        std::fs::write(&gemini_path, "gemini").expect("write GEMINI");

        let detected_paths = detect_workspace_guidance_paths(
            workspace_root,
            WorkspaceGuidanceSearchScope::SingleRoot,
            runtime_prompt_workspace_guidance_kinds(),
        );

        assert_eq!(detected_paths.len(), 2);
        assert_eq!(detected_paths[0].kind, WorkspaceGuidanceKind::Agents);
        assert_eq!(detected_paths[0].path, agents_path);
        assert_eq!(detected_paths[1].kind, WorkspaceGuidanceKind::Claude);
        assert_eq!(detected_paths[1].path, claude_path);
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
}
