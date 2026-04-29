use serde::{Deserialize, Serialize};

use super::tool_result_line::ToolResultLine;
use super::turn_shared::{
    external_skill_invoke_context_from_payload_summary, parse_external_skill_invoke_context,
};
use crate::tools::runtime_config::ToolRuntimeConfig;

pub(crate) const ACTIVE_EXTERNAL_SKILLS_EVENT_KIND: &str = "active_external_skills_refreshed";
const ACTIVE_EXTERNAL_SKILLS_MARKER: &str = "[active_external_skills]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ActiveExternalSkill {
    pub skill_id: String,
    pub display_name: String,
    pub instructions: String,
    pub skill_root: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub blocked_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ActiveExternalSkillsState {
    pub skills: Vec<ActiveExternalSkill>,
}

impl ActiveExternalSkillsState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

pub(crate) fn collect_active_external_skills_from_tool_result_text(
    tool_result_text: &str,
) -> Vec<ActiveExternalSkill> {
    let mut active_skills = Vec::new();

    for line in tool_result_text.lines() {
        let Some(skill_context) = parse_external_skill_invoke_context(line) else {
            continue;
        };

        upsert_active_external_skill(
            &mut active_skills,
            ActiveExternalSkill {
                skill_id: skill_context.skill_id,
                display_name: skill_context.display_name,
                instructions: skill_context.instructions,
                skill_root: skill_context
                    .skill_root
                    .map(|skill_root| skill_root.display().to_string()),
                allowed_tools: skill_context.allowed_tools,
                blocked_tools: skill_context.blocked_tools,
            },
        );
    }

    active_skills
}

pub(crate) fn collect_active_external_skills_from_tool_result_text_with_config(
    tool_result_text: &str,
    config: &ToolRuntimeConfig,
) -> Vec<ActiveExternalSkill> {
    let mut active_skills = collect_active_external_skills_from_tool_result_text(tool_result_text);

    for line in tool_result_text.lines() {
        let Some(tool_result_line) = ToolResultLine::parse(line) else {
            continue;
        };
        if crate::tools::canonical_tool_name(tool_result_line.tool_name()) != "file.read" {
            continue;
        }
        let Some(payload_json) = tool_result_line.payload_summary_json() else {
            continue;
        };
        let Some(path) = payload_json
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Ok(Some(skill_payload)) =
            crate::tools::model_visible_external_skill_context_payload_for_path(
                config,
                std::path::Path::new(path),
            )
        else {
            continue;
        };
        let Some(skill_context) =
            external_skill_invoke_context_from_payload_summary(&skill_payload)
        else {
            continue;
        };

        upsert_active_external_skill(
            &mut active_skills,
            ActiveExternalSkill {
                skill_id: skill_context.skill_id,
                display_name: skill_context.display_name,
                instructions: skill_context.instructions,
                skill_root: skill_context
                    .skill_root
                    .map(|skill_root| skill_root.display().to_string()),
                allowed_tools: skill_context.allowed_tools,
                blocked_tools: skill_context.blocked_tools,
            },
        );
    }

    active_skills
}

pub(crate) fn merge_active_external_skills(
    existing: Option<ActiveExternalSkillsState>,
    updates: Vec<ActiveExternalSkill>,
) -> Option<ActiveExternalSkillsState> {
    let mut merged = existing.unwrap_or_default();

    for update in updates {
        upsert_active_external_skill(&mut merged.skills, update);
    }

    (!merged.is_empty()).then_some(merged)
}

pub(crate) fn render_active_external_skills_section(
    active_skills: &ActiveExternalSkillsState,
) -> Option<String> {
    if active_skills.skills.is_empty() {
        return None;
    }

    let mut sections = vec![
        ACTIVE_EXTERNAL_SKILLS_MARKER.to_owned(),
        "The following external skills are already active for this session. Continue following them until superseded or the session ends.".to_owned(),
        "Do not re-activate a listed skill unless you need refreshed instructions.".to_owned(),
    ];

    for skill in &active_skills.skills {
        sections.push(format!(
            "Loaded external skill:\n- id: {}\n- name: {}",
            skill.skill_id, skill.display_name
        ));
        if let Some(skill_root) = skill.skill_root.as_deref() {
            sections.push(format!("Skill directory: {skill_root}"));
        }
        if !skill.allowed_tools.is_empty() {
            sections.push(format!("Allowed tools: {}", skill.allowed_tools.join(", ")));
        }
        if !skill.blocked_tools.is_empty() {
            sections.push(format!("Blocked tools: {}", skill.blocked_tools.join(", ")));
        }
        sections.push(skill.instructions.clone());
    }

    Some(sections.join("\n\n"))
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn active_external_skills_from_event_payload(
    payload: &serde_json::Value,
) -> Option<ActiveExternalSkillsState> {
    let active_skills = payload.get("active_external_skills")?.clone();
    serde_json::from_value(active_skills).ok()
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn load_persisted_active_external_skills(
    repo: &crate::session::repository::SessionRepository,
    session_id: &str,
) -> Result<Option<ActiveExternalSkillsState>, String> {
    let latest_event =
        repo.load_latest_event_by_kind(session_id, ACTIVE_EXTERNAL_SKILLS_EVENT_KIND)?;
    Ok(latest_event
        .as_ref()
        .and_then(|event| active_external_skills_from_event_payload(&event.payload_json)))
}

fn upsert_active_external_skill(
    active_skills: &mut Vec<ActiveExternalSkill>,
    update: ActiveExternalSkill,
) {
    let existing_index = active_skills
        .iter()
        .position(|skill| skill.skill_id == update.skill_id);

    if let Some(existing_index) = existing_index {
        let unchanged = active_skills
            .get(existing_index)
            .is_some_and(|existing| existing == &update);
        if unchanged {
            return;
        }
        active_skills.remove(existing_index);
    }

    active_skills.push(update);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_dir;
    use crate::tools::runtime_config::{ExternalSkillsRuntimePolicy, ToolRuntimeConfig};
    use std::collections::BTreeSet;
    use std::fs;

    #[test]
    fn collect_active_external_skills_from_tool_result_text_deduplicates_by_skill_id() {
        let first = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-1",
                "payload_semantics": "external_skill_context",
                "payload_summary": serde_json::to_string(&serde_json::json!({
                    "skill_id": "demo-skill",
                    "display_name": "Demo Skill",
                    "instructions": "first"
                }))
                .expect("encode payload"),
                "payload_chars": 128,
                "payload_truncated": false
            })
        );
        let second = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-2",
                "payload_semantics": "external_skill_context",
                "payload_summary": serde_json::to_string(&serde_json::json!({
                    "skill_id": "demo-skill",
                    "display_name": "Demo Skill",
                    "instructions": "updated",
                    "metadata": {
                        "blocked_tools": ["web.fetch"]
                    }
                }))
                .expect("encode payload"),
                "payload_chars": 128,
                "payload_truncated": false
            })
        );
        let third = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-3",
                "payload_semantics": "external_skill_context",
                "payload_summary": serde_json::to_string(&serde_json::json!({
                    "skill_id": "other-skill",
                    "display_name": "Other Skill",
                    "instructions": "other"
                }))
                .expect("encode payload"),
                "payload_chars": 128,
                "payload_truncated": false
            })
        );
        let tool_result_text = [first, second, third].join("\n");

        let active_skills =
            collect_active_external_skills_from_tool_result_text(tool_result_text.as_str());

        assert_eq!(active_skills.len(), 2);
        assert_eq!(active_skills[0].skill_id, "demo-skill");
        assert_eq!(active_skills[0].instructions, "updated");
        assert_eq!(active_skills[0].blocked_tools, vec!["web.fetch"]);
        assert_eq!(active_skills[1].skill_id, "other-skill");
    }

    #[test]
    fn render_active_external_skills_section_lists_loaded_skills() {
        let rendered = render_active_external_skills_section(&ActiveExternalSkillsState {
            skills: vec![ActiveExternalSkill {
                skill_id: "demo-skill".to_owned(),
                display_name: "Demo Skill".to_owned(),
                instructions: "<skill_content name=\"Demo Skill\">demo</skill_content>".to_owned(),
                skill_root: Some("/tmp/demo-skill".to_owned()),
                allowed_tools: vec!["shell.exec".to_owned()],
                blocked_tools: vec!["web.fetch".to_owned()],
            }],
        })
        .expect("render active skills");

        assert!(rendered.contains(ACTIVE_EXTERNAL_SKILLS_MARKER));
        assert!(rendered.contains("demo-skill"));
        assert!(rendered.contains("Demo Skill"));
        assert!(rendered.contains("Skill directory: /tmp/demo-skill"));
        assert!(rendered.contains("Allowed tools: shell.exec"));
        assert!(rendered.contains("Blocked tools: web.fetch"));
        assert!(rendered.contains("<skill_content name=\"Demo Skill\">demo</skill_content>"));
    }

    #[test]
    fn collect_active_external_skills_from_skill_file_read_activates_visible_skill() {
        let root = unique_temp_dir("loong-active-skill-file-read");
        fs::create_dir_all(root.join(".loong/skills/demo-skill")).expect("create skill root");
        let skill_path = root.join(".loong/skills/demo-skill/SKILL.md");
        fs::write(
            &skill_path,
            "---\nname: demo-skill\ndescription: Use this skill for demo verification.\ninvocation_policy: both\n---\n\n# Demo Skill\n\nFollow the demo workflow.\n",
        )
        .expect("write skill file");

        let config = ToolRuntimeConfig {
            file_root: Some(root.clone()),
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: false,
            },
            ..Default::default()
        };
        let tool_result_text = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-1",
                "payload_summary": serde_json::to_string(&serde_json::json!({
                    "path": skill_path.display().to_string(),
                    "content": "# Demo Skill\n\nFollow the demo workflow.\n"
                }))
                .expect("encode payload"),
                "payload_chars": 180,
                "payload_truncated": false
            })
        );

        let active_skills = collect_active_external_skills_from_tool_result_text_with_config(
            tool_result_text.as_str(),
            &config,
        );
        let expected_skill_root = std::fs::canonicalize(root.join(".loong/skills/demo-skill"))
            .expect("canonical skill root");
        let actual_skill_root = active_skills[0]
            .skill_root
            .as_deref()
            .map(std::path::PathBuf::from)
            .map(|path| std::fs::canonicalize(path).expect("canonical active skill root"));

        assert_eq!(active_skills.len(), 1);
        assert_eq!(active_skills[0].skill_id, "demo-skill");
        assert_eq!(active_skills[0].display_name, "Demo Skill");
        assert_eq!(actual_skill_root.as_ref(), Some(&expected_skill_root));
        assert!(
            active_skills[0]
                .instructions
                .contains("<skill_content name=\"Demo Skill\""),
            "skill file reads should synthesize structured activation instructions"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_active_external_skills_from_truncated_skill_file_read_activates_visible_skill() {
        let root = unique_temp_dir("loong-active-skill-file-read-truncated");
        fs::create_dir_all(root.join(".loong/skills/demo-skill")).expect("create skill root");
        let skill_path = root.join(".loong/skills/demo-skill/SKILL.md");
        let long_body = format!(
            "---\nname: demo-skill\ndescription: Use this skill for demo verification.\ninvocation_policy: both\n---\n\n# Demo Skill\n\n{}\n",
            "Follow the demo workflow.\n".repeat(40)
        );
        fs::write(&skill_path, long_body).expect("write skill file");

        let config = ToolRuntimeConfig {
            file_root: Some(root.clone()),
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: false,
            },
            ..Default::default()
        };
        let tool_result_text = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-1",
                "payload_summary": serde_json::to_string(&serde_json::json!({
                    "path": skill_path.display().to_string(),
                    "bytes": 1200,
                    "truncated": false,
                    "content_preview": "preview",
                    "content_chars": 1200,
                    "content_truncated": true
                }))
                .expect("encode payload"),
                "payload_chars": 180,
                "payload_truncated": true
            })
        );

        let active_skills = collect_active_external_skills_from_tool_result_text_with_config(
            tool_result_text.as_str(),
            &config,
        );

        assert_eq!(active_skills.len(), 1);
        assert_eq!(active_skills[0].skill_id, "demo-skill");
        assert!(
            active_skills[0]
                .instructions
                .contains("<skill_content name=\"Demo Skill\""),
            "truncated file.read summaries should still activate visible skills from path context"
        );

        fs::remove_dir_all(&root).ok();
    }
}
