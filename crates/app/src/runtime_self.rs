use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeSelfLane {
    StandingInstructions,
    SoulGuidance,
    IdentityContext,
    UserContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeSelfSourceSpec {
    relative_path: &'static str,
    lane: RuntimeSelfLane,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RuntimeSelfModel {
    pub standing_instructions: Vec<String>,
    pub soul_guidance: Vec<String>,
    pub identity_context: Vec<String>,
    pub user_context: Vec<String>,
}

impl RuntimeSelfModel {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.standing_instructions.is_empty()
            && self.soul_guidance.is_empty()
            && self.identity_context.is_empty()
            && self.user_context.is_empty()
    }
}

pub(crate) fn load_runtime_self_model(workspace_root: &Path) -> RuntimeSelfModel {
    let Some(canonical_workspace_root) = canonical_workspace_root(workspace_root) else {
        return RuntimeSelfModel::default();
    };

    let candidate_roots = candidate_roots(workspace_root);
    let mut loaded_paths = BTreeSet::new();
    let mut model = RuntimeSelfModel::default();

    for root in candidate_roots {
        for spec in RUNTIME_SELF_SOURCE_SPECS {
            let candidate_path = root.join(spec.relative_path);
            let Some(content) =
                read_runtime_self_source(canonical_workspace_root.as_path(), &candidate_path)
            else {
                continue;
            };

            let path_key = normalized_path_key(&candidate_path);
            let inserted = loaded_paths.insert(path_key);
            if !inserted {
                continue;
            }

            append_runtime_self_content(&mut model, spec.lane, content);
        }
    }

    model
}

pub(crate) fn render_runtime_self_section(model: &RuntimeSelfModel) -> Option<String> {
    if model.is_empty() {
        return None;
    }

    let mut sections = Vec::new();
    sections.push("## Runtime Self Context".to_owned());

    push_rendered_lane(
        &mut sections,
        "### Standing Instructions",
        &model.standing_instructions,
    );
    push_rendered_lane(&mut sections, "### Soul Guidance", &model.soul_guidance);
    push_rendered_lane(&mut sections, "### User Context", &model.user_context);

    Some(sections.join("\n\n"))
}

fn candidate_roots(workspace_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(workspace_root.to_path_buf());

    let nested_workspace_root = workspace_root.join("workspace");
    if nested_workspace_root.is_dir() {
        roots.push(nested_workspace_root);
    }

    roots
}

fn read_runtime_self_source(canonical_workspace_root: &Path, path: &Path) -> Option<String> {
    let canonical_path = path.canonicalize().ok()?;
    let is_within_workspace = canonical_path.starts_with(canonical_workspace_root);
    if !is_within_workspace {
        return None;
    }

    let content = std::fs::read_to_string(canonical_path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_owned())
}

fn canonical_workspace_root(workspace_root: &Path) -> Option<PathBuf> {
    workspace_root.canonicalize().ok()
}

fn normalized_path_key(path: &Path) -> String {
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical_path.display().to_string()
}

fn append_runtime_self_content(
    model: &mut RuntimeSelfModel,
    lane: RuntimeSelfLane,
    content: String,
) {
    match lane {
        RuntimeSelfLane::StandingInstructions => {
            model.standing_instructions.push(content);
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
    fn render_runtime_self_section_returns_none_for_empty_model() {
        let model = RuntimeSelfModel::default();
        let rendered = render_runtime_self_section(&model);

        assert_eq!(rendered, None);
    }

    #[cfg(unix)]
    #[test]
    fn load_runtime_self_model_ignores_linked_agents_file_outside_workspace_root() {
        let temp_dir = tempdir().expect("tempdir");
        let sandbox_root = temp_dir.path();
        let workspace_root = sandbox_root.join("workspace");
        let outside_root = sandbox_root.join("outside");
        let outside_agents_path = outside_root.join("AGENTS.md");
        let linked_agents_path = workspace_root.join("AGENTS.md");

        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&outside_root).expect("create outside root");
        std::fs::write(&outside_agents_path, "outside standing instructions")
            .expect("write outside agents");
        create_symlink(&outside_agents_path, &linked_agents_path).expect("create agents symlink");

        let model = load_runtime_self_model(workspace_root.as_path());

        assert!(model.standing_instructions.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn load_runtime_self_model_ignores_linked_nested_workspace_outside_workspace_root() {
        let temp_dir = tempdir().expect("tempdir");
        let sandbox_root = temp_dir.path();
        let workspace_root = sandbox_root.join("workspace");
        let linked_nested_workspace_root = workspace_root.join("workspace");
        let outside_root = sandbox_root.join("outside");
        let outside_nested_workspace_root = outside_root.join("nested");
        let outside_agents_path = outside_nested_workspace_root.join("AGENTS.md");

        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&outside_nested_workspace_root)
            .expect("create outside nested workspace");
        std::fs::write(&outside_agents_path, "outside nested standing instructions")
            .expect("write outside nested agents");
        create_symlink(
            &outside_nested_workspace_root,
            &linked_nested_workspace_root,
        )
        .expect("create nested workspace symlink");

        let model = load_runtime_self_model(workspace_root.as_path());

        assert!(model.standing_instructions.is_empty());
    }
}
