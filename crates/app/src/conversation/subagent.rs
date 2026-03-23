use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::runtime_config::ToolRuntimeNarrowing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentMode {
    Inline,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentTerminalReason {
    Completed,
    Failed,
    TimedOut,
    SpawnFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentExecution {
    pub mode: ConstrainedSubagentMode,
    pub depth: usize,
    pub max_depth: usize,
    pub active_children: usize,
    pub max_active_children: usize,
    pub timeout_seconds: u64,
    pub allow_shell_in_child: bool,
    pub child_tool_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "ToolRuntimeNarrowing::is_empty")]
    pub runtime_narrowing: ToolRuntimeNarrowing,
    pub kernel_bound: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentSpawnEventPayload {
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub execution: ConstrainedSubagentExecution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentTerminalEventPayload {
    pub terminal_reason: ConstrainedSubagentTerminalReason,
    pub execution: ConstrainedSubagentExecution,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ConstrainedSubagentExecution {
    pub fn spawn_payload(&self, task: &str, label: Option<&str>) -> Value {
        self.spawn_payload_with_runtime_self_continuity(task, label, None)
    }

    pub(crate) fn spawn_payload_with_runtime_self_continuity(
        &self,
        task: &str,
        label: Option<&str>,
        runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    ) -> Value {
        json!(ConstrainedSubagentSpawnEventPayload {
            task: task.to_owned(),
            label: label.map(ToOwned::to_owned),
            execution: self.clone(),
            runtime_self_continuity: runtime_self_continuity.cloned(),
        })
    }

    pub fn terminal_payload(
        &self,
        terminal_reason: ConstrainedSubagentTerminalReason,
        duration_ms: u64,
        turn_count: Option<usize>,
        error: Option<&str>,
    ) -> Value {
        json!(ConstrainedSubagentTerminalEventPayload {
            terminal_reason,
            execution: self.clone(),
            duration_ms,
            turn_count,
            error: error.map(ToOwned::to_owned),
        })
    }

    pub fn from_event_payload(payload: &Value) -> Option<Self> {
        let execution = payload.get("execution")?.clone();
        serde_json::from_value(execution).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_identity::{ResolvedRuntimeIdentity, RuntimeIdentitySource};
    use crate::runtime_self::RuntimeSelfModel;
    use crate::runtime_self_continuity::RuntimeSelfContinuity;

    #[test]
    fn constrained_subagent_execution_round_trips_event_payload() {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Async,
            depth: 1,
            max_depth: 2,
            active_children: 0,
            max_active_children: 3,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec![
                "file.read".to_owned(),
                "file.write".to_owned(),
                "file.edit".to_owned(),
            ],
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: true,
        };

        let payload = execution.spawn_payload("research", Some("child"));
        assert_eq!(
            ConstrainedSubagentExecution::from_event_payload(&payload),
            Some(execution)
        );
    }

    #[test]
    fn constrained_subagent_execution_preserves_runtime_self_continuity_in_spawn_payload() {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Inline,
            depth: 1,
            max_depth: 2,
            active_children: 0,
            max_active_children: 2,
            timeout_seconds: 30,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["web.fetch".to_owned()],
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: false,
        };
        let continuity = RuntimeSelfContinuity {
            runtime_self: RuntimeSelfModel {
                standing_instructions: vec!["Keep continuity explicit.".to_owned()],
                tool_usage_policy: vec!["Search memory before guessing workspace facts.".to_owned()],
                soul_guidance: vec!["Prefer rigorous execution.".to_owned()],
                identity_context: vec!["# Identity\n- Name: Child".to_owned()],
                user_context: vec!["User prefers concise output.".to_owned()],
            },
            resolved_identity: Some(ResolvedRuntimeIdentity {
                source: RuntimeIdentitySource::WorkspaceSelf,
                content: "# Identity\n- Name: Child".to_owned(),
            }),
            session_profile_projection: Some(
                "## Session Profile\nDurable preferences and advisory session context carried into this session:\nUser prefers concise output.".to_owned(),
            ),
        };

        let payload = execution.spawn_payload_with_runtime_self_continuity(
            "research",
            Some("child"),
            Some(&continuity),
        );

        assert_eq!(
            payload["runtime_self_continuity"]["resolved_identity"]["content"],
            continuity
                .resolved_identity
                .as_ref()
                .expect("resolved identity")
                .content
        );
        assert_eq!(
            payload["runtime_self_continuity"]["runtime_self"]["tool_usage_policy"][0],
            "Search memory before guessing workspace facts."
        );
    }
}
