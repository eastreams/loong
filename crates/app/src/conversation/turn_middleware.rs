use std::collections::BTreeSet;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::LoongClawConfig;
use crate::tools::ToolView;
use crate::{CliResult, KernelContext};

use super::context_engine::{
    AssembledConversationContext, ContextArtifactDescriptor, ContextArtifactKind,
    ToolOutputStreamingPolicy,
};
use super::prompt_orchestrator::seed_prompt_fragments_from_context;
use super::prompt_orchestrator::sync_prompt_fragments_into_context;
use super::runtime_binding::ConversationRuntimeBinding;
use super::{PromptFragment, PromptLane};

pub const TURN_MIDDLEWARE_API_VERSION: u16 = 1;
pub const SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID: &str = "system-prompt-addition";
pub const SYSTEM_PROMPT_TOOL_VIEW_TURN_MIDDLEWARE_ID: &str = "system-prompt-tool-view";

pub(crate) type BuiltInTurnMiddlewareFactory = fn() -> Box<dyn ConversationTurnMiddleware>;

#[derive(Clone, Copy)]
pub(crate) struct BuiltInTurnMiddlewareSpec {
    pub id: &'static str,
    pub factory: BuiltInTurnMiddlewareFactory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TurnMiddlewareCapability {
    ContextTransform,
    SessionBootstrap,
    MessageIngestion,
    AfterTurn,
    ContextCompaction,
    SubagentLifecycle,
}

impl TurnMiddlewareCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnMiddlewareCapability::ContextTransform => "context_transform",
            TurnMiddlewareCapability::SessionBootstrap => "session_bootstrap",
            TurnMiddlewareCapability::MessageIngestion => "message_ingestion",
            TurnMiddlewareCapability::AfterTurn => "after_turn",
            TurnMiddlewareCapability::ContextCompaction => "context_compaction",
            TurnMiddlewareCapability::SubagentLifecycle => "subagent_lifecycle",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareMetadata {
    pub id: &'static str,
    pub api_version: u16,
    pub capabilities: BTreeSet<TurnMiddlewareCapability>,
}

impl TurnMiddlewareMetadata {
    pub fn new(
        id: &'static str,
        capabilities: impl IntoIterator<Item = TurnMiddlewareCapability>,
    ) -> Self {
        Self {
            id,
            api_version: TURN_MIDDLEWARE_API_VERSION,
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn capability_names(&self) -> Vec<&'static str> {
        self.capabilities
            .iter()
            .copied()
            .map(TurnMiddlewareCapability::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPromptAdditionTurnMiddleware;

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPromptToolViewTurnMiddleware;

fn build_system_prompt_addition_turn_middleware() -> Box<dyn ConversationTurnMiddleware> {
    Box::new(SystemPromptAdditionTurnMiddleware)
}

fn build_system_prompt_tool_view_turn_middleware() -> Box<dyn ConversationTurnMiddleware> {
    Box::new(SystemPromptToolViewTurnMiddleware)
}

pub(crate) const BUILTIN_TURN_MIDDLEWARES: &[BuiltInTurnMiddlewareSpec] = &[
    BuiltInTurnMiddlewareSpec {
        id: SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID,
        factory: build_system_prompt_addition_turn_middleware,
    },
    BuiltInTurnMiddlewareSpec {
        id: SYSTEM_PROMPT_TOOL_VIEW_TURN_MIDDLEWARE_ID,
        factory: build_system_prompt_tool_view_turn_middleware,
    },
];

pub(crate) fn builtin_turn_middlewares() -> Vec<Box<dyn ConversationTurnMiddleware>> {
    BUILTIN_TURN_MIDDLEWARES
        .iter()
        .map(|spec| (spec.factory)())
        .collect()
}

#[async_trait]
pub trait ConversationTurnMiddleware: Send + Sync {
    fn id(&self) -> &'static str;

    fn metadata(&self) -> TurnMiddlewareMetadata {
        TurnMiddlewareMetadata::new(self.id(), [])
    }

    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn ingest(
        &self,
        _session_id: &str,
        _message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn transform_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        assembled: AssembledConversationContext,
        _runtime_tool_view: &ToolView,
        _requested_tool_view: &ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        Ok(assembled)
    }

    async fn after_turn(
        &self,
        _session_id: &str,
        _user_input: &str,
        _assistant_reply: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn compact_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn prepare_subagent_spawn(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[async_trait]
impl ConversationTurnMiddleware for SystemPromptAdditionTurnMiddleware {
    fn id(&self) -> &'static str {
        SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID
    }

    fn metadata(&self) -> TurnMiddlewareMetadata {
        TurnMiddlewareMetadata::new(self.id(), [TurnMiddlewareCapability::ContextTransform])
    }

    async fn transform_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        include_system_prompt: bool,
        mut assembled: AssembledConversationContext,
        _runtime_tool_view: &ToolView,
        _requested_tool_view: &ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        if !include_system_prompt {
            return Ok(assembled);
        }

        let addition = assembled.system_prompt_addition.clone();

        apply_system_prompt_addition(&mut assembled, addition.as_deref());
        Ok(assembled)
    }
}

#[async_trait]
impl ConversationTurnMiddleware for SystemPromptToolViewTurnMiddleware {
    fn id(&self) -> &'static str {
        SYSTEM_PROMPT_TOOL_VIEW_TURN_MIDDLEWARE_ID
    }

    fn metadata(&self) -> TurnMiddlewareMetadata {
        TurnMiddlewareMetadata::new(self.id(), [TurnMiddlewareCapability::ContextTransform])
    }

    async fn transform_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        include_system_prompt: bool,
        mut assembled: AssembledConversationContext,
        runtime_tool_view: &ToolView,
        requested_tool_view: &ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        if include_system_prompt && requested_tool_view != runtime_tool_view {
            apply_tool_view_to_system_prompt(&mut assembled, requested_tool_view);
        }
        Ok(assembled)
    }
}

pub(crate) fn apply_system_prompt_addition(
    assembled: &mut AssembledConversationContext,
    addition: Option<&str>,
) {
    let Some(addition) = addition
        .map(str::trim)
        .filter(|content| !content.is_empty())
    else {
        return;
    };

    seed_prompt_fragments_from_context(assembled);

    if !assembled.prompt_fragments.is_empty() {
        let fragment = PromptFragment::new(
            "system-prompt-addition",
            PromptLane::TaskDirective,
            "system-prompt-addition",
            addition,
            ContextArtifactKind::RuntimeContract,
        )
        .with_dedupe_key("system-prompt-addition")
        .with_cacheable(true);

        assembled.prompt_fragments.insert(0, fragment);
        sync_prompt_fragments_into_context(assembled);
        return;
    }

    for (index, message) in assembled.messages.iter_mut().enumerate() {
        let is_system = message.get("role").and_then(Value::as_str) == Some("system");

        if !is_system {
            continue;
        }

        if let Some(object) = message.as_object_mut() {
            let merged_content = match object.get("content").and_then(Value::as_str) {
                Some(existing) if !existing.trim().is_empty() => {
                    format!("{addition}\n\n{}", existing.trim())
                }
                _ => addition.to_owned(),
            };

            object.insert("content".to_owned(), Value::String(merged_content));
            ensure_runtime_contract_artifact(&mut assembled.artifacts, index);
            return;
        }
    }

    for artifact in &mut assembled.artifacts {
        artifact.message_index += 1;
    }

    assembled.messages.insert(
        0,
        json!({
            "role": "system",
            "content": addition,
        }),
    );

    ensure_runtime_contract_artifact(&mut assembled.artifacts, 0);
}

fn apply_tool_view_to_system_prompt(
    assembled: &mut AssembledConversationContext,
    tool_view: &ToolView,
) {
    seed_prompt_fragments_from_context(assembled);

    let capability_snapshot = crate::tools::capability_snapshot_for_view(tool_view);
    let capability_fragment_index = assembled
        .prompt_fragments
        .iter()
        .position(|fragment| fragment.lane == PromptLane::CapabilitySnapshot);
    let mut updated_prompt_fragments = false;

    if let Some(capability_fragment_index) = capability_fragment_index
        && let Some(capability_fragment) = assembled
            .prompt_fragments
            .get_mut(capability_fragment_index)
    {
        capability_fragment.content = capability_snapshot;
        updated_prompt_fragments = true;
    }

    let mut discovery_fragment_insert_index: Option<usize> = None;
    let mut selected_discovery_fragment: Option<PromptFragment> = None;
    let original_prompt_fragments = std::mem::take(&mut assembled.prompt_fragments);

    for mut fragment in original_prompt_fragments {
        if fragment.lane != PromptLane::ToolDiscoveryDelta {
            assembled.prompt_fragments.push(fragment);
            continue;
        }

        updated_prompt_fragments = true;

        if discovery_fragment_insert_index.is_none() {
            discovery_fragment_insert_index = Some(assembled.prompt_fragments.len());
        }

        let Some(discovery_state) = fragment.tool_discovery_state.clone() else {
            continue;
        };
        let Some(filtered_state) = discovery_state.filtered_for_tool_view(tool_view) else {
            continue;
        };

        fragment.content = filtered_state.render_delta_prompt();
        fragment.tool_discovery_state = Some(filtered_state);
        selected_discovery_fragment = Some(fragment);
    }

    if let Some(selected_discovery_fragment) = selected_discovery_fragment {
        let insert_index =
            discovery_fragment_insert_index.unwrap_or(assembled.prompt_fragments.len());
        assembled
            .prompt_fragments
            .insert(insert_index, selected_discovery_fragment);
    }

    if updated_prompt_fragments {
        sync_prompt_fragments_into_context(assembled);
        return;
    }

    for message in &mut assembled.messages {
        let is_system = message.get("role").and_then(Value::as_str) == Some("system");
        if !is_system {
            continue;
        }

        let Some(content) = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(snapshot_start) = content.find("[available_tools]") else {
            continue;
        };
        let snapshot = crate::tools::capability_snapshot_for_view(tool_view);

        let prefix = content[..snapshot_start].trim_end();
        let rewritten = if prefix.is_empty() {
            snapshot
        } else {
            format!("{prefix}\n\n{snapshot}")
        };

        if let Some(object) = message.as_object_mut() {
            object.insert("content".to_owned(), Value::String(rewritten));
        }
        return;
    }
}

fn ensure_runtime_contract_artifact(
    artifacts: &mut Vec<ContextArtifactDescriptor>,
    message_index: usize,
) {
    if artifacts.iter().any(|artifact| {
        artifact.message_index == message_index
            && artifact.artifact_kind == ContextArtifactKind::RuntimeContract
    }) {
        return;
    }

    artifacts.push(ContextArtifactDescriptor {
        message_index,
        artifact_kind: ContextArtifactKind::RuntimeContract,
        maskable: false,
        streaming_policy: ToolOutputStreamingPolicy::BufferFull,
    });
}

#[async_trait]
impl<T> ConversationTurnMiddleware for Box<T>
where
    T: ConversationTurnMiddleware + ?Sized,
{
    fn id(&self) -> &'static str {
        self.as_ref().id()
    }

    fn metadata(&self) -> TurnMiddlewareMetadata {
        self.as_ref().metadata()
    }

    async fn bootstrap(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .bootstrap(config, session_id, kernel_ctx)
            .await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref().ingest(session_id, message, kernel_ctx).await
    }

    async fn transform_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        assembled: AssembledConversationContext,
        runtime_tool_view: &ToolView,
        requested_tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.as_ref()
            .transform_context(
                config,
                session_id,
                include_system_prompt,
                assembled,
                runtime_tool_view,
                requested_tool_view,
                binding,
            )
            .await
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .after_turn(
                session_id,
                user_input,
                assistant_reply,
                messages,
                kernel_ctx,
            )
            .await
    }

    async fn compact_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn builtin_turn_middlewares_preserve_context_artifact_descriptors() {
        let assembled = AssembledConversationContext {
            messages: vec![
                json!({
                    "role": "system",
                    "content": "base system\n\n[available_tools]\n- delegate: spawn a child session"
                }),
                json!({
                    "role": "system",
                    "content": "## Memory Summary\nOlder context"
                }),
                json!({
                    "role": "user",
                    "content": "latest user turn"
                }),
            ],
            artifacts: vec![
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::SystemPrompt,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::RuntimeContract,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 1,
                    artifact_kind: ContextArtifactKind::Summary,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 2,
                    artifact_kind: ContextArtifactKind::ConversationTurn,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
            ],
            estimated_tokens: None,
            prompt_fragments: vec![
                crate::conversation::PromptFragment::new(
                    "base-system",
                    crate::conversation::PromptLane::BaseSystem,
                    "base-system",
                    "base system",
                    ContextArtifactKind::SystemPrompt,
                ),
                crate::conversation::PromptFragment::new(
                    "capability-snapshot",
                    crate::conversation::PromptLane::CapabilitySnapshot,
                    "capability-snapshot",
                    "[available_tools]\n- delegate: spawn a child session",
                    ContextArtifactKind::RuntimeContract,
                ),
            ],
            system_prompt_addition: Some("runtime-policy-addition".to_owned()),
        };
        let runtime_tool_view = crate::tools::runtime_tool_view();
        let requested_tool_view = crate::tools::ToolView::from_tool_names(["file.read"]);

        let assembled = SystemPromptAdditionTurnMiddleware
            .transform_context(
                &crate::config::LoongClawConfig::default(),
                "session-artifact-preservation",
                true,
                assembled,
                &runtime_tool_view,
                &requested_tool_view,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("system prompt addition middleware should succeed");
        let transformed = SystemPromptToolViewTurnMiddleware
            .transform_context(
                &crate::config::LoongClawConfig::default(),
                "session-artifact-preservation",
                true,
                assembled,
                &runtime_tool_view,
                &requested_tool_view,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("tool view middleware should succeed");

        assert_eq!(transformed.artifacts.len(), 5);
        assert!(
            transformed
                .artifacts
                .iter()
                .any(
                    |artifact| artifact.artifact_kind == ContextArtifactKind::SystemPrompt
                        && artifact.message_index == 0
                )
        );
        assert!(
            transformed
                .artifacts
                .iter()
                .any(
                    |artifact| artifact.artifact_kind == ContextArtifactKind::RuntimeContract
                        && artifact.message_index == 0
                )
        );
        assert!(transformed.artifacts.iter().any(|artifact| {
            artifact.artifact_kind == ContextArtifactKind::Summary
                && transformed.messages[artifact.message_index]["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("## Memory Summary"))
        }));
        assert!(
            transformed
                .artifacts
                .iter()
                .any(
                    |artifact| artifact.artifact_kind == ContextArtifactKind::ConversationTurn
                        && transformed.messages[artifact.message_index]["content"]
                            == "latest user turn"
                )
        );
        assert!(
            transformed
                .artifacts
                .iter()
                .all(|artifact| artifact.message_index < transformed.messages.len())
        );
        assert!(
            transformed.prompt_fragments.iter().any(|fragment| {
                fragment.lane == crate::conversation::PromptLane::TaskDirective
                    && fragment.content == "runtime-policy-addition"
            }),
            "system prompt addition should become a task directive fragment: {:?}",
            transformed.prompt_fragments
        );

        let system_content = transformed.messages[0]["content"]
            .as_str()
            .expect("system content");
        assert!(system_content.contains("runtime-policy-addition"));
        assert!(system_content.contains("Non-core tools are intentionally hidden"));
    }

    #[tokio::test]
    async fn tool_view_middleware_filters_tool_discovery_fragment_to_requested_view() {
        let discovery_state = super::super::tool_discovery_state::ToolDiscoveryState {
            schema_version: 1,
            query: Some("read note.md".to_owned()),
            exact_tool_id: Some("file.read".to_owned()),
            entries: vec![super::super::tool_discovery_state::ToolDiscoveryEntry {
                tool_id: "file.read".to_owned(),
                summary: "Read a file.".to_owned(),
                search_hint: None,
                argument_hint: None,
                capability_family: None,
                usage_guidance: None,
                required_fields: vec!["path".to_owned()],
                required_field_groups: vec![vec!["path".to_owned()]],
            }],
            diagnostics: None,
        };
        let discovery_content = discovery_state.render_delta_prompt();
        let assembled = AssembledConversationContext {
            messages: vec![json!({
                "role": "system",
                "content": "placeholder system prompt"
            })],
            artifacts: vec![
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::SystemPrompt,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::RuntimeContract,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::ToolHint,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
            ],
            estimated_tokens: None,
            prompt_fragments: vec![
                crate::conversation::PromptFragment::new(
                    "base-system",
                    crate::conversation::PromptLane::BaseSystem,
                    "base-system",
                    "base system",
                    ContextArtifactKind::SystemPrompt,
                ),
                crate::conversation::PromptFragment::new(
                    "capability-snapshot",
                    crate::conversation::PromptLane::CapabilitySnapshot,
                    "capability-snapshot",
                    "[available_tools]\n- file.read: read a file",
                    ContextArtifactKind::RuntimeContract,
                ),
                crate::conversation::PromptFragment::new(
                    "tool-discovery-delta",
                    crate::conversation::PromptLane::ToolDiscoveryDelta,
                    "tool-discovery-delta",
                    discovery_content,
                    ContextArtifactKind::ToolHint,
                )
                .with_dedupe_key("tool-discovery-delta")
                .with_render_policy(crate::conversation::PromptRenderPolicy::GovernedAdvisory {
                    allowed_root_headings: &[],
                })
                .with_tool_discovery_state(discovery_state),
            ],
            system_prompt_addition: None,
        };
        let runtime_tool_view = crate::tools::runtime_tool_view();
        let requested_tool_view =
            crate::tools::ToolView::from_tool_names(["tool.search", "tool.invoke"]);

        let transformed = SystemPromptToolViewTurnMiddleware
            .transform_context(
                &crate::config::LoongClawConfig::default(),
                "session-tool-discovery-filter",
                true,
                assembled,
                &runtime_tool_view,
                &requested_tool_view,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("tool view middleware should succeed");

        let system_content = transformed.messages[0]["content"]
            .as_str()
            .expect("system content");
        let discovery_fragment = transformed
            .prompt_fragments
            .iter()
            .find(|fragment| fragment.lane == crate::conversation::PromptLane::ToolDiscoveryDelta)
            .expect("tool discovery fragment");

        assert!(system_content.contains("[tool_discovery_delta]"));
        assert!(system_content.contains("no currently visible tools"));
        assert!(!system_content.contains("file.read"));
        assert!(
            discovery_fragment
                .content
                .contains("no currently visible tools")
        );
        assert!(!discovery_fragment.content.contains("file.read"));
        assert_eq!(
            discovery_fragment
                .tool_discovery_state
                .as_ref()
                .and_then(|state| state.exact_tool_id.as_deref()),
            None
        );
    }

    #[tokio::test]
    async fn system_prompt_addition_middleware_skips_addition_when_system_prompt_is_disabled() {
        let assembled = AssembledConversationContext {
            messages: Vec::new(),
            artifacts: Vec::new(),
            estimated_tokens: None,
            prompt_fragments: Vec::new(),
            system_prompt_addition: Some("runtime-policy-addition".to_owned()),
        };
        let runtime_tool_view = crate::tools::runtime_tool_view();

        let transformed = SystemPromptAdditionTurnMiddleware
            .transform_context(
                &crate::config::LoongClawConfig::default(),
                "session-no-system-prompt",
                false,
                assembled,
                &runtime_tool_view,
                &runtime_tool_view,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("system prompt addition middleware should succeed");

        assert!(transformed.messages.is_empty());
        assert!(transformed.artifacts.is_empty());
        assert!(transformed.prompt_fragments.is_empty());
    }

    #[tokio::test]
    async fn tool_view_middleware_removes_all_duplicate_tool_discovery_fragments() {
        let discovery_state = super::super::tool_discovery_state::ToolDiscoveryState {
            schema_version: 1,
            query: None,
            exact_tool_id: Some("file.read".to_owned()),
            entries: vec![super::super::tool_discovery_state::ToolDiscoveryEntry {
                tool_id: "file.read".to_owned(),
                summary: "Read a file.".to_owned(),
                search_hint: None,
                argument_hint: None,
                capability_family: None,
                usage_guidance: None,
                required_fields: vec!["path".to_owned()],
                required_field_groups: vec![vec!["path".to_owned()]],
            }],
            diagnostics: None,
        };
        let discovery_content = discovery_state.render_delta_prompt();
        let duplicate_fragment = || {
            crate::conversation::PromptFragment::new(
                "tool-discovery-delta",
                crate::conversation::PromptLane::ToolDiscoveryDelta,
                "tool-discovery-delta",
                discovery_content.clone(),
                ContextArtifactKind::ToolHint,
            )
            .with_dedupe_key("tool-discovery-delta")
            .with_render_policy(crate::conversation::PromptRenderPolicy::GovernedAdvisory {
                allowed_root_headings: &[],
            })
            .with_tool_discovery_state(discovery_state.clone())
        };
        let assembled = AssembledConversationContext {
            messages: vec![json!({
                "role": "system",
                "content": "placeholder system prompt"
            })],
            artifacts: vec![
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::SystemPrompt,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::RuntimeContract,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::ToolHint,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
            ],
            estimated_tokens: None,
            prompt_fragments: vec![
                crate::conversation::PromptFragment::new(
                    "base-system",
                    crate::conversation::PromptLane::BaseSystem,
                    "base-system",
                    "base system",
                    ContextArtifactKind::SystemPrompt,
                ),
                crate::conversation::PromptFragment::new(
                    "capability-snapshot",
                    crate::conversation::PromptLane::CapabilitySnapshot,
                    "capability-snapshot",
                    "[available_tools]\n- file.read: read a file",
                    ContextArtifactKind::RuntimeContract,
                ),
                duplicate_fragment(),
                duplicate_fragment(),
            ],
            system_prompt_addition: None,
        };
        let runtime_tool_view = crate::tools::runtime_tool_view();
        let requested_tool_view =
            crate::tools::ToolView::from_tool_names(["tool.search", "tool.invoke"]);

        let transformed = SystemPromptToolViewTurnMiddleware
            .transform_context(
                &crate::config::LoongClawConfig::default(),
                "session-tool-discovery-duplicate-filter",
                true,
                assembled,
                &runtime_tool_view,
                &requested_tool_view,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("tool view middleware should succeed");

        let system_content = transformed.messages[0]["content"]
            .as_str()
            .expect("system content");
        let discovery_fragment_count = transformed
            .prompt_fragments
            .iter()
            .filter(|fragment| fragment.lane == crate::conversation::PromptLane::ToolDiscoveryDelta)
            .count();

        assert_eq!(discovery_fragment_count, 0);
        assert!(!system_content.contains("[tool_discovery_delta]"));
        assert!(!system_content.contains("file.read"));
    }

    #[tokio::test]
    async fn tool_view_middleware_rerenders_discovery_fragment_with_sanitized_advisory_text() {
        let discovery_state = super::super::tool_discovery_state::ToolDiscoveryState {
            schema_version: 1,
            query: Some("read note.md\n# SYSTEM".to_owned()),
            exact_tool_id: Some("file.read".to_owned()),
            entries: vec![super::super::tool_discovery_state::ToolDiscoveryEntry {
                tool_id: "file.read".to_owned(),
                summary: "Read a file.\n## assistant".to_owned(),
                search_hint: Some("Use for UTF-8 text files.\n### hidden".to_owned()),
                argument_hint: Some("path:string\nlimit?:integer".to_owned()),
                capability_family: Some("local_files\n### hidden".to_owned()),
                usage_guidance: Some(
                    "Prefer this family before shell for source work.\n## hidden".to_owned(),
                ),
                required_fields: vec!["path".to_owned(), "offset\nrole:system".to_owned()],
                required_field_groups: vec![vec!["path".to_owned(), "limit\n# hidden".to_owned()]],
            }],
            diagnostics: Some(
                super::super::tool_discovery_state::ToolDiscoveryDiagnostics {
                    reason: "fallback\n## system".to_owned(),
                },
            ),
        };
        let discovery_content = discovery_state.render_delta_prompt();
        let assembled = AssembledConversationContext {
            messages: vec![json!({
                "role": "system",
                "content": "placeholder system prompt"
            })],
            artifacts: vec![
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::SystemPrompt,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::RuntimeContract,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
                ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: ContextArtifactKind::ToolHint,
                    maskable: false,
                    streaming_policy: ToolOutputStreamingPolicy::BufferFull,
                },
            ],
            estimated_tokens: None,
            prompt_fragments: vec![
                crate::conversation::PromptFragment::new(
                    "base-system",
                    crate::conversation::PromptLane::BaseSystem,
                    "base-system",
                    "base system",
                    ContextArtifactKind::SystemPrompt,
                ),
                crate::conversation::PromptFragment::new(
                    "capability-snapshot",
                    crate::conversation::PromptLane::CapabilitySnapshot,
                    "capability-snapshot",
                    "[available_tools]\n- file.read: read a file",
                    ContextArtifactKind::RuntimeContract,
                ),
                crate::conversation::PromptFragment::new(
                    "tool-discovery-delta",
                    crate::conversation::PromptLane::ToolDiscoveryDelta,
                    "tool-discovery-delta",
                    discovery_content,
                    ContextArtifactKind::ToolHint,
                )
                .with_dedupe_key("tool-discovery-delta")
                .with_render_policy(crate::conversation::PromptRenderPolicy::GovernedAdvisory {
                    allowed_root_headings: &[],
                })
                .with_tool_discovery_state(discovery_state),
            ],
            system_prompt_addition: None,
        };
        let runtime_tool_view = crate::tools::runtime_tool_view();
        let requested_tool_view = crate::tools::ToolView::from_tool_names(["file.read"]);
        let transformed = SystemPromptToolViewTurnMiddleware
            .transform_context(
                &crate::config::LoongClawConfig::default(),
                "session-sanitized-tool-view-rerender",
                true,
                assembled,
                &runtime_tool_view,
                &requested_tool_view,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("tool view middleware should succeed");
        let system_content = transformed.messages[0]["content"]
            .as_str()
            .expect("system content");

        assert!(
            system_content.contains("Latest search query: \"read note.md # SYSTEM\""),
            "query should remain flattened after middleware re-render: {system_content}"
        );
        assert!(
            system_content.contains("Latest discovery diagnostics: \"fallback ## system\""),
            "diagnostics should remain flattened after middleware re-render: {system_content}"
        );
        assert!(
            system_content.contains("search_hint: \"Use for UTF-8 text files. ### hidden\""),
            "search hint should remain flattened after middleware re-render: {system_content}"
        );
        assert!(
            system_content.contains("required_fields: \"path\", \"offset role:system\""),
            "required fields should remain flattened after middleware re-render: {system_content}"
        );
        assert!(
            !system_content.contains("\n# SYSTEM"),
            "middleware re-render must not reintroduce raw headings: {system_content}"
        );
        assert!(
            !system_content.contains("\n## assistant"),
            "middleware re-render must not reintroduce raw summary headings: {system_content}"
        );
    }
}
