use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use loong_contracts::Capability;
use serde_json::Value;

use crate::config::LoongConfig;
use crate::{CliResult, KernelContext};

#[cfg(feature = "memory-sqlite")]
use crate::memory;
use std::collections::BTreeSet;
#[cfg(feature = "memory-sqlite")]
use std::path::Path;

#[cfg(feature = "memory-sqlite")]
use super::compaction::{compact_window, compaction_policy_from_config};
#[cfg(feature = "memory-sqlite")]
use super::load_compaction_session_snapshot;
use super::runtime_binding::ConversationRuntimeBinding;

pub const CONTEXT_ENGINE_API_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextArtifactKind {
    SystemPrompt,
    Profile,
    Summary,
    RetrievedMemory,
    ConversationTurn,
    ToolResult,
    ToolHint,
    RuntimeContract,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolOutputStreamingPolicy {
    BufferFull,
    StreamChunks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextArtifactDescriptor {
    pub message_index: usize,
    pub artifact_kind: ContextArtifactKind,
    pub maskable: bool,
    pub streaming_policy: ToolOutputStreamingPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextEngineCapability {
    KernelMemoryWindowRead,
    LegacyMessageAssembly,
    SessionBootstrap,
    MessageIngestion,
    ContextCompaction,
    SystemPromptAddition,
    SubagentLifecycle,
}

impl ContextEngineCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextEngineCapability::KernelMemoryWindowRead => "kernel_memory_window_read",
            ContextEngineCapability::LegacyMessageAssembly => "legacy_message_assembly",
            ContextEngineCapability::SessionBootstrap => "session_bootstrap",
            ContextEngineCapability::MessageIngestion => "message_ingestion",
            ContextEngineCapability::ContextCompaction => "context_compaction",
            ContextEngineCapability::SystemPromptAddition => "system_prompt_addition",
            ContextEngineCapability::SubagentLifecycle => "subagent_lifecycle",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineMetadata {
    pub id: &'static str,
    pub api_version: u16,
    pub capabilities: BTreeSet<ContextEngineCapability>,
}

impl ContextEngineMetadata {
    pub fn new(
        id: &'static str,
        capabilities: impl IntoIterator<Item = ContextEngineCapability>,
    ) -> Self {
        Self {
            id,
            api_version: CONTEXT_ENGINE_API_VERSION,
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn capability_names(&self) -> Vec<&'static str> {
        self.capabilities
            .iter()
            .copied()
            .map(ContextEngineCapability::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssembledConversationContext {
    pub messages: Vec<Value>,
    pub artifacts: Vec<ContextArtifactDescriptor>,
    pub estimated_tokens: Option<usize>,
    pub prompt_fragments: Vec<crate::conversation::PromptFragment>,
    pub system_prompt_addition: Option<String>,
}

impl AssembledConversationContext {
    pub fn from_messages(messages: Vec<Value>) -> Self {
        Self {
            messages,
            artifacts: Vec::new(),
            estimated_tokens: None,
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        }
    }

    pub fn prompt_frame_summary(&self) -> crate::conversation::PromptFrameSummary {
        crate::conversation::summarize_assembled_prompt_frame(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextEngineBootstrapResult {
    pub bootstrapped: bool,
    pub imported_messages: Option<usize>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextEngineIngestResult {
    pub ingested: bool,
}

#[async_trait]
pub trait ConversationContextEngine: Send + Sync {
    fn id(&self) -> &'static str;

    fn metadata(&self) -> ContextEngineMetadata {
        ContextEngineMetadata::new(self.id(), [])
    }

    async fn bootstrap(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        Ok(ContextEngineBootstrapResult::default())
    }

    async fn ingest(
        &self,
        _session_id: &str,
        _message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        Ok(ContextEngineIngestResult::default())
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
        _config: &LoongConfig,
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

    async fn assemble_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.assemble_messages(config, session_id, include_system_prompt, binding)
            .await
            .map(AssembledConversationContext::from_messages)
    }

    async fn assemble_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>>;
}

#[async_trait]
impl<T> ConversationContextEngine for Box<T>
where
    T: ConversationContextEngine + ?Sized,
{
    fn id(&self) -> &'static str {
        self.as_ref().id()
    }

    fn metadata(&self) -> ContextEngineMetadata {
        self.as_ref().metadata()
    }

    async fn bootstrap(
        &self,
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.as_ref()
            .bootstrap(config, session_id, kernel_ctx)
            .await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        self.as_ref().ingest(session_id, message, kernel_ctx).await
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
        config: &LoongConfig,
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

    async fn assemble_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.as_ref()
            .assemble_context(config, session_id, include_system_prompt, binding)
            .await
    }

    async fn assemble_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        self.as_ref()
            .assemble_messages(config, session_id, include_system_prompt, binding)
            .await
    }
}

#[derive(Default)]
pub struct DefaultContextEngine;

#[derive(Default)]
pub struct LegacyContextEngine;

#[cfg(feature = "memory-sqlite")]
enum PersistMemoryWindowOutcome {
    Persisted,
    Conflict,
}

#[async_trait]
impl ConversationContextEngine for DefaultContextEngine {
    fn id(&self) -> &'static str {
        "default"
    }

    fn metadata(&self) -> ContextEngineMetadata {
        #[cfg(feature = "memory-sqlite")]
        let capabilities = [
            ContextEngineCapability::KernelMemoryWindowRead,
            ContextEngineCapability::ContextCompaction,
        ];
        #[cfg(not(feature = "memory-sqlite"))]
        let capabilities: [ContextEngineCapability; 0] = [];
        ContextEngineMetadata::new("default", capabilities)
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        _messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        #[cfg(feature = "memory-sqlite")]
        {
            const MAX_COMPACTION_CONFLICT_RETRIES: usize = 3;

            for _ in 0..MAX_COMPACTION_CONFLICT_RETRIES {
                let snapshot = load_compaction_session_snapshot(session_id, kernel_ctx).await?;
                let Some(compact_policy) = compaction_policy_from_config(config) else {
                    return Ok(());
                };
                let Some(compacted) = compact_window(&snapshot.turns, compact_policy) else {
                    return Ok(());
                };

                match persist_memory_window(
                    session_id,
                    &compacted,
                    Some(snapshot.turn_count),
                    kernel_ctx,
                )
                .await?
                {
                    PersistMemoryWindowOutcome::Persisted => return Ok(()),
                    PersistMemoryWindowOutcome::Conflict => continue,
                }
            }

            Err("context compaction aborted after repeated concurrent turn updates".to_owned())
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, kernel_ctx);
            Ok(())
        }
    }

    async fn assemble_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        if !binding.is_kernel_bound() {
            let provider_binding = crate::provider::ProviderRuntimeBinding::advisory_only();
            let projected = crate::provider::build_projected_context_for_session_with_binding(
                config,
                session_id,
                include_system_prompt,
                provider_binding,
            )
            .await?;
            return Ok(AssembledConversationContext {
                messages: projected.messages,
                artifacts: projected.artifacts,
                estimated_tokens: None,
                prompt_fragments: projected.prompt_fragments,
                system_prompt_addition: None,
            });
        }

        #[cfg(feature = "memory-sqlite")]
        {
            let kernel_ctx = binding
                .kernel_context()
                .ok_or_else(|| "kernel-bound context engine requires kernel context".to_owned())?;
            let provider_binding = crate::provider::ProviderRuntimeBinding::kernel(kernel_ctx);
            let envelope = load_stage_envelope(config, session_id, binding).await?;
            let runtime_tool_view = crate::tools::runtime_tool_view_from_loong_config(config);
            let projected = crate::provider::project_hydrated_memory_context_for_view_with_binding(
                config,
                include_system_prompt,
                &runtime_tool_view,
                provider_binding,
                &envelope.hydrated,
            )
            .await;
            return Ok(AssembledConversationContext {
                messages: projected.messages,
                artifacts: projected.artifacts,
                estimated_tokens: None,
                prompt_fragments: projected.prompt_fragments,
                system_prompt_addition: None,
            });
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = binding;
            crate::provider::build_messages_for_session(config, session_id, include_system_prompt)
                .map(AssembledConversationContext::from_messages)
        }
    }

    async fn assemble_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        self.assemble_context(config, session_id, include_system_prompt, binding)
            .await
            .map(|assembled| assembled.messages)
    }
}

#[async_trait]
impl ConversationContextEngine for LegacyContextEngine {
    fn id(&self) -> &'static str {
        "legacy"
    }

    fn metadata(&self) -> ContextEngineMetadata {
        ContextEngineMetadata::new("legacy", [ContextEngineCapability::LegacyMessageAssembly])
    }

    async fn assemble_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        crate::provider::build_messages_for_session(config, session_id, include_system_prompt)
    }
}

async fn persist_memory_window(
    session_id: &str,
    turns: &[memory::WindowTurn],
    expected_turn_count: Option<usize>,
    kernel_ctx: &KernelContext,
) -> CliResult<PersistMemoryWindowOutcome> {
    let request = memory::build_replace_turns_request_with_expectation(
        session_id,
        turns,
        expected_turn_count,
    );
    let caps = BTreeSet::from([Capability::MemoryWrite]);
    let outcome = kernel_ctx
        .kernel
        .execute_memory_core(
            kernel_ctx.pack_id(),
            &kernel_ctx.token,
            &caps,
            None,
            request,
        )
        .await
        .map_err(|error| format!("persist compacted memory window via kernel failed: {error}"))?;

    match outcome.status.as_str() {
        "ok" => Ok(PersistMemoryWindowOutcome::Persisted),
        "conflict" => Ok(PersistMemoryWindowOutcome::Conflict),
        _ => Err(format!(
            "persist compacted memory window via kernel returned non-ok status: {}",
            outcome.status
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
async fn load_stage_envelope(
    config: &LoongConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<memory::StageEnvelope> {
    if let Some(ctx) = binding.kernel_context() {
        let tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
        let workspace_root = tool_runtime_config
            .effective_workspace_root()
            .map(Path::to_path_buf);
        let request = memory::build_read_stage_envelope_request_for_memory_config(
            session_id,
            workspace_root.as_deref(),
        );
        let caps = BTreeSet::from([Capability::MemoryRead]);
        let outcome = ctx
            .kernel
            .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
            .await
            .map_err(|error| format!("load staged memory envelope via kernel failed: {error}"))?;

        if outcome.status != "ok" {
            return Err(format!(
                "load staged memory envelope via kernel returned non-ok status: {}",
                outcome.status
            ));
        }

        return memory::decode_stage_envelope(&outcome.payload)
            .ok_or_else(|| "decode staged memory envelope via kernel failed".to_owned());
    }

    memory::hydrate_stage_envelope_for_memory_config(session_id, None, &config.memory)
        .map_err(|error| format!("load staged memory envelope failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MemoryProfile;
    use crate::test_support::TurnTestHarness;

    #[cfg(feature = "memory-sqlite")]
    async fn provider_messages_with_kernel_binding(
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &crate::KernelContext,
    ) -> Vec<Value> {
        let envelope = load_stage_envelope(
            config,
            session_id,
            ConversationRuntimeBinding::kernel(kernel_ctx),
        )
        .await
        .expect("load staged memory envelope");
        let runtime_tool_view = crate::tools::runtime_tool_view_from_loong_config(config);
        crate::provider::project_hydrated_memory_context_for_view_with_binding(
            config,
            true,
            &runtime_tool_view,
            crate::provider::ProviderRuntimeBinding::kernel(kernel_ctx),
            &envelope.hydrated,
        )
        .await
        .messages
    }

    #[test]
    fn default_engine_metadata_has_stable_identity() {
        let metadata = DefaultContextEngine.metadata();
        assert_eq!(metadata.id, "default");
        assert_eq!(metadata.api_version, CONTEXT_ENGINE_API_VERSION);
    }

    #[test]
    fn legacy_engine_metadata_includes_legacy_capability() {
        let metadata = LegacyContextEngine.metadata();
        assert_eq!(metadata.id, "legacy");
        assert!(
            metadata
                .capabilities
                .contains(&ContextEngineCapability::LegacyMessageAssembly),
            "legacy engine should expose legacy assembly capability"
        );
        assert_eq!(metadata.capability_names(), vec!["legacy_message_assembly"]);
    }

    #[test]
    fn capability_names_for_future_hooks_are_stable() {
        assert_eq!(
            ContextEngineCapability::SessionBootstrap.as_str(),
            "session_bootstrap"
        );
        assert_eq!(
            ContextEngineCapability::MessageIngestion.as_str(),
            "message_ingestion"
        );
        assert_eq!(
            ContextEngineCapability::SystemPromptAddition.as_str(),
            "system_prompt_addition"
        );
        assert_eq!(
            ContextEngineCapability::SubagentLifecycle.as_str(),
            "subagent_lifecycle"
        );
    }

    #[test]
    fn assembled_context_from_messages_defaults_to_empty_artifacts() {
        let assembled = AssembledConversationContext::from_messages(vec![Value::Null]);
        assert!(assembled.artifacts.is_empty());
    }

    #[test]
    fn assembled_context_prompt_frame_summary_defaults_to_empty_buckets() {
        let assembled = AssembledConversationContext::from_messages(vec![Value::Null]);
        let frame_summary = assembled.prompt_frame_summary();
        let sliding_window_bucket = frame_summary
            .bucket(crate::conversation::PromptFrameLayer::RecentWindow)
            .expect("sliding window bucket");

        assert_eq!(frame_summary.fragments.len(), 0);
        assert_eq!(sliding_window_bucket.message_count, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_assembles_runtime_self_through_kernel_audit_path() {
        let harness = TurnTestHarness::with_capabilities(std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]));
        let agents_path = harness.temp_dir.join("AGENTS.md");
        let agents_text = "Keep runtime self reads on the audited path.";

        std::fs::write(&agents_path, agents_text).expect("write AGENTS");

        let mut config = LoongConfig::default();
        config.tools.file_root = Some(harness.temp_dir.display().to_string());

        let messages = DefaultContextEngine
            .assemble_messages(
                &config,
                "kernel-runtime-self-session",
                true,
                ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx)),
            )
            .await
            .expect("assemble messages");

        let system_content = messages[0]["content"].as_str().expect("system content");

        assert!(system_content.contains(agents_text));

        let audit_events = harness.audit.snapshot();
        let has_tool_plane_event = audit_events.iter().any(|event| {
            matches!(
                &event.kind,
                loong_kernel::AuditEventKind::PlaneInvoked {
                    plane: loong_contracts::ExecutionPlane::Tool,
                    ..
                }
            )
        });

        assert!(
            has_tool_plane_event,
            "kernel-bound runtime self loading should emit tool-plane audit"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_kernel_bound_messages_match_provider_summary_projection() {
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let session_id = "kernel-summary-session";
        let sqlite_path = harness.temp_dir.join("memory.sqlite3");
        let sqlite_path_text = sqlite_path.display().to_string();
        let mut config = LoongConfig::default();

        config.tools.file_root = Some(harness.temp_dir.display().to_string());
        config.memory.profile = MemoryProfile::WindowPlusSummary;
        config.memory.sliding_window = 2;
        config.memory.sqlite_path = sqlite_path_text.clone();

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);

        crate::session::store::append_session_turn_direct(
            session_id,
            "user",
            "turn 1",
            &memory_config,
        )
        .expect("append turn 1 should succeed");
        crate::session::store::append_session_turn_direct(
            session_id,
            "assistant",
            "turn 2",
            &memory_config,
        )
        .expect("append turn 2 should succeed");
        crate::session::store::append_session_turn_direct(
            session_id,
            "user",
            "turn 3",
            &memory_config,
        )
        .expect("append turn 3 should succeed");
        crate::session::store::append_session_turn_direct(
            session_id,
            "assistant",
            "turn 4",
            &memory_config,
        )
        .expect("append turn 4 should succeed");

        let kernel_ctx = crate::context::bootstrap_kernel_context_with_config(
            "context-engine-summary-projection",
            60,
            &config,
        )
        .expect("bootstrap kernel context with config");
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
        let kernel_messages = DefaultContextEngine
            .assemble_messages(&config, session_id, true, binding)
            .await
            .expect("assemble messages");
        let provider_messages =
            provider_messages_with_kernel_binding(&config, session_id, &kernel_ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "kernel-bound assembly should preserve summary projection parity"
        );
        assert!(
            kernel_messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("## Memory Summary"))
            }),
            "expected kernel-bound assembly to keep the summary block"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_kernel_bound_messages_match_provider_profile_projection() {
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let session_id = "kernel-profile-session";
        let sqlite_path = harness.temp_dir.join("memory.sqlite3");
        let sqlite_path_text = sqlite_path.display().to_string();
        let profile_note = "Imported ZeroClaw preferences";
        let mut config = LoongConfig::default();

        config.tools.file_root = Some(harness.temp_dir.display().to_string());
        config.memory.profile = MemoryProfile::ProfilePlusWindow;
        config.memory.profile_note = Some(profile_note.to_owned());
        config.memory.sliding_window = 2;
        config.memory.sqlite_path = sqlite_path_text.clone();

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);

        crate::session::store::append_session_turn_direct(
            session_id,
            "assistant",
            "turn 1",
            &memory_config,
        )
        .expect("append turn should succeed");

        let kernel_ctx = crate::context::bootstrap_kernel_context_with_config(
            "context-engine-profile-projection",
            60,
            &config,
        )
        .expect("bootstrap kernel context with config");
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
        let kernel_messages = DefaultContextEngine
            .assemble_messages(&config, session_id, true, binding)
            .await
            .expect("assemble messages");
        let provider_messages =
            provider_messages_with_kernel_binding(&config, session_id, &kernel_ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "kernel-bound assembly should preserve profile projection parity"
        );
        assert!(
            kernel_messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains(profile_note))
            }),
            "expected kernel-bound assembly to keep the profile block"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_kernel_bound_messages_match_provider_durable_recall_projection() {
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let session_id = "kernel-durable-recall-session";
        let sqlite_path = harness.temp_dir.join("memory.sqlite3");
        let sqlite_path_text = sqlite_path.display().to_string();
        let curated_memory_path = harness.temp_dir.join("MEMORY.md");

        std::fs::write(
            &curated_memory_path,
            "# Durable Notes\n\nRemember the deploy freeze window.\n",
        )
        .expect("write durable recall");

        let mut config = LoongConfig::default();
        config.tools.file_root = Some(harness.temp_dir.display().to_string());
        config.memory.sqlite_path = sqlite_path_text;

        let kernel_ctx = crate::context::bootstrap_kernel_context_with_config(
            "context-engine-governed-profile-projection",
            60,
            &config,
        )
        .expect("bootstrap kernel context with config");
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
        let kernel_messages = DefaultContextEngine
            .assemble_messages(&config, session_id, true, binding)
            .await
            .expect("assemble messages");
        let provider_messages =
            provider_messages_with_kernel_binding(&config, session_id, &kernel_ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "kernel-bound assembly should preserve durable recall projection parity"
        );
        assert!(
            kernel_messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"].as_str().is_some_and(|content| {
                        content.contains("## Advisory Durable Recall")
                            && content.contains("Remember the deploy freeze window.")
                    })
            }),
            "expected kernel-bound assembly to keep the durable recall block"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_kernel_bound_workspace_recall_system_reorders_retrieved_memory() {
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let session_id = "kernel-workspace-recall-session";
        let sqlite_path = harness.temp_dir.join("memory.sqlite3");
        let sqlite_path_text = sqlite_path.display().to_string();
        let curated_memory_path = harness.temp_dir.join("MEMORY.md");

        std::fs::write(
            &curated_memory_path,
            "# Durable Notes\n\nPromote workspace recall above history.\n",
        )
        .expect("write durable recall");

        let mut config = LoongConfig::default();
        config.tools.file_root = Some(harness.temp_dir.display().to_string());
        config.memory.sqlite_path = sqlite_path_text;
        config.memory.system_id = Some(crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID.to_owned());

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);
        crate::session::store::append_session_turn_direct(
            session_id,
            "user",
            "turn 1",
            &memory_config,
        )
        .expect("append turn 1 should succeed");
        crate::session::store::append_session_turn_direct(
            session_id,
            "assistant",
            "turn 2",
            &memory_config,
        )
        .expect("append turn 2 should succeed");

        let binding =
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx));
        let assembled = DefaultContextEngine
            .assemble_context(&config, session_id, true, binding)
            .await
            .expect("assemble context");

        assert!(
            assembled.messages.len() >= 3,
            "expected system prompt, retrieved memory, and history turns"
        );
        let retrieved_artifact = assembled
            .artifacts
            .iter()
            .find(|artifact| artifact.artifact_kind == ContextArtifactKind::RetrievedMemory)
            .expect("retrieved memory artifact");
        let retrieved_index = retrieved_artifact.message_index;
        let retrieved_message = &assembled.messages[retrieved_index];

        assert_eq!(retrieved_message["role"], "system");
        assert!(
            retrieved_message["content"]
                .as_str()
                .is_some_and(|content| content.contains("Promote workspace recall above history.")),
            "expected a retrieved memory message containing workspace recall content"
        );
        let first_user_index = assembled
            .messages
            .iter()
            .position(|message| message["role"] == "user")
            .expect("first history message index");
        assert!(
            retrieved_index < first_user_index,
            "retrieved memory (index {retrieved_index}) should precede history (index {first_user_index})"
        );
        assert_eq!(assembled.messages[first_user_index]["content"], "turn 1");
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_kernel_bound_messages_match_provider_governed_profile_projection() {
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let session_id = "kernel-governed-profile-session";
        let sqlite_path = harness.temp_dir.join("memory.sqlite3");
        let sqlite_path_text = sqlite_path.display().to_string();
        let profile_note = "# Identity\n\n- Name: Advisory shadow";
        let mut config = LoongConfig::default();

        config.tools.file_root = Some(harness.temp_dir.display().to_string());
        config.memory.profile = MemoryProfile::ProfilePlusWindow;
        config.memory.profile_note = Some(profile_note.to_owned());
        config.memory.sliding_window = 2;
        config.memory.sqlite_path = sqlite_path_text.clone();

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);

        crate::session::store::append_session_turn_direct(
            session_id,
            "assistant",
            "turn 1",
            &memory_config,
        )
        .expect("append turn should succeed");

        let binding =
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx));
        let kernel_messages = DefaultContextEngine
            .assemble_messages(&config, session_id, true, binding)
            .await
            .expect("assemble messages");
        let provider_messages =
            provider_messages_with_kernel_binding(&config, session_id, &harness.kernel_ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "kernel-bound assembly should preserve governed profile projection parity"
        );
    }
}
