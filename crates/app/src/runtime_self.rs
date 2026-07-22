use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace_guidance::{self, WorkspaceGuidanceSearchScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeSelfLane {
    ToolUsagePolicy,
    SoulGuidance,
    IdentityContext,
    UserContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeSelfSourceSpec {
    relative_path: &'static str,
    lane: RuntimeSelfLane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeSelfTruncationCause {
    SourceBudget,
    TotalBudget,
}

struct TruncatedRuntimeSelfSourceContent {
    rendered_content: String,
    budgeted_chars: usize,
}

const RUNTIME_SELF_SOURCE_SPECS: &[RuntimeSelfSourceSpec] = &[
    RuntimeSelfSourceSpec {
        relative_path: "TOOLS.md",
        lane: RuntimeSelfLane::ToolUsagePolicy,
    },
    RuntimeSelfSourceSpec {
        relative_path: "SOUL.md",
        lane: RuntimeSelfLane::SoulGuidance,
    },
    RuntimeSelfSourceSpec {
        relative_path: "IDENTITY.md",
        lane: RuntimeSelfLane::IdentityContext,
    },
    RuntimeSelfSourceSpec {
        relative_path: "USER.md",
        lane: RuntimeSelfLane::UserContext,
    },
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RuntimeSelfModel {
    pub standing_instructions: Vec<String>,
    pub tool_usage_policy: Vec<String>,
    pub soul_guidance: Vec<String>,
    pub identity_context: Vec<String>,
    pub user_context: Vec<String>,
}

impl RuntimeSelfModel {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.standing_instructions.is_empty()
            && self.tool_usage_policy.is_empty()
            && self.soul_guidance.is_empty()
            && self.identity_context.is_empty()
            && self.user_context.is_empty()
    }
}

pub(crate) fn render_runtime_self_section(model: &RuntimeSelfModel) -> Option<String> {
    let has_renderable_content = !model.tool_usage_policy.is_empty()
        || !model.soul_guidance.is_empty()
        || !model.user_context.is_empty();

    if !has_renderable_content {
        return None;
    }

    let mut sections = Vec::new();
    sections.push("## Runtime Self Context".to_owned());

    push_rendered_lane(
        &mut sections,
        "### Tool Usage Policy",
        &model.tool_usage_policy,
    );
    push_rendered_lane(&mut sections, "### Soul Guidance", &model.soul_guidance);
    push_rendered_lane(&mut sections, "### User Context", &model.user_context);

    Some(sections.join("\n\n"))
}

pub(crate) fn runtime_self_source_candidates(
    workspace_root: &Path,
    _tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Vec<(PathBuf, RuntimeSelfLane)> {
    let candidate_roots = candidate_workspace_roots(workspace_root);
    let mut source_candidates = Vec::new();

    for root in candidate_roots {
        for spec in RUNTIME_SELF_SOURCE_SPECS {
            let candidate_path = root.join(spec.relative_path);
            source_candidates.push((candidate_path, spec.lane));
        }
    }

    source_candidates
}

pub(crate) fn candidate_workspace_roots(workspace_root: &Path) -> Vec<PathBuf> {
    let search_scope = WorkspaceGuidanceSearchScope::WorkspaceAndNestedWorkspace;
    workspace_guidance::candidate_workspace_roots(workspace_root, search_scope)
}

pub(crate) fn ingest_runtime_self_source(
    model: &mut RuntimeSelfModel,
    loaded_paths: &mut BTreeSet<String>,
    remaining_total_chars: &mut usize,
    lane: RuntimeSelfLane,
    path: &Path,
    content: &str,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> bool {
    let path_key = path.to_string_lossy().into_owned();
    let inserted = loaded_paths.insert(path_key);
    if !inserted {
        return false;
    }

    let truncated_content = truncate_runtime_self_source_content(
        path,
        content,
        *remaining_total_chars,
        tool_runtime_config,
    );
    let Some(truncated_content) = truncated_content else {
        return false;
    };

    let budgeted_chars = truncated_content.budgeted_chars;
    let rendered_content = truncated_content.rendered_content;

    *remaining_total_chars = remaining_total_chars.saturating_sub(budgeted_chars);
    append_runtime_self_content(model, lane, rendered_content);

    true
}

pub(crate) fn append_runtime_self_content(
    model: &mut RuntimeSelfModel,
    lane: RuntimeSelfLane,
    content: String,
) {
    match lane {
        RuntimeSelfLane::ToolUsagePolicy => {
            model.tool_usage_policy.push(content);
        }
        RuntimeSelfLane::SoulGuidance => {
            model.soul_guidance.push(content);
        }
        RuntimeSelfLane::IdentityContext => {
            model.identity_context.push(content);
        }
        RuntimeSelfLane::UserContext => {
            model.user_context.push(content);
        }
    }
}

fn truncate_runtime_self_source_content(
    path: &Path,
    content: &str,
    remaining_total_chars: usize,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Option<TruncatedRuntimeSelfSourceContent> {
    if remaining_total_chars == 0 {
        let source_label = runtime_self_source_label(path);
        let rendered_content = runtime_self_truncation_notice_text(
            source_label.as_str(),
            RuntimeSelfTruncationCause::TotalBudget,
        );
        let budgeted_chars = 0;

        return Some(TruncatedRuntimeSelfSourceContent {
            rendered_content,
            budgeted_chars,
        });
    }

    let runtime_self_policy = &tool_runtime_config.runtime_self;
    let max_source_chars = runtime_self_policy.max_source_chars;
    let effective_limit = max_source_chars.min(remaining_total_chars);
    let content_char_count = content.chars().count();
    if content_char_count <= effective_limit {
        let rendered_content = content.to_owned();
        let budgeted_chars = content_char_count;

        return Some(TruncatedRuntimeSelfSourceContent {
            rendered_content,
            budgeted_chars,
        });
    }

    let total_budget_is_tighter = remaining_total_chars < max_source_chars;
    let truncation_cause = if total_budget_is_tighter {
        RuntimeSelfTruncationCause::TotalBudget
    } else {
        RuntimeSelfTruncationCause::SourceBudget
    };
    let source_label = runtime_self_source_label(path);
    let truncation_notice =
        runtime_self_truncation_notice_text(source_label.as_str(), truncation_cause);
    let notice_char_count = truncation_notice.chars().count();
    let separator = "\n\n";
    let separator_char_count = separator.chars().count();
    let minimum_notice_limit = notice_char_count + separator_char_count + 1;

    if effective_limit < minimum_notice_limit {
        let rendered_content = compact_runtime_self_truncation_notice(
            source_label.as_str(),
            truncation_cause,
            effective_limit,
        );
        let budgeted_chars = effective_limit;

        return Some(TruncatedRuntimeSelfSourceContent {
            rendered_content,
            budgeted_chars,
        });
    }

    let prefix_limit = effective_limit - notice_char_count - separator_char_count;
    let content_prefix = take_runtime_self_prefix(content, prefix_limit);
    let rendered_content = format!("{content_prefix}{separator}{truncation_notice}");
    let budgeted_chars = effective_limit;

    Some(TruncatedRuntimeSelfSourceContent {
        rendered_content,
        budgeted_chars,
    })
}

fn runtime_self_source_label(path: &Path) -> String {
    let file_name = path.file_name();
    let file_name = file_name.and_then(|value| value.to_str());
    let file_name = file_name.unwrap_or("runtime self source");
    file_name.to_owned()
}

fn runtime_self_truncation_notice_text(
    source_label: &str,
    truncation_cause: RuntimeSelfTruncationCause,
) -> String {
    let budget_label = match truncation_cause {
        RuntimeSelfTruncationCause::SourceBudget => "per-source budget",
        RuntimeSelfTruncationCause::TotalBudget => "remaining total budget",
    };

    format!("[runtime self source truncated: {source_label} exceeded the {budget_label}]")
}

fn compact_runtime_self_truncation_notice(
    source_label: &str,
    truncation_cause: RuntimeSelfTruncationCause,
    max_chars: usize,
) -> String {
    let detailed_notice = runtime_self_truncation_notice_text(source_label, truncation_cause);
    if detailed_notice.chars().count() <= max_chars {
        return detailed_notice;
    }

    let source_notice = format!("[runtime self truncated: {source_label}]");
    if source_notice.chars().count() <= max_chars {
        return source_notice;
    }

    let generic_notice = "[runtime self truncated]".to_owned();
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

fn take_runtime_self_prefix(content: &str, max_chars: usize) -> String {
    content.chars().take(max_chars).collect()
}

fn push_rendered_lane(sections: &mut Vec<String>, heading: &str, entries: &[String]) {
    if entries.is_empty() {
        return;
    }

    let mut lane_sections = Vec::new();
    lane_sections.push(heading.to_owned());

    let joined_entries = entries.join("\n\n");
    lane_sections.push(joined_entries);

    let rendered_lane = lane_sections.join("\n\n");
    sections.push(rendered_lane);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn runtime_self_tool_runtime_config(
        workspace_root: &Path,
        max_source_chars: usize,
        max_total_chars: usize,
    ) -> crate::tools::runtime_config::ToolRuntimeConfig {
        let runtime_self_policy =
            crate::tools::runtime_config::RuntimeSelfRuntimePolicy::from_limits(
                max_source_chars,
                max_total_chars,
            );

        crate::tools::runtime_config::ToolRuntimeConfig {
            file_root: Some(workspace_root.to_path_buf()),
            runtime_self: runtime_self_policy,
            ..crate::tools::runtime_config::ToolRuntimeConfig::default()
        }
    }

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[test]
    fn runtime_self_source_candidates_include_root_and_nested_workspace_sources() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace root");

        let candidates = runtime_self_source_candidates(
            workspace_root,
            &crate::tools::runtime_config::ToolRuntimeConfig::default(),
        );

        assert_eq!(candidates.len(), 8);
        assert_eq!(
            candidates[0],
            (
                workspace_root.join("TOOLS.md"),
                RuntimeSelfLane::ToolUsagePolicy
            )
        );
        assert_eq!(
            candidates[7],
            (
                nested_workspace_root.join("USER.md"),
                RuntimeSelfLane::UserContext
            )
        );
    }

    #[test]
    fn ingest_runtime_self_source_merges_same_lane_sources_in_call_order() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace root");

        let root_tools_path = workspace_root.join("TOOLS.md");
        let nested_tools_path = nested_workspace_root.join("TOOLS.md");

        let mut remaining_total_chars = 10_000usize;
        let mut loaded_paths = BTreeSet::new();
        let mut model = RuntimeSelfModel::default();
        let tool_runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();

        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            RuntimeSelfLane::ToolUsagePolicy,
            &root_tools_path,
            "Root TOOLS guidance.",
            &tool_runtime_config,
        );
        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            RuntimeSelfLane::ToolUsagePolicy,
            &nested_tools_path,
            "Nested workspace TOOLS guidance.",
            &tool_runtime_config,
        );

        assert_eq!(
            model.tool_usage_policy,
            vec![
                "Root TOOLS guidance.".to_owned(),
                "Nested workspace TOOLS guidance.".to_owned()
            ]
        );
    }

    #[test]
    fn render_runtime_self_section_includes_dedicated_tool_usage_policy_lane() {
        let tools_text = "When durable workspace facts may matter, search memory before answering.";
        let model = RuntimeSelfModel {
            tool_usage_policy: vec![tools_text.to_owned()],
            ..RuntimeSelfModel::default()
        };
        let rendered = render_runtime_self_section(&model).expect("render runtime self");

        assert!(!rendered.contains("### Standing Instructions"));
        assert!(rendered.contains("### Tool Usage Policy"));
        assert!(rendered.contains(tools_text));
    }

    #[test]
    fn render_runtime_self_section_keeps_root_and_nested_tool_policy_order_stable() {
        let root_tools_text = "Root tool policy guidance.";
        let nested_tools_text = "Nested workspace tool policy guidance.";
        let model = RuntimeSelfModel {
            tool_usage_policy: vec![root_tools_text.to_owned(), nested_tools_text.to_owned()],
            ..RuntimeSelfModel::default()
        };
        let rendered = render_runtime_self_section(&model).expect("render runtime self");

        let root_index = rendered
            .find(root_tools_text)
            .expect("root tool policy should be rendered");
        let nested_index = rendered
            .find(nested_tools_text)
            .expect("nested tool policy should be rendered");

        assert!(root_index < nested_index);
    }

    #[test]
    fn runtime_self_source_candidates_ignore_import_only_workspace_guidance_files() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();

        let candidates = runtime_self_source_candidates(
            workspace_root,
            &crate::tools::runtime_config::ToolRuntimeConfig::default(),
        );

        assert!(
            candidates
                .iter()
                .all(
                    |(path, _lane)| path.file_name().is_none_or(|name| name != "AGENTS.md"
                        && name != "CLAUDE.md"
                        && name != "GEMINI.md"
                        && name != "OPENCODE.md")
                )
        );
    }

    #[test]
    fn render_runtime_self_section_returns_none_for_empty_model() {
        let model = RuntimeSelfModel::default();
        let rendered = render_runtime_self_section(&model);

        assert_eq!(rendered, None);
    }

    #[test]
    fn render_runtime_self_section_keeps_tool_usage_policy_only_models() {
        let model = RuntimeSelfModel {
            tool_usage_policy: vec!["Prefer audited tool paths.".to_owned()],
            ..RuntimeSelfModel::default()
        };

        let rendered = render_runtime_self_section(&model).expect("rendered runtime self");

        assert!(rendered.contains("## Runtime Self Context"));
        assert!(rendered.contains("### Tool Usage Policy"));
        assert!(rendered.contains("Prefer audited tool paths."));
    }

    #[test]
    fn render_runtime_self_section_returns_none_for_identity_only_model() {
        let model = RuntimeSelfModel {
            identity_context: vec!["# Identity\n\n- Name: Workspace helper".to_owned()],
            ..RuntimeSelfModel::default()
        };

        let rendered = render_runtime_self_section(&model);

        assert_eq!(rendered, None);
    }

    #[test]
    fn ingest_runtime_self_source_truncates_oversized_source_content() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let tools_path = workspace_root.join("TOOLS.md");
        let prefix = "Keep runtime self bounded.\n";
        let tail_marker = "TAIL_MARKER_SHOULD_NOT_SURVIVE";
        let oversized_content = format!("{prefix}{}\n{tail_marker}", "a".repeat(24_000),);

        let mut remaining_total_chars = 32_768usize;
        let mut loaded_paths = BTreeSet::new();
        let mut model = RuntimeSelfModel::default();
        let tool_runtime_config = runtime_self_tool_runtime_config(workspace_root, 4_096, 32_768);

        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            RuntimeSelfLane::ToolUsagePolicy,
            &tools_path,
            oversized_content.as_str(),
            &tool_runtime_config,
        );
        let rendered = model
            .tool_usage_policy
            .first()
            .expect("tool usage policy")
            .as_str();

        assert!(rendered.contains(prefix));
        assert!(
            rendered.contains("runtime self source truncated"),
            "expected truncation notice in rendered source, got: {rendered}"
        );
        assert!(
            !rendered.contains(tail_marker),
            "oversized source tail should be truncated, got: {rendered}"
        );
    }

    #[test]
    fn ingest_runtime_self_source_enforces_total_runtime_self_budget() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let root_tools = workspace_root.join("TOOLS.md");
        let root_user = workspace_root.join("USER.md");
        let tools_text = "a".repeat(1_024);
        let user_text = "later user context should still surface a truncation notice";
        let mut remaining_total_chars = tools_text.chars().count();
        let mut loaded_paths = BTreeSet::new();
        let mut model = RuntimeSelfModel::default();
        let tool_runtime_config =
            runtime_self_tool_runtime_config(workspace_root, 10_000, remaining_total_chars);

        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            RuntimeSelfLane::ToolUsagePolicy,
            &root_tools,
            tools_text.as_str(),
            &tool_runtime_config,
        );
        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            RuntimeSelfLane::UserContext,
            &root_user,
            user_text,
            &tool_runtime_config,
        );
        let rendered_user_context = model.user_context.join("\n\n");

        assert!(
            rendered_user_context.contains("runtime self source truncated"),
            "expected total-budget truncation notice in user context, got: {rendered_user_context}"
        );
        assert_eq!(model.tool_usage_policy, vec![tools_text]);
        assert!(rendered_user_context.contains("USER.md"));
        assert!(rendered_user_context.contains("remaining total budget"));
    }

    #[test]
    fn ingest_runtime_self_source_uses_compact_notice_when_remaining_budget_cannot_fit_full_notice()
    {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let tools_path = workspace_root.join("TOOLS.md");
        let user_path = workspace_root.join("USER.md");
        let tools_text = "a".repeat(1_024);
        let compact_budget = 24usize;
        let raw_user_prefix = "later user context raw p";
        let user_text =
            "later user context raw prefix should not leak into compact truncation rendering";
        let total_budget = tools_text.chars().count() + compact_budget;
        let tool_runtime_config =
            runtime_self_tool_runtime_config(workspace_root, 10_000, total_budget);
        let mut remaining_total_chars = total_budget;
        let mut loaded_paths = BTreeSet::new();
        let mut model = RuntimeSelfModel::default();

        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            RuntimeSelfLane::ToolUsagePolicy,
            &tools_path,
            tools_text.as_str(),
            &tool_runtime_config,
        );
        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            RuntimeSelfLane::UserContext,
            &user_path,
            user_text,
            &tool_runtime_config,
        );
        let rendered_user_context = model.user_context.join("\n\n");

        assert!(rendered_user_context.contains("runtime self truncated"));
        assert!(!rendered_user_context.contains(raw_user_prefix));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_self_source_candidates_ignore_linked_agents_file_outside_workspace_root() {
        let workspace_dir = tempdir().expect("workspace tempdir");
        let outside_dir = tempdir().expect("outside tempdir");
        let workspace_root = workspace_dir.path();
        let outside_agents_path = outside_dir.path().join("AGENTS.md");
        let linked_agents_path = workspace_root.join("AGENTS.md");

        std::fs::write(&outside_agents_path, "outside standing instructions")
            .expect("write outside agents");
        create_symlink(&outside_agents_path, &linked_agents_path).expect("create agents symlink");

        let candidates = runtime_self_source_candidates(
            workspace_root,
            &crate::tools::runtime_config::ToolRuntimeConfig::default(),
        );

        assert!(
            !candidates
                .iter()
                .any(|(path, _lane)| path == &linked_agents_path),
            "workspace-guidance symlink should not become a runtime-self source"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runtime_self_source_candidates_do_not_resolve_linked_nested_workspace() {
        let workspace_dir = tempdir().expect("workspace tempdir");
        let outside_dir = tempdir().expect("outside tempdir");
        let workspace_root = workspace_dir.path();
        let linked_nested_workspace_root = workspace_root.join("workspace");
        let outside_nested_workspace_root = outside_dir.path().join("nested");
        let outside_agents_path = outside_nested_workspace_root.join("AGENTS.md");

        std::fs::create_dir_all(&outside_nested_workspace_root)
            .expect("create outside nested workspace");
        std::fs::write(&outside_agents_path, "outside nested standing instructions")
            .expect("write outside nested agents");
        create_symlink(
            &outside_nested_workspace_root,
            &linked_nested_workspace_root,
        )
        .expect("create nested workspace symlink");

        let candidates = runtime_self_source_candidates(
            workspace_root,
            &crate::tools::runtime_config::ToolRuntimeConfig::default(),
        );

        assert!(
            candidates
                .iter()
                .any(|(path, _lane)| path.starts_with(&linked_nested_workspace_root))
        );
    }
}
