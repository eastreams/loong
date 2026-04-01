use super::context_engine::ContextArtifactKind;
use super::tool_discovery_state::ToolDiscoveryState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PromptLane {
    TaskDirective,
    BaseSystem,
    RuntimeSelf,
    RuntimeIdentity,
    Continuity,
    CapabilitySnapshot,
    ToolDiscoveryDelta,
}

impl PromptLane {
    pub const fn ordered() -> &'static [PromptLane] {
        &[
            PromptLane::TaskDirective,
            PromptLane::Continuity,
            PromptLane::BaseSystem,
            PromptLane::RuntimeSelf,
            PromptLane::RuntimeIdentity,
            PromptLane::CapabilitySnapshot,
            PromptLane::ToolDiscoveryDelta,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFragment {
    pub fragment_id: String,
    pub lane: PromptLane,
    pub source_id: &'static str,
    pub content: String,
    pub artifact_kind: ContextArtifactKind,
    pub maskable: bool,
    pub cacheable: bool,
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

        Self {
            fragment_id,
            lane,
            source_id,
            content,
            artifact_kind,
            maskable: false,
            cacheable: false,
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
    pub(crate) fn with_tool_discovery_state(
        mut self,
        tool_discovery_state: ToolDiscoveryState,
    ) -> Self {
        self.tool_discovery_state = Some(tool_discovery_state);
        self
    }
}
