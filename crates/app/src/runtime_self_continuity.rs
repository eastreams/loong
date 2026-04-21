use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{LoongConfig, PersonalizationConfig};
use crate::runtime_identity::{self, ResolvedRuntimeIdentity};
use crate::runtime_self::{self, RuntimeSelfModel};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{SessionEventRecord, SessionRepository};
use crate::tools::runtime_config::ToolRuntimeConfig;
use crate::workspace_guidance::{self, WorkspaceGuidanceModel};

const DURABLE_RECALL_INTRO: &str = concat!(
    "Advisory durable recall exported immediately before context compaction. ",
    "It may enrich future recall. ",
    "It does not replace Runtime Self Context. ",
    "It does not override Resolved Runtime Identity or Session Profile.",
);

// Lane-separated continuity carrier for runtime self state.
// Live workspace/config state remains authoritative.
// Stored continuity only fills missing lanes across compaction and delegation boundaries.
// Session-local conversation content must never be promoted automatically.
// Profile projections are preserved for future durable-recall consumers without becoming
// an identity override path.
pub(crate) const RUNTIME_SELF_CONTINUITY_EVENT_KIND: &str = "runtime_self_continuity_refreshed";
pub(crate) const RUNTIME_SELF_CONTINUITY_MARKER: &str = "[runtime_self_continuity]";

const RUNTIME_DURABLE_RECALL_INTRO: &str = concat!(
    "Advisory durable recall loaded from workspace memory files. ",
    "It may enrich the current session. ",
    "It does not replace Runtime Self Context. ",
    "It does not override Resolved Runtime Identity or Session Profile.",
);

const COMPACTION_SUMMARY_SCOPE_NOTE: &str = concat!(
    "Session-local recall only. ",
    "Does not replace Runtime Self Context. ",
    "Does not override Resolved Runtime Identity, Session Profile, ",
    "or advisory durable recall.",
);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSelfContinuity {
    #[serde(default)]
    pub workspace_guidance: WorkspaceGuidanceModel,
    pub runtime_self: RuntimeSelfModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_identity: Option<ResolvedRuntimeIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_profile_projection: Option<String>,
}

pub(crate) fn compaction_summary_scope_note() -> &'static str {
    COMPACTION_SUMMARY_SCOPE_NOTE
}

impl RuntimeSelfContinuity {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let profile_projection = normalize_projection(self.session_profile_projection.as_deref());
        self.workspace_guidance.is_empty()
            && self.runtime_self.is_empty()
            && self.resolved_identity.is_none()
            && profile_projection.is_none()
    }

    #[must_use]
    pub fn has_prompt_projection(&self) -> bool {
        let profile_projection = normalize_projection(self.session_profile_projection.as_deref());
        !self.workspace_guidance.is_empty()
            || !self.runtime_self.is_empty()
            || self.resolved_identity.is_some()
            || profile_projection.is_some()
    }

    #[must_use]
    fn normalize_legacy_workspace_guidance(mut self) -> Self {
        if self.workspace_guidance.is_empty() && !self.runtime_self.standing_instructions.is_empty()
        {
            self.workspace_guidance.entries =
                std::mem::take(&mut self.runtime_self.standing_instructions);
        } else if !self.runtime_self.standing_instructions.is_empty() {
            self.runtime_self.standing_instructions.clear();
        }

        self
    }
}

pub(crate) fn resolve_runtime_self_continuity(
    workspace_root: Option<&Path>,
    profile_note: Option<&str>,
    personalization: Option<&PersonalizationConfig>,
) -> Option<RuntimeSelfContinuity> {
    let workspace_guidance = match workspace_root {
        Some(workspace_root) => workspace_guidance::load_workspace_guidance_model(workspace_root),
        None => WorkspaceGuidanceModel::default(),
    };
    let runtime_self = match workspace_root {
        Some(workspace_root) => runtime_self::load_runtime_self_model(workspace_root),
        None => RuntimeSelfModel::default(),
    };
    let resolved_identity =
        runtime_identity::resolve_runtime_identity(Some(&runtime_self), profile_note);
    let session_profile_projection =
        runtime_identity::render_session_profile_section(profile_note, personalization);
    let continuity = RuntimeSelfContinuity {
        workspace_guidance,
        runtime_self,
        resolved_identity,
        session_profile_projection,
    };

    (!continuity.is_empty()).then_some(continuity)
}

pub(crate) fn resolve_runtime_self_continuity_for_config(
    config: &LoongConfig,
) -> Option<RuntimeSelfContinuity> {
    resolve_runtime_self_continuity_for_config_with_workspace_root(config, None)
}

pub(crate) fn resolve_runtime_self_continuity_for_config_with_workspace_root(
    config: &LoongConfig,
    workspace_root_override: Option<&Path>,
) -> Option<RuntimeSelfContinuity> {
    let tool_runtime_config = ToolRuntimeConfig::from_loong_config(config, None);
    let configured_workspace_root = tool_runtime_config.effective_workspace_root();
    let workspace_root = workspace_root_override.or(configured_workspace_root);
    let profile_note = config.memory.trimmed_profile_note();
    let personalization = config.memory.trimmed_personalization();
    resolve_runtime_self_continuity(
        workspace_root,
        profile_note.as_deref(),
        personalization.as_ref(),
    )
}

pub(crate) fn runtime_self_continuity_from_event_payload(
    payload: &Value,
) -> Option<RuntimeSelfContinuity> {
    let continuity = payload.get("runtime_self_continuity")?.clone();
    let continuity: RuntimeSelfContinuity = serde_json::from_value(continuity).ok()?;
    Some(continuity.normalize_legacy_workspace_guidance())
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn load_persisted_runtime_self_continuity(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<RuntimeSelfContinuity>, String> {
    load_persisted_runtime_self_continuity_with_delegate_events(repo, session_id, None)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn load_persisted_runtime_self_continuity_with_delegate_events(
    repo: &SessionRepository,
    session_id: &str,
    delegate_events: Option<&[SessionEventRecord]>,
) -> Result<Option<RuntimeSelfContinuity>, String> {
    let latest_event =
        repo.load_latest_event_by_kind(session_id, RUNTIME_SELF_CONTINUITY_EVENT_KIND)?;
    let latest_continuity = latest_event
        .as_ref()
        .and_then(|event| runtime_self_continuity_from_event_payload(&event.payload_json));
    if latest_continuity.is_some() {
        return Ok(latest_continuity);
    }

    let loaded_delegate_events = match delegate_events {
        Some(_) => None,
        None => Some(repo.list_delegate_lifecycle_events(session_id)?),
    };
    let delegate_events = match delegate_events {
        Some(events) => events,
        None => loaded_delegate_events.as_deref().unwrap_or(&[]),
    };
    let delegate_continuity = delegate_events
        .iter()
        .rev()
        .find_map(|event| runtime_self_continuity_from_event_payload(&event.payload_json));
    Ok(delegate_continuity)
}

pub(crate) fn merge_runtime_self_continuity(
    primary: Option<RuntimeSelfContinuity>,
    fallback: Option<&RuntimeSelfContinuity>,
) -> Option<RuntimeSelfContinuity> {
    let primary = primary.map(RuntimeSelfContinuity::normalize_legacy_workspace_guidance);
    let fallback = fallback.map(|value| value.clone().normalize_legacy_workspace_guidance());
    let Some(fallback) = fallback else {
        return primary;
    };

    let Some(mut merged) = primary else {
        return Some(fallback.clone());
    };

    if merged.workspace_guidance.is_empty() {
        merged.workspace_guidance = fallback.workspace_guidance.clone();
    }
    if merged.runtime_self.tool_usage_policy.is_empty() {
        merged.runtime_self.tool_usage_policy = fallback.runtime_self.tool_usage_policy.clone();
    }
    if merged.runtime_self.soul_guidance.is_empty() {
        merged.runtime_self.soul_guidance = fallback.runtime_self.soul_guidance.clone();
    }
    if merged.runtime_self.identity_context.is_empty() {
        merged.runtime_self.identity_context = fallback.runtime_self.identity_context.clone();
    }
    if merged.runtime_self.user_context.is_empty() {
        merged.runtime_self.user_context = fallback.runtime_self.user_context.clone();
    }
    if merged.resolved_identity.is_none() {
        merged.resolved_identity = fallback.resolved_identity.clone();
    }

    let merged_projection = normalize_projection(merged.session_profile_projection.as_deref());
    if merged_projection.is_none() {
        merged.session_profile_projection = fallback.session_profile_projection.clone();
    }

    Some(merged)
}

pub(crate) fn missing_runtime_self_continuity(
    stored: &RuntimeSelfContinuity,
    live: Option<&RuntimeSelfContinuity>,
) -> Option<RuntimeSelfContinuity> {
    let stored = stored.clone().normalize_legacy_workspace_guidance();
    let live = live
        .cloned()
        .map(RuntimeSelfContinuity::normalize_legacy_workspace_guidance);
    let Some(live) = live else {
        return stored.has_prompt_projection().then_some(stored.clone());
    };

    let mut missing = RuntimeSelfContinuity::default();

    if live.workspace_guidance.is_empty() {
        missing.workspace_guidance = stored.workspace_guidance.clone();
    }
    if live.runtime_self.tool_usage_policy.is_empty() {
        missing.runtime_self.tool_usage_policy = stored.runtime_self.tool_usage_policy.clone();
    }
    if live.runtime_self.soul_guidance.is_empty() {
        missing.runtime_self.soul_guidance = stored.runtime_self.soul_guidance.clone();
    }
    if live.runtime_self.identity_context.is_empty() {
        missing.runtime_self.identity_context = stored.runtime_self.identity_context.clone();
    }
    if live.runtime_self.user_context.is_empty() {
        missing.runtime_self.user_context = stored.runtime_self.user_context.clone();
    }
    if live.resolved_identity.is_none() {
        missing.resolved_identity = stored.resolved_identity.clone();
    }

    let live_projection = normalize_projection(live.session_profile_projection.as_deref());
    if live_projection.is_none() {
        missing.session_profile_projection = stored.session_profile_projection.clone();
    }

    missing.has_prompt_projection().then_some(missing)
}

pub(crate) fn render_runtime_self_continuity_section(
    continuity: &RuntimeSelfContinuity,
    inherited: bool,
) -> Option<String> {
    let continuity = continuity.clone().normalize_legacy_workspace_guidance();
    if !continuity.has_prompt_projection() {
        return None;
    }

    let continuity_scope = if inherited {
        "Rehydrate the inherited runtime self state below when a live lane is missing."
    } else {
        "Rehydrate the preserved runtime self state below when a live lane is missing."
    };
    let continuity_note = "Session-local conversation content must not be promoted into durable self state automatically.";
    let workspace_guidance_section =
        workspace_guidance::render_workspace_guidance_section(&continuity.workspace_guidance);
    let runtime_self_section = runtime_self::render_runtime_self_section(&continuity.runtime_self);
    let resolved_identity_section = continuity
        .resolved_identity
        .as_ref()
        .map(runtime_identity::render_runtime_identity_section);

    let mut sections = Vec::new();
    sections.push(RUNTIME_SELF_CONTINUITY_MARKER.to_owned());
    sections.push(continuity_scope.to_owned());
    sections.push(continuity_note.to_owned());

    if let Some(workspace_guidance_section) = workspace_guidance_section {
        sections.push(workspace_guidance_section);
    }
    if let Some(runtime_self_section) = runtime_self_section {
        sections.push(runtime_self_section);
    }
    if let Some(resolved_identity_section) = resolved_identity_section {
        sections.push(resolved_identity_section);
    }
    let session_profile_projection =
        normalize_projection(continuity.session_profile_projection.as_deref());
    if let Some(session_profile_projection) = session_profile_projection {
        sections.push(session_profile_projection);
    }

    Some(sections.join("\n\n"))
}

fn normalize_projection(value: Option<&str>) -> Option<String> {
    let projection = value?;
    let trimmed_projection = projection.trim();
    if trimmed_projection.is_empty() {
        return None;
    }

    Some(trimmed_projection.to_owned())
}

pub(crate) const fn durable_recall_intro() -> &'static str {
    DURABLE_RECALL_INTRO
}

pub(crate) const fn runtime_durable_recall_intro() -> &'static str {
    RUNTIME_DURABLE_RECALL_INTRO
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LoongConfig;
    use crate::test_support::ScopedCurrentDir;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn runtime_self_continuity_from_event_payload_defaults_missing_tool_usage_policy_lane() {
        let payload = json!({
            "runtime_self_continuity": {
                "workspace_guidance": {
                    "entries": ["Keep continuity explicit."]
                },
                "runtime_self": {
                    "soul_guidance": ["Prefer rigorous execution."],
                    "identity_context": ["# Identity\n\n- Name: Stored continuity identity"],
                    "user_context": ["The operator prefers concise technical summaries."]
                },
                "resolved_identity": {
                    "source": "workspace_self",
                    "content": "# Identity\n\n- Name: Stored continuity identity"
                }
            }
        });

        let continuity =
            runtime_self_continuity_from_event_payload(&payload).expect("deserialize continuity");

        assert!(continuity.runtime_self.tool_usage_policy.is_empty());
        assert_eq!(
            continuity.workspace_guidance.entries,
            vec!["Keep continuity explicit.".to_owned()]
        );
    }

    #[test]
    fn runtime_self_continuity_from_event_payload_promotes_legacy_standing_instructions() {
        let payload = json!({
            "runtime_self_continuity": {
                "runtime_self": {
                    "standing_instructions": ["Legacy AGENTS guidance."],
                    "tool_usage_policy": ["Use the smallest audited tool first."]
                }
            }
        });

        let continuity =
            runtime_self_continuity_from_event_payload(&payload).expect("deserialize continuity");

        assert_eq!(
            continuity.workspace_guidance.entries,
            vec!["Legacy AGENTS guidance.".to_owned()]
        );
        assert!(continuity.runtime_self.standing_instructions.is_empty());
    }

    #[test]
    fn missing_runtime_self_continuity_rehydrates_missing_tool_usage_policy_lane() {
        let stored = RuntimeSelfContinuity {
            workspace_guidance: WorkspaceGuidanceModel {
                entries: vec!["Keep continuity explicit.".to_owned()],
            },
            runtime_self: RuntimeSelfModel {
                tool_usage_policy: vec![
                    "Search memory before guessing workspace facts.".to_owned(),
                ],
                ..RuntimeSelfModel::default()
            },
            ..RuntimeSelfContinuity::default()
        };
        let live = RuntimeSelfContinuity::default();

        let missing = missing_runtime_self_continuity(&stored, Some(&live))
            .expect("missing continuity should preserve tool usage policy");

        assert_eq!(
            missing.workspace_guidance.entries,
            vec!["Keep continuity explicit.".to_owned()]
        );
        assert_eq!(
            missing.runtime_self.tool_usage_policy,
            vec!["Search memory before guessing workspace facts.".to_owned()]
        );
    }

    #[test]
    fn merge_runtime_self_continuity_rehydrates_missing_tool_usage_policy_lane() {
        let fallback = RuntimeSelfContinuity {
            workspace_guidance: WorkspaceGuidanceModel {
                entries: vec!["Keep continuity explicit.".to_owned()],
            },
            runtime_self: RuntimeSelfModel {
                tool_usage_policy: vec![
                    "Search memory before guessing workspace facts.".to_owned(),
                ],
                ..RuntimeSelfModel::default()
            },
            ..RuntimeSelfContinuity::default()
        };
        let primary = Some(RuntimeSelfContinuity::default());

        let merged = merge_runtime_self_continuity(primary, Some(&fallback))
            .expect("merged continuity should preserve tool usage policy");

        assert_eq!(
            merged.workspace_guidance.entries,
            vec!["Keep continuity explicit.".to_owned()]
        );
        assert_eq!(
            merged.runtime_self.tool_usage_policy,
            vec!["Search memory before guessing workspace facts.".to_owned()]
        );
    }

    #[test]
    fn resolve_runtime_self_continuity_for_config_prefers_runtime_workspace_root() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path().join("workspace-root");
        let decoy_tool_root = temp_dir.path().join("tool-root");
        let agents_path = workspace_root.join("AGENTS.md");
        let agents_text = "Keep runtime self rooted in the active workspace.";
        let mut config = LoongConfig::default();

        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&decoy_tool_root).expect("create decoy tool root");
        std::fs::write(&agents_path, agents_text).expect("write AGENTS");

        config.tools.file_root = Some(decoy_tool_root.display().to_string());
        config.tools.runtime_workspace_root = Some(workspace_root.display().to_string());

        let continuity = resolve_runtime_self_continuity_for_config(&config)
            .expect("workspace-root continuity should load");

        assert!(
            continuity
                .workspace_guidance
                .entries
                .iter()
                .any(|entry| entry.contains(agents_text))
        );
    }

    #[test]
    fn render_runtime_self_continuity_section_renders_projection_only_continuity() {
        let continuity = RuntimeSelfContinuity {
            session_profile_projection: Some(
                "## Session Profile\nDurable preferences and advisory session context carried into this session:\nOperator prefers concise technical summaries.".to_owned(),
            ),
            ..RuntimeSelfContinuity::default()
        };

        let rendered = render_runtime_self_continuity_section(&continuity, false)
            .expect("projection-only continuity should render");

        assert!(
            rendered.contains("## Session Profile"),
            "expected session profile section, got: {rendered}"
        );
        assert!(
            rendered.contains("Operator prefers concise technical summaries."),
            "expected projected profile text, got: {rendered}"
        );
    }

    #[test]
    fn render_runtime_self_continuity_section_renders_workspace_guidance() {
        let continuity = RuntimeSelfContinuity {
            workspace_guidance: WorkspaceGuidanceModel {
                entries: vec!["Keep AGENTS guidance inherited.".to_owned()],
            },
            ..RuntimeSelfContinuity::default()
        };

        let rendered = render_runtime_self_continuity_section(&continuity, true)
            .expect("workspace guidance continuity should render");

        assert!(rendered.contains("## Workspace Guidance"));
        assert!(rendered.contains("Keep AGENTS guidance inherited."));
    }

    #[test]
    fn resolve_runtime_self_continuity_for_config_does_not_treat_cwd_as_explicit_workspace_root() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let agents_path = workspace_root.join("AGENTS.md");
        let mut config = LoongConfig::default();
        let _guard = ScopedCurrentDir::new(workspace_root);

        std::fs::write(&agents_path, "cwd runtime self should stay advisory-only")
            .expect("write AGENTS");
        config.tools.file_root = None;

        let continuity = resolve_runtime_self_continuity_for_config(&config);

        assert_eq!(
            continuity, None,
            "implicit cwd fallback should not become a live runtime-self workspace root"
        );
    }
}
