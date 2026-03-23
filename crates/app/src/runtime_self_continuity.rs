use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::LoongClawConfig;
use crate::runtime_identity::{self, ResolvedRuntimeIdentity};
use crate::runtime_self::{self, RuntimeSelfModel};
use crate::tools::runtime_config::ToolRuntimeConfig;

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSelfContinuity {
    pub runtime_self: RuntimeSelfModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_identity: Option<ResolvedRuntimeIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_profile_projection: Option<String>,
}

impl RuntimeSelfContinuity {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let profile_projection = normalize_projection(self.session_profile_projection.as_deref());
        self.runtime_self.is_empty()
            && self.resolved_identity.is_none()
            && profile_projection.is_none()
    }

    #[must_use]
    pub fn has_prompt_projection(&self) -> bool {
        !self.runtime_self.is_empty() || self.resolved_identity.is_some()
    }
}

pub(crate) fn resolve_runtime_self_continuity(
    workspace_root: Option<&Path>,
    profile_note: Option<&str>,
) -> Option<RuntimeSelfContinuity> {
    let runtime_self = match workspace_root {
        Some(workspace_root) => runtime_self::load_runtime_self_model(workspace_root),
        None => RuntimeSelfModel::default(),
    };
    let resolved_identity =
        runtime_identity::resolve_runtime_identity(Some(&runtime_self), profile_note);
    let session_profile_projection = runtime_identity::render_session_profile_section(profile_note);
    let continuity = RuntimeSelfContinuity {
        runtime_self,
        resolved_identity,
        session_profile_projection,
    };

    (!continuity.is_empty()).then_some(continuity)
}

pub(crate) fn resolve_runtime_self_continuity_for_config(
    config: &LoongClawConfig,
) -> Option<RuntimeSelfContinuity> {
    let tool_runtime_config = ToolRuntimeConfig::from_loongclaw_config(config, None);
    let workspace_root = tool_runtime_config.file_root.as_deref();
    let profile_note = config.memory.trimmed_profile_note();
    resolve_runtime_self_continuity(workspace_root, profile_note.as_deref())
}

pub(crate) fn runtime_self_continuity_from_event_payload(
    payload: &Value,
) -> Option<RuntimeSelfContinuity> {
    let continuity = payload.get("runtime_self_continuity")?.clone();
    serde_json::from_value(continuity).ok()
}

pub(crate) fn merge_runtime_self_continuity(
    primary: Option<RuntimeSelfContinuity>,
    fallback: Option<&RuntimeSelfContinuity>,
) -> Option<RuntimeSelfContinuity> {
    let Some(fallback) = fallback else {
        return primary;
    };

    let Some(mut merged) = primary else {
        return Some(fallback.clone());
    };

    if merged.runtime_self.standing_instructions.is_empty() {
        merged.runtime_self.standing_instructions =
            fallback.runtime_self.standing_instructions.clone();
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
    let Some(live) = live else {
        return stored.has_prompt_projection().then_some(stored.clone());
    };

    let mut missing = RuntimeSelfContinuity::default();

    if live.runtime_self.standing_instructions.is_empty() {
        missing.runtime_self.standing_instructions =
            stored.runtime_self.standing_instructions.clone();
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
    if !continuity.has_prompt_projection() {
        return None;
    }

    let continuity_scope = if inherited {
        "Rehydrate the inherited runtime self state below when a live lane is missing."
    } else {
        "Rehydrate the preserved runtime self state below when a live lane is missing."
    };
    let continuity_note = "Session-local conversation content must not be promoted into durable self state automatically.";
    let runtime_self_section = runtime_self::render_runtime_self_section(&continuity.runtime_self);
    let resolved_identity_section = continuity
        .resolved_identity
        .as_ref()
        .map(runtime_identity::render_runtime_identity_section);

    let mut sections = Vec::new();
    sections.push(RUNTIME_SELF_CONTINUITY_MARKER.to_owned());
    sections.push(continuity_scope.to_owned());
    sections.push(continuity_note.to_owned());

    if let Some(runtime_self_section) = runtime_self_section {
        sections.push(runtime_self_section);
    }
    if let Some(resolved_identity_section) = resolved_identity_section {
        sections.push(resolved_identity_section);
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
    use serde_json::json;

    #[test]
    fn runtime_self_continuity_from_event_payload_defaults_missing_tool_usage_policy_lane() {
        let payload = json!({
            "runtime_self_continuity": {
                "runtime_self": {
                    "standing_instructions": ["Keep continuity explicit."],
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
            continuity.runtime_self.standing_instructions,
            vec!["Keep continuity explicit.".to_owned()]
        );
    }

    #[test]
    fn missing_runtime_self_continuity_rehydrates_missing_tool_usage_policy_lane() {
        let stored = RuntimeSelfContinuity {
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
            missing.runtime_self.tool_usage_policy,
            vec!["Search memory before guessing workspace facts.".to_owned()]
        );
    }

    #[test]
    fn merge_runtime_self_continuity_rehydrates_missing_tool_usage_policy_lane() {
        let fallback = RuntimeSelfContinuity {
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
            merged.runtime_self.tool_usage_policy,
            vec!["Search memory before guessing workspace facts.".to_owned()]
        );
    }
}
