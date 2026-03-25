use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loongclaw_contracts::ToolCoreRequest;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tools;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeSelfLane {
    StandingInstructions,
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

const RUNTIME_SELF_SOURCE_SPECS: &[RuntimeSelfSourceSpec] = &[
    RuntimeSelfSourceSpec {
        relative_path: "AGENTS.md",
        lane: RuntimeSelfLane::StandingInstructions,
    },
    RuntimeSelfSourceSpec {
        relative_path: "CLAUDE.md",
        lane: RuntimeSelfLane::StandingInstructions,
    },
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

pub(crate) fn load_runtime_self_model(workspace_root: &Path) -> RuntimeSelfModel {
    let tool_runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
        file_root: Some(workspace_root.to_path_buf()),
        ..crate::tools::runtime_config::ToolRuntimeConfig::default()
    };

    load_runtime_self_model_with_config(workspace_root, &tool_runtime_config)
}

pub(crate) fn load_runtime_self_model_with_config(
    workspace_root: &Path,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> RuntimeSelfModel {
    let source_candidates = runtime_self_source_candidates(workspace_root);
    let mut loaded_paths = BTreeSet::new();
    let mut model = RuntimeSelfModel::default();
    let mut remaining_total_chars = tool_runtime_config.runtime_self.max_total_chars;

    for (candidate_path, lane) in source_candidates {
        if remaining_total_chars == 0 {
            break;
        }

        let Some(content) =
            read_runtime_self_source(workspace_root, &candidate_path, tool_runtime_config)
        else {
            continue;
        };

        ingest_runtime_self_source(
            &mut model,
            &mut loaded_paths,
            &mut remaining_total_chars,
            lane,
            &candidate_path,
            content.as_str(),
            tool_runtime_config,
        );
    }

    model
}

pub(crate) fn render_runtime_self_section(model: &RuntimeSelfModel) -> Option<String> {
    let has_renderable_content = !model.standing_instructions.is_empty()
        || !model.tool_usage_policy.is_empty()
        || !model.soul_guidance.is_empty()
        || !model.user_context.is_empty();

    if !has_renderable_content {
        return None;
    }

    let mut sections = Vec::new();
    sections.push("## Runtime Self Context".to_owned());

    push_rendered_lane(
        &mut sections,
        "### Standing Instructions",
        &model.standing_instructions,
    );
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
    let mut roots = Vec::new();
    let nested_workspace_root = workspace_root.join("workspace");

    roots.push(workspace_root.to_path_buf());

    if nested_workspace_root.is_dir() {
        roots.push(nested_workspace_root);
    }

    roots
}

fn read_runtime_self_source(
    workspace_root: &Path,
    path: &Path,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Option<String> {
    let request_path = request_path_from_workspace_root(workspace_root, path)?;
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

fn request_path_from_workspace_root(workspace_root: &Path, path: &Path) -> Option<String> {
    let relative_path = path.strip_prefix(workspace_root).ok()?;
    let request_path = relative_path.to_string_lossy().to_string();
    Some(request_path)
}

pub(crate) fn normalized_path_key(path: &Path) -> String {
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical_path.display().to_string()
}

pub(crate) fn ingest_runtime_self_source(
    model: &mut RuntimeSelfModel,
    loaded_paths: &mut BTreeSet<String>,
    remaining_total_chars: &mut usize,
    lane: RuntimeSelfLane,
    path: &Path,
    content: &str,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) {
    let path_key = normalized_path_key(path);
    let inserted = loaded_paths.insert(path_key);
    if !inserted {
        return;
    }

    let truncated_content = truncate_runtime_self_source_content(
        path,
        content,
        *remaining_total_chars,
        tool_runtime_config,
    );
    let Some(truncated_content) = truncated_content else {
        return;
    };

    let stored_char_count = truncated_content.chars().count();
    *remaining_total_chars = remaining_total_chars.saturating_sub(stored_char_count);
    append_runtime_self_content(model, lane, truncated_content);
}

pub(crate) fn append_runtime_self_content(
    model: &mut RuntimeSelfModel,
    lane: RuntimeSelfLane,
    content: String,
) {
    match lane {
        RuntimeSelfLane::StandingInstructions => {
            model.standing_instructions.push(content);
        }
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
) -> Option<String> {
    let runtime_self_policy = &tool_runtime_config.runtime_self;
    let max_source_chars = runtime_self_policy.max_source_chars;
    let effective_limit = max_source_chars.min(remaining_total_chars);
    if effective_limit == 0 {
        return None;
    }

    let content_char_count = content.chars().count();
    if content_char_count <= effective_limit {
        return Some(content.to_owned());
    }

    let total_budget_is_tighter = remaining_total_chars < max_source_chars;
    let truncation_cause = if total_budget_is_tighter {
        RuntimeSelfTruncationCause::TotalBudget
    } else {
        RuntimeSelfTruncationCause::SourceBudget
    };
    let source_label = runtime_self_source_label(path);
    let truncation_notice = runtime_self_truncation_notice(source_label.as_str(), truncation_cause);
    let notice_char_count = truncation_notice.chars().count();

    if effective_limit <= notice_char_count {
        let fallback_content = take_runtime_self_prefix(content, effective_limit);
        let trimmed_fallback = fallback_content.trim();
        if trimmed_fallback.is_empty() {
            return None;
        }

        return Some(trimmed_fallback.to_owned());
    }

    let prefix_limit = effective_limit - notice_char_count;
    let content_prefix = take_runtime_self_prefix(content, prefix_limit);
    let rendered_content = format!("{content_prefix}{truncation_notice}");
    Some(rendered_content)
}

fn runtime_self_source_label(path: &Path) -> String {
    let file_name = path.file_name();
    let file_name = file_name.and_then(|value| value.to_str());
    let file_name = file_name.unwrap_or("runtime self source");
    file_name.to_owned()
}

fn runtime_self_truncation_notice(
    source_label: &str,
    truncation_cause: RuntimeSelfTruncationCause,
) -> String {
    let budget_label = match truncation_cause {
        RuntimeSelfTruncationCause::SourceBudget => "per-source budget",
        RuntimeSelfTruncationCause::TotalBudget => "remaining total budget",
    };

    format!("\n\n[runtime self source truncated: {source_label} exceeded the {budget_label}]")
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

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[test]
    fn load_runtime_self_model_reads_root_and_nested_workspace_sources() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace root");

        let agents_path = workspace_root.join("AGENTS.md");
        let soul_path = nested_workspace_root.join("SOUL.md");
        let identity_path = workspace_root.join("IDENTITY.md");
        let user_path = nested_workspace_root.join("USER.md");

        std::fs::write(&agents_path, "Keep standing instructions visible.").expect("write AGENTS");
        std::fs::write(&soul_path, "Prefer rigorous execution.").expect("write SOUL");
        std::fs::write(&identity_path, "You are the runtime helper.").expect("write IDENTITY");
        std::fs::write(&user_path, "The operator prefers concise output.").expect("write USER");

        let model = load_runtime_self_model(workspace_root);

        assert_eq!(model.standing_instructions.len(), 1);
        assert_eq!(model.soul_guidance.len(), 1);
        assert_eq!(model.identity_context.len(), 1);
        assert_eq!(model.user_context.len(), 1);
        assert!(model.standing_instructions[0].contains("standing instructions"));
        assert!(model.soul_guidance[0].contains("rigorous execution"));
        assert!(model.identity_context[0].contains("runtime helper"));
        assert!(model.user_context[0].contains("concise output"));
    }

    #[test]
    fn load_runtime_self_model_merges_same_lane_sources_in_stable_order() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace root");

        let root_agents_path = workspace_root.join("AGENTS.md");
        let root_claude_path = workspace_root.join("CLAUDE.md");
        let nested_agents_path = nested_workspace_root.join("AGENTS.md");

        let root_agents_text = "Root AGENTS standing instructions.";
        let root_claude_text = "Root CLAUDE standing instructions.";
        let nested_agents_text = "Nested workspace AGENTS standing instructions.";

        std::fs::write(&root_agents_path, root_agents_text).expect("write root AGENTS");
        std::fs::write(&root_claude_path, root_claude_text).expect("write root CLAUDE");
        std::fs::write(&nested_agents_path, nested_agents_text).expect("write nested AGENTS");

        let model = load_runtime_self_model(workspace_root);

        assert_eq!(
            model.standing_instructions,
            vec![
                root_agents_text.to_owned(),
                root_claude_text.to_owned(),
                nested_agents_text.to_owned(),
            ]
        );
    }

    #[test]
    fn render_runtime_self_section_includes_dedicated_tool_usage_policy_lane() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();

        let agents_path = workspace_root.join("AGENTS.md");
        let tools_path = workspace_root.join("TOOLS.md");

        let agents_text = "Keep standing instructions visible.";
        let tools_text = "When durable workspace facts may matter, search memory before answering.";

        std::fs::write(&agents_path, agents_text).expect("write AGENTS");
        std::fs::write(&tools_path, tools_text).expect("write TOOLS");

        let model = load_runtime_self_model(workspace_root);
        let rendered = render_runtime_self_section(&model).expect("render runtime self");

        assert!(rendered.contains("### Standing Instructions"));
        assert!(rendered.contains(agents_text));
        assert!(rendered.contains("### Tool Usage Policy"));
        assert!(rendered.contains(tools_text));
    }

    #[test]
    fn render_runtime_self_section_keeps_root_and_nested_tool_policy_order_stable() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace root");

        let root_tools_path = workspace_root.join("TOOLS.md");
        let nested_tools_path = nested_workspace_root.join("TOOLS.md");

        let root_tools_text = "Root tool policy guidance.";
        let nested_tools_text = "Nested workspace tool policy guidance.";

        std::fs::write(&root_tools_path, root_tools_text).expect("write root TOOLS");
        std::fs::write(&nested_tools_path, nested_tools_text).expect("write nested TOOLS");

        let model = load_runtime_self_model(workspace_root);
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
    fn load_runtime_self_model_truncates_oversized_source_content() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let agents_path = workspace_root.join("AGENTS.md");
        let prefix = "Keep standing instructions visible.\n";
        let tail_marker = "TAIL_MARKER_SHOULD_NOT_SURVIVE";
        let oversized_content = format!("{prefix}{}\n{tail_marker}", "a".repeat(24_000),);

        std::fs::write(&agents_path, oversized_content).expect("write oversized AGENTS");

        let model = load_runtime_self_model(workspace_root);
        let rendered = model
            .standing_instructions
            .first()
            .expect("standing instructions")
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
    fn load_runtime_self_model_enforces_total_runtime_self_budget() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let nested_workspace_root = workspace_root.join("workspace");

        std::fs::create_dir_all(&nested_workspace_root).expect("create nested workspace root");

        let root_agents = workspace_root.join("AGENTS.md");
        let root_claude = workspace_root.join("CLAUDE.md");
        let root_tools = workspace_root.join("TOOLS.md");
        let root_soul = workspace_root.join("SOUL.md");
        let root_user = workspace_root.join("USER.md");
        let nested_agents = nested_workspace_root.join("AGENTS.md");
        let nested_tools = nested_workspace_root.join("TOOLS.md");
        let nested_soul = nested_workspace_root.join("SOUL.md");
        let nested_user = nested_workspace_root.join("USER.md");

        let repeated_body = "b".repeat(18_500);
        let nested_tail_marker = "NESTED_USER_TAIL_MARKER_SHOULD_NOT_SURVIVE";
        let root_content = format!("root\n{repeated_body}");
        let nested_user_content = format!("nested user\n{repeated_body}\n{nested_tail_marker}");

        std::fs::write(&root_agents, &root_content).expect("write root AGENTS");
        std::fs::write(&root_claude, &root_content).expect("write root CLAUDE");
        std::fs::write(&root_tools, &root_content).expect("write root TOOLS");
        std::fs::write(&root_soul, &root_content).expect("write root SOUL");
        std::fs::write(&root_user, &root_content).expect("write root USER");
        std::fs::write(&nested_agents, &root_content).expect("write nested AGENTS");
        std::fs::write(&nested_tools, &root_content).expect("write nested TOOLS");
        std::fs::write(&nested_soul, &root_content).expect("write nested SOUL");
        std::fs::write(&nested_user, &nested_user_content).expect("write nested USER");

        let model = load_runtime_self_model(workspace_root);
        let total_chars = model
            .standing_instructions
            .iter()
            .chain(model.tool_usage_policy.iter())
            .chain(model.soul_guidance.iter())
            .chain(model.identity_context.iter())
            .chain(model.user_context.iter())
            .map(|entry| entry.chars().count())
            .sum::<usize>();
        let rendered_user_context = model.user_context.join("\n\n");

        assert!(
            total_chars <= 150_000,
            "runtime self total chars should stay within the default budget, got {total_chars}"
        );
        assert!(
            rendered_user_context.contains("runtime self source truncated"),
            "expected total-budget truncation notice in user context, got: {rendered_user_context}"
        );
        assert!(
            !rendered_user_context.contains(nested_tail_marker),
            "later runtime-self sources should not bypass the total budget"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_runtime_self_model_ignores_linked_agents_file_outside_workspace_root() {
        let workspace_dir = tempdir().expect("workspace tempdir");
        let outside_dir = tempdir().expect("outside tempdir");
        let workspace_root = workspace_dir.path();
        let outside_agents_path = outside_dir.path().join("AGENTS.md");
        let linked_agents_path = workspace_root.join("AGENTS.md");

        std::fs::write(&outside_agents_path, "outside standing instructions")
            .expect("write outside agents");
        create_symlink(&outside_agents_path, &linked_agents_path).expect("create agents symlink");

        let model = load_runtime_self_model(workspace_root);

        assert!(
            model.standing_instructions.is_empty(),
            "linked file outside workspace root should be rejected"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_runtime_self_model_ignores_linked_nested_workspace_outside_workspace_root() {
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

        let model = load_runtime_self_model(workspace_root);

        assert!(
            model.standing_instructions.is_empty(),
            "linked nested workspace outside workspace root should be rejected"
        );
    }
}
