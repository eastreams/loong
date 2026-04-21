use serde::{Deserialize, Serialize};

use super::context_engine::ContextArtifactKind;
use super::tool_discovery_state::ToolDiscoveryState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptFrameLayer {
    StableRuntimeGuidance,
    SessionLatchedContext,
    AdvisoryProfile,
    SessionLocalRecall,
    RecentWindow,
    TurnEphemeralTail,
}

impl PromptFrameLayer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StableRuntimeGuidance => "stable_runtime_guidance",
            Self::SessionLatchedContext => "session_latched_context",
            Self::AdvisoryProfile => "advisory_profile",
            Self::SessionLocalRecall => "session_local_recall",
            Self::RecentWindow => "recent_window",
            Self::TurnEphemeralTail => "turn_ephemeral_tail",
        }
    }
}

impl std::fmt::Display for PromptFrameLayer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptFrameAuthority {
    CoreSystem,
    WorkspaceGuidance,
    RuntimeSelf,
    RuntimeIdentity,
    CapabilityContract,
    AdvisoryProfile,
    SessionLocalRecall,
    LiveTurn,
}

impl PromptFrameAuthority {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CoreSystem => "core_system",
            Self::WorkspaceGuidance => "workspace_guidance",
            Self::RuntimeSelf => "runtime_self",
            Self::RuntimeIdentity => "runtime_identity",
            Self::CapabilityContract => "capability_contract",
            Self::AdvisoryProfile => "advisory_profile",
            Self::SessionLocalRecall => "session_local_recall",
            Self::LiveTurn => "live_turn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptLane {
    TaskDirective,
    BaseSystem,
    WorkspaceGuidance,
    RuntimeSelf,
    RuntimeIdentity,
    Continuity,
    ExecutionDiscipline,
    CapabilitySnapshot,
    ToolDiscoveryDelta,
}

impl PromptLane {
    pub const fn ordered() -> &'static [PromptLane] {
        &[
            PromptLane::TaskDirective,
            PromptLane::Continuity,
            PromptLane::BaseSystem,
            PromptLane::WorkspaceGuidance,
            PromptLane::RuntimeSelf,
            PromptLane::RuntimeIdentity,
            PromptLane::ExecutionDiscipline,
            PromptLane::CapabilitySnapshot,
            PromptLane::ToolDiscoveryDelta,
        ]
    }

    pub const fn default_frame_layer(self) -> PromptFrameLayer {
        match self {
            PromptLane::TaskDirective => PromptFrameLayer::TurnEphemeralTail,
            PromptLane::BaseSystem => PromptFrameLayer::StableRuntimeGuidance,
            PromptLane::WorkspaceGuidance => PromptFrameLayer::StableRuntimeGuidance,
            PromptLane::RuntimeSelf => PromptFrameLayer::StableRuntimeGuidance,
            PromptLane::RuntimeIdentity => PromptFrameLayer::SessionLatchedContext,
            PromptLane::Continuity => PromptFrameLayer::SessionLatchedContext,
            PromptLane::ExecutionDiscipline => PromptFrameLayer::StableRuntimeGuidance,
            PromptLane::CapabilitySnapshot => PromptFrameLayer::SessionLatchedContext,
            PromptLane::ToolDiscoveryDelta => PromptFrameLayer::SessionLocalRecall,
        }
    }

    pub const fn default_frame_authority(self) -> PromptFrameAuthority {
        match self {
            PromptLane::TaskDirective => PromptFrameAuthority::LiveTurn,
            PromptLane::BaseSystem => PromptFrameAuthority::CoreSystem,
            PromptLane::WorkspaceGuidance => PromptFrameAuthority::WorkspaceGuidance,
            PromptLane::RuntimeSelf => PromptFrameAuthority::RuntimeSelf,
            PromptLane::RuntimeIdentity => PromptFrameAuthority::RuntimeIdentity,
            PromptLane::Continuity => PromptFrameAuthority::RuntimeSelf,
            PromptLane::ExecutionDiscipline => PromptFrameAuthority::CoreSystem,
            PromptLane::CapabilitySnapshot => PromptFrameAuthority::CapabilityContract,
            PromptLane::ToolDiscoveryDelta => PromptFrameAuthority::SessionLocalRecall,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRenderPolicy {
    TrustedLiteral,
    GovernedAdvisory {
        allowed_root_headings: &'static [&'static str],
    },
}

impl PromptRenderPolicy {
    pub const fn for_lane(lane: PromptLane) -> Self {
        match lane {
            PromptLane::ToolDiscoveryDelta => PromptRenderPolicy::GovernedAdvisory {
                allowed_root_headings: &[],
            },
            PromptLane::TaskDirective
            | PromptLane::BaseSystem
            | PromptLane::WorkspaceGuidance
            | PromptLane::RuntimeSelf
            | PromptLane::RuntimeIdentity
            | PromptLane::Continuity
            | PromptLane::ExecutionDiscipline
            | PromptLane::CapabilitySnapshot => PromptRenderPolicy::TrustedLiteral,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFragment {
    pub fragment_id: String,
    pub lane: PromptLane,
    pub source_id: &'static str,
    pub content: String,
    pub render_policy: PromptRenderPolicy,
    pub artifact_kind: ContextArtifactKind,
    pub maskable: bool,
    pub cacheable: bool,
    pub frame_layer: PromptFrameLayer,
    pub frame_authority: PromptFrameAuthority,
    pub dedupe_key: Option<String>,
    pub(crate) tool_discovery_state: Option<ToolDiscoveryState>,
}

impl PromptFragment {
    pub fn new(
        fragment_id: impl Into<String>,
        lane: PromptLane,
        source_id: &'static str,
        content: impl Into<String>,
        artifact_kind: ContextArtifactKind,
    ) -> Self {
        let fragment_id = fragment_id.into();
        let content = content.into();
        let render_policy = PromptRenderPolicy::for_lane(lane);
        let frame_layer = lane.default_frame_layer();
        let frame_authority = lane.default_frame_authority();

        Self {
            fragment_id,
            lane,
            source_id,
            content,
            render_policy,
            artifact_kind,
            maskable: false,
            cacheable: false,
            frame_layer,
            frame_authority,
            dedupe_key: None,
            tool_discovery_state: None,
        }
    }

    #[must_use]
    pub fn with_dedupe_key(mut self, dedupe_key: impl Into<String>) -> Self {
        let dedupe_key = dedupe_key.into();

        self.dedupe_key = Some(dedupe_key);
        self
    }

    #[must_use]
    pub fn with_maskable(mut self, maskable: bool) -> Self {
        self.maskable = maskable;
        self
    }

    #[must_use]
    pub fn with_cacheable(mut self, cacheable: bool) -> Self {
        self.cacheable = cacheable;
        self
    }

    #[must_use]
    pub fn with_render_policy(mut self, render_policy: PromptRenderPolicy) -> Self {
        self.render_policy = render_policy;
        self
    }

    #[must_use]
    pub fn with_frame_layer(mut self, frame_layer: PromptFrameLayer) -> Self {
        self.frame_layer = frame_layer;
        self
    }

    #[must_use]
    pub fn with_frame_authority(mut self, frame_authority: PromptFrameAuthority) -> Self {
        self.frame_authority = frame_authority;
        self
    }

    #[must_use]
    pub(crate) fn with_tool_discovery_state(
        mut self,
        tool_discovery_state: ToolDiscoveryState,
    ) -> Self {
        self.tool_discovery_state = Some(tool_discovery_state);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::PromptLane;

    #[test]
    fn prompt_lane_order_preserves_directive_prefix_and_discovery_suffix() {
        let ordered_lanes = PromptLane::ordered();
        let base_system_index = ordered_lanes
            .iter()
            .position(|lane| *lane == PromptLane::BaseSystem)
            .expect("base system lane");
        let capability_index = ordered_lanes
            .iter()
            .position(|lane| *lane == PromptLane::CapabilitySnapshot)
            .expect("capability snapshot lane");
        let task_directive_index = ordered_lanes
            .iter()
            .position(|lane| *lane == PromptLane::TaskDirective)
            .expect("task directive lane");
        let tool_discovery_index = ordered_lanes
            .iter()
            .position(|lane| *lane == PromptLane::ToolDiscoveryDelta)
            .expect("tool discovery lane");

        assert!(
            task_directive_index < base_system_index,
            "task directive fragments should stay ahead of the base system prompt"
        );
        assert!(
            capability_index < tool_discovery_index,
            "session-latched capability fragments should render before recall-oriented discovery deltas"
        );
    }
}
